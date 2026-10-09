//! Release-aware cold backups. Delivery receipts are preserved byte-for-byte;
//! restoring artifacts also restores their configuration and immutable runtime.

use crate::container::{DockerManager, preflight};
use anyhow::{Context, Result, anyhow, bail};
use flate2::read::GzDecoder;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::io::{Read, Write};
use std::path::{Component, Path, PathBuf};
use tempfile::TempDir;

pub const RELEASE_CONTEXT_FILE: &str = "BACKUP_RELEASE.json";
const DELIVERY_FILE: &str = "DELIVERY_MANIFEST.json";
const COMPOSE_FILE: &str = "docker-compose.yml";
const RELEASE_ROOTS: &[&str] = &["app", "im-app", "repo-collab-app", "config", "script"];
const MAX_CONTEXT_SIZE: u64 = 4 * 1024 * 1024;

/// GNU sparse members are regular filesystem files. `tar` validates their
/// extent map while reading entries and reconstructs their logical zero holes.
/// Links and other special types must never acquire regular-file semantics.
pub(crate) fn is_archive_regular_file(kind: tar::EntryType) -> bool {
    kind.is_file() || kind.is_gnu_sparse()
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct RuntimeImage {
    pub reference: String,
    pub image_id: String,
    pub os: String,
    pub architecture: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReleaseContext {
    pub contract_version: u32,
    pub service_version: String,
    /// Exact saved files, including the original delivery receipt and config.
    pub files: BTreeMap<String, String>,
    pub roots: Vec<String>,
    pub runtime_images: Vec<RuntimeImage>,
}

/// Owns the frozen files until the surrounding cold backup has finished.
pub struct ReleaseSnapshot {
    directory: TempDir,
    pub context: ReleaseContext,
    contains_cold_data: bool,
    cold_data_paths: BTreeSet<String>,
    rollback_data: bool,
}

impl ReleaseSnapshot {
    pub fn root(&self) -> &Path {
        self.directory.path()
    }

    pub fn source_paths(&self) -> Vec<PathBuf> {
        let mut paths: Vec<PathBuf> = self
            .context
            .roots
            .iter()
            .map(|root| self.root().join(root))
            .collect();
        paths.push(self.root().join(RELEASE_CONTEXT_FILE));
        paths
    }
}

/// Custom deployment directories and independent env files are supported. The
/// selected user env (including a symlink target) must survive all release/data
/// replacement operations; an env stored inside those roots is rejected early.
pub fn deployment_root(manager: &DockerManager) -> Result<PathBuf> {
    if manager
        .get_compose_file()
        .file_name()
        .and_then(|v| v.to_str())
        != Some(COMPOSE_FILE)
    {
        bail!(
            "Release backup requires a docker-compose.yml basename; custom Compose filenames need their matching full package restore workflow"
        );
    }
    let compose_root = manager
        .get_compose_file()
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or(Path::new("."))
        .canonicalize()
        .context("Cannot resolve the selected backup deployment directory")?;
    let env_parent = manager
        .get_env_file()
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or(Path::new("."))
        .canonicalize()
        .context("Cannot resolve the selected backup environment directory")?;
    let env_name = manager
        .get_env_file()
        .file_name()
        .ok_or_else(|| anyhow!("Selected environment file has no basename"))?;
    let env_path = env_parent.join(env_name);
    let env_target = env_path
        .canonicalize()
        .context("Cannot resolve the selected environment file for backup")?;
    if !env_target.is_file() {
        bail!("Selected environment file must be a regular file");
    }
    for path in [&env_path, &env_target] {
        for root in RELEASE_ROOTS
            .iter()
            .copied()
            .chain(["data", COMPOSE_FILE, DELIVERY_FILE])
        {
            if path.starts_with(compose_root.join(root)) {
                bail!(
                    "Selected user environment lives inside a directory/file replaced by release or data rollback; move it outside release/data roots before backup"
                );
            }
        }
    }
    if compose_root.join(DELIVERY_FILE).exists() {
        validate_mysql_data_layout(manager, &compose_root)?;
    }
    Ok(compose_root)
}

/// Cold data backups cover only bind mounts within the selected data root.
/// Named volumes and external paths require a different backup mechanism.
pub fn validate_mysql_data_layout(manager: &DockerManager, root: &Path) -> Result<String> {
    let delivery = load_delivery(root)?;
    let schema = crate::mysql_manifest::parse_schema_manifest(&fs::read_to_string(
        root.join(&delivery.mysql.manifest),
    )?)?;
    let compose = manager.load_compose_config()?;
    let mysql = compose
        .services
        .0
        .get(&schema.mysql_target.service)
        .and_then(Option::as_ref)
        .ok_or_else(|| anyhow!("Selected Compose has no declared MySQL service"))?;
    let mut data_targets = 0;
    for volume in &mysql.volumes {
        let value = serde_json::to_value(volume)?;
        let targets_mysql = match &value {
            serde_json::Value::String(short) => {
                let mut parts = short.rsplit(':');
                parts.next() == Some("/var/lib/mysql") || parts.next() == Some("/var/lib/mysql")
            }
            serde_json::Value::Object(long) => {
                long.get("target").and_then(serde_json::Value::as_str) == Some("/var/lib/mysql")
            }
            _ => false,
        };
        if targets_mysql {
            data_targets += 1;
            if value.is_object()
                && value.get("type").and_then(serde_json::Value::as_str) != Some("bind")
            {
                bail!(
                    "Cold backup does not cover named MySQL volumes; declare a bind below the selected data root"
                );
            }
        }
    }
    if data_targets != 1 {
        bail!("Cold backup requires exactly one explicit MySQL data bind");
    }
    let mounts = manager.extract_mount_directories(&compose)?;
    let data_mount = mounts.iter().find(|mount| {
        mount.service_name == schema.mysql_target.service && mount.container_path == "/var/lib/mysql"
    }).ok_or_else(|| anyhow!("Cold backup does not cover a named/external MySQL data volume; require an explicit bind under this deployment's data directory"))?;
    let host = data_mount
        .host_path
        .as_deref()
        .ok_or_else(|| anyhow!("MySQL data bind has no host path"))?;
    let requested = lexical_absolute(Path::new(host))?;
    let root = root.canonicalize()?;
    let data_root = root.join("data");
    // Resolve any existing ancestors so symlinks cannot turn an apparently local
    // bind into an external store that this directory backup does not cover.
    let mut ancestor = requested.as_path();
    while !ancestor.exists() {
        ancestor = ancestor
            .parent()
            .ok_or_else(|| anyhow!("Cannot resolve MySQL data bind"))?;
    }
    let resolved = ancestor.canonicalize()?;
    let source = resolved.join(requested.strip_prefix(ancestor)?);
    if !source.starts_with(&data_root) || source == data_root {
        bail!(
            "Cold backup does not cover external or symlinked MySQL data; bind /var/lib/mysql below the selected deployment's data root"
        );
    }
    if source.exists() && !source.is_dir() {
        bail!("MySQL cold backup source must be a directory");
    }
    Ok(source
        .strip_prefix(root)?
        .to_string_lossy()
        .replace('\\', "/"))
}

fn lexical_absolute(path: &Path) -> Result<PathBuf> {
    let input = if path.is_absolute() {
        path.to_path_buf()
    } else {
        std::env::current_dir()?.join(path)
    };
    let mut result = PathBuf::new();
    for component in input.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                result.pop();
            }
            other => result.push(other.as_os_str()),
        }
    }
    Ok(result)
}

