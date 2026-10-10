//! # 下载模块
//!
//! 提供统一的文件下载接口，支持：
//! - 普通 HTTP 下载
//! - 阿里云 OSS 公网文件下载（扩展超时）
//! - **断点续传下载** ⭐
//! - 进度回调和监控
//! - 文件完整性验证
//! - 智能缓存和断点续传
//!
//! ## 主要特性
//!
//! ### 智能下载策略
//! - 自动检测下载方式（HTTP/扩展超时HTTP）
//! - 支持阿里云 OSS 大文件下载（公网访问）
//! - 扩展超时时间避免大文件下载失败
//! - **智能断点续传** - 自动检测已下载部分，从中断点继续
//!
//! ### 进度监控
//! - 实时下载进度回调
//! - 下载速度计算
//! - 剩余时间估算
//!
//! ### 文件完整性
//! - SHA-256 哈希验证
//! - 损坏文件自动重试
//! - 完整性校验缓存
//!
//! ### 断点续传
//! - HTTP Range 请求支持
//! - 自动检测已下载部分
//! - 智能文件完整性验证
//! - 支持大文件下载恢复

use crate::error::DuckError;
use anyhow::{Context, Result};
use chrono;
use futures::stream::StreamExt;
use reqwest::Client;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::path::Path;
use std::time::Duration;
use tokio::fs::{File, OpenOptions};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tracing::{info, warn};

fn parse_content_range(value: &str) -> Option<(u64, u64, u64)> {
    let (range, total) = value.strip_prefix("bytes ")?.split_once('/')?;
    let (start, end) = range.split_once('-')?;
    Some((start.parse().ok()?, end.parse().ok()?, total.parse().ok()?))
}

fn validate_response_resource(
    response: &reqwest::Response,
    metadata: &DownloadMetadata,
) -> Result<()> {
    let Some(expected) = metadata.validator.as_deref() else {
        return Ok(());
    };
    let header = metadata.validator_header.as_deref().unwrap_or_else(|| {
        if expected.starts_with('"') {
            "etag"
        } else {
            "last-modified"
        }
    });
    let actual = response
        .headers()
        .get(header)
        .and_then(|value| value.to_str().ok());
    if actual != Some(expected) {
        return Err(anyhow::anyhow!(
            "Download response resource validator differs from HEAD"
        ));
    }
    Ok(())
}

fn hash_digest(hasher: &Sha256) -> String {
    hasher
        .clone()
        .finalize()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

async fn hash_file_prefix(path: &Path, length: u64) -> Result<Sha256> {
    let mut file = File::open(path).await?;
    let mut remaining = length;
    let mut buffer = vec![0u8; 1024 * 1024];
    let mut hasher = Sha256::new();
    while remaining > 0 {
        let count = file
            .read(&mut buffer[..remaining.min(1024 * 1024) as usize])
            .await?;
        if count == 0 {
            return Err(anyhow::anyhow!(
                "Partial download is shorter than its integrity checkpoint"
            ));
        }
        hasher.update(&buffer[..count]);
        remaining -= count as u64;
    }
    Ok(hasher)
}

struct ResumeCheckpoint {
    size: u64,
    hasher: Sha256,
}

/// 下载进度状态枚举
#[derive(Debug, Clone)]
pub enum DownloadStatus {
    Starting,
    Downloading,
    Resuming, // 断点续传状态 ⭐
    Paused,
    Completed,
    Failed(String),
}

/// 下载进度信息
#[derive(Debug, Clone)]
pub struct DownloadProgress {
    pub task_id: String,
    pub file_name: String,
    pub downloaded_bytes: u64,
    pub total_bytes: u64,
    pub download_speed: f64, // bytes/sec
    pub eta_seconds: u64,
    pub percentage: f64,
    pub status: DownloadStatus,
}

/// 下载任务元数据 ⭐
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DownloadMetadata {
    pub url: String,
    pub expected_size: u64,
    pub expected_hash: Option<String>,
    #[serde(default)]
    pub validator: Option<String>,
    #[serde(default)]
    pub validator_header: Option<String>,
    #[serde(default)]
    pub partial_sha256: Option<String>,
    #[serde(default)]
    pub completed: bool,
    pub downloaded_bytes: u64,
    pub start_time: String,
    pub last_update: String,
    pub version: String, // 下载任务版本，用于区分不同的下载
}

impl DownloadMetadata {
    /// 创建新的下载元数据
    pub fn new(
        url: String,
        expected_size: u64,
        expected_hash: Option<String>,
        version: String,
    ) -> Self {
        let now = chrono::Utc::now().to_rfc3339();
        Self {
            url,
            expected_size,
            expected_hash,
            validator: None,
            validator_header: None,
            partial_sha256: Some(hash_digest(&Sha256::new())),
            completed: false,
            downloaded_bytes: 0,
            start_time: now.clone(),
            last_update: now,
            version,
        }
    }

    /// 更新下载进度
    pub fn update_progress(&mut self, downloaded_bytes: u64) {
        self.downloaded_bytes = downloaded_bytes;
        self.last_update = chrono::Utc::now().to_rfc3339();
    }

    /// 检查是否为相同的下载任务
    pub fn is_same_task(&self, url: &str, expected_size: u64, version: &str) -> bool {
        self.url == url && self.expected_size == expected_size && self.version == version
    }
}

/// 断点续传下载参数
struct ResumeDownloadParams<'a, F> {
    url: &'a str,
    download_path: &'a Path,
    progress_callback: Option<F>,
    existing_size: Option<u64>,
    prefix_hasher: Option<Sha256>,
    total_size: u64,
    task_id: &'a str,
    metadata: &'a mut DownloadMetadata,
}

/// 流式下载参数
struct StreamDownloadParams<'a, F> {
    response: reqwest::Response,
    file: &'a mut File,
    download_path: &'a Path,
    progress_callback: Option<F>,
    task_id: &'a str,
    start_byte: u64,
    prefix_hasher: Option<Sha256>,
    total_size: u64,
    is_resume: bool,
    metadata: &'a mut DownloadMetadata,
}

/// 下载器类型
#[derive(Debug, Clone)]
pub enum DownloaderType {
    Http,
    HttpExtendedTimeout,
}

/// 文件下载器配置
#[derive(Debug, Clone)]
pub struct DownloaderConfig {
    pub timeout_seconds: u64,
    pub chunk_size: usize,
    pub retry_count: u32,
    pub enable_progress_logging: bool,
    pub enable_resume: bool,            // 启用断点续传 ⭐
    pub resume_threshold: u64,          // 断点续传阈值（字节），小于此值的文件重新下载 ⭐
    pub progress_interval_seconds: u64, // 进度显示时间间隔（秒）⭐
    pub progress_bytes_interval: u64,   // 进度显示字节间隔 ⭐
    pub enable_metadata: bool,          // 启用元数据管理 ⭐
    pub metadata_checkpoint_bytes: u64,
    pub retain_completed_metadata: bool,
}

