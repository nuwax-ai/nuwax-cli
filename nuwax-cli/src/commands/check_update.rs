use anyhow::{Context, Result};
use chrono::DateTime;
use client_core::constants::api;
use rust_i18n::t;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use tracing::{debug, error, info, warn};

/// GitHub 仓库信息从 Cargo.toml 中的 repository 字段解析
/// 在编译时从环境变量 CARGO_PKG_REPOSITORY 中提取
fn parse_github_repo() -> (&'static str, &'static str) {
    const REPOSITORY: &str = env!("CARGO_PKG_REPOSITORY");

    // 解析 GitHub URL，支持格式: https://github.com/owner/repo
    if let Some(path) = REPOSITORY.strip_prefix("https://github.com/") {
        if let Some((owner, repo)) = path.split_once('/') {
            // 移除可能的 .git 后缀
            let repo = repo.trim_end_matches(".git");
            return (owner, repo);
        }
    }

    // 如果解析失败，抛出编译错误
    panic!(
        "{}",
        t!("check_update.parse_repo_error", repository = REPOSITORY)
    );
}

//cli 命令工具请求的地址
pub const CLI_API_URL_PATH: &str = "/api/v1/cli/versions/latest.json";

/// 获取完整的 CLI API URL（环境感知）
pub fn get_cli_api_url() -> String {
    format!("{}{CLI_API_URL_PATH}", api::get_base_url())
}

use crate::cli::CheckUpdateCommand;

/// GitHub Release API 响应结构
#[derive(Debug, Deserialize)]
pub struct GitHubRelease {
    pub tag_name: String,
    #[allow(dead_code)]
    pub name: String,
    pub body: String,
    #[allow(dead_code)]
    pub draft: bool,
    #[allow(dead_code)]
    pub prerelease: bool,
    pub published_at: String,
    #[allow(dead_code)]
    #[serde(default)]
    pub html_url: Option<String>,
    pub assets: Vec<GitHubAsset>,
}

#[derive(Debug, Deserialize)]
pub struct GitHubAsset {
    pub name: String,
    #[allow(dead_code)]
    pub size: u64,
    #[allow(dead_code)]
    pub download_count: u64,
    pub browser_download_url: String,
    #[allow(dead_code)]
    pub content_type: String,
}

/// Tauri updater API 响应结构
#[derive(Debug, Deserialize)]
pub struct TauriUpdaterResponse {
    pub version: String,
    pub notes: String,
    pub pub_date: String,
    pub platforms: HashMap<String, TauriPlatformInfo>,
}

#[derive(Debug, Deserialize)]
pub struct TauriPlatformInfo {
    pub signature: String,
    pub url: String,
}

/// 版本信息
#[derive(Debug, Serialize)]
pub struct VersionInfo {
    pub current_version: String,
    pub latest_version: String,
    pub is_update_available: bool,
    pub release_notes: String,
    pub download_url: Option<String>,
    pub published_at: String,
}

/// 更新源配置
#[derive(Debug, Clone)]
pub enum UpdateSource {
    /// 版本检查服务器（优先）
    VersionServer,
    /// GitHub API（备用）
    GitHub,
}

/// 更新源管理器
pub struct UpdateSourceManager {
    sources: Vec<UpdateSource>,
}