fn checked_relative(path: &str) -> Result<&str> {
    if path.is_empty()
        || path.contains('\\')
        || path
            .split('/')
            .any(|part| part.is_empty() || part == "." || part == "..")
        || Path::new(path)
            .components()
            .any(|part| !matches!(part, Component::Normal(_)))
    {
        bail!("Backup contains an invalid relative release path");
    }
    Ok(path)
}

fn file_hash(path: &Path) -> Result<String> {
    let mut source =
        fs::File::open(path).with_context(|| format!("Cannot hash {}", path.display()))?;
    let mut hash = Sha256::new();
    let mut buffer = [0u8; 64 * 1024];
    loop {
        let count = source.read(&mut buffer)?;
        if count == 0 {
            break;
        }
        hash.update(&buffer[..count]);
    }
    Ok(hash
        .finalize()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect())
}

fn is_digest(value: &str) -> bool {
    value
        .strip_prefix("sha256:")
        .is_some_and(|value| value.len() == 64 && value.bytes().all(|b| b.is_ascii_hexdigit()))
}

fn validate_context(context: &ReleaseContext) -> Result<()> {
    if context.contract_version != 1 || context.runtime_images.is_empty() {
        bail!("Unsupported or incomplete backup release context; use its matching full package");
    }
    let mut roots = BTreeSet::new();
    for root in &context.roots {
        if !(RELEASE_ROOTS.contains(&root.as_str())
            || root == COMPOSE_FILE
            || root == DELIVERY_FILE)
            || !roots.insert(root.as_str())
        {
            bail!("Invalid or duplicate release root in backup context");
        }
    }
    if !roots.contains(COMPOSE_FILE) || !roots.contains(DELIVERY_FILE) {
        bail!("Backup release context is missing Compose or delivery metadata");
    }
    for (path, hash) in &context.files {
        checked_relative(path)?;
        let root = path
            .split('/')
            .next()
            .ok_or_else(|| anyhow!("Invalid release file"))?;
        if !roots.contains(root) || hash.len() != 64 || !hash.bytes().all(|b| b.is_ascii_hexdigit())
        {
            bail!("Backup release file is outside the declared context or has an invalid hash");
        }
    }
    if !context.files.contains_key(COMPOSE_FILE) || !context.files.contains_key(DELIVERY_FILE) {
        bail!("Backup release context is missing required file fingerprints");
    }
    let mut references = BTreeSet::new();
    for image in &context.runtime_images {
        if image.reference.is_empty()
            || !references.insert(image.reference.as_str())
            || !is_digest(&image.image_id)
            || image.os != "linux"
            || !matches!(image.architecture.as_str(), "amd64" | "arm64")
        {
            bail!("Invalid or duplicate immutable runtime image in backup context");
        }
    }
    Ok(())
}

fn load_delivery(root: &Path) -> Result<preflight::DeliveryManifest> {
    preflight::parse_delivery_manifest(&fs::read_to_string(root.join(DELIVERY_FILE))?)
        .context("Cannot parse the release delivery manifest")
}

fn validate_runtime_delivery(
    context: &ReleaseContext,
    delivery: &preflight::DeliveryManifest,
) -> Result<()> {
    for component in delivery.components.values() {
        let runtime = context
            .runtime_images
            .iter()
            .find(|image| image.reference == component.target)
            .ok_or_else(|| anyhow!("Saved runtime does not contain a delivery component image"))?;
        // Docker's classic image store exposes the platform config ID; containerd
        // may expose the original OCI index ID. The producer records both.
        if runtime.image_id != component.image_id && runtime.image_id != component.config_id {
            bail!(
                "Local component runtime does not match the delivery receipt; load the matching full package images before creating or restoring this backup"
            );
        }
    }
    Ok(())
}

fn copy_regular_tree(source: &Path, destination: &Path) -> Result<()> {
    let metadata = fs::symlink_metadata(source)?;
    if metadata.file_type().is_symlink() {
        bail!(
            "Release snapshots cannot contain symlinks: {}",
            source.display()
        );
    }
    if metadata.is_dir() {
        fs::create_dir_all(destination)?;
        for entry in fs::read_dir(source)? {
            let entry = entry?;
            copy_regular_tree(&entry.path(), &destination.join(entry.file_name()))?;
        }
        fs::set_permissions(destination, metadata.permissions())?;
    } else if metadata.is_file() {
        if let Some(parent) = destination.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::copy(source, destination)?;
        fs::set_permissions(destination, metadata.permissions())?;
    } else {
        bail!(
            "Release snapshots require regular files/directories: {}",
            source.display()
        );
    }
    Ok(())
}

/// Pure filesystem half of snapshot capture; image identities come from the
/// selected manager, never a guessed registry tag or reconstructed receipt.
pub fn capture_release_files(
    root: &Path,
    service_version: &str,
    runtime_images: Vec<RuntimeImage>,
) -> Result<ReleaseSnapshot> {
    let delivery = load_delivery(root)?;
    preflight::verify_delivery_manifest(&delivery, root, None)?;
    let directory = tempfile::tempdir_in(root.parent().unwrap_or(Path::new(".")))?;
    let mut roots = vec![COMPOSE_FILE.to_string(), DELIVERY_FILE.to_string()];
    for name in RELEASE_ROOTS {
        if root.join(name).exists() {
            roots.push((*name).to_string());
        }
    }
    for name in &roots {
        copy_regular_tree(&root.join(name), &directory.path().join(name))?;
    }
    let mut files = BTreeMap::new();
    for entry in walkdir::WalkDir::new(directory.path()) {
        let entry = entry?;
        if entry.file_type().is_file() {
            let path = entry
                .path()
                .strip_prefix(directory.path())?
                .to_string_lossy()
                .replace('\\', "/");
            files.insert(path, file_hash(entry.path())?);
        }
    }
    let context = ReleaseContext {
        contract_version: 1,
        service_version: service_version.to_string(),
        files,
        roots,
        runtime_images,
    };
    validate_context(&context)?;
    validate_runtime_delivery(&context, &delivery)?;
    fs::write(
        directory.path().join(RELEASE_CONTEXT_FILE),
        serde_json::to_vec_pretty(&context)?,
    )?;
    preflight::verify_delivery_manifest(&delivery, directory.path(), None)?;
    Ok(ReleaseSnapshot {
        directory,
        context,
        contains_cold_data: false,
        cold_data_paths: BTreeSet::new(),
        rollback_data: false,
    })
}