impl Default for DownloaderConfig {
    fn default() -> Self {
        Self {
            timeout_seconds: 60 * 60, // 60分钟
            chunk_size: 8192,         // 8KB
            retry_count: 3,
            enable_progress_logging: true,
            enable_resume: true,                        // 默认启用断点续传 ⭐
            resume_threshold: 1024 * 1024,              // 1MB，小于1MB的文件重新下载 ⭐
            progress_interval_seconds: 10,              // 每10秒显示一次进度 ⭐
            progress_bytes_interval: 100 * 1024 * 1024, // 每100MB显示一次进度 ⭐
            enable_metadata: true,                      // 默认启用元数据管理 ⭐
            metadata_checkpoint_bytes: 64 * 1024 * 1024,
            retain_completed_metadata: false,
        }
    }
}

/// 文件下载器
pub struct FileDownloader {
    config: DownloaderConfig,
    client: Client,
    custom_client: Option<Client>, // 支持自定义HTTP客户端（用于认证） ⭐
}

impl FileDownloader {
    /// 创建新的文件下载器
    pub fn new(config: DownloaderConfig) -> Self {
        let client = Client::builder()
            .timeout(Duration::from_secs(config.timeout_seconds))
            .user_agent(crate::constants::api::http::USER_AGENT) // 🆕 添加User-Agent ⭐
            .build()
            .expect("Failed to create HTTP client");

        Self {
            config,
            client,
            custom_client: None,
        }
    }

    /// 创建支持自定义HTTP客户端的下载器（用于认证场景）⭐
    pub fn new_with_custom_client(config: DownloaderConfig, custom_client: Client) -> Self {
        let fallback_client = Client::builder()
            .timeout(Duration::from_secs(config.timeout_seconds))
            .user_agent(crate::constants::api::http::USER_AGENT) // 🆕 添加User-Agent ⭐
            .build()
            .expect("Failed to create fallback HTTP client");

        Self {
            config,
            client: fallback_client,
            custom_client: Some(custom_client),
        }
    }

    /// 获取要使用的HTTP客户端（优先使用自定义客户端）⭐
    fn get_http_client(&self) -> &Client {
        self.custom_client.as_ref().unwrap_or(&self.client)
    }

    /// 创建默认配置的下载器
    pub fn with_default_config() -> Self {
        Self::new(DownloaderConfig::default())
    }

    /// 检查 URL 是否为阿里云 OSS 链接
    pub fn is_aliyun_oss_url(&self, url: &str) -> bool {
        url.starts_with("https://") && url.contains("aliyuncs.com") && url.contains("oss-")
    }

    /// 检查 URL 是否为对象存储或CDN服务 ⭐
    pub fn is_object_storage_or_cdn_url(&self, url: &str) -> bool {
        let url_lower = url.to_lowercase();

        // 阿里云OSS
        if url_lower.contains("aliyuncs.com") && url_lower.contains("oss-") {
            return true;
        }

        // 腾讯云COS
        if url_lower.contains("myqcloud.com") && url_lower.contains("cos.") {
            return true;
        }

        // 华为云OBS
        if url_lower.contains("myhuaweicloud.com") && url_lower.contains("obs.") {
            return true;
        }

        // AWS S3
        if url_lower.contains("amazonaws.com")
            && (url_lower.contains("s3.") || url_lower.contains(".s3-"))
        {
            return true;
        }

        // 七牛云
        if url_lower.contains("qiniudn.com")
            || url_lower.contains("clouddn.com")
            || url_lower.contains("qnssl.com")
        {
            return true;
        }

        // 又拍云
        if url_lower.contains("upaiyun.com") || url_lower.contains("upyun.com") {
            return true;
        }

        // 百度云BOS
        if url_lower.contains("bcebos.com") || url_lower.contains("baidubce.com") {
            return true;
        }

        // 京东云OSS
        if url_lower.contains("jdcloud.com") && url_lower.contains("oss.") {
            return true;
        }

        // 常见CDN服务
        if url_lower.contains("cloudfront.net") ||  // AWS CloudFront
           url_lower.contains("fastly.com") ||      // Fastly
           url_lower.contains("jsdelivr.net") ||    // jsDelivr
           url_lower.contains("unpkg.com") ||       // unpkg
           url_lower.contains("cdnjs.com") ||       // cdnjs
           url_lower.contains("bootcdn.cn") ||      // BootCDN
           url_lower.contains("staticfile.org")
        {
            // 静态文件CDN
            return true;
        }

        false
    }

    /// 判断下载器类型
    pub fn get_downloader_type(&self, url: &str) -> DownloaderType {
        if self.is_object_storage_or_cdn_url(url) {
            // 所有对象存储和CDN URL 都使用扩展超时 HTTP 下载（公网访问）
            DownloaderType::HttpExtendedTimeout
        } else {
            DownloaderType::Http
        }
    }

    /// 检查服务器是否支持Range请求 ⭐
    async fn check_range_support(
        &self,
        url: &str,
    ) -> Result<(bool, u64, Option<String>, Option<String>)> {
        info!("Checking Range support: {}", url);

        let response = self
            .get_http_client()
            .head(url)
            .send()
            .await
            .context("Failed to check Range support")?;

        info!("HTTP response status: {}", response.status());

        if !response.status().is_success() {
            return Err(anyhow::anyhow!(
                "Server response error: HTTP {}",
                response.status()
            ));
        }

        // 🆕 详细调试信息 ⭐
        info!("Response headers:");
        for (name, value) in response.headers().iter() {
            if let Ok(value_str) = value.to_str() {
                info!("   {}: {}", name, value_str);
            } else {
                info!("   {}: <non-UTF8 value>", name);
            }
        }

        let total_size = response.content_length().unwrap_or(0);
        info!("Content-Length parsed result: {} bytes", total_size);

        // 🆕 修复content_length解析问题 ⭐
        let total_size = if total_size == 0 {
            // 如果reqwest解析失败，手动从响应头部解析
            if let Some(content_length_header) = response.headers().get("content-length") {
                if let Ok(content_length_str) = content_length_header.to_str() {
                    if let Ok(parsed_size) = content_length_str.parse::<u64>() {
                        info!("Manually parsed Content-Length: {} bytes", parsed_size);
                        parsed_size
                    } else {
                        warn!("Content-Length parse failed: {}", content_length_str);
                        0
                    }
                } else {
                    warn!("Content-Length header is not a valid UTF-8 string");
                    0
                }
            } else {
                warn!("No Content-Length header in response");
                0
            }
        } else {
            total_size
        };

        // 原始的Range支持检测
        let explicit_range_support = response
            .headers()
            .get("accept-ranges")
            .and_then(|v| v.to_str().ok())
            .map(|v| v.contains("bytes"))
            .unwrap_or(false);

        // 🆕 对对象存储和CDN服务器采用更宽松的检测策略 ⭐
        let is_object_storage_or_cdn = self.is_object_storage_or_cdn_url(url);
        let supports_range = if is_object_storage_or_cdn {
            // 对象存储和CDN服务器通常支持Range请求，即使不明确返回Accept-Ranges头部
            info!(
                "Detected object storage/CDN server, assuming Range support (force-enabled resume)"
            );
            true
        } else {
            explicit_range_support
        };

        info!("Range support detection results:");
        info!(
            "   Server type: {}",
            if is_object_storage_or_cdn {
                "Object storage/CDN"
            } else {
                "Regular HTTP"
            }
        );
        info!("   Explicit Range support: {}", explicit_range_support);
        info!("   Final determination: {}", supports_range);
        if let Some(accept_ranges) = response.headers().get("accept-ranges") {
            info!("   Accept-Ranges header: {:?}", accept_ranges);
        } else {
            info!("   Accept-Ranges header: not provided");
        }

        let (validator_header, validator) = if let Some(value) = response
            .headers()
            .get(reqwest::header::ETAG)
            .and_then(|value| value.to_str().ok())
            .filter(|value| !value.starts_with("W/"))
        {
            (Some("etag".to_owned()), Some(value.to_owned()))
        } else if let Some(value) = response
            .headers()
            .get(reqwest::header::LAST_MODIFIED)
            .and_then(|value| value.to_str().ok())
        {
            (Some("last-modified".to_owned()), Some(value.to_owned()))
        } else {
            (None, None)
        };
        Ok((supports_range, total_size, validator, validator_header))
    }