/// 将 Tauri updater 格式转换为 GitHub Release 格式
fn convert_tauri_to_github_release(tauri_response: TauriUpdaterResponse) -> GitHubRelease {
    use tracing::debug;

    // 将平台信息转换为 assets
    let assets: Vec<GitHubAsset> = tauri_response
        .platforms
        .into_iter()
        .map(|(platform, info)| {
            // 从URL中提取文件名
            let name = info
                .url
                .split('/')
                .next_back()
                .unwrap_or(&platform)
                .to_string();

            debug!(
                "转换平台资产: platform={}, name={}, url={}",
                platform, name, info.url
            );

            // 根据文件扩展名推断content_type
            let content_type = if name.ends_with(".tar.gz") || name.ends_with(".tgz") {
                "application/gzip".to_string()
            } else if name.ends_with(".zip") {
                "application/zip".to_string()
            } else if name.ends_with(".msi") {
                "application/x-msi".to_string()
            } else if name.ends_with(".AppImage") {
                "application/x-executable".to_string()
            } else {
                "application/octet-stream".to_string()
            };

            GitHubAsset {
                name: format!("{platform}|{name}"), // 包含平台信息以便调试
                size: 0,                            // Tauri format doesn't include size
                download_count: 0,                  // Tauri format doesn't include download count
                browser_download_url: info.url,
                content_type,
            }
        })
        .collect();

    GitHubRelease {
        tag_name: tauri_response.version.clone(),
        name: format!("Release {}", tauri_response.version),
        body: tauri_response.notes,
        draft: false,
        prerelease: false,
        published_at: tauri_response.pub_date,
        html_url: None,
        assets,
    }
}

impl UpdateSourceManager {
    /// 创建默认的更新源管理器（版本检查服务器优先，GitHub 备用）
    pub fn new() -> Self {
        Self {
            sources: vec![UpdateSource::VersionServer, UpdateSource::GitHub],
        }
    }

    /// 获取版本信息，按优先级尝试各个源
    pub async fn fetch_latest_version(&self) -> Result<GitHubRelease> {
        let mut last_error = None;

        for source in &self.sources {
            match source {
                UpdateSource::VersionServer => {
                    info!("{}", t!("check_update.try_version_server"));
                    match self.fetch_from_version_server().await {
                        Ok(release) => {
                            info!("{}", t!("check_update.version_server_success"));
                            return Ok(release);
                        }
                        Err(e) => {
                            warn!("{}", t!("check_update.version_server_failed", error = e.to_string()));
                            last_error = Some(e);
                        }
                    }
                }
                UpdateSource::GitHub => {
                    info!("{}", t!("check_update.try_github"));
                    match self.fetch_from_github().await {
                        Ok(release) => {
                            info!("{}", t!("check_update.github_success"));
                            return Ok(release);
                        }
                        Err(e) => {
                            warn!("{}", t!("check_update.github_failed", error = e.to_string()));
                            last_error = Some(e);
                        }
                    }
                }
            }
        }

        Err(last_error.unwrap_or_else(|| anyhow::anyhow!("{}", t!("check_update.all_sources_failed"))))
    }

    /// 从版本检查服务器获取版本信息
    async fn fetch_from_version_server(&self) -> Result<GitHubRelease> {
        let client = reqwest::Client::new();
        let url = get_cli_api_url();

        info!("{}", t!("check_update.checking_version", url = url.as_str()));

        let response = client
            .get(&url)
            .header("User-Agent", format!("nuwax-cli/{}", get_current_version()))
            .timeout(std::time::Duration::from_secs(10))
            .send()
            .await
            .context(t!("check_update.connect_server_failed"))?;

        if !response.status().is_success() {
            let status = response.status();
            let error_text = response.text().await.unwrap_or_default();
            return Err(anyhow::anyhow!(
                "{}",
                t!("check_update.server_api_failed", status = status.to_string(), error = error_text)
            ));
        }

        // 先尝试解析为 Tauri updater 格式
        let tauri_response: TauriUpdaterResponse = response
            .json()
            .await
            .context(t!("check_update.parse_server_response_failed"))?;
        let release = convert_tauri_to_github_release(tauri_response);
        Ok(release)
    }

    /// 从GitHub获取版本信息
    async fn fetch_from_github(&self) -> Result<GitHubRelease> {
        let repo = GitHubRepo::default();
        let client = reqwest::Client::new();
        let url = repo.latest_release_url();

        info!("{}", t!("check_update.checking_version", url = url.as_str()));

        let response = client
            .get(&url)
            .header("User-Agent", format!("nuwax-cli/{}", get_current_version()))
            .timeout(std::time::Duration::from_secs(15))
            .send()
            .await
            .context(t!("check_update.connect_github_failed"))?;

        if !response.status().is_success() {
            let status = response.status();
            let error_text = response.text().await.unwrap_or_default();
            return Err(anyhow::anyhow!(
                "{}",
                t!("check_update.github_api_failed", status = status.to_string(), error = error_text)
            ));
        }

        let release: GitHubRelease = response.json().await.context(t!("check_update.parse_github_response_failed"))?;
        Ok(release)
    }
}