async fn compose_images(manager: &DockerManager) -> Result<BTreeSet<String>> {
    let output = manager.run_compose_command(&["config", "--images"]).await?;
    if !output.status.success() {
        bail!("Cannot resolve runtime image references from the selected Compose configuration");
    }
    Ok(String::from_utf8(output.stdout)?
        .lines()
        .map(str::trim)
        .filter(|name| !name.is_empty())
        .map(str::to_string)
        .collect())
}

async fn inspect_runtime(manager: &DockerManager, reference: &str) -> Result<RuntimeImage> {
    let output = manager
        .run_docker_command(&["image", "inspect", "--format", "{{json .}}", reference])
        .await?;
    if !output.status.success() {
        bail!(
            "Required immutable runtime image is not available locally; load images from the matching full package before rollback"
        );
    }
    let value: serde_json::Value = serde_json::from_slice(&output.stdout)
        .context("Cannot decode the selected runtime image identity")?;
    let required = |name: &str| -> Result<String> {
        value
            .get(name)
            .and_then(serde_json::Value::as_str)
            .map(str::to_string)
            .ok_or_else(|| anyhow!("Runtime image is missing identity field {name}"))
    };
    Ok(RuntimeImage {
        reference: reference.to_string(),
        image_id: required("Id")?,
        os: required("Os")?,
        architecture: required("Architecture")?,
    })
}

pub async fn capture_release_snapshot(
    manager: &DockerManager,
    service_version: &str,
) -> Result<Option<ReleaseSnapshot>> {
    let root = deployment_root(manager)?;
    if !root.join(DELIVERY_FILE).exists() {
        return Ok(None);
    }
    let data_directory = root.join(validate_mysql_data_layout(manager, &root)?);
    let mut has_mysql_files = false;
    for entry in walkdir::WalkDir::new(&data_directory) {
        let entry = entry.context("Cannot inspect MySQL cold data before backup")?;
        if entry.file_type().is_file() && entry.metadata()?.len() > 0 {
            has_mysql_files = true;
            break;
        }
    }
    if !has_mysql_files {
        bail!(
            "MySQL data directory contains no cold files; cannot create a complete release backup"
        );
    }
    let mut images = Vec::new();
    for reference in compose_images(manager).await? {
        images.push(inspect_runtime(manager, &reference).await?);
    }
    capture_release_files(&root, service_version, images).map(Some)
}

/// Validate an archive's release bytes without unpacking data, stopping a service
/// or changing a runtime tag. Returns None only for explicitly legacy backups.
pub fn stage_release_restore(
    archive_path: &Path,
    target_root: &Path,
    rollback_data: bool,
) -> Result<Option<ReleaseSnapshot>> {
    let mut archive = tar::Archive::new(GzDecoder::new(fs::File::open(archive_path)?));
    let mut context = None;
    let mut carries_delivery = false;
    let mut carries_data = false;
    let mut cold_data_paths = BTreeSet::new();
    let mut archive_paths = BTreeSet::new();
    for entry in archive.entries()? {
        let mut entry = entry?;
        let name = entry.path()?.to_string_lossy().into_owned();
        let relative = checked_relative(name.trim_end_matches('/'))?;
        let kind = entry.header().entry_type();
        let regular = is_archive_regular_file(kind);
        if !(regular || kind.is_dir()) || (regular && name.ends_with('/')) {
            bail!(
                "Cold backup contains a link or special archive entry; refusing unsafe restore before stopping services"
            );
        }
        // tar reconstructs sparse holes with signed SeekFrom::Current offsets.
        // Reject an otherwise valid u64 extent map that cannot be unpacked.
        if kind.is_gnu_sparse() && entry.size() > i64::MAX as u64 {
            bail!("Cold backup sparse file exceeds supported filesystem offsets");
        }
        if !archive_paths.insert(relative.to_string()) {
            bail!(
                "Cold backup contains duplicate archive paths; refusing an ambiguous restore before stopping services"
            );
        }
        if name == DELIVERY_FILE {
            if !kind.is_file() {
                bail!("Backup delivery manifest must use a regular archive header");
            }
            carries_delivery = true;
        }
        if name.starts_with("data/") && regular && entry.size() > 0 {
            carries_data = true;
            cold_data_paths.insert(name.clone());
        }
        if name == RELEASE_CONTEXT_FILE {
            if context.is_some()
                || !entry.header().entry_type().is_file()
                || entry.size() > MAX_CONTEXT_SIZE
            {
                bail!("Invalid or duplicate backup release context");
            }
            let mut bytes = Vec::new();
            entry.read_to_end(&mut bytes)?;
            context = Some(serde_json::from_slice::<ReleaseContext>(&bytes)?);
        }
    }
    let Some(context) = context else {
        if carries_delivery || target_root.join(DELIVERY_FILE).exists() {
            bail!(
                "This backup lacks a complete release context; restore its matching full package before restoring application files or create a new release-aware backup"
            );
        }
        return Ok(None);
    };
    validate_context(&context)?;
    if rollback_data && !carries_data {
        bail!(
            "The release backup has no cold data files; refusing to erase current data for --rollback-data"
        );
    }
    let directory = tempfile::tempdir_in(target_root.parent().unwrap_or(Path::new(".")))?;
    for root in &context.roots {
        if RELEASE_ROOTS.contains(&root.as_str()) {
            fs::create_dir_all(directory.path().join(root))?;
        }
    }
    let mut seen = BTreeSet::new();
    let mut archive = tar::Archive::new(GzDecoder::new(fs::File::open(archive_path)?));
    for entry in archive.entries()? {
        let mut entry = entry?;
        let name = entry.path()?.to_string_lossy().into_owned();
        checked_relative(name.trim_end_matches('/'))?;
        let Some(expected) = context.files.get(&name) else {
            continue;
        };
        if !is_archive_regular_file(entry.header().entry_type()) || !seen.insert(name.clone()) {
            bail!("Release backup contains a non-regular or duplicate declared file");
        }
        let path = directory.path().join(&name);
        fs::create_dir_all(
            path.parent()
                .ok_or_else(|| anyhow!("Missing release file parent"))?,
        )?;
        let mut destination = fs::File::create(&path)?;
        let expected_size = entry.size();
        if std::io::copy(&mut entry, &mut destination)? != expected_size {
            bail!("Release backup file {name} is truncated");
        }
        destination.flush()?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(
                &path,
                fs::Permissions::from_mode(entry.header().mode()? & 0o777),
            )?;
        }
        if file_hash(&path)? != *expected {
            bail!("Release backup file {name} does not match its saved fingerprint");
        }
    }
    if seen.len() != context.files.len() {
        bail!("Release backup is missing declared files; refusing a partial release restore");
    }
    let delivery = load_delivery(directory.path())?;
    preflight::verify_delivery_manifest(&delivery, directory.path(), None)?;
    validate_runtime_delivery(&context, &delivery)?;
    if !rollback_data {
        let current = load_delivery(target_root)
            .context("Application-only rollback needs the current release receipt")?;
        if current.mysql.files != delivery.mysql.files {
            bail!(
                "Application-only rollback cannot change the saved MySQL schema/config fingerprints while keeping current data; use --rollback-data with its matching cold backup or the matching full-package migration workflow (file hashes include comments)"
            );
        }
    }
    Ok(Some(ReleaseSnapshot {
        directory,
        context,
        contains_cold_data: carries_data,
        cold_data_paths,
        rollback_data,
    }))
}