    /// 获取下载元数据文件路径 ⭐
    fn get_metadata_path(&self, download_path: &Path) -> std::path::PathBuf {
        download_path.with_extension("download")
    }

    /// 保存下载元数据 ⭐
    async fn save_metadata(&self, download_path: &Path, metadata: &DownloadMetadata) -> Result<()> {
        self.save_metadata_with_logging(download_path, metadata, true)
            .await
    }

    /// 保存下载元数据（可控制日志输出）⭐
    async fn save_metadata_with_logging(
        &self,
        download_path: &Path,
        metadata: &DownloadMetadata,
        show_log: bool,
    ) -> Result<()> {
        if !self.config.enable_metadata {
            return Ok(());
        }

        let metadata_path = self.get_metadata_path(download_path);
        let json_content = serde_json::to_string_pretty(metadata)
            .map_err(|e| DuckError::custom(format!("Failed to serialize metadata: {e}")))?;

        crate::atomic_file::write_atomic(
            &metadata_path,
            json_content.as_bytes(),
            crate::atomic_file::PermissionsPolicy::Private,
        )
        .map_err(|e| DuckError::custom(format!("Failed to save metadata: {e}")))?;

        if show_log {
            info!("Saved download metadata: {}", metadata_path.display());
        }
        Ok(())
    }

    /// 清理下载元数据 ⭐
    async fn cleanup_metadata(&self, download_path: &Path) -> Result<()> {
        if !self.config.enable_metadata {
            return Ok(());
        }

        let metadata_path = self.get_metadata_path(download_path);
        if metadata_path.exists() {
            tokio::fs::remove_file(&metadata_path)
                .await
                .map_err(|e| DuckError::custom(format!("Failed to cleanup metadata: {e}")))?;
            info!("Cleaned up download metadata: {}", metadata_path.display());
        }
        Ok(())
    }

    /// 智能检查断点续传可行性 ⭐
    async fn check_resume_feasibility(
        &self,
        download_path: &Path,
        total_size: u64,
        expected_hash: Option<&str>,
        url: &str,
        version: &str,
        validator: Option<&str>,
    ) -> Result<Option<ResumeCheckpoint>> {
        info!("Checking resume feasibility...");

        // 1. 检查文件是否存在
        if !download_path.exists() {
            info!("Target file does not exist, cannot resume");
            return Ok(None);
        }

        // 2. 获取当前文件大小
        let file_metadata = tokio::fs::metadata(download_path)
            .await
            .map_err(|e| DuckError::custom(format!("Failed to read file metadata: {e}")))?;
        let existing_size = file_metadata.len();

        info!(
            "Current file size: {} bytes ({:.2} MB)",
            existing_size,
            existing_size as f64 / 1024.0 / 1024.0
        );

        // A partial file is reusable only for the same HTTP resource and release.
        // Missing/corrupt metadata must never authorize appending another package.
        let saved = match tokio::fs::read(self.get_metadata_path(download_path)).await {
            Ok(bytes) => serde_json::from_slice::<DownloadMetadata>(&bytes).ok(),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
            Err(error) => return Err(error.into()),
        };
        let same_task = saved.as_ref().is_some_and(|metadata| {
            metadata.is_same_task(url, total_size, version)
                && metadata.expected_hash.as_deref() == expected_hash
                && metadata.validator.as_deref() == validator
        });
        if !same_task || (total_size > 0 && existing_size >= total_size) {
            info!("Partial download identity or size is invalid; restarting download");
            tokio::fs::remove_file(download_path).await?;
            self.cleanup_metadata(download_path).await?;
            return Ok(None);
        }

        let saved =
            saved.ok_or_else(|| anyhow::anyhow!("Partial download identity disappeared"))?;
        let checkpoint_size = if saved.partial_sha256.is_some() {
            saved.downloaded_bytes
        } else if expected_hash.is_some() {
            // Older partial metadata has no local prefix digest. A published SHA
            // still authenticates the completed package after legacy resume.
            existing_size
        } else {
            info!("Partial download has no integrity checkpoint; restarting safely");
            tokio::fs::remove_file(download_path).await?;
            self.cleanup_metadata(download_path).await?;
            return Ok(None);
        };
        if checkpoint_size > existing_size || checkpoint_size < self.config.resume_threshold {
            tokio::fs::remove_file(download_path).await?;
            self.cleanup_metadata(download_path).await?;
            return Ok(None);
        }
        let hasher = hash_file_prefix(download_path, checkpoint_size).await?;
        if saved
            .partial_sha256
            .as_ref()
            .is_some_and(|expected| expected != &hash_digest(&hasher))
        {
            warn!("Partial download prefix integrity failed; restarting download");
            tokio::fs::remove_file(download_path).await?;
            self.cleanup_metadata(download_path).await?;
            return Ok(None);
        }
        if checkpoint_size < existing_size {
            // An uncheckpointed tail may survive process termination. Retain the
            // verified prefix and redownload only that bounded tail.
            OpenOptions::new()
                .write(true)
                .open(download_path)
                .await?
                .set_len(checkpoint_size)
                .await?;
        }
        Ok(Some(ResumeCheckpoint {
            size: checkpoint_size,
            hasher,
        }))
    }