/// GitHub仓库配置
pub struct GitHubRepo {
    pub owner: String,
    pub repo: String,
}

impl GitHubRepo {
    pub fn new(owner: &str, repo: &str) -> Self {
        Self {
            owner: owner.to_string(),
            repo: repo.to_string(),
        }
    }

    /// 创建默认的仓库配置（从 Cargo.toml 读取）
    pub fn default() -> Self {
        let (owner, repo) = parse_github_repo();
        Self::new(owner, repo)
    }

    /// 获取最新release API URL
    pub fn latest_release_url(&self) -> String {
        format!(
            "https://api.github.com/repos/{}/{}/releases/latest",
            self.owner, self.repo
        )
    }
}

/// 获取当前版本
pub fn get_current_version() -> String {
    format!("v{}", env!("CARGO_PKG_VERSION"))
}

/// 从更新源获取最新版本信息
pub async fn fetch_latest_version_multi_source() -> Result<GitHubRelease> {
    let source_manager = UpdateSourceManager::new();
    source_manager.fetch_latest_version().await
}

/// 比较版本号
pub fn compare_versions(current: &str, latest: &str) -> std::cmp::Ordering {
    // 简单的版本比较，假设版本格式为 v1.2.3 或 1.2.3
    let normalize_version = |v: &str| -> String { v.trim_start_matches('v').to_string() };

    let current_norm = normalize_version(current);
    let latest_norm = normalize_version(latest);

    // 使用语义版本比较（简化版）
    let parse_version = |v: &str| -> Vec<u32> {
        v.split('.')
            .map(|s| s.parse::<u32>().unwrap_or(0))
            .collect()
    };

    let current_parts = parse_version(&current_norm);
    let latest_parts = parse_version(&latest_norm);

    current_parts.cmp(&latest_parts)
}

/// 检查更新
pub async fn check_for_updates() -> Result<VersionInfo> {
    // 添加详细的调试日志
    info!("{}", t!("check_update.start_check"));

    let current_version = get_current_version();
    info!("{}", t!("check_update.current_version", version = current_version));
    info!("{}", t!("check_update.fetching_latest"));

    let latest_release = fetch_latest_version_multi_source().await?;
    let latest_version = latest_release.tag_name.clone();
    info!("{}", t!("check_update.server_version", version = latest_version));

    // 版本比较
    let comparison = compare_versions(&current_version, &latest_version);
    info!(
        "{}",
        t!("check_update.version_comparison",
            result = format!("{:?}", comparison),
            current = current_version,
            latest = latest_version)
    );

    let is_update_available = comparison == std::cmp::Ordering::Less;
    if is_update_available {
        info!(
            "{}",
            t!("check_update.update_available",
                current = current_version,
                latest = latest_version)
        );
    } else {
        info!(
            "{}",
            t!("check_update.already_latest",
                current = current_version,
                latest = latest_version)
        );
    }

    // 查找适合当前平台的下载链接
    debug!("{}", t!("check_update.finding_platform_asset"));
    let download_url = find_platform_asset(&latest_release.assets);
    if let Some(url) = &download_url {
        debug!("{}", t!("check_update.found_platform_asset", url = url));
    } else {
        warn!("{}", t!("check_update.no_platform_asset"));
    }

    let version_info = VersionInfo {
        current_version,
        latest_version,
        is_update_available,
        release_notes: latest_release.body,
        download_url,
        published_at: latest_release.published_at,
    };

    info!("{}", t!("check_update.check_complete", available = is_update_available));
    Ok(version_info)
}