/// Data-only recovery keeps the active release. Require matching schema files
/// and exact current runtime identities, not merely the availability of old IDs.
pub async fn validate_data_only_restore(
    manager: &DockerManager,
    archive: &Path,
    target_root: &Path,
) -> Result<()> {
    let Some(snapshot) = stage_release_restore(archive, target_root, false)? else {
        return Ok(());
    };
    if !snapshot.contains_cold_data {
        bail!("Backup has no cold data files; refusing an empty data-only restore");
    }
    let candidate = DockerManager::with_project(
        snapshot.root().join(COMPOSE_FILE),
        manager.get_env_file().to_path_buf(),
        manager.project_name.clone(),
    )?;
    require_mysql_data_files(
        &snapshot,
        &validate_mysql_data_layout(&candidate, snapshot.root())?,
    )?;
    validate_restore_runtime(manager, &snapshot).await?;
    let selected = compose_images(manager).await?;
    let saved: BTreeSet<_> = snapshot
        .context
        .runtime_images
        .iter()
        .map(|v| v.reference.clone())
        .collect();
    if selected != saved {
        bail!(
            "Data-only restore requires the backup's active runtime references; use release-aware rollback instead"
        );
    }
    for image in &snapshot.context.runtime_images {
        let current = inspect_runtime(manager, &image.reference).await?;
        if current.image_id != image.image_id
            || current.os != image.os
            || current.architecture != image.architecture
        {
            bail!(
                "Data-only restore requires the exact backup runtimes to remain active; use release-aware rollback instead"
            );
        }
    }
    Ok(())
}

/// Before stopping the current stack, ensure the old Compose still resolves to
/// exactly the saved refs with the user's current env and all old IDs are local.
pub async fn validate_restore_runtime(
    manager: &DockerManager,
    snapshot: &ReleaseSnapshot,
) -> Result<()> {
    let delivery = load_delivery(snapshot.root())?;
    let schema = crate::mysql_manifest::parse_schema_manifest(&fs::read_to_string(
        snapshot.root().join(&delivery.mysql.manifest),
    )?)?;
    crate::mysql_manifest::validate_manifest_files(&schema, snapshot.root())?;
    preflight::validate_initdb_mount_contract_at(
        &snapshot.root().join(COMPOSE_FILE),
        snapshot.root(),
        &schema,
    )?;
    preflight::preflight_deploy_config_at(
        &snapshot.root().join(COMPOSE_FILE),
        manager.get_env_file(),
        snapshot.root(),
        &schema.database_names(),
        Some(&schema),
        Some(&delivery),
    )
    .context("The backup release is not compatible with the preserved user environment")?;
    let candidate = DockerManager::with_project(
        snapshot.root().join(COMPOSE_FILE),
        manager.get_env_file().to_path_buf(),
        manager.project_name.clone(),
    )?;
    let mysql_data = validate_mysql_data_layout(&candidate, snapshot.root())?;
    if snapshot.rollback_data {
        require_mysql_data_files(snapshot, &mysql_data)?;
    } else {
        require_unchanged_mysql_bind(manager, &mysql_data)?;
    }
    let references = compose_images(&candidate).await?;
    let expected: BTreeSet<_> = snapshot
        .context
        .runtime_images
        .iter()
        .map(|v| v.reference.clone())
        .collect();
    if references != expected {
        bail!(
            "Preserved environment changes the backup's runtime image refs; restore matching configuration before rollback"
        );
    }
    for saved in &snapshot.context.runtime_images {
        let actual = inspect_runtime(manager, &saved.image_id).await?;
        if actual.image_id != saved.image_id
            || actual.os != saved.os
            || actual.architecture != saved.architecture
        {
            bail!(
                "Saved runtime image identity or architecture does not match the locally available image"
            );
        }
        if saved.reference.contains('@') {
            let pinned = inspect_runtime(manager, &saved.reference).await?;
            if pinned.image_id != saved.image_id {
                bail!(
                    "Saved immutable image reference is unavailable or resolves to another image"
                );
            }
        }
    }
    Ok(())
}

fn require_mysql_data_files(snapshot: &ReleaseSnapshot, mysql_data: &str) -> Result<()> {
    let prefix = format!("{mysql_data}/");
    if !snapshot
        .cold_data_paths
        .iter()
        .any(|path| path.starts_with(&prefix))
    {
        bail!(
            "Backup does not contain cold files for the declared MySQL data bind; refusing to erase current data"
        );
    }
    Ok(())
}

fn require_unchanged_mysql_bind(current: &DockerManager, saved_mysql_data: &str) -> Result<()> {
    let current_root = deployment_root(current)?;
    let current_mysql_data = validate_mysql_data_layout(current, &current_root)?;
    if current_mysql_data != saved_mysql_data {
        bail!(
            "Rollback that retains current data cannot change the MySQL data bind; use --rollback-data with a matching release cold backup rather than switching to old or empty database storage"
        );
    }
    Ok(())
}

/// Called only after the current stack has stopped. No registry pull is needed.
pub async fn restore_runtime_tags(
    manager: &DockerManager,
    snapshot: &ReleaseSnapshot,
) -> Result<()> {
    for image in &snapshot.context.runtime_images {
        if image.reference.contains('@') || is_digest(&image.reference) {
            continue;
        }
        let output = manager
            .run_docker_command(&["tag", &image.image_id, &image.reference])
            .await?;
        if !output.status.success() {
            bail!("Could not restore the saved runtime image tag; services remain stopped");
        }
    }
    Ok(())
}

fn remove_path(path: &Path) -> Result<()> {
    match fs::symlink_metadata(path) {
        Ok(metadata) if metadata.is_dir() && !metadata.file_type().is_symlink() => {
            fs::remove_dir_all(path)?
        }
        Ok(_) => fs::remove_file(path)?,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => return Err(error.into()),
    }
    Ok(())
}

/// Replace only the frozen release roots; .env and data are never touched here.
/// On a rename failure, put the previous roots back before returning an error.
pub fn apply_release_snapshot(snapshot: &ReleaseSnapshot, target_root: &Path) -> Result<()> {
    let previous = tempfile::tempdir_in(target_root.parent().unwrap_or(Path::new(".")))?;
    let mut changed: Vec<&str> = Vec::new();
    for root in &snapshot.context.roots {
        let target = target_root.join(root);
        let old = previous.path().join(root);
        let outcome = (|| -> Result<()> {
            if fs::symlink_metadata(&target).is_ok() {
                fs::rename(&target, &old)?;
            }
            changed.push(root);
            fs::rename(snapshot.root().join(root), &target)?;
            Ok(())
        })();
        if let Err(error) = outcome {
            for root in changed.into_iter().rev() {
                let target = target_root.join(root);
                remove_path(&target)?;
                let old = previous.path().join(root);
                if fs::symlink_metadata(&old).is_ok() {
                    fs::rename(&old, &target)?;
                }
            }
            return Err(error)
                .context("Release restore failed; previous release files were restored");
        }
    }
    Ok(())
}

