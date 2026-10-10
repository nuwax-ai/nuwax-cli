//! Release-bound package caching. Complete receipts follow the package's final
//! name; in-progress bytes never appear under a deployable archive name.

use crate::atomic_file::{PermissionsPolicy, write_atomic};
use crate::downloader::{DownloadMetadata, DownloadProgress, FileDownloader};
use crate::utils::archive::{self, ArchiveFormat};
use anyhow::{Context, Result, bail};
use reqwest::Client;
use serde::{Deserialize, Serialize};
use std::io::ErrorKind;
use std::path::{Path, PathBuf};
use tracing::{info, warn};

const RECORD_NAME: &str = "package-cache.json";
const PART_NAME: &str = ".package-download.part";

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PackageIdentity {
    pub version: String,
    pub architecture: String,
    pub download_type: String,
    pub url: String,
    pub expected_sha256: Option<String>,
}

impl PackageIdentity {
    pub fn new(
        version: &str,
        architecture: &str,
        download_type: &str,
        url: &str,
        expected_hash: Option<&str>,
    ) -> Result<Self> {
        if version.is_empty() || !matches!(architecture, "x86_64" | "aarch64") {
            bail!("Invalid package version or architecture");
        }
        if !matches!(download_type, "full" | "patch") {
            bail!("Invalid package download type");
        }
        let parsed = url::Url::parse(url).context("Invalid package download URL")?;
        if !matches!(parsed.scheme(), "http" | "https") {
            bail!("Package downloads require HTTP or HTTPS");
        }
        let expected_sha256 = normalize_sha256(expected_hash)?;
        Ok(Self {
            version: version.to_owned(),
            architecture: architecture.to_owned(),
            download_type: download_type.to_owned(),
            url: url.to_owned(),
            expected_sha256,
        })
    }
}