/// 查找适合当前平台的资源
fn find_platform_asset(assets: &[GitHubAsset]) -> Option<String> {
    let os = std::env::consts::OS;
    let arch = std::env::consts::ARCH;

    info!("{}", t!("check_update.detected_platform", os = os, arch = arch));
    info!("{}", t!("check_update.asset_count", count = assets.len()));

    // 构建目标平台键（兼容 Tauri updater 格式）
    let target_platform = match (os, arch) {
        ("windows", "x86_64") => "windows-x86_64",
        ("windows", "x86") => "windows-x86",
        ("linux", "x86_64") => "linux-x86_64",
        ("linux", "aarch64") => "linux-aarch64",
        ("macos", "x86_64") => "darwin-x86_64",
        ("macos", "aarch64") => "darwin-aarch64",
        _ => return None,
    };

    info!("{}", t!("check_update.target_platform", platform = target_platform));

    // 首先尝试精确匹配平台键
    for (index, asset) in assets.iter().enumerate() {
        info!(
            "{}",
            t!("check_update.checking_asset",
                index = index,
                name = asset.name,
                size = asset.size,
                url = asset.browser_download_url)
        );

        // 检查是否包含平台键
        if asset.name.contains(target_platform) {
            info!("{}", t!("check_update.found_exact_match", name = asset.name));
            return Some(asset.browser_download_url.clone());
        }
    }

    // 如果没有精确匹配，尝试从文件名匹配
    let platform_patterns = match (os, arch) {
        ("windows", "x86_64") => vec!["windows", "win64", "x86_64-pc-windows", "x64"],
        ("windows", "x86") => vec!["windows", "win32", "i686-pc-windows", "x86"],
        ("linux", "x86_64") => vec!["linux", "x86_64-unknown-linux", "x64", "amd64"],
        ("linux", "aarch64") => vec!["linux", "aarch64-unknown-linux", "arm64", "aarch64"],
        ("macos", "x86_64") => vec!["macos", "darwin", "x86_64-apple-darwin", "x64"],
        ("macos", "aarch64") => vec![
            "macos",
            "darwin",
            "aarch64-apple-darwin",
            "arm64",
            "aarch64",
        ],
        _ => vec![os, arch],
    };

    info!("{}", t!("check_update.platform_patterns", patterns = format!("{:?}", platform_patterns)));

    // 查找匹配的资源
    for (index, asset) in assets.iter().enumerate() {
        let name_lower = asset.name.to_lowercase();
        let url_lower = asset.browser_download_url.to_lowercase();

        info!(
            "{}",
            t!("check_update.pattern_match_check",
                index = index,
                name = asset.name,
                url = asset.browser_download_url)
        );

        // 检查名称或URL是否包含平台模式
        if platform_patterns
            .iter()
            .any(|pattern| name_lower.contains(pattern) || url_lower.contains(pattern))
        {
            info!("{}", t!("check_update.found_pattern_match", name = asset.name));
            // 优先选择可执行文件
            if name_lower.contains("nuwax-cli")
                || name_lower.ends_with(".exe")
                || name_lower.ends_with(".tar.gz")
                || name_lower.ends_with(".msi")
                || name_lower.ends_with(".appimage")
            {
                info!("{}", t!("check_update.selected_asset", name = asset.name));
                return Some(asset.browser_download_url.clone());
            }
        }
    }

    warn!("{}", t!("check_update.no_match_try_executable"));
    // 如果没找到精确匹配，返回第一个看起来像可执行文件的资源
    for (index, asset) in assets.iter().enumerate() {
        let name = asset.name.to_lowercase();
        let is_executable = name.contains("nuwax-cli")
            || name.ends_with(".exe")
            || name.ends_with(".tar.gz")
            || name.ends_with(".msi")
            || name.ends_with(".appimage");

        info!(
            "{}",
            t!("check_update.checking_executable",
                index = index,
                name = asset.name,
                is_executable = is_executable)
        );

        if is_executable {
            info!("{}", t!("check_update.found_executable", name = asset.name));
            return Some(asset.browser_download_url.clone());
        }
    }

    warn!("{}", t!("check_update.no_executable_found"));
    None
}