#[cfg(test)]
pub(crate) mod release_snapshot_tests {
    use super::*;
    use flate2::write::GzEncoder;

    /// Fixed old-GNU sparse format, independent of host filesystem support and
    /// of Builder's automatic sparse detection (including Windows/macOS).
    pub(crate) fn append_sparse_member<W: Write>(
        archive: &mut tar::Builder<W>,
        name: &str,
        logical_size: u64,
        extents: &[(u64, u64)],
        stored: &[u8],
    ) -> Result<()> {
        let mut header = tar::Header::new_gnu();
        header.set_entry_type(tar::EntryType::GNUSparse);
        header.set_mode(0o600);
        header.set_size(stored.len() as u64);
        let gnu = header
            .as_gnu_mut()
            .ok_or_else(|| anyhow!("Missing GNU fixture header"))?;
        gnu.set_real_size(logical_size);
        for (block, &(offset, length)) in gnu.sparse.iter_mut().zip(extents) {
            block.set_offset(offset);
            block.set_length(length);
        }
        header.set_cksum();
        archive.append_data(&mut header, name, stored)?;
        Ok(())
    }

    fn append_snapshot<W: Write>(
        archive: &mut tar::Builder<W>,
        snapshot: &ReleaseSnapshot,
    ) -> Result<()> {
        for name in snapshot
            .context
            .files
            .keys()
            .chain(std::iter::once(&RELEASE_CONTEXT_FILE.to_string()))
        {
            archive.append_path_with_name(snapshot.root().join(name), name)?;
        }
        Ok(())
    }

    pub(crate) fn write_release(
        root: &Path,
        version: &str,
        schema: &str,
    ) -> Result<Vec<RuntimeImage>> {
        write_release_with_bind(root, version, schema, "./data/mysql")
    }