/// The legacy manifest's `external` sentinel means no remote SHA is provided.
pub fn normalize_sha256(value: Option<&str>) -> Result<Option<String>> {
    match value.map(str::trim) {
        None | Some("") | Some("external") => Ok(None),
        Some(value) => {
            let value = value.strip_prefix("sha256:").unwrap_or(value);
            if value.len() != 64 || !value.bytes().all(|byte| byte.is_ascii_hexdigit()) {
                bail!("Invalid published package SHA-256");
            }
            Ok(Some(value.to_ascii_lowercase()))
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
struct RemoteResource {
    size: Option<u64>,
    etag: Option<String>,
    last_modified: Option<String>,
}

impl RemoteResource {
    async fn probe(client: &Client, url: &str) -> Result<Self> {
        let response = client
            .head(url)
            .send()
            .await
            .context("Failed to inspect package download resource")?
            .error_for_status()
            .context("Package download resource returned an error")?;
        let header = |name| {
            response
                .headers()
                .get(name)
                .and_then(|value| value.to_str().ok())
                .map(str::to_owned)
        };
        Ok(Self {
            size: header(reqwest::header::CONTENT_LENGTH).and_then(|value| value.parse().ok()),
            etag: header(reqwest::header::ETAG).filter(|value| !value.starts_with("W/")),
            last_modified: header(reqwest::header::LAST_MODIFIED),
        })
    }

    fn has_validator(&self) -> bool {
        self.etag.is_some() || self.last_modified.is_some()
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "state", rename_all = "snake_case")]
enum CacheState {
    Partial,
    Complete {
        filename: String,
        size: u64,
        sha256: String,
    },
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct CacheRecord {
    schema_version: u32,
    identity: PackageIdentity,
    remote: RemoteResource,
    #[serde(flatten)]
    state: CacheState,
}

fn save_record(path: &Path, record: &CacheRecord) -> Result<()> {
    let bytes = serde_json::to_vec_pretty(record)?;
    write_atomic(path, &bytes, PermissionsPolicy::Private)
        .context("Failed to save package cache identity atomically")
}

async fn read_record(path: &Path) -> Result<Option<CacheRecord>> {
    match tokio::fs::read(path).await {
        Ok(bytes) => match serde_json::from_slice(&bytes) {
            Ok(record) => Ok(Some(record)),
            Err(_) => {
                warn!("Package cache receipt is corrupt; the cache will be revalidated");
                Ok(None)
            }
        },
        Err(error) if error.kind() == ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error).context("Failed to read package cache receipt"),
    }
}

fn reject_nonregular(path: &Path) -> Result<()> {
    match std::fs::symlink_metadata(path) {
        Ok(metadata) if !metadata.is_file() => {
            bail!(
                "Package cache path must be a regular file: {}",
                path.display()
            );
        }
        Ok(_) => Ok(()),
        Err(error) if error.kind() == ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error).context("Failed to inspect package cache file"),
    }
}

async fn remove_partial(part: &Path) -> Result<()> {
    for path in [part.to_owned(), part.with_extension("download")] {
        reject_nonregular(&path)?;
        match tokio::fs::remove_file(&path).await {
            Ok(()) => {}
            Err(error) if error.kind() == ErrorKind::NotFound => {}
            Err(error) => return Err(error).context("Failed to discard stale partial download"),
        }
    }
    Ok(())
}

async fn valid_complete(path: &Path, size: u64, sha256: &str) -> Result<bool> {
    reject_nonregular(path)?;
    match tokio::fs::metadata(path).await {
        Ok(metadata) if metadata.len() == size => {
            Ok(FileDownloader::calculate_file_hash(path).await? == sha256)
        }
        Ok(_) => Ok(false),
        Err(error) if error.kind() == ErrorKind::NotFound => Ok(false),
        Err(error) => Err(error).context("Failed to inspect completed package"),
    }
}

fn promote_package(part: &Path, final_path: &Path) -> Result<()> {
    reject_nonregular(final_path)?;
    let mut temporary = tempfile::TempPath::try_from_path(part)?;
    // Retain fully downloaded bytes if rename/replace fails (for example disk or
    // antivirus errors). The complete receipt permits an ordinary retry.
    temporary.disable_cleanup(true);
    temporary
        .persist(final_path)
        .map_err(|error| error.error)
        .context("Failed to promote verified package to its final cache name")
}

fn sync_parent(_directory: &Path) -> Result<()> {
    #[cfg(unix)]
    std::fs::File::open(_directory)?.sync_all()?;
    Ok(())
}

/// Resolve a previously downloaded candidate without network traffic. When a
/// receipt exists it must describe this exact release; invalid or partial state
/// cannot fall back to another archive lying in the directory. Content integrity
/// is checked by download_package, and deployment still runs package preflight.
pub fn resolve_cached_package(directory: &Path, identity: &PackageIdentity) -> Result<PathBuf> {
    let record_path = directory.join(RECORD_NAME);
    reject_nonregular(&record_path)?;
    let bytes = match std::fs::read(&record_path) {
        Ok(bytes) => Some(bytes),
        Err(error) if error.kind() == ErrorKind::NotFound => None,
        Err(error) => return Err(error).context("Failed to read candidate package receipt"),
    };
    let filename = if let Some(bytes) = bytes {
        let record: CacheRecord = serde_json::from_slice(&bytes)
            .context("Candidate package cache receipt is corrupt; download the release again")?;
        if record.schema_version != 1 || record.identity != *identity {
            bail!("Candidate package cache identity does not match the selected release");
        }
        let CacheState::Complete {
            filename,
            size,
            sha256,
        } = record.state
        else {
            bail!("Candidate package cache is incomplete; finish downloading the release");
        };
        if ![ArchiveFormat::Zip, ArchiveFormat::TarGz]
            .iter()
            .any(|format| {
                filename == archive::generate_docker_filename(&identity.architecture, *format)
            })
            || normalize_sha256(Some(&sha256))?.as_deref() != Some(sha256.as_str())
            || identity
                .expected_sha256
                .as_ref()
                .is_some_and(|expected| expected != &sha256)
        {
            bail!("Candidate package receipt contains an invalid archive identity");
        }
        let path = directory.join(&filename);
        reject_nonregular(&path)?;
        if std::fs::metadata(&path)
            .with_context(|| format!("Candidate package is missing: {}", path.display()))?
            .len()
            != size
        {
            bail!("Candidate package size does not match its completed cache receipt");
        }
        filename
    } else {
        // Old CLI caches have no release receipt. Keep this compatibility path
        // deterministic: use the selected URL's format, never read_dir order.
        let url = url::Url::parse(&identity.url).context("Invalid selected package URL")?;
        let path = url.path().to_ascii_lowercase();
        let format = if path.ends_with(".zip") {
            Some(ArchiveFormat::Zip)
        } else if path.ends_with(".tar.gz") || path.ends_with(".tgz") {
            Some(ArchiveFormat::TarGz)
        } else {
            None
        };
        if let Some(format) = format {
            archive::generate_docker_filename(&identity.architecture, format)
        } else {
            let candidates: Vec<_> = [ArchiveFormat::Zip, ArchiveFormat::TarGz]
                .iter()
                .map(|format| archive::generate_docker_filename(&identity.architecture, *format))
                .filter(|filename| directory.join(filename).is_file())
                .collect();
            if candidates.len() != 1 {
                bail!(
                    "Legacy package cache has no unambiguous archive for the selected architecture"
                );
            }
            candidates
                .into_iter()
                .next()
                .ok_or_else(|| anyhow::anyhow!("Legacy candidate package is missing"))?
        }
    };
    let path = directory.join(filename);
    reject_nonregular(&path)?;
    if !path.is_file() {
        bail!("Candidate package is missing: {}", path.display());
    }
    Ok(path)
}

/// Reuses only a complete archive bound to this release and remote resource.
/// A receipt is written before the final rename, so either side of an interrupted
/// promotion can be recognized on retry. Archive validation still runs upstream.
pub async fn download_package<F>(
    client: &Client,
    downloader: &FileDownloader,
    directory: &Path,
    identity: &PackageIdentity,
    progress_callback: Option<F>,
) -> Result<PathBuf>
where
    F: Fn(DownloadProgress) + Send + Sync + 'static,
{
    tokio::fs::create_dir_all(directory).await?;
    let record_path = directory.join(RECORD_NAME);
    let part = directory.join(PART_NAME);
    reject_nonregular(&record_path)?;
    reject_nonregular(&part)?;
    reject_nonregular(&part.with_extension("download"))?;
    let remote = RemoteResource::probe(client, &identity.url).await?;
    let record = read_record(&record_path).await?;
    let same_resource = record.as_ref().is_some_and(|record| {
        record.schema_version == 1 && record.identity == *identity && record.remote == remote
    });
    if same_resource
        && let Some(CacheRecord {
            state:
                CacheState::Complete {
                    filename,
                    size,
                    sha256,
                },
            ..
        }) = record.as_ref()
    {
        let valid_name = [ArchiveFormat::Zip, ArchiveFormat::TarGz]
            .iter()
            .any(|format| {
                *filename == archive::generate_docker_filename(&identity.architecture, *format)
            });
        let remote_hash_matches = identity
            .expected_sha256
            .as_ref()
            .is_none_or(|expected| expected == sha256);
        // Without a published SHA, require a server validator to detect URL reuse.
        if valid_name
            && remote_hash_matches
            && (identity.expected_sha256.is_some() || remote.has_validator())
        {
            let final_path = directory.join(filename);
            if valid_complete(&final_path, *size, sha256).await? {
                info!(
                    "Using verified completed package cache: {}",
                    final_path.display()
                );
                remove_partial(&part).await?;
                return Ok(final_path);
            }
            if valid_complete(&part, *size, sha256).await? {
                promote_package(&part, &final_path)?;
                sync_parent(directory)?;
                info!(
                    "Recovered completed package promotion: {}",
                    final_path.display()
                );
                remove_partial(&part).await?;
                return Ok(final_path);
            }
        }
    }

    let reusable_partial = same_resource
        && (identity.expected_sha256.is_some() || remote.has_validator())
        && matches!(
            record.as_ref().map(|record| &record.state),
            Some(CacheState::Partial)
        );
    if !reusable_partial {
        remove_partial(&part).await?;
    }
    let complete_partial = if reusable_partial && part.exists() {
        let size = tokio::fs::metadata(&part).await?.len();
        if remote.size.is_some_and(|expected| expected != size) {
            false
        } else if let Some(expected) = identity.expected_sha256.as_deref() {
            FileDownloader::calculate_file_hash(&part).await? == expected
        } else {
            let checkpoint = match tokio::fs::read(part.with_extension("download")).await {
                Ok(bytes) => serde_json::from_slice::<DownloadMetadata>(&bytes).ok(),
                Err(error) if error.kind() == ErrorKind::NotFound => None,
                Err(error) => {
                    return Err(error).context("Failed to read package completion checkpoint");
                }
            };
            let valid_checkpoint = checkpoint.as_ref().is_some_and(|checkpoint| {
                checkpoint.completed
                    && checkpoint.is_same_task(
                        &identity.url,
                        remote.size.unwrap_or(0),
                        &identity.version,
                    )
                    && checkpoint.expected_hash == identity.expected_sha256
                    && checkpoint.validator.as_deref()
                        == remote.etag.as_deref().or(remote.last_modified.as_deref())
                    && checkpoint.downloaded_bytes == size
                    && checkpoint.partial_sha256.is_some()
            });
            if valid_checkpoint && let Some(checkpoint) = checkpoint {
                Some(FileDownloader::calculate_file_hash(&part).await?) == checkpoint.partial_sha256
            } else {
                false
            }
        }
    } else {
        false
    };
    save_record(
        &record_path,
        &CacheRecord {
            schema_version: 1,
            identity: identity.clone(),
            remote: remote.clone(),
            state: CacheState::Partial,
        },
    )?;
    if !complete_partial {
        downloader
            .download_file_with_options(
                &identity.url,
                &part,
                progress_callback,
                identity.expected_sha256.as_deref(),
                Some(&identity.version),
            )
            .await
            .context("Failed to download package into resumable cache")?;
    }
    if RemoteResource::probe(client, &identity.url).await? != remote {
        remove_partial(&part).await?;
        bail!("Remote package changed during download; retry with the current release");
    }
    let size = tokio::fs::metadata(&part).await?.len();
    if size == 0 || remote.size.is_some_and(|expected| expected != size) {
        remove_partial(&part).await?;
        bail!("Downloaded package size does not match the remote resource");
    }
    let sha256 = FileDownloader::calculate_file_hash(&part).await?;
    if identity
        .expected_sha256
        .as_ref()
        .is_some_and(|expected| expected != &sha256)
    {
        remove_partial(&part).await?;
        bail!("Downloaded package failed published SHA-256 verification");
    }
    let format = archive::detect_format_by_magic(&part)
        .context("Downloaded package is not a supported archive")?;
    let filename = archive::generate_docker_filename(&identity.architecture, format);
    let final_path = directory.join(&filename);
    reject_nonregular(&final_path)?;
    save_record(
        &record_path,
        &CacheRecord {
            schema_version: 1,
            identity: identity.clone(),
            remote,
            state: CacheState::Complete {
                filename,
                size,
                sha256,
            },
        },
    )?;
    promote_package(&part, &final_path)?;
    sync_parent(directory)?;
    remove_partial(&part).await?;
    info!("Verified package cached: {}", final_path.display());
    Ok(final_path)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::downloader::DownloaderConfig;
    use sha2::{Digest, Sha256};
    use std::io::Write;
    use std::sync::{Arc, Mutex};
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpListener;

    #[derive(Default, Clone)]
    struct Traffic {
        gets: usize,
        bytes: usize,
        ranges: Vec<String>,
        if_ranges: Vec<String>,
    }

    struct Resource {
        body: Vec<u8>,
        etag: Option<String>,
        last_modified: Option<String>,
        get_etag: Option<Option<String>>,
        get_etag_after: usize,
        get_last_modified: Option<Option<String>>,
        omit_head_length: bool,
        range_end: Option<usize>,
        ignore_range: bool,
        interrupt_next: bool,
        invalid_range: bool,
        traffic: Traffic,
    }

    struct TestServer {
        url: String,
        resource: Arc<Mutex<Resource>>,
        task: tokio::task::JoinHandle<()>,
    }

    impl Drop for TestServer {
        fn drop(&mut self) {
            self.task.abort();
        }
    }

    impl TestServer {
        async fn new(body: Vec<u8>) -> Self {
            let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
            let url = format!("http://{}/package.zip", listener.local_addr().unwrap());
            let resource = Arc::new(Mutex::new(Resource {
                body,
                etag: Some("\"release-a\"".into()),
                last_modified: None,
                get_etag: None,
                get_etag_after: 0,
                get_last_modified: None,
                omit_head_length: false,
                range_end: None,
                ignore_range: false,
                interrupt_next: false,
                invalid_range: false,
                traffic: Traffic::default(),
            }));
            let state = Arc::clone(&resource);
            let task = tokio::spawn(async move {
                loop {
                    let (mut stream, _) = listener.accept().await.unwrap();
                    let mut bytes = Vec::new();
                    let mut buffer = [0u8; 4096];
                    while !bytes.windows(4).any(|bytes| bytes == b"\r\n\r\n") {
                        let count = stream.read(&mut buffer).await.unwrap();
                        if count == 0 {
                            break;
                        }
                        bytes.extend_from_slice(&buffer[..count]);
                    }
                    let request = String::from_utf8(bytes).unwrap();
                    let request = request.to_ascii_lowercase();
                    let range = request
                        .lines()
                        .find_map(|line| line.strip_prefix("range: "));
                    let if_range = request
                        .lines()
                        .find_map(|line| line.strip_prefix("if-range: "));
                    let get = request.starts_with("get ");
                    let (header, body) = {
                        let mut resource = state.lock().unwrap();
                        let size = resource.body.len();
                        let requested_start = range
                            .map(|range| {
                                range
                                    .strip_prefix("bytes=")
                                    .unwrap()
                                    .trim_end_matches('-')
                                    .parse::<usize>()
                                    .unwrap()
                            })
                            .unwrap_or(0);
                        let ranged_response = range.is_some() && !resource.ignore_range;
                        let start = if ranged_response { requested_start } else { 0 };
                        let range_end = if ranged_response {
                            resource.range_end.unwrap_or(size - 1)
                        } else {
                            size - 1
                        };
                        let mut header = format!(
                            "HTTP/1.1 {}\r\nAccept-Ranges: bytes\r\nConnection: close\r\n",
                            if ranged_response {
                                "206 Partial Content"
                            } else {
                                "200 OK"
                            },
                        );
                        if get || !resource.omit_head_length {
                            header.push_str(&format!(
                                "Content-Length: {}\r\n",
                                if get { range_end + 1 - start } else { size }
                            ));
                        }
                        let etag = if get && resource.traffic.gets >= resource.get_etag_after {
                            resource.get_etag.as_ref().unwrap_or(&resource.etag)
                        } else {
                            &resource.etag
                        };
                        if let Some(etag) = etag {
                            header.push_str(&format!("ETag: {etag}\r\n"));
                        }
                        let modified = if get {
                            resource
                                .get_last_modified
                                .as_ref()
                                .unwrap_or(&resource.last_modified)
                        } else {
                            &resource.last_modified
                        };
                        if let Some(modified) = modified {
                            header.push_str(&format!("Last-Modified: {modified}\r\n"));
                        }
                        if ranged_response {
                            header.push_str(&format!(
                                "Content-Range: bytes {}-{}/{}\r\n",
                                if resource.invalid_range {
                                    start + 1
                                } else {
                                    start
                                },
                                range_end,
                                size
                            ));
                        }
                        header.push_str("\r\n");
                        let end = if get && resource.interrupt_next {
                            size / 2
                        } else {
                            range_end + 1
                        };
                        let body = if get {
                            resource.body[start..end].to_vec()
                        } else {
                            Vec::new()
                        };
                        if get {
                            resource.interrupt_next = false;
                            resource.traffic.gets += 1;
                            resource.traffic.bytes += body.len();
                            if let Some(range) = range {
                                resource.traffic.ranges.push(range.to_owned());
                            }
                            if let Some(if_range) = if_range {
                                resource.traffic.if_ranges.push(if_range.to_owned());
                            }
                        }
                        (header, body)
                    };
                    if stream.write_all(header.as_bytes()).await.is_ok() {
                        let _ = stream.write_all(&body).await;
                    }
                    let _ = stream.shutdown().await;
                }
            });
            Self {
                url,
                resource,
                task,
            }
        }

        fn traffic(&self) -> Traffic {
            self.resource.lock().unwrap().traffic.clone()
        }
    }

    fn archive_bytes(format: ArchiveFormat) -> Vec<u8> {
        let content = vec![b'x'; 2048];
        match format {
            ArchiveFormat::Zip => {
                let cursor = std::io::Cursor::new(Vec::new());
                let mut writer = zip::ZipWriter::new(cursor);
                writer
                    .start_file(
                        "docker/config/test.txt",
                        zip::write::SimpleFileOptions::default()
                            .compression_method(zip::CompressionMethod::Stored),
                    )
                    .unwrap();
                writer.write_all(&content).unwrap();
                writer.finish().unwrap().into_inner()
            }
            ArchiveFormat::TarGz => {
                let gzip =
                    flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
                let mut writer = tar::Builder::new(gzip);
                let mut header = tar::Header::new_gnu();
                header.set_size(content.len() as u64);
                header.set_mode(0o644);
                header.set_cksum();
                writer
                    .append_data(&mut header, "docker/config/test.txt", content.as_slice())
                    .unwrap();
                writer.into_inner().unwrap().finish().unwrap()
            }
        }
    }

    fn hash(bytes: &[u8]) -> String {
        Sha256::digest(bytes)
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect()
    }

    fn identity(server: &TestServer, hash: Option<&str>, kind: &str) -> PackageIdentity {
        PackageIdentity::new("0.0.108.1", "x86_64", kind, &server.url, hash).unwrap()
    }

    async fn fetch(directory: &Path, identity: &PackageIdentity) -> Result<PathBuf> {
        let client = Client::builder().no_proxy().build().unwrap();
        let downloader = FileDownloader::new_with_custom_client(
            DownloaderConfig {
                resume_threshold: 1,
                enable_progress_logging: false,
                retain_completed_metadata: true,
                metadata_checkpoint_bytes: 512,
                ..DownloaderConfig::default()
            },
            client.clone(),
        );
        download_package::<fn(DownloadProgress)>(&client, &downloader, directory, identity, None)
            .await
    }

    #[tokio::test]
    async fn final_archive_reused_after_candidate_rejection_for_both_formats_and_kinds() {
        for (format, kind) in [
            (ArchiveFormat::Zip, "full"),
            (ArchiveFormat::TarGz, "patch"),
        ] {
            let body = archive_bytes(format);
            let server = TestServer::new(body.clone()).await;
            let directory = tempfile::tempdir().unwrap();
            let request = identity(&server, None, kind);
            let first = fetch(directory.path(), &request).await.unwrap();
            assert_eq!(
                first.file_name().unwrap(),
                archive::generate_docker_filename("x86_64", format).as_str()
            );
            // Candidate validation failed after download: no version is committed,
            // but the final archive must remain reusable on the next invocation.
            assert!(!directory.path().join(PART_NAME).exists());
            let rejected_candidate =
                Err::<(), _>(anyhow::anyhow!("missing required operator setting"));
            assert!(rejected_candidate.is_err());
            let second = fetch(directory.path(), &request).await.unwrap();
            assert_eq!(first, second);
            assert_eq!(std::fs::read(second).unwrap(), body);
            let traffic = server.traffic();
            assert_eq!(traffic.gets, 1);
            assert_eq!(traffic.bytes, body.len());
        }
    }

    #[tokio::test]
    async fn interrupted_partial_resumes_with_range_and_if_range_without_duplicate_bytes() {
        let body = archive_bytes(ArchiveFormat::Zip);
        let server = TestServer::new(body.clone()).await;
        server.resource.lock().unwrap().interrupt_next = true;
        let directory = tempfile::tempdir().unwrap();
        let request = identity(&server, None, "full");
        assert!(fetch(directory.path(), &request).await.is_err());
        let prefix = std::fs::read(directory.path().join(PART_NAME)).unwrap();
        assert!(!prefix.is_empty());
        assert!(prefix.len() < body.len());
        assert!(!directory.path().join("docker-x86_64.zip").exists());
        let path = fetch(directory.path(), &request).await.unwrap();
        assert_eq!(std::fs::read(path).unwrap(), body);
        let traffic = server.traffic();
        assert_eq!(traffic.gets, 2);
        assert_eq!(traffic.bytes, body.len());
        assert_eq!(traffic.ranges, [format!("bytes={}-", prefix.len())]);
        assert_eq!(traffic.if_ranges, ["\"release-a\""]);
        fetch(directory.path(), &request).await.unwrap();
        assert_eq!(server.traffic().gets, 2);
    }

    #[tokio::test]
    async fn corrupt_content_and_receipt_are_not_cache_hits() {
        let body = archive_bytes(ArchiveFormat::Zip);
        let server = TestServer::new(body.clone()).await;
        let directory = tempfile::tempdir().unwrap();
        let sha = hash(&body);
        let request = identity(&server, Some(&sha), "full");
        let path = fetch(directory.path(), &request).await.unwrap();
        let mut corrupt = body.clone();
        corrupt[50] ^= 1;
        std::fs::write(&path, &corrupt).unwrap();
        fetch(directory.path(), &request).await.unwrap();
        assert_eq!(server.traffic().gets, 2);
        assert_eq!(std::fs::read(&path).unwrap(), body);
        std::fs::write(directory.path().join(RECORD_NAME), b"truncated { json").unwrap();
        fetch(directory.path(), &request).await.unwrap();
        assert_eq!(server.traffic().gets, 3);
        // A forged self-hash cannot override the manifest's trusted SHA.
        let mut record = read_record(&directory.path().join(RECORD_NAME))
            .await
            .unwrap()
            .unwrap();
        if let CacheState::Complete { sha256, .. } = &mut record.state {
            *sha256 = hash(&corrupt);
        }
        save_record(&directory.path().join(RECORD_NAME), &record).unwrap();
        std::fs::write(&path, &corrupt).unwrap();
        fetch(directory.path(), &request).await.unwrap();
        assert_eq!(server.traffic().gets, 4);
        assert_eq!(std::fs::read(&path).unwrap(), body);
    }

    #[tokio::test]
    async fn every_release_identity_dimension_invalidates_cache() {
        let body = archive_bytes(ArchiveFormat::Zip);
        let server = TestServer::new(body.clone()).await;
        let directory = tempfile::tempdir().unwrap();
        let mut request = identity(&server, None, "full");
        fetch(directory.path(), &request).await.unwrap();
        request.version = "0.0.108.2".into();
        fetch(directory.path(), &request).await.unwrap();
        request.architecture = "aarch64".into();
        fetch(directory.path(), &request).await.unwrap();
        request.download_type = "patch".into();
        fetch(directory.path(), &request).await.unwrap();
        request.url = format!("{}?release=other", server.url);
        fetch(directory.path(), &request).await.unwrap();
        assert_eq!(server.traffic().gets, 5);
        assert_eq!(server.traffic().bytes, body.len() * 5);
        assert!(server.traffic().ranges.is_empty());
    }

    #[tokio::test]
    async fn same_url_republished_with_new_etag_redownloads_even_at_same_length() {
        let body = archive_bytes(ArchiveFormat::Zip);
        let server = TestServer::new(body.clone()).await;
        let directory = tempfile::tempdir().unwrap();
        let request = identity(&server, None, "full");
        let path = fetch(directory.path(), &request).await.unwrap();
        {
            let mut resource = server.resource.lock().unwrap();
            resource.body[50] ^= 1;
            resource.etag = Some("\"release-b\"".into());
        }
        fetch(directory.path(), &request).await.unwrap();
        assert_eq!(server.traffic().gets, 2);
        assert_ne!(std::fs::read(path).unwrap(), body);
    }

    #[tokio::test]
    async fn no_remote_validator_or_sha_never_trusts_a_self_recorded_hash() {
        let body = archive_bytes(ArchiveFormat::Zip);
        let server = TestServer::new(body).await;
        server.resource.lock().unwrap().etag = None;
        let directory = tempfile::tempdir().unwrap();
        let request = identity(&server, None, "full");
        fetch(directory.path(), &request).await.unwrap();
        fetch(directory.path(), &request).await.unwrap();
        assert_eq!(server.traffic().gets, 2);
    }

    #[tokio::test]
    async fn published_sha_mismatch_cannot_promote_a_final_package() {
        let body = archive_bytes(ArchiveFormat::Zip);
        let server = TestServer::new(body.clone()).await;
        let directory = tempfile::tempdir().unwrap();
        let request = identity(&server, Some(&"0".repeat(64)), "full");
        assert!(fetch(directory.path(), &request).await.is_err());
        assert!(!directory.path().join("docker-x86_64.zip").exists());
        assert!(!directory.path().join(PART_NAME).exists());
        assert_eq!(server.traffic().bytes, body.len());
    }

    #[tokio::test]
    async fn completed_receipt_recovers_an_interrupted_final_rename_without_get() {
        let body = archive_bytes(ArchiveFormat::Zip);
        let server = TestServer::new(body.clone()).await;
        let directory = tempfile::tempdir().unwrap();
        let request = identity(&server, None, "full");
        let path = fetch(directory.path(), &request).await.unwrap();
        std::fs::rename(&path, directory.path().join(PART_NAME)).unwrap();
        // A previous final package exists when a release is replaced.
        std::fs::write(&path, b"previous invalid package").unwrap();
        fetch(directory.path(), &request).await.unwrap();
        assert_eq!(std::fs::read(path).unwrap(), body);
        assert_eq!(server.traffic().gets, 1);
    }

    #[tokio::test]
    async fn fully_downloaded_partial_with_published_sha_finishes_without_get() {
        let body = archive_bytes(ArchiveFormat::Zip);
        let server = TestServer::new(body.clone()).await;
        let directory = tempfile::tempdir().unwrap();
        let sha = hash(&body);
        let request = identity(&server, Some(&sha), "full");
        let path = fetch(directory.path(), &request).await.unwrap();
        std::fs::rename(&path, directory.path().join(PART_NAME)).unwrap();
        let mut record = read_record(&directory.path().join(RECORD_NAME))
            .await
            .unwrap()
            .unwrap();
        record.state = CacheState::Partial;
        save_record(&directory.path().join(RECORD_NAME), &record).unwrap();
        fetch(directory.path(), &request).await.unwrap();
        assert_eq!(std::fs::read(path).unwrap(), body);
        assert_eq!(server.traffic().gets, 1);
    }

    #[tokio::test]
    async fn invalid_range_response_is_rejected_before_appending() {
        let body = archive_bytes(ArchiveFormat::Zip);
        let server = TestServer::new(body.clone()).await;
        server.resource.lock().unwrap().interrupt_next = true;
        let directory = tempfile::tempdir().unwrap();
        let request = identity(&server, None, "full");
        assert!(fetch(directory.path(), &request).await.is_err());
        let part = directory.path().join(PART_NAME);
        let prefix = std::fs::read(&part).unwrap();
        server.resource.lock().unwrap().invalid_range = true;
        assert!(fetch(directory.path(), &request).await.is_err());
        assert_eq!(std::fs::read(&part).unwrap(), prefix);
        server.resource.lock().unwrap().invalid_range = false;
        let path = fetch(directory.path(), &request).await.unwrap();
        assert_eq!(std::fs::read(path).unwrap(), body);
    }

    #[tokio::test]
    async fn stale_partial_identity_and_corrupt_resume_metadata_restart_without_range() {
        let body = archive_bytes(ArchiveFormat::Zip);
        let server = TestServer::new(body.clone()).await;
        let directory = tempfile::tempdir().unwrap();
        let mut request = identity(&server, None, "full");
        server.resource.lock().unwrap().interrupt_next = true;
        assert!(fetch(directory.path(), &request).await.is_err());
        request.version = "0.0.109.0".into();
        server.resource.lock().unwrap().interrupt_next = true;
        assert!(fetch(directory.path(), &request).await.is_err());
        std::fs::write(
            directory.path().join(PART_NAME).with_extension("download"),
            b"bad json",
        )
        .unwrap();
        let path = fetch(directory.path(), &request).await.unwrap();
        assert_eq!(std::fs::read(path).unwrap(), body);
        assert!(server.traffic().ranges.is_empty());
        assert_eq!(server.traffic().gets, 3);
    }

    #[tokio::test]
    async fn api_entry_returns_final_cache_name_and_reuses_identical_release() {
        let body = archive_bytes(ArchiveFormat::Zip);
        let server = TestServer::new(body.clone()).await;
        let directory = tempfile::tempdir().unwrap();
        let sha = hash(&body);
        let request = identity(&server, Some(&sha), "full");
        let api = crate::api::ApiClient::new(None, None);
        let first = api
            .download_service_package(directory.path(), &request)
            .await
            .unwrap();
        let second = api
            .download_service_package(directory.path(), &request)
            .await
            .unwrap();
        assert_eq!(first, second);
        assert_eq!(first.file_name().unwrap(), "docker-x86_64.zip");
        assert_eq!(server.traffic().gets, 1);
        assert_eq!(server.traffic().bytes, body.len());
    }

    #[tokio::test]
    async fn candidate_resolution_selects_receipt_archive_over_stale_formats() {
        let body = archive_bytes(ArchiveFormat::TarGz);
        let server = TestServer::new(body).await;
        let directory = tempfile::tempdir().unwrap();
        std::fs::write(directory.path().join("docker-x86_64.zip"), b"stale ZIP").unwrap();
        std::fs::write(
            directory.path().join("docker-aarch64.zip"),
            b"other architecture",
        )
        .unwrap();
        let request = identity(&server, None, "full");
        let downloaded = fetch(directory.path(), &request).await.unwrap();
        assert_eq!(
            resolve_cached_package(directory.path(), &request).unwrap(),
            downloaded
        );
        assert_eq!(downloaded.file_name().unwrap(), "docker-x86_64.tar.gz");
        let mut other_release = request.clone();
        other_release.version = "0.0.109.0".into();
        assert!(resolve_cached_package(directory.path(), &other_release).is_err());
        let mut record = read_record(&directory.path().join(RECORD_NAME))
            .await
            .unwrap()
            .unwrap();
        if let CacheState::Complete { filename, .. } = &mut record.state {
            *filename = "docker-aarch64.zip".into();
        }
        save_record(&directory.path().join(RECORD_NAME), &record).unwrap();
        assert!(resolve_cached_package(directory.path(), &request).is_err());
        std::fs::write(directory.path().join(RECORD_NAME), b"corrupt receipt").unwrap();
        assert!(resolve_cached_package(directory.path(), &request).is_err());
    }

    #[test]
    fn legacy_candidate_resolution_uses_url_and_rejects_ambiguity() {
        let directory = tempfile::tempdir().unwrap();
        let zip = directory.path().join("docker-x86_64.zip");
        let tar = directory.path().join("docker-x86_64.tar.gz");
        std::fs::write(&zip, b"ZIP").unwrap();
        std::fs::write(&tar, b"TAR.GZ").unwrap();
        std::fs::write(directory.path().join("docker-aarch64.zip"), b"ARM").unwrap();
        let request = PackageIdentity::new(
            "0.0.108.0",
            "x86_64",
            "full",
            "https://example.com/package.tar.gz",
            None,
        )
        .unwrap();
        assert_eq!(
            resolve_cached_package(directory.path(), &request).unwrap(),
            tar
        );
        let mut unknown_format = request.clone();
        unknown_format.url = "https://example.com/download".into();
        assert!(resolve_cached_package(directory.path(), &unknown_format).is_err());
        std::fs::remove_file(&tar).unwrap();
        assert_eq!(
            resolve_cached_package(directory.path(), &unknown_format).unwrap(),
            zip
        );
        std::fs::remove_file(&zip).unwrap();
        assert!(resolve_cached_package(directory.path(), &unknown_format).is_err());
    }

    #[test]
    fn package_identity_rejects_invalid_hashes_and_unsupported_inputs() {
        assert!(
            PackageIdentity::new(
                "0.0.108",
                "x86_64",
                "full",
                "https://example.com/package.zip",
                Some("invalid-hash")
            )
            .is_err()
        );
        assert!(
            PackageIdentity::new(
                "0.0.108",
                "mips",
                "full",
                "https://example.com/package.zip",
                None
            )
            .is_err()
        );
        assert!(
            PackageIdentity::new(
                "0.0.108",
                "x86_64",
                "invalid",
                "https://example.com/package.zip",
                None
            )
            .is_err()
        );
        assert!(
            PackageIdentity::new("0.0.108", "x86_64", "full", "file:///tmp/package.zip", None)
                .is_err()
        );
        assert_eq!(normalize_sha256(Some("external")).unwrap(), None);
        assert_eq!(
            normalize_sha256(Some(&format!("sha256:{}", "A".repeat(64)))).unwrap(),
            Some("a".repeat(64))
        );
    }

    #[tokio::test]
    async fn corrupt_partial_prefix_restarts_without_range_when_remote_sha_is_absent() {
        let body = archive_bytes(ArchiveFormat::Zip);
        let server = TestServer::new(body.clone()).await;
        server.resource.lock().unwrap().interrupt_next = true;
        let directory = tempfile::tempdir().unwrap();
        let request = identity(&server, None, "full");
        assert!(fetch(directory.path(), &request).await.is_err());
        let part = directory.path().join(PART_NAME);
        let mut prefix = std::fs::read(&part).unwrap();
        prefix[50] ^= 1;
        std::fs::write(&part, prefix).unwrap();
        let path = fetch(directory.path(), &request).await.unwrap();
        assert_eq!(std::fs::read(path).unwrap(), body);
        assert_eq!(server.traffic().gets, 2);
        assert!(server.traffic().ranges.is_empty());
    }

    #[tokio::test]
    async fn process_termination_tail_is_truncated_to_the_last_verified_checkpoint() {
        let body = archive_bytes(ArchiveFormat::Zip);
        let server = TestServer::new(body.clone()).await;
        server.resource.lock().unwrap().interrupt_next = true;
        let directory = tempfile::tempdir().unwrap();
        let request = identity(&server, None, "full");
        assert!(fetch(directory.path(), &request).await.is_err());
        let part = directory.path().join(PART_NAME);
        let metadata_path = part.with_extension("download");
        let metadata: DownloadMetadata =
            serde_json::from_slice(&std::fs::read(&metadata_path).unwrap()).unwrap();
        let checkpoint = std::fs::read(&part).unwrap();
        assert_eq!(metadata.downloaded_bytes, checkpoint.len() as u64);
        assert_eq!(metadata.partial_sha256, Some(hash(&checkpoint)));
        assert!(!metadata.completed);
        // Equivalent on-disk state to SIGKILL after writing bytes but before the
        // next atomic metadata checkpoint: these extra bytes are not trusted.
        std::fs::OpenOptions::new()
            .append(true)
            .open(&part)
            .unwrap()
            .write_all(b"uncheckpointed corrupt tail")
            .unwrap();
        let path = fetch(directory.path(), &request).await.unwrap();
        assert_eq!(std::fs::read(path).unwrap(), body);
        assert_eq!(
            server.traffic().ranges,
            [format!("bytes={}-", checkpoint.len())]
        );
        assert_eq!(server.traffic().bytes, body.len());
    }

    #[tokio::test]
    async fn checkpointless_legacy_partial_without_remote_sha_restarts_safely() {
        let body = archive_bytes(ArchiveFormat::Zip);
        let server = TestServer::new(body.clone()).await;
        server.resource.lock().unwrap().interrupt_next = true;
        let directory = tempfile::tempdir().unwrap();
        let request = identity(&server, None, "full");
        assert!(fetch(directory.path(), &request).await.is_err());
        let metadata_path = directory.path().join(PART_NAME).with_extension("download");
        let mut metadata: DownloadMetadata =
            serde_json::from_slice(&std::fs::read(&metadata_path).unwrap()).unwrap();
        metadata.partial_sha256 = None;
        write_atomic(
            &metadata_path,
            &serde_json::to_vec(&metadata).unwrap(),
            PermissionsPolicy::Private,
        )
        .unwrap();
        let path = fetch(directory.path(), &request).await.unwrap();
        assert_eq!(std::fs::read(path).unwrap(), body);
        assert!(server.traffic().ranges.is_empty());
    }

    #[tokio::test]
    async fn completed_transport_checkpoint_recovers_before_final_receipt_write() {
        let body = archive_bytes(ArchiveFormat::Zip);
        let server = TestServer::new(body.clone()).await;
        let directory = tempfile::tempdir().unwrap();
        let request = identity(&server, None, "full");
        let client = Client::builder().no_proxy().build().unwrap();
        let remote = RemoteResource::probe(&client, &server.url).await.unwrap();
        save_record(
            &directory.path().join(RECORD_NAME),
            &CacheRecord {
                schema_version: 1,
                identity: request.clone(),
                remote,
                state: CacheState::Partial,
            },
        )
        .unwrap();
        let downloader = FileDownloader::new_with_custom_client(
            DownloaderConfig {
                retain_completed_metadata: true,
                ..DownloaderConfig::default()
            },
            client,
        );
        let part = directory.path().join(PART_NAME);
        downloader
            .download_file_with_options::<fn(DownloadProgress)>(
                &server.url,
                &part,
                None,
                None,
                Some(&request.version),
            )
            .await
            .unwrap();
        assert_eq!(server.traffic().gets, 1);
        let path = fetch(directory.path(), &request).await.unwrap();
        assert_eq!(std::fs::read(path).unwrap(), body);
        assert_eq!(server.traffic().gets, 1);
        assert!(!part.exists());
        assert!(!part.with_extension("download").exists());
    }

    #[tokio::test]
    async fn stale_head_and_different_get_validator_cannot_publish_a_package() {
        for use_etag in [true, false] {
            let body = archive_bytes(ArchiveFormat::Zip);
            let server = TestServer::new(body).await;
            {
                let mut resource = server.resource.lock().unwrap();
                if use_etag {
                    resource.get_etag = Some(Some("\"different-release\"".into()));
                } else {
                    resource.etag = None;
                    resource.last_modified = Some("Sat, 10 Oct 2026 00:00:00 GMT".into());
                    resource.get_last_modified = Some(Some("Sat, 10 Oct 2026 01:00:00 GMT".into()));
                }
            }
            let directory = tempfile::tempdir().unwrap();
            let request = identity(&server, None, "full");
            let error = fetch(directory.path(), &request).await.unwrap_err();
            assert!(format!("{error:#}").contains("validator differs from HEAD"));
            assert!(!directory.path().join("docker-x86_64.zip").exists());
            assert!(!directory.path().join(PART_NAME).exists());
            assert_eq!(server.traffic().gets, 1);
        }
    }

    #[tokio::test]
    async fn resume_or_full_fallback_validator_change_preserves_verified_prefix() {
        for ignore_range in [false, true] {
            let body = archive_bytes(ArchiveFormat::Zip);
            let server = TestServer::new(body.clone()).await;
            server.resource.lock().unwrap().interrupt_next = true;
            let directory = tempfile::tempdir().unwrap();
            let request = identity(&server, None, "full");
            assert!(fetch(directory.path(), &request).await.is_err());
            let part = directory.path().join(PART_NAME);
            let prefix = std::fs::read(&part).unwrap();
            {
                let mut resource = server.resource.lock().unwrap();
                resource.ignore_range = ignore_range;
                resource.get_etag_after = if ignore_range { 2 } else { 0 };
                resource.get_etag = Some(Some("\"different-release\"".into()));
            }
            let error = fetch(directory.path(), &request).await.unwrap_err();
            assert!(format!("{error:#}").contains("validator differs from HEAD"));
            assert_eq!(std::fs::read(&part).unwrap(), prefix);
            assert!(!directory.path().join("docker-x86_64.zip").exists());
            assert_eq!(server.traffic().gets, if ignore_range { 3 } else { 2 });
        }
    }

    #[tokio::test]
    async fn unknown_head_size_does_not_accept_a_nonfinal_range_as_complete() {
        let body = archive_bytes(ArchiveFormat::Zip);
        let server = TestServer::new(body.clone()).await;
        {
            let mut resource = server.resource.lock().unwrap();
            resource.omit_head_length = true;
            resource.interrupt_next = true;
        }
        let directory = tempfile::tempdir().unwrap();
        let request = identity(&server, None, "full");
        assert!(fetch(directory.path(), &request).await.is_err());
        let part = directory.path().join(PART_NAME);
        let prefix = std::fs::read(&part).unwrap();
        server.resource.lock().unwrap().range_end = Some(body.len() - 8);
        assert!(fetch(directory.path(), &request).await.is_err());
        assert_eq!(std::fs::read(&part).unwrap(), prefix);
        assert!(!directory.path().join("docker-x86_64.zip").exists());
        server.resource.lock().unwrap().range_end = None;
        let path = fetch(directory.path(), &request).await.unwrap();
        assert_eq!(std::fs::read(path).unwrap(), body);
        assert_eq!(server.traffic().gets, 3);
    }
}