/// 显示版本检查结果
pub fn display_version_info(version_info: &VersionInfo) {
    info!("{}", t!("check_update.version_info_title"));
    info!("{}", t!("check_update.current_version_display", version = version_info.current_version));
    info!("{}", t!("check_update.latest_version_display", version = version_info.latest_version));

    if version_info.is_update_available {
        info!("{}", t!("check_update.new_version_available"));
        if let Some(ref url) = version_info.download_url {
            info!("{}", t!("check_update.download_url", url = url));
        }

        // 显示发布说明（截取前500字符）
        if !version_info.release_notes.is_empty() {
            let notes = if version_info.release_notes.len() > 500 {
                format!("{}...", &version_info.release_notes[..500])
            } else {
                version_info.release_notes.clone()
            };
            info!("{}", t!("check_update.release_notes", notes = notes));
        }

        // 解析并显示发布时间
        if let Ok(published_time) = DateTime::parse_from_rfc3339(&version_info.published_at) {
            info!("{}", t!("check_update.published_at", time = published_time.format("%Y-%m-%d %H:%M:%S")));
        }

        info!("{}", t!("check_update.install_command_hint"));
    } else {
        info!("{}", t!("check_update.using_latest_version"));
    }
}

/// 检查版本并决定是否需要安装
pub async fn should_install(target_version: Option<&str>, force: bool) -> Result<(String, String)> {
    let current_version = get_current_version();

    let target_version = if let Some(version) = target_version {
        version.to_string()
    } else {
        // 获取最新版本
        let latest_release = fetch_latest_version_multi_source().await?;
        latest_release.tag_name
    };

    if !force && compare_versions(&current_version, &target_version) != std::cmp::Ordering::Less {
        return Err(anyhow::anyhow!(
            "{}",
            t!("check_update.already_latest_or_higher",
                current = current_version,
                target = target_version)
        ));
    }

    Ok((current_version, target_version))
}

/// 下载并安装新版本
pub async fn install_release(url: &str, version: &str) -> Result<()> {
    let client = reqwest::Client::new();

    // 创建临时目录
    let temp_dir = std::env::temp_dir().join("nuwax-cli-updates");
    std::fs::create_dir_all(&temp_dir)?;

    // 确定文件名
    let default_filename = format!("nuwax-cli-{version}");
    let filename = url.split('/').next_back().unwrap_or(&default_filename);
    let download_path = temp_dir.join(filename);

    info!("{}", t!("check_update.downloading_version", version = version, url = url));
    info!("{}", t!("check_update.temp_save_path", path = download_path.display()));

    // 下载文件
    let response = client
        .get(url)
        .header("User-Agent", format!("nuwax-cli/{}", get_current_version()))
        .send()
        .await
        .context(t!("check_update.download_failed"))?;

    if !response.status().is_success() {
        return Err(anyhow::anyhow!("{}", t!("check_update.download_http_failed", status = response.status())));
    }

    let total_size = response.content_length().unwrap_or(0);
    info!("{}", t!("check_update.file_size", size = total_size));

    let bytes = response.bytes().await?;
    std::fs::write(&download_path, bytes)?;

    info!("{}", t!("check_update.download_complete_start_install"));

    // 获取当前可执行文件路径
    let current_exe = std::env::current_exe().context(t!("check_update.cannot_get_exe_path"))?;

    info!("{}", t!("check_update.current_exe", path = current_exe.display()));

    // 处理不同文件类型的安装
    install_downloaded_file(&download_path, &current_exe, version).await?;

    // 清理临时文件
    if let Err(e) = std::fs::remove_file(&download_path) {
        warn!("{}", t!("check_update.cleanup_temp_failed", error = e.to_string()));
    }

    info!(
        "{}",
        t!("check_update.install_complete", version = version)
    );
    info!("{}", t!("check_update.rerun_command_hint"));

    Ok(())
}