    fn write_release_with_bind(
        root: &Path,
        version: &str,
        schema: &str,
        mysql_bind: &str,
    ) -> Result<Vec<RuntimeImage>> {
        fs::create_dir_all(root.join("im-app"))?;
        fs::create_dir_all(root.join("repo-collab-app/dist"))?;
        fs::create_dir_all(root.join("config"))?;
        fs::create_dir_all(root.join("app"))?;
        fs::write(root.join(".env"), "USER_KEY=original\n")?;
        fs::write(
            root.join(COMPOSE_FILE),
            "services:\n  mysql:\n    image: unit/mysql:latest\n    volumes:\n      - ./data/mysql:/var/lib/mysql\n  im:\n    image: unit/im:latest\n    volumes:\n      - ./im-app/nuwax-im-web-bootstrap.jar:/app/web.jar\n      - ./im-app/nuwax-im-gateway-bootstrap.jar:/app/gateway.jar\n  collab:\n    image: unit/collab:latest\n    volumes:\n      - ./repo-collab-app/dist:/app/dist\n".replace("./data/mysql:/var/lib/mysql", &format!("{mysql_bind}:/var/lib/mysql")),
        )?;
        fs::write(
            root.join("config/schema.sql"),
            format!("USE agent_platform; CREATE TABLE sample (id INT); -- {schema}\n"),
        )?;
        fs::write(
            root.join("config/bootstrap.sql"),
            "CREATE DATABASE IF NOT EXISTS `agent_platform` CHARACTER SET utf8mb4 COLLATE utf8mb4_unicode_ci;\n",
        )?;
        fs::write(root.join("config/permissions.sh"), "#!/bin/sh\nexit 0\n")?;
        fs::write(
            root.join("config/mysql-schema-manifest.json"),
            r#"{
          "contract_version":1,"requires":{"cli_capability":"mysql-schema-manifest-v1"},
          "mysql_target":{"service":"mysql","internal_port":3306},"application_connections":[],
          "databases":[{"name":"agent_platform","bootstrap_only":false}],
          "bootstrap":{"path":"config/bootstrap.sql","idempotent":true,"initdb_target":"00_bootstrap.sql"},
          "permissions":{"path":"config/permissions.sh","user_env":"MYSQL_USER","databases":"bootstrap","initdb_target":"01_permissions.sh"},
          "schemas":[{"database":"agent_platform","path":"config/schema.sql","initdb_target":"10_schema.sql"}],
          "first_install_seeds":[]
        }"#,
        )?;
        fs::write(
            root.join("im-app/nuwax-im-web-bootstrap.jar"),
            format!("web-{version}"),
        )?;
        fs::write(
            root.join("im-app/nuwax-im-gateway-bootstrap.jar"),
            format!("gateway-{version}"),
        )?;
        fs::write(
            root.join("repo-collab-app/dist/index.js"),
            format!("entry-{version}"),
        )?;
        let image_id = format!(
            "sha256:{}",
            if version == "A" {
                "1".repeat(64)
            } else {
                "2".repeat(64)
            }
        );
        let collab_id = format!(
            "sha256:{}",
            if version == "A" {
                "3".repeat(64)
            } else {
                "4".repeat(64)
            }
        );
        let im_files = [
            "im-app/nuwax-im-web-bootstrap.jar",
            "im-app/nuwax-im-gateway-bootstrap.jar",
        ]
        .into_iter()
        .map(|path| Ok((path.to_string(), file_hash(&root.join(path))?)))
        .collect::<Result<BTreeMap<_, _>>>()?;
        let collab_files = BTreeMap::from([(
            "repo-collab-app/dist/index.js".to_string(),
            file_hash(&root.join("repo-collab-app/dist/index.js"))?,
        )]);
        let mysql_files = [
            "config/bootstrap.sql",
            "config/permissions.sh",
            "config/schema.sql",
            "config/mysql-schema-manifest.json",
        ]
        .into_iter()
        .map(|path| Ok((path.to_string(), file_hash(&root.join(path))?)))
        .collect::<Result<BTreeMap<_, _>>>()?;
        let mut payload = serde_json::json!({
            "contract_version": 1,
            "architecture": "amd64",
            "components": {
                "nuwax-im": {"version":version,"image_id":image_id,"config_id":image_id,"source":image_id,"target":"unit/im:latest","artifacts":im_files},
                "repo-collab": {"version":version,"image_id":collab_id,"config_id":collab_id,"source":collab_id,"target":"unit/collab:latest","artifacts":collab_files}
            },
            "mysql": {"manifest":"config/mysql-schema-manifest.json","files":mysql_files},
            "compose": {"path":COMPOSE_FILE,"sha256":file_hash(&root.join(COMPOSE_FILE))?}
        });
        let release_hash: String = Sha256::digest(serde_json::to_vec(&payload)?)
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect();
        payload["release_sha256"] = serde_json::Value::String(release_hash);
        fs::write(
            root.join(DELIVERY_FILE),
            serde_json::to_vec_pretty(&payload)?,
        )?;
        Ok(vec![
            RuntimeImage {
                reference: "unit/mysql:latest".to_string(),
                image_id: format!("sha256:{}", "6".repeat(64)),
                os: "linux".to_string(),
                architecture: "amd64".to_string(),
            },
            RuntimeImage {
                reference: "unit/im:latest".to_string(),
                image_id,
                os: "linux".to_string(),
                architecture: "amd64".to_string(),
            },
            RuntimeImage {
                reference: "unit/collab:latest".to_string(),
                image_id: collab_id,
                os: "linux".to_string(),
                architecture: "amd64".to_string(),
            },
        ])
    }

    fn write_archive(snapshot: &ReleaseSnapshot, path: &Path, with_data: bool) -> Result<()> {
        let encoder = GzEncoder::new(fs::File::create(path)?, flate2::Compression::fast());
        let mut archive = tar::Builder::new(encoder);
        append_snapshot(&mut archive, snapshot)?;
        if with_data {
            let mut header = tar::Header::new_gnu();
            header.set_size(4);
            header.set_mode(0o600);
            header.set_cksum();
            archive.append_data(&mut header, "data/mysql/cold-test", &b"cold"[..])?;
        }
        archive.into_inner()?.finish()?;
        Ok(())
    }

    #[test]
    fn sparse_cold_data_with_only_holes_is_nonempty_and_keeps_metadata_strict() -> Result<()> {
        let temp = tempfile::tempdir()?;
        let root = temp.path().join("deployment");
        let snapshot = capture_release_files(&root, "A", write_release(&root, "A", "schema")?)?;
        let path = temp.path().join("sparse.tar.gz");
        let encoder = GzEncoder::new(fs::File::create(&path)?, flate2::Compression::fast());
        let mut archive = tar::Builder::new(encoder);
        append_snapshot(&mut archive, &snapshot)?;
        append_sparse_member(
            &mut archive,
            "data/mysql/cold-test",
            8192,
            &[(8192, 0)],
            &[],
        )?;
        archive.into_inner()?.finish()?;
        let staged = stage_release_restore(&path, &root, true)?.expect("sparse cold release");
        require_mysql_data_files(&staged, "data/mysql")?;
        preflight::verify_backup_component_compatibility(&path, &root)?;

        // An S header carrying JSON still fails: only cold/artifact files gain
        // sparse semantics, not the bounded release metadata format.
        for name in [RELEASE_CONTEXT_FILE, DELIVERY_FILE] {
            let bytes = fs::read(snapshot.root().join(name))?;
            let encoder = GzEncoder::new(fs::File::create(&path)?, flate2::Compression::fast());
            let mut archive = tar::Builder::new(encoder);
            append_sparse_member(
                &mut archive,
                name,
                bytes.len() as u64,
                &[(0, bytes.len() as u64)],
                &bytes,
            )?;
            archive.into_inner()?.finish()?;
            assert!(stage_release_restore(&path, &root, true).is_err());
            if name == DELIVERY_FILE {
                assert!(preflight::verify_backup_component_compatibility(&path, &root).is_err());
            }
        }
        Ok(())
    }

    #[test]
    fn malformed_sparse_maps_and_unsafe_sparse_paths_fail_before_restore() -> Result<()> {
        let temp = tempfile::tempdir()?;
        let root = temp.path().join("legacy");
        fs::create_dir(&root)?;
        let path = temp.path().join("invalid.tar.gz");
        for (logical_size, extents, stored) in [
            (1024, vec![(0, 512), (256, 512)], vec![1; 1024]), // overlapping extents
            (512, vec![(1024, 512)], vec![1; 512]),            // beyond declared logical size
            (512, vec![(0, 1024)], vec![1; 512]),              // more stored bytes than payload
            (513, vec![(0, 1), (512, 1)], vec![1; 2]),         // unaligned stored extent
            (u64::MAX, vec![(u64::MAX, 0)], vec![]), // unsupported signed filesystem offset
        ] {
            let encoder = GzEncoder::new(fs::File::create(&path)?, flate2::Compression::fast());
            let mut archive = tar::Builder::new(encoder);
            append_sparse_member(
                &mut archive,
                "data/mysql/cold-test",
                logical_size,
                &extents,
                &stored,
            )?;
            archive.into_inner()?.finish()?;
            assert!(stage_release_restore(&path, &root, true).is_err());
        }
        for name in [
            "../outside",
            "data/mysql/cold-test/",
            "data/mysql/cold-test",
        ] {
            let encoder = GzEncoder::new(fs::File::create(&path)?, flate2::Compression::fast());
            let mut archive = tar::Builder::new(encoder);
            let mut header = tar::Header::new_gnu();
            header.set_entry_type(tar::EntryType::GNUSparse);
            header.set_mode(0o600);
            header.set_size(0);
            let gnu = header.as_gnu_mut().expect("GNU fixture");
            gnu.set_real_size(8192);
            gnu.sparse[0].set_offset(8192);
            gnu.sparse[0].set_length(0);
            // Raw path bytes allow the unsafe names that Builder rejects.
            header.as_mut_bytes()[..name.len()].copy_from_slice(name.as_bytes());
            header.set_cksum();
            archive.append(&header, std::io::empty())?;
            if name == "data/mysql/cold-test" {
                append_sparse_member(&mut archive, name, 8192, &[(8192, 0)], &[])?;
            }
            archive.into_inner()?.finish()?;
            assert!(stage_release_restore(&path, &root, true).is_err());
        }
        let mut complete = tar::Builder::new(Vec::new());
        append_sparse_member(
            &mut complete,
            "data/mysql/cold-test",
            512,
            &[(0, 512)],
            &[1; 512],
        )?;
        let bytes = complete.into_inner()?;
        let mut truncated = GzEncoder::new(fs::File::create(&path)?, flate2::Compression::fast());
        truncated.write_all(&bytes[..512 + 100])?;
        truncated.finish()?;
        assert!(stage_release_restore(&path, &root, true).is_err());
        Ok(())
    }

    #[test]
    fn sparse_release_artifact_hashes_cover_zero_holes_and_reject_tampering() -> Result<()> {
        let temp = tempfile::tempdir()?;
        let root = temp.path().join("deployment");
        let images = write_release(&root, "A", "schema")?;
        let name = "repo-collab-app/dist/index.js";
        let mut logical = vec![0; 16 * 1024 + 512];
        logical[..512].fill(1);
        logical[16 * 1024..].fill(2);
        fs::write(root.join(name), &logical)?;
        let mut receipt: serde_json::Value =
            serde_json::from_slice(&fs::read(root.join(DELIVERY_FILE))?)?;
        receipt["components"]["repo-collab"]["artifacts"][name] =
            serde_json::Value::String(file_hash(&root.join(name))?);
        receipt
            .as_object_mut()
            .expect("fixture receipt")
            .remove("release_sha256");
        let hash: String = Sha256::digest(serde_json::to_vec(&receipt)?)
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect();
        receipt["release_sha256"] = serde_json::Value::String(hash);
        fs::write(root.join(DELIVERY_FILE), serde_json::to_vec(&receipt)?)?;
        let snapshot = capture_release_files(&root, "A", images)?;
        let path = temp.path().join("sparse-artifact.tar.gz");
        for tampered in [false, true] {
            let encoder = GzEncoder::new(fs::File::create(&path)?, flate2::Compression::fast());
            let mut archive = tar::Builder::new(encoder);
            for file in snapshot
                .context
                .files
                .keys()
                .filter(|file| *file != name)
                .chain(std::iter::once(&RELEASE_CONTEXT_FILE.to_string()))
            {
                archive.append_path_with_name(snapshot.root().join(file), file)?;
            }
            let mut stored = logical[..512].to_vec();
            stored.extend_from_slice(&logical[16 * 1024..]);
            if tampered {
                stored[0] ^= 1;
            }
            append_sparse_member(
                &mut archive,
                name,
                logical.len() as u64,
                &[(0, 512), (16 * 1024, 512)],
                &stored,
            )?;
            archive.into_inner()?.finish()?;
            let staged = stage_release_restore(&path, &root, false);
            let compatible = preflight::verify_backup_component_compatibility(&path, &root);
            if tampered {
                assert!(staged.is_err());
                assert!(compatible.is_err());
            } else {
                let staged = staged?.expect("sparse artifact release");
                assert_eq!(fs::read(staged.root().join(name))?, logical);
                compatible?;
            }
        }
        Ok(())
    }

    #[test]
    fn release_snapshot_roundtrip_restores_receipt_keeps_env_data_and_empty_app() -> Result<()> {
        let temp = tempfile::tempdir()?;
        let root = temp.path().join("deployment");
        let images = write_release(&root, "A", "same-schema")?;
        let original_receipt = fs::read(root.join(DELIVERY_FILE))?;
        let snapshot = capture_release_files(&root, "A", images)?;
        assert!(
            !snapshot
                .context
                .files
                .keys()
                .any(|path| path == ".env" || path.starts_with("data/"))
        );
        let archive = temp.path().join("A.tar.gz");
        write_archive(&snapshot, &archive, true)?;
        write_release(&root, "B", "same-schema")?;
        fs::write(root.join(".env"), "USER_KEY=keep-current\n")?;
        fs::create_dir_all(root.join("data/mysql"))?;
        fs::write(root.join("data/mysql/current"), b"keep-current-data")?;
        let restored = stage_release_restore(&archive, &root, false)?.expect("release context");
        assert!(restored.root().join("app").is_dir());
        apply_release_snapshot(&restored, &root)?;
        assert_eq!(fs::read(root.join(DELIVERY_FILE))?, original_receipt);
        assert_eq!(fs::read(root.join(".env"))?, b"USER_KEY=keep-current\n");
        assert_eq!(
            fs::read(root.join("data/mysql/current"))?,
            b"keep-current-data"
        );
        assert_eq!(
            fs::read(root.join("im-app/nuwax-im-web-bootstrap.jar"))?,
            b"web-A"
        );
        preflight::verify_delivery_manifest(&load_delivery(&root)?, &root, Some("amd64"))?;
        Ok(())
    }

    #[test]
    fn release_snapshot_rejects_schema_change_without_data_rollback() -> Result<()> {
        let temp = tempfile::tempdir()?;
        let root = temp.path().join("deployment");
        let snapshot = capture_release_files(&root, "A", write_release(&root, "A", "schema-A")?)?;
        let archive = temp.path().join("A.tar.gz");
        write_archive(&snapshot, &archive, true)?;
        write_release(&root, "B", "schema-B")?;
        let error = stage_release_restore(&archive, &root, false)
            .err()
            .expect("schema guard");
        assert!(error.to_string().contains("--rollback-data"));
        assert_eq!(
            fs::read(root.join("im-app/nuwax-im-web-bootstrap.jar"))?,
            b"web-B"
        );
        assert!(stage_release_restore(&archive, &root, true)?.is_some());
        Ok(())
    }

    #[test]
    fn release_snapshot_missing_context_or_data_and_tampering_fail_before_restore() -> Result<()> {
        let temp = tempfile::tempdir()?;
        let root = temp.path().join("deployment");
        let snapshot = capture_release_files(&root, "A", write_release(&root, "A", "schema")?)?;
        let archive = temp.path().join("A.tar.gz");
        write_archive(&snapshot, &archive, false)?;
        assert!(stage_release_restore(&archive, &root, true).is_err());
        fs::write(
            snapshot.root().join("im-app/nuwax-im-web-bootstrap.jar"),
            b"tampered",
        )?;
        write_archive(&snapshot, &archive, false)?;
        assert!(stage_release_restore(&archive, &root, false).is_err());
        let encoder = GzEncoder::new(fs::File::create(&archive)?, flate2::Compression::fast());
        let mut old = tar::Builder::new(encoder);
        old.append_path_with_name(
            root.join("im-app/nuwax-im-web-bootstrap.jar"),
            "im-app/nuwax-im-web-bootstrap.jar",
        )?;
        old.into_inner()?.finish()?;
        assert!(stage_release_restore(&archive, &root, false).is_err());
        assert_eq!(
            fs::read(root.join("im-app/nuwax-im-web-bootstrap.jar"))?,
            b"web-A"
        );
        Ok(())
    }

    #[test]
    fn release_snapshot_context_cannot_overwrite_env_or_record_another_component_runtime()
    -> Result<()> {
        let temp = tempfile::tempdir()?;
        let root = temp.path().join("deployment");
        let mut images = write_release(&root, "A", "schema")?;
        images
            .iter_mut()
            .find(|image| image.reference == "unit/im:latest")
            .ok_or_else(|| anyhow!("Missing fixture component image"))?
            .image_id = format!("sha256:{}", "5".repeat(64));
        assert!(capture_release_files(&root, "A", images).is_err());
        let snapshot = capture_release_files(&root, "A", write_release(&root, "A", "schema")?)?;
        let mut context = snapshot.context.clone();
        context.roots.push(".env".to_string());
        assert!(validate_context(&context).is_err());
        let custom =
            DockerManager::with_project(root.join("compose-custom.yml"), root.join(".env"), None)?;
        assert!(deployment_root(&custom).is_err());
        let selected =
            DockerManager::with_project(root.join(COMPOSE_FILE), root.join(".env"), None)?;
        assert_eq!(deployment_root(&selected)?, root.canonicalize()?);
        let external_env = temp.path().join("independent.env");
        fs::write(&external_env, "USER_KEY=independent\n")?;
        let independent = DockerManager::with_project(root.join(COMPOSE_FILE), external_env, None)?;
        assert_eq!(deployment_root(&independent)?, root.canonicalize()?);
        let unsafe_env = root.join("config/credentials.env");
        fs::write(&unsafe_env, "USER_KEY=unsafe-placement\n")?;
        let unsafe_manager =
            DockerManager::with_project(root.join(COMPOSE_FILE), unsafe_env, None)?;
        assert!(deployment_root(&unsafe_manager).is_err());
        Ok(())
    }

    #[test]
    fn release_snapshot_mysql_data_layout_covers_custom_bind_and_rejects_named_external()
    -> Result<()> {
        let temp = tempfile::tempdir()?;
        let root = temp.path().join("deployment");
        write_release(&root, "A", "schema")?;
        let manager =
            DockerManager::with_project(root.join(COMPOSE_FILE), root.join(".env"), None)?;
        let base = fs::read_to_string(root.join(COMPOSE_FILE))?;
        assert_eq!(validate_mysql_data_layout(&manager, &root)?, "data/mysql");
        fs::write(
            root.join(COMPOSE_FILE),
            base.replace(
                "./data/mysql:/var/lib/mysql",
                "./data/custom-mysql:/var/lib/mysql",
            ),
        )?;
        manager.invalidate_compose_config_cache();
        assert_eq!(
            validate_mysql_data_layout(&manager, &root)?,
            "data/custom-mysql"
        );
        fs::write(root.join(COMPOSE_FILE), base.replace("      - ./data/mysql:/var/lib/mysql", "      - type: bind\n        source: ./data/custom-mysql\n        target: /var/lib/mysql"))?;
        manager.invalidate_compose_config_cache();
        assert_eq!(
            validate_mysql_data_layout(&manager, &root)?,
            "data/custom-mysql"
        );
        fs::write(
            root.join(COMPOSE_FILE),
            base.replace("./data/mysql:/var/lib/mysql", "mysql-store:/var/lib/mysql"),
        )?;
        manager.invalidate_compose_config_cache();
        assert!(validate_mysql_data_layout(&manager, &root).is_err());
        let external = temp.path().join("external-mysql");
        fs::create_dir_all(&external)?;
        fs::write(
            root.join(COMPOSE_FILE),
            base.replace(
                "./data/mysql:/var/lib/mysql",
                &format!("{}:/var/lib/mysql", external.display()),
            ),
        )?;
        manager.invalidate_compose_config_cache();
        assert!(validate_mysql_data_layout(&manager, &root).is_err());
        let snapshot = capture_release_files(&root, "A", write_release(&root, "A", "schema")?)?;
        let archive = temp.path().join("A.tar.gz");
        write_archive(&snapshot, &archive, true)?;
        let staged = stage_release_restore(&archive, &root, true)?.expect("staged release");
        require_mysql_data_files(&staged, "data/mysql")?;
        assert!(require_mysql_data_files(&staged, "data/custom-mysql").is_err());
        Ok(())
    }

    #[test]
    fn release_snapshot_retaining_data_rejects_changed_bind_with_identical_schema_runtime()
    -> Result<()> {
        let temp = tempfile::tempdir()?;
        let saved_root = temp.path().join("saved");
        let current_root = temp.path().join("current");
        let saved_images = write_release(&saved_root, "A", "same-schema")?;
        let current_images =
            write_release_with_bind(&current_root, "A", "same-schema", "./data/custom-mysql")?;
        assert_eq!(saved_images, current_images);
        assert_eq!(
            load_delivery(&saved_root)?.mysql.files,
            load_delivery(&current_root)?.mysql.files
        );
        let saved = DockerManager::with_project(
            saved_root.join(COMPOSE_FILE),
            saved_root.join(".env"),
            None,
        )?;
        let current = DockerManager::with_project(
            current_root.join(COMPOSE_FILE),
            current_root.join(".env"),
            None,
        )?;
        let saved_bind = validate_mysql_data_layout(&saved, &saved_root)?;
        let error = require_unchanged_mysql_bind(&current, &saved_bind)
            .expect_err("changed bind must fail");
        assert!(error.to_string().contains("--rollback-data"));
        assert_eq!(fs::read(current_root.join(".env"))?, b"USER_KEY=original\n");
        write_release(&current_root, "A", "same-schema")?;
        current.invalidate_compose_config_cache();
        require_unchanged_mysql_bind(&current, &saved_bind)?;
        Ok(())
    }

    #[test]
    fn release_snapshot_rejects_data_links_and_duplicate_paths_before_any_restore() -> Result<()> {
        let temp = tempfile::tempdir()?;
        let root = temp.path().join("deployment");
        let images = write_release(&root, "A", "schema")?;
        fs::create_dir_all(root.join("data/mysql"))?;
        fs::write(root.join("data/mysql/current"), b"current-database")?;
        let external_directory = temp.path().join("outside");
        fs::create_dir_all(&external_directory)?;
        let external = external_directory.join("sentinel");
        fs::write(&external, b"outside-unchanged")?;
        let snapshot = capture_release_files(&root, "A", images)?;
        for kind in [
            tar::EntryType::Symlink,
            tar::EntryType::Link,
            tar::EntryType::Char,
            tar::EntryType::Block,
            tar::EntryType::Fifo,
            tar::EntryType::Regular,
        ] {
            let path = temp.path().join("unsafe-data.tar.gz");
            let encoder = GzEncoder::new(fs::File::create(&path)?, flate2::Compression::fast());
            let mut archive = tar::Builder::new(encoder);
            for name in snapshot
                .context
                .files
                .keys()
                .chain(std::iter::once(&RELEASE_CONTEXT_FILE.to_string()))
            {
                archive.append_path_with_name(snapshot.root().join(name), name)?;
            }
            let mut cold = tar::Header::new_gnu();
            cold.set_size(4);
            cold.set_mode(0o600);
            cold.set_cksum();
            archive.append_data(&mut cold, "data/mysql/cold-test", &b"cold"[..])?;
            let mut extra = tar::Header::new_gnu();
            extra.set_entry_type(kind);
            extra.set_mode(0o600);
            if kind.is_file() {
                extra.set_size(4);
                extra.set_cksum();
                archive.append_data(&mut extra, "data/mysql/cold-test", &b"evil"[..])?;
            } else {
                extra.set_size(0);
                let link_target = if kind.is_symlink() {
                    "../../../outside"
                } else {
                    "../../../outside/sentinel"
                };
                extra.set_link_name(link_target)?;
                extra.set_cksum();
                archive.append_data(&mut extra, "data/mysql/link", std::io::empty())?;
                if kind.is_symlink() {
                    let mut follower = tar::Header::new_gnu();
                    follower.set_size(4);
                    follower.set_mode(0o600);
                    follower.set_cksum();
                    archive.append_data(&mut follower, "data/mysql/link/sentinel", &b"evil"[..])?;
                }
            }
            archive.into_inner()?.finish()?;
            assert!(stage_release_restore(&path, &root, true).is_err());
            assert_eq!(fs::read(&external)?, b"outside-unchanged");
            assert_eq!(
                fs::read(root.join("data/mysql/current"))?,
                b"current-database"
            );
        }
        Ok(())
    }
}