    /// 下载文件（支持断点续传）⭐
    pub async fn download_file<F>(
        &self,
        url: &str,
        download_path: &Path,
        progress_callback: Option<F>,
    ) -> Result<()>
    where
        F: Fn(DownloadProgress) + Send + Sync + 'static,
    {
        self.download_file_with_options(url, download_path, progress_callback, None, None)
            .await
    }

    /// 下载文件（带额外选项）⭐
    pub async fn download_file_with_options<F>(
        &self,
        url: &str,
        download_path: &Path,
        progress_callback: Option<F>,
        expected_hash: Option<&str>,
        version: Option<&str>,
    ) -> Result<()>
    where
        F: Fn(DownloadProgress) + Send + Sync + 'static,
    {
        let downloader_type = self.get_downloader_type(url);
        let version = version.unwrap_or("unknown");

        info!("Starting file download");
        info!("   URL: {}", url);
        info!("   Target path: {}", download_path.display());
        info!("   Downloader type: {:?}", downloader_type);
        info!(
            "   Resume: {}",
            if self.config.enable_resume {
                "enabled"
            } else {
                "disabled"
            }
        );
        if let Some(hash) = expected_hash {
            info!("   Expected hash: {}", hash);
        }
        info!("   Version: {}", version);

        // 检查Range支持和文件大小
        let (supports_range, total_size, validator, validator_header) =
            self.check_range_support(url).await?;

        if total_size > 0 {
            info!(
                "📦 Server file size: {} bytes ({:.2} MB)",
                total_size,
                total_size as f64 / 1024.0 / 1024.0
            );
        }

        if supports_range && self.config.enable_resume {
            info!("Server supports Range requests, enabling resume");
        } else if !supports_range {
            warn!("Server does not support Range requests, using regular download");
        }

        // 智能检查断点续传可行性
        let resume = if supports_range && self.config.enable_resume {
            self.check_resume_feasibility(
                download_path,
                total_size,
                expected_hash,
                url,
                version,
                validator.as_deref(),
            )
            .await?
        } else {
            None
        };

        let (existing_size, prefix_hasher) = match resume {
            Some(checkpoint) => (Some(checkpoint.size), Some(checkpoint.hasher)),
            None => (None, None),
        };

        // 创建下载元数据
        let mut metadata = DownloadMetadata::new(
            url.to_string(),
            total_size,
            expected_hash.map(|s| s.to_string()),
            version.to_string(),
        );

        metadata.validator = validator;
        metadata.validator_header = validator_header;

        // 如果是续传，更新进度
        if let Some(resume_size) = existing_size {
            metadata.update_progress(resume_size);
            metadata.partial_sha256 = prefix_hasher.as_ref().map(hash_digest);
        }

        // 保存初始元数据
        self.save_metadata(download_path, &metadata).await?;

        let task_id = match downloader_type {
            DownloaderType::Http => "http_download",
            DownloaderType::HttpExtendedTimeout => "extended_http_download",
        };
        let result = self
            .download_with_resume_internal(ResumeDownloadParams {
                url,
                download_path,
                progress_callback,
                existing_size,
                prefix_hasher,
                total_size,
                task_id,
                metadata: &mut metadata,
            })
            .await;

        // 处理下载结果
        match result {
            Ok(_) => {
                // Do not publish success or remove resume state before integrity checks.
                if let Some(hash) = expected_hash {
                    let actual_hash = Self::calculate_file_hash(download_path).await?;
                    if !actual_hash.eq_ignore_ascii_case(hash) {
                        tokio::fs::remove_file(download_path).await?;
                        self.cleanup_metadata(download_path).await?;
                        return Err(anyhow::anyhow!("File hash verification failed"));
                    }
                }
                metadata.completed = true;
                self.save_metadata_with_logging(download_path, &metadata, false)
                    .await?;
                if !self.config.retain_completed_metadata {
                    self.cleanup_metadata(download_path).await?;
                }
                Ok(())
            }
            Err(e) => {
                // 下载失败，保留元数据用于下次续传
                warn!("Download failed: {}", e);
                info!("Preserving metadata for next resume");
                Err(e)
            }
        }
    }