/// 安装下载的文件
async fn install_downloaded_file(
    download_path: &PathBuf,
    current_exe: &PathBuf,
    version: &str,
) -> Result<()> {
    let download_name = download_path
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("");

    if download_name.ends_with(".tar.gz") || download_name.ends_with(".tgz") {
        // 处理压缩包
        install_from_archive(download_path, current_exe, version).await
    } else if download_name.ends_with(".exe") || download_name.contains("nuwax-cli") {
        // 直接可执行文件
        install_executable(download_path, current_exe).await
    } else {
        Err(anyhow::anyhow!("{}", t!("check_update.unsupported_format", format = download_name)))
    }
}

/// 安装可执行文件
async fn install_executable(download_path: &PathBuf, current_exe: &PathBuf) -> Result<()> {
    // 创建备份
    let backup_path = if cfg!(target_os = "windows") {
        current_exe.with_extension("exe.backup")
    } else {
        PathBuf::from(format!("{}.backup", current_exe.display()))
    };

    if let Err(e) = std::fs::copy(current_exe, &backup_path) {
        warn!("{}", t!("check_update.backup_failed", error = e.to_string()));
    } else {
        info!("{}", t!("check_update.backup_created", path = backup_path.display()));
    }

    // 在 Unix 系统上设置可执行权限
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut perms = std::fs::metadata(download_path)?.permissions();
        perms.set_mode(0o755);
        std::fs::set_permissions(download_path, perms)?;
    }

    // 使用 self-replace 库进行文件替换
    info!("{}", t!("check_update.replacing_exe"));
    match self_replace::self_replace(download_path) {
        Ok(()) => {
            info!("{}", t!("check_update.replace_success"));
            Ok(())
        }
        Err(e) => {
            warn!("{}", t!("check_update.replace_failed", error = e.to_string()));

            // 尝试恢复备份
            if backup_path.exists() {
                info!("{}", t!("check_update.trying_restore"));
                match std::fs::copy(&backup_path, current_exe) {
                    Ok(_) => {
                        warn!("{}", t!("check_update.restore_success"));
                        return Err(anyhow::anyhow!("{}", t!("check_update.replace_failed_restored", error = e.to_string())));
                    }
                    Err(restore_err) => {
                        error!("{}", t!("check_update.restore_failed", error = restore_err.to_string()));
                        return Err(anyhow::anyhow!(
                            "{}",
                            t!("check_update.replace_and_restore_failed",
                                error = e.to_string(),
                                restore_error = restore_err.to_string())
                        ));
                    }
                }
            }

            Err(anyhow::anyhow!("{}", t!("check_update.replace_failed", error = e.to_string())))
        }
    }
}

/// 从压缩包安装
async fn install_from_archive(
    archive_path: &Path,
    current_exe: &PathBuf,
    _version: &str,
) -> Result<()> {
    use std::process::Command;

    let temp_dir = std::env::temp_dir().join("nuwax-cli-extract");
    std::fs::create_dir_all(&temp_dir)?;

    info!("{}", t!("check_update.extracting_archive"));

    // 解压 tar.gz 文件
    let output = Command::new("tar")
        .args([
            "-xzf",
            &archive_path.to_string_lossy(),
            "-C",
            &temp_dir.to_string_lossy(),
        ])
        .output()
        .context(t!("check_update.extract_failed_tar"))?;

    if !output.status.success() {
        return Err(anyhow::anyhow!(
            "{}",
            t!("check_update.extract_failed", error = String::from_utf8_lossy(&output.stderr))
        ));
    }

    // 查找可执行文件
    let executable_path = find_executable_in_dir(&temp_dir)?;

    // 安装可执行文件
    install_executable(&executable_path, current_exe).await?;

    // 清理解压目录
    if let Err(e) = std::fs::remove_dir_all(&temp_dir) {
        warn!("{}", t!("check_update.cleanup_extract_failed", error = e.to_string()));
    }

    Ok(())
}