    /// 内部断点续传下载实现 ⭐
    async fn download_with_resume_internal<F>(
        &self,
        params: ResumeDownloadParams<'_, F>,
    ) -> Result<()>
    where
        F: Fn(DownloadProgress) + Send + Sync + 'static,
    {
        let start_byte = params.existing_size.unwrap_or(0);
        let mut total_size = params.total_size;
        let is_resume = params.existing_size.is_some();

        // 构建请求
        let mut request = self.get_http_client().get(params.url);

        if is_resume {
            info!("Resume download: starting from byte {}", start_byte);
            request = request.header("Range", format!("bytes={start_byte}-"));
            if let Some(validator) = params.metadata.validator.as_deref() {
                request = request.header(reqwest::header::IF_RANGE, validator);
            }
        }

        let response = request
            .send()
            .await
            .context("Failed to start download request")?;

        // 检查响应状态
        let expected_status = if is_resume { 206 } else { 200 };

        // 🆕 断点续传失败自动回退机制 ⭐
        if is_resume && response.status().as_u16() != 206 {
            warn!(
                "⚠️ Resume request failed: HTTP {} (expected: 206)",
                response.status()
            );

            // 检查是否是服务器不支持Range的错误
            if response.status().as_u16() == 200 || response.status().as_u16() == 416 {
                warn!("Server may not support Range request, falling back to full download");

                if response.status().as_u16() == 200 {
                    validate_response_resource(&response, params.metadata)?;
                }

                // 重新发起不带Range头的请求
                info!("Restarting full download request");
                let new_response = self
                    .get_http_client()
                    .get(params.url)
                    .send()
                    .await
                    .context("Failed to start re-download request")?;

                if new_response.status().as_u16() != 200 {
                    return Err(anyhow::anyhow!(
                        "Re-download failed: HTTP {}",
                        new_response.status()
                    ));
                }

                validate_response_resource(&new_response, params.metadata)?;
                if total_size == 0 {
                    total_size = new_response.content_length().unwrap_or(0);
                } else if new_response
                    .content_length()
                    .is_some_and(|length| length != total_size)
                {
                    return Err(anyhow::anyhow!(
                        "Re-download response length differs from HEAD"
                    ));
                }

                // 删除已有文件，重新开始下载
                if params.download_path.exists() {
                    info!("Deleting partially downloaded file, preparing to re-download");
                    tokio::fs::remove_file(params.download_path)
                        .await
                        .with_context(|| {
                            format!(
                                "Failed to delete partial file {}",
                                params.download_path.display()
                            )
                        })?;
                }

                // 清理元数据
                let _ = self.cleanup_metadata(params.download_path).await;

                // 创建新文件并从头开始下载
                let mut file = File::create(params.download_path).await.with_context(|| {
                    format!("Failed to create file {}", params.download_path.display())
                })?;

                // 重置元数据
                params.metadata.downloaded_bytes = 0;
                params.metadata.start_time = chrono::Utc::now().to_rfc3339();
                self.save_metadata(params.download_path, params.metadata)
                    .await?;

                return self
                    .download_stream_with_resume(StreamDownloadParams {
                        response: new_response,
                        file: &mut file,
                        download_path: params.download_path,
                        progress_callback: params.progress_callback,
                        task_id: params.task_id,
                        start_byte: 0,
                        prefix_hasher: None,
                        total_size,
                        is_resume: false,
                        metadata: params.metadata,
                    })
                    .await;
            } else {
                return Err(anyhow::anyhow!(
                    "Download failed: HTTP {} (expected: {})",
                    response.status(),
                    expected_status,
                ));
            }
        } else if response.status().as_u16() != expected_status {
            return Err(anyhow::anyhow!(
                "Download failed: HTTP {} (expected: {})",
                response.status(),
                expected_status,
            ));
        }

        validate_response_resource(&response, params.metadata)?;
        if is_resume {
            let content_range = response
                .headers()
                .get(reqwest::header::CONTENT_RANGE)
                .and_then(|value| value.to_str().ok())
                .and_then(parse_content_range)
                .ok_or_else(|| anyhow::anyhow!("Invalid Content-Range on resumed download"))?;
            if content_range.0 != start_byte
                || content_range.1 < content_range.0
                || content_range.1 >= content_range.2
                || content_range.1 + 1 != content_range.2
                || (params.total_size > 0 && content_range.2 != params.total_size)
                || response
                    .content_length()
                    .is_some_and(|length| length != content_range.1 - content_range.0 + 1)
            {
                return Err(anyhow::anyhow!(
                    "Content-Range does not match resumed download"
                ));
            }
            total_size = content_range.2;
        } else if total_size == 0 {
            total_size = response.content_length().unwrap_or(0);
        } else if response
            .content_length()
            .is_some_and(|length| length != total_size)
        {
            return Err(anyhow::anyhow!(
                "Download response length differs from HEAD"
            ));
        }

        // 打开文件（追加模式或创建模式）
        let mut file = if is_resume {
            info!("Opening file in append mode");
            OpenOptions::new()
                .create(true)
                .append(true)
                .open(params.download_path)
                .await
                .with_context(|| {
                    format!("Failed to open file {}", params.download_path.display())
                })?
        } else {
            info!("Creating new file");
            File::create(params.download_path).await.with_context(|| {
                format!("Failed to create file {}", params.download_path.display())
            })?
        };

        // 执行下载
        self.download_stream_with_resume(StreamDownloadParams {
            response,
            file: &mut file,
            download_path: params.download_path,
            progress_callback: params.progress_callback,
            task_id: params.task_id,
            start_byte,
            prefix_hasher: params.prefix_hasher,
            total_size,
            is_resume,
            metadata: params.metadata,
        })
        .await
    }

    async fn save_checkpoint(
        &self,
        file: &mut File,
        path: &Path,
        metadata: &mut DownloadMetadata,
        downloaded: u64,
        hasher: &Sha256,
    ) -> Result<()> {
        file.flush().await?;
        file.sync_all().await?;
        metadata.update_progress(downloaded);
        metadata.partial_sha256 = Some(hash_digest(hasher));
        self.save_metadata_with_logging(path, metadata, false).await
    }

    /// 通用的流式下载处理（支持断点续传）⭐
    async fn download_stream_with_resume<F>(
        &self,
        params: StreamDownloadParams<'_, F>,
    ) -> Result<()>
    where
        F: Fn(DownloadProgress) + Send + Sync + 'static,
    {
        let mut downloaded = params.start_byte;
        let mut hasher = params.prefix_hasher.unwrap_or_default();
        let mut checkpoint_bytes = downloaded;
        let mut checkpoint_time = std::time::Instant::now();
        let mut stream = params.response.bytes_stream();
        let mut last_progress_time = std::time::Instant::now();
        let mut last_progress_bytes = downloaded;
        let progress_interval =
            std::time::Duration::from_secs(self.config.progress_interval_seconds);

        // 首次进度回调
        if let Some(callback) = params.progress_callback.as_ref() {
            let status = if params.is_resume {
                DownloadStatus::Resuming
            } else {
                DownloadStatus::Starting
            };
            callback(DownloadProgress {
                task_id: params.task_id.to_string(),
                file_name: params
                    .download_path
                    .file_name()
                    .unwrap_or_default()
                    .to_string_lossy()
                    .to_string(),
                downloaded_bytes: downloaded,
                total_bytes: params.total_size,
                download_speed: 0.0,
                eta_seconds: 0,
                percentage: if params.total_size > 0 {
                    downloaded as f64 / params.total_size as f64 * 100.0
                } else {
                    0.0
                },
                status,
            });
        }

        while let Some(chunk) = stream.next().await {
            let chunk = match chunk {
                Ok(chunk) => chunk,
                Err(error) => {
                    let checkpoint = self
                        .save_checkpoint(
                            params.file,
                            params.download_path,
                            params.metadata,
                            downloaded,
                            &hasher,
                        )
                        .await;
                    return Err(with_checkpoint_error(
                        anyhow::Error::new(error).context("Failed to download data"),
                        checkpoint,
                    ));
                }
            };
            if let Err(error) = params.file.write_all(&chunk).await {
                let checkpoint = self
                    .save_checkpoint(
                        params.file,
                        params.download_path,
                        params.metadata,
                        downloaded,
                        &hasher,
                    )
                    .await;
                return Err(with_checkpoint_error(
                    anyhow::Error::new(error).context("Failed to write file"),
                    checkpoint,
                ));
            }
            hasher.update(&chunk);
            downloaded += chunk.len() as u64;
            if self.config.enable_metadata
                && (downloaded - checkpoint_bytes >= self.config.metadata_checkpoint_bytes.max(1)
                    || checkpoint_time.elapsed() >= Duration::from_secs(60))
            {
                self.save_checkpoint(
                    params.file,
                    params.download_path,
                    params.metadata,
                    downloaded,
                    &hasher,
                )
                .await?;
                checkpoint_bytes = downloaded;
                checkpoint_time = std::time::Instant::now();
            }

            // 调用进度回调
            if let Some(callback) = params.progress_callback.as_ref() {
                let progress = if params.total_size > 0 {
                    downloaded as f64 / params.total_size as f64 * 100.0
                } else {
                    0.0
                };

                callback(DownloadProgress {
                    task_id: params.task_id.to_string(),
                    file_name: params
                        .download_path
                        .file_name()
                        .unwrap_or_default()
                        .to_string_lossy()
                        .to_string(),
                    downloaded_bytes: downloaded,
                    total_bytes: params.total_size,
                    download_speed: 0.0,
                    eta_seconds: 0,
                    percentage: progress,
                    status: DownloadStatus::Downloading,
                });
            }

            // 进度显示逻辑
            if self.config.enable_progress_logging {
                let now = std::time::Instant::now();
                let bytes_since_last = downloaded - last_progress_bytes;
                let time_since_last = now.duration_since(last_progress_time);

                let should_show_progress = bytes_since_last >= self.config.progress_bytes_interval
                    || time_since_last >= progress_interval
                    || (params.total_size > 0 && downloaded >= params.total_size);

                if should_show_progress {
                    if params.total_size > 0 {
                        let percentage =
                            (downloaded as f64 / params.total_size as f64 * 100.0) as u32;
                        let status_icon = if params.is_resume
                            && downloaded <= params.start_byte + 50 * 1024 * 1024
                        {
                            "🔄"
                        } else {
                            "📥"
                        };

                        let speed_mbps = if time_since_last.as_secs_f64() > 0.0 {
                            (bytes_since_last as f64 / 1024.0 / 1024.0)
                                / time_since_last.as_secs_f64()
                        } else {
                            0.0
                        };

                        info!(
                            "{} Download progress: {}% ({:.1}/{:.1} MB) Speed: {:.1} MB/s",
                            status_icon,
                            percentage,
                            downloaded as f64 / 1024.0 / 1024.0,
                            params.total_size as f64 / 1024.0 / 1024.0,
                            speed_mbps
                        );
                    } else {
                        info!("Downloaded: {:.1} MB", downloaded as f64 / 1024.0 / 1024.0);
                    }

                    last_progress_time = now;
                    last_progress_bytes = downloaded;
                }
            }
        }

        self.save_checkpoint(
            params.file,
            params.download_path,
            params.metadata,
            downloaded,
            &hasher,
        )
        .await?;
        if params.total_size > 0 && downloaded != params.total_size {
            return Err(anyhow::anyhow!(
                "Incomplete download: received {} bytes, expected {}",
                downloaded,
                params.total_size
            ));
        }

        let download_type = if params.is_resume {
            "Resume download"
        } else {
            "Download"
        };
        info!("{} completed", download_type);
        info!("   File path: {}", params.download_path.display());
        info!(
            "   Final size: {} bytes ({:.2} MB)",
            downloaded,
            downloaded as f64 / 1024.0 / 1024.0
        );
        if params.is_resume {
            info!(
                "   Resumed size: {} bytes ({:.2} MB)",
                downloaded - params.start_byte,
                (downloaded - params.start_byte) as f64 / 1024.0 / 1024.0
            );
        }

        Ok(())
    }

    /// 计算文件的SHA256哈希值
    pub async fn calculate_file_hash(file_path: &Path) -> Result<String> {
        if !file_path.exists() {
            return Err(anyhow::anyhow!(
                "File does not exist: {}",
                file_path.display()
            ));
        }

        let length = tokio::fs::metadata(file_path).await?.len();
        Ok(hash_digest(&hash_file_prefix(file_path, length).await?))
    }

    /// 验证文件完整性
    pub async fn verify_file_integrity(file_path: &Path, expected_hash: &str) -> Result<bool> {
        info!("Verifying file integrity: {}", file_path.display());

        // 计算当前文件的哈希值
        let actual_hash = Self::calculate_file_hash(file_path).await?;

        // 比较哈希值（忽略大小写）
        let matches = actual_hash.to_lowercase() == expected_hash.to_lowercase();

        if matches {
            info!(
                "File integrity verification passed: {}",
                file_path.display()
            );
        } else {
            warn!(
                "File integrity verification failed: {}",
                file_path.display()
            );
            warn!("   Expected hash: {}", expected_hash);
            warn!("   Actual hash: {}", actual_hash);
        }

        Ok(matches)
    }
}

/// Preserve the typed primary failure even when recording the resumable prefix
/// also fails. The secondary diagnostic is context, never a replacement source.
fn with_checkpoint_error(primary: anyhow::Error, checkpoint: Result<()>) -> anyhow::Error {
    match checkpoint {
        Ok(()) => primary,
        Err(secondary) => primary.context(format!("Resume checkpoint also failed: {secondary:#}")),
    }
}

/// 简化的下载功能，用于向后兼容
pub async fn download_file_simple(url: &str, download_path: &Path) -> Result<()> {
    let downloader = FileDownloader::with_default_config();
    downloader
        .download_file::<fn(DownloadProgress)>(url, download_path, None)
        .await
}

/// 带进度回调的下载功能
pub async fn download_file_with_progress<F>(
    url: &str,
    download_path: &Path,
    progress_callback: Option<F>,
) -> Result<()>
where
    F: Fn(DownloadProgress) + Send + Sync + 'static,
{
    let downloader = FileDownloader::with_default_config();
    downloader
        .download_file(url, download_path, progress_callback)
        .await
}

/// 创建自定义配置的下载器
pub fn create_downloader(config: DownloaderConfig) -> FileDownloader {
    FileDownloader::new(config)
}

#[cfg(test)]
mod tests {
    use super::*;

    mod source_error_tests {
        use super::*;
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        use tokio::net::TcpListener;

        #[derive(Clone, Copy)]
        enum Failure {
            BodyTimeout,
            Disconnect,
            HeaderTimeout,
            FallbackHeaderTimeout,
            WriteChunks,
        }

        struct Server {
            url: String,
            task: tokio::task::JoinHandle<()>,
        }

        impl Drop for Server {
            fn drop(&mut self) {
                self.task.abort();
            }
        }

        impl Server {
            async fn start(failure: Failure) -> Self {
                let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
                let url = format!("http://{}/body", listener.local_addr().unwrap());
                let task = tokio::spawn(async move {
                    loop {
                        let (mut socket, _) = listener.accept().await.unwrap();
                        tokio::spawn(async move {
                            let mut request = Vec::new();
                            let mut buffer = [0_u8; 2048];
                            while !request.windows(4).any(|bytes| bytes == b"\r\n\r\n") {
                                let length = socket.read(&mut buffer).await.unwrap();
                                if length == 0 {
                                    return;
                                }
                                request.extend_from_slice(&buffer[..length]);
                            }
                            let head = request.starts_with(b"HEAD ");
                            let range = String::from_utf8_lossy(&request)
                                .to_ascii_lowercase()
                                .contains("\r\nrange:");
                            if matches!(failure, Failure::HeaderTimeout)
                                || matches!(failure, Failure::FallbackHeaderTimeout) && !range
                            {
                                tokio::time::sleep(Duration::from_secs(4)).await;
                                return;
                            }
                            let response = b"HTTP/1.1 200 OK\r\nContent-Length: 65536\r\nAccept-Ranges: bytes\r\nETag: \"source-error-fixture\"\r\nConnection: close\r\n\r\n";
                            if socket.write_all(response).await.is_err() || head {
                                return;
                            }
                            if socket.write_all(b"data").await.is_err() {
                                return;
                            }
                            match failure {
                                Failure::BodyTimeout => {
                                    tokio::time::sleep(Duration::from_secs(4)).await
                                }
                                Failure::Disconnect => {}
                                Failure::WriteChunks | Failure::FallbackHeaderTimeout => {
                                    tokio::time::sleep(Duration::from_millis(50)).await;
                                    let _ = socket.write_all(&vec![0_u8; 65532]).await;
                                }
                                Failure::HeaderTimeout => {
                                    unreachable!("header timeout returns before body")
                                }
                            }
                        });
                    }
                });
                Self { url, task }
            }
        }