/// 在目录中查找可执行文件
fn find_executable_in_dir(dir: &PathBuf) -> Result<PathBuf> {
    for entry in std::fs::read_dir(dir)? {
        let entry = entry?;
        let path = entry.path();

        if path.is_file() {
            let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");

            if name.contains("nuwax-cli") || name.ends_with(".exe") {
                return Ok(path);
            }
        }

        // 递归查找子目录
        if path.is_dir() {
            if let Ok(found) = find_executable_in_dir(&path) {
                return Ok(found);
            }
        }
    }

    Err(anyhow::anyhow!("{}", t!("check_update.no_executable_in_archive")))
}

/// 处理 check-update 命令
pub async fn handle_check_update_command(command: CheckUpdateCommand) -> Result<()> {
    match command {
        CheckUpdateCommand::Check => {
            info!("{}", t!("check_update.checking_updates"));

            match check_for_updates().await {
                Ok(version_info) => {
                    display_version_info(&version_info);
                }
                Err(e) => {
                    warn!("{}", t!("check_update.check_failed", error = e.to_string()));
                    info!("{}", t!("check_update.current_version_display", version = get_current_version()));
                    info!("{}", t!("check_update.possible_reasons"));
                    info!("   - {}", t!("check_update.reason_network"));
                    info!("   - {}", t!("check_update.reason_server_unavailable"));
                    info!("   - {}", t!("check_update.reason_github_unavailable"));
                    info!("   - {}", t!("check_update.reason_no_release"));
                    return Err(e);
                }
            }
        }

        CheckUpdateCommand::Install { version, force } => {
            info!("{}", t!("check_update.start_install"));

            // 检查是否需要安装
            let (current_version, target_version) =
                match should_install(version.as_deref(), force).await {
                    Ok(versions) => versions,
                    Err(e) => {
                        if force {
                            warn!("⚠️  {}", e);
                            info!("{}", t!("check_update.force_continue"));
                            // 如果强制安装但没指定版本，返回错误
                            if version.is_none() {
                                return Err(anyhow::anyhow!("{}", t!("check_update.force_needs_version")));
                            }
                            (get_current_version(), version.as_ref().unwrap().clone())
                        } else {
                            warn!("❌ {}", e);
                            return Err(e);
                        }
                    }
                };

            info!(
                "{}",
                t!("check_update.preparing_update",
                    current = current_version,
                    target = target_version)
            );

            // 获取指定版本的下载链接
            let download_url = if let Some(ref ver) = version {
                // 指定了版本，需要获取该版本的信息
                get_version_download_url(ver).await?
            } else {
                // 没有指定版本，获取最新版本的下载链接
                let version_info = check_for_updates().await?;
                version_info
                    .download_url
                    .ok_or_else(|| anyhow::anyhow!("{}", t!("check_update.no_platform_download_url")))?
            };

            info!("{}", t!("check_update.start_download_install", version = target_version));

            match install_release(&download_url, &target_version).await {
                Ok(_) => {
                    info!("{}", t!("check_update.install_success"));
                    info!("{}", t!("check_update.restart_to_verify"));
                }
                Err(e) => {
                    warn!("{}", t!("check_update.install_failed", error = e.to_string()));
                    info!("{}", t!("check_update.possible_solutions"));
                    info!("   - {}", t!("check_update.solution_network"));
                    info!("   - {}", t!("check_update.solution_disk_space"));
                    info!("   - {}", t!("check_update.solution_admin"));
                    return Err(e);
                }
            }
        }
    }

    Ok(())
}

/// 获取指定版本的下载链接
async fn get_version_download_url(version: &str) -> Result<String> {
    // 这里应该获取指定版本的release信息
    // 为了简化，我们先使用最新版本，后续可以扩展支持获取指定版本
    let version_info = check_for_updates().await?;

    version_info
        .download_url
        .ok_or_else(|| anyhow::anyhow!("{}", t!("check_update.no_version_download_url", version = version)))
}