        fn downloader() -> FileDownloader {
            let _ = rustls::crypto::ring::default_provider().install_default();
            let config = DownloaderConfig {
                timeout_seconds: 1,
                enable_progress_logging: false,
                resume_threshold: 1,
                ..DownloaderConfig::default()
            };
            let client = Client::builder()
                .no_proxy()
                .timeout(Duration::from_secs(1))
                .build()
                .unwrap();
            FileDownloader::new_with_custom_client(config, client)
        }

        fn assert_timeout_source(error: &anyhow::Error) {
            let request = error
                .downcast_ref::<reqwest::Error>()
                .expect("typed HTTP source must remain available");
            assert!(
                request.is_timeout(),
                "HTTP source must retain timeout classification: {error:#}"
            );
            let rendered = format!("{error:#}").to_ascii_lowercase();
            assert!(
                rendered.contains("timed out"),
                "full causal rendering must expose the root timeout: {rendered}"
            );
        }

        async fn stream_failure(
            failure: Failure,
            checkpoint_failure: bool,
            readonly: bool,
        ) -> anyhow::Error {
            let directory = tempfile::tempdir().unwrap();
            let path = directory.path().join("partial.data");
            let mut file = if readonly {
                tokio::fs::write(&path, b"previous").await.unwrap();
                File::open(&path).await.unwrap()
            } else {
                File::create(&path).await.unwrap()
            };
            if checkpoint_failure {
                std::fs::create_dir(path.with_extension("download")).unwrap();
            }
            let downloader = downloader();
            let server = Server::start(failure).await;
            let response = downloader
                .get_http_client()
                .get(&server.url)
                .send()
                .await
                .unwrap();
            let mut metadata =
                DownloadMetadata::new(server.url.clone(), 65536, None, "test-version".into());
            let result = downloader
                .download_stream_with_resume::<fn(DownloadProgress)>(StreamDownloadParams {
                    response,
                    file: &mut file,
                    download_path: &path,
                    progress_callback: None,
                    task_id: "source-error-test",
                    start_byte: 0,
                    prefix_hasher: None,
                    total_size: 65536,
                    is_resume: false,
                    metadata: &mut metadata,
                })
                .await;
            let error = result.expect_err("real HTTP fixture must fail in the requested boundary");
            if !readonly && !checkpoint_failure {
                assert_eq!(tokio::fs::read(&path).await.unwrap(), b"data");
                let saved: DownloadMetadata = serde_json::from_slice(
                    &tokio::fs::read(path.with_extension("download"))
                        .await
                        .unwrap(),
                )
                .unwrap();
                assert_eq!(saved.downloaded_bytes, 4);
                assert!(saved.partial_sha256.is_some());
            }
            error
        }

        #[tokio::test]
        async fn real_body_timeout_keeps_typed_http_source_and_root_cause() {
            let error = stream_failure(Failure::BodyTimeout, false, false).await;
            assert_timeout_source(&error);
            assert!(format!("{error:#}").contains("Failed to download data"));
        }

        #[tokio::test]
        async fn real_body_disconnect_keeps_http_source_and_checkpoint() {
            let error = stream_failure(Failure::Disconnect, false, false).await;
            let request = error
                .downcast_ref::<reqwest::Error>()
                .expect("typed disconnect source");
            assert!(!request.is_timeout());
            assert!(
                error.chain().count() > 2,
                "nested transport cause must survive: {error:#}"
            );
        }

        #[tokio::test]
        async fn real_body_timeout_checkpoint_failure_keeps_the_primary_timeout() {
            let error = stream_failure(Failure::BodyTimeout, true, false).await;
            assert_timeout_source(&error);
            assert!(format!("{error:#}").contains("Resume checkpoint also failed"));
        }

        #[tokio::test]
        async fn real_write_failure_checkpoint_failure_keeps_the_primary_io_error() {
            let error = stream_failure(Failure::WriteChunks, true, true).await;
            assert!(
                error.downcast_ref::<std::io::Error>().is_some(),
                "typed write source must survive: {error:#}"
            );
            let rendered = format!("{error:#}");
            assert!(
                rendered.contains("Failed to write file"),
                "primary write boundary lost: {rendered}"
            );
            assert!(rendered.contains("Resume checkpoint also failed"));
        }

        #[tokio::test]
        async fn real_head_timeout_keeps_http_source() {
            let downloader = downloader();
            let server = Server::start(Failure::HeaderTimeout).await;
            let error = downloader
                .check_range_support(&server.url)
                .await
                .unwrap_err();
            assert_timeout_source(&error);
            assert!(format!("{error:#}").contains("Failed to check Range support"));
        }

        #[tokio::test]
        async fn real_initial_get_timeout_keeps_http_source() {
            let directory = tempfile::tempdir().unwrap();
            let path = directory.path().join("partial.data");
            let downloader = downloader();
            let server = Server::start(Failure::HeaderTimeout).await;
            let mut metadata =
                DownloadMetadata::new(server.url.clone(), 65536, None, "test-version".into());
            let error = downloader
                .download_with_resume_internal::<fn(DownloadProgress)>(ResumeDownloadParams {
                    url: &server.url,
                    download_path: &path,
                    progress_callback: None,
                    existing_size: None,
                    prefix_hasher: None,
                    total_size: 65536,
                    task_id: "source-error-test",
                    metadata: &mut metadata,
                })
                .await
                .unwrap_err();
            assert_timeout_source(&error);
            assert!(format!("{error:#}").contains("Failed to start download request"));
        }

        #[tokio::test]
        async fn real_fallback_get_timeout_keeps_http_source() {
            let directory = tempfile::tempdir().unwrap();
            let path = directory.path().join("partial.data");
            let downloader = downloader();
            let server = Server::start(Failure::FallbackHeaderTimeout).await;
            let mut metadata =
                DownloadMetadata::new(server.url.clone(), 65536, None, "test-version".into());
            let error = downloader
                .download_with_resume_internal::<fn(DownloadProgress)>(ResumeDownloadParams {
                    url: &server.url,
                    download_path: &path,
                    progress_callback: None,
                    existing_size: Some(4),
                    prefix_hasher: None,
                    total_size: 65536,
                    task_id: "source-error-test",
                    metadata: &mut metadata,
                })
                .await
                .unwrap_err();
            assert_timeout_source(&error);
            assert!(format!("{error:#}").contains("Failed to start re-download request"));
        }
    }

    #[test]
    fn test_aliyun_oss_url_detection() {
        let downloader = FileDownloader::with_default_config();

        // 测试您提供的真实阿里云 OSS URL
        let real_oss_url = "https://nuwa-packages.oss-rg-china-mainland.aliyuncs.com/nuwax-client-releases/docker/20250705082538/docker.zip";
        assert!(
            downloader.is_aliyun_oss_url(real_oss_url),
            "应该识别为阿里云 OSS URL"
        );

        // 测试其他阿里云 OSS URL 格式
        let test_cases = vec![
            ("https://bucket.oss-cn-hangzhou.aliyuncs.com/file.zip", true),
            (
                "https://my-bucket.oss-us-west-1.aliyuncs.com/path/file.tar.gz",
                true,
            ),
            (
                "https://test.oss-ap-southeast-1.aliyuncs.com/docker.zip",
                true,
            ),
            ("https://example.com/file.zip", false),
            (
                "https://github.com/user/repo/releases/download/v1.0.0/file.zip",
                false,
            ),
            ("ftp://bucket.oss-cn-beijing.aliyuncs.com/file.zip", false),
        ];

        for (url, expected) in test_cases {
            assert_eq!(
                downloader.is_aliyun_oss_url(url),
                expected,
                "URL: {url} 应该返回 {expected}"
            );
        }
    }

    #[test]
    fn test_downloader_type_detection() {
        let downloader = FileDownloader::with_default_config();

        // 测试您的真实 OSS URL（公网访问）
        let real_oss_url = "https://nuwa-packages.oss-rg-china-mainland.aliyuncs.com/nuwax-client-releases/docker/20250705082538/docker.zip";
        let downloader_type = downloader.get_downloader_type(real_oss_url);

        match downloader_type {
            DownloaderType::HttpExtendedTimeout => {
                println!("✅ 正确识别为扩展超时 HTTP 下载（公网访问）")
            }
            DownloaderType::Http => println!("❌ 错误识别为普通 HTTP 下载"),
        }

        // 对于阿里云 OSS 文件，应该使用扩展超时HTTP下载
        assert!(
            matches!(downloader_type, DownloaderType::HttpExtendedTimeout),
            "OSS文件应该使用扩展超时HTTP下载"
        );

        // 测试普通 HTTP URL
        let http_url = "https://github.com/user/repo/releases/download/v1.0.0/file.zip";
        assert!(
            matches!(
                downloader.get_downloader_type(http_url),
                DownloaderType::Http
            ),
            "普通 HTTP URL 应该使用标准下载"
        );
    }

    #[test]
    fn test_calculate_file_hash() {
        // This is a placeholder test for file hash calculation
        // In a real scenario, you would test with actual file data
    }

    /// 测试OSS URL检测和Range支持检测 ⭐
    #[tokio::test]
    async fn test_oss_url_detection_and_range_support() {
        let downloader = FileDownloader::with_default_config();

        // 测试用户提供的OSS URL
        let oss_url = "https://nuwa-packages.oss-rg-china-mainland.aliyuncs.com/docker/20250712133533/docker.zip";

        // 1. 测试URL检测
        println!("🔍 测试URL检测功能");
        let is_aliyun_oss = downloader.is_aliyun_oss_url(oss_url);
        let is_object_storage = downloader.is_object_storage_or_cdn_url(oss_url);
        let downloader_type = downloader.get_downloader_type(oss_url);

        println!("   URL: {oss_url}");
        println!("   是否阿里云OSS: {is_aliyun_oss}");
        println!("   是否对象存储/CDN: {is_object_storage}");
        println!("   下载器类型: {downloader_type:?}");

        assert!(is_aliyun_oss, "应该识别为阿里云OSS URL");
        assert!(is_object_storage, "应该识别为对象存储URL");

        // 2. 测试Range支持检测
        println!("\n🔍 测试Range支持检测功能");
        println!("   开始HEAD请求检测...");

        // 🆕 手动执行HEAD请求进行调试 ⭐
        let client = downloader.get_http_client();
        println!("   创建HTTP客户端完成");

        match client.head(oss_url).send().await {
            Ok(response) => {
                println!("   HTTP响应状态: {}", response.status());
                println!("   响应头部详情:");
                for (name, value) in response.headers().iter() {
                    if let Ok(value_str) = value.to_str() {
                        println!("     {name}: {value_str}");
                    } else {
                        println!("     {name}: <non-UTF8 value>");
                    }
                }

                let content_length = response.content_length();
                println!("   Content-Length (reqwest解析): {content_length:?}");

                // 🆕 使用修复后的解析逻辑 ⭐
                let actual_size = if let Some(size) = content_length {
                    if size == 0 {
                        // 手动解析Content-Length头部
                        if let Some(content_length_header) =
                            response.headers().get("content-length")
                        {
                            if let Ok(content_length_str) = content_length_header.to_str() {
                                if let Ok(parsed_size) = content_length_str.parse::<u64>() {
                                    println!("   手动解析Content-Length: {parsed_size} bytes");
                                    parsed_size
                                } else {
                                    println!("   Content-Length解析失败: {content_length_str}");
                                    0
                                }
                            } else {
                                println!("   Content-Length头部不是有效的UTF-8");
                                0
                            }
                        } else {
                            println!("   没有Content-Length头部");
                            0
                        }
                    } else {
                        size
                    }
                } else {
                    println!("   reqwest未返回Content-Length");
                    0
                };

                println!(
                    "   最终文件大小: {} bytes ({:.2} GB)",
                    actual_size,
                    actual_size as f64 / 1024.0 / 1024.0 / 1024.0
                );
            }
            Err(e) => {
                println!("   HEAD请求失败: {e}");
                panic!("HEAD请求应该成功");
            }
        }

        // 3. 使用原始的check_range_support方法
        println!("\n🔍 使用原始的check_range_support方法");
        match downloader.check_range_support(oss_url).await {
            Ok((supports_range, total_size, _validator, _validator_header)) => {
                println!("   Range支持: {supports_range}");
                println!(
                    "   文件大小: {} bytes ({:.2} GB)",
                    total_size,
                    total_size as f64 / 1024.0 / 1024.0 / 1024.0
                );

                assert!(supports_range, "OSS服务器应该支持Range请求");
                if total_size == 0 {
                    println!("   ⚠️ 警告：文件大小为0，这可能表明check_range_support方法有问题");
                }
            }
            Err(e) => {
                println!("   检测失败: {e}");
                panic!("Range支持检测应该成功");
            }
        }

        println!("\n✅ 所有检测功能正常工作！");
    }
}
