use anyhow::{Context, Result};
use chrono::DateTime;
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

//cli 命令工具请求的地址 (阿里云 OSS)
pub const CLI_API_URL: &str = "https://nuwa-packages.oss-rg-china-mainland.aliyuncs.com/nuwax-cli/latest/latest.json";

/// 获取完整的 CLI API URL
pub fn get_cli_api_url() -> String {
    CLI_API_URL.to_string()
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
                "Converting platform asset: platform={}, name={}, url={}",
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
                    info!("📡 Trying version check server API...");
                    match self.fetch_from_version_server().await {
                        Ok(release) => {
                            info!("✅ Version check server API success");
                            return Ok(release);
                        }
                        Err(e) => {
                            warn!("⚠️ Version check server API failed: {error}", error = e.to_string());
                            last_error = Some(e);
                        }
                    }
                }
                UpdateSource::GitHub => {
                    info!("📡 Trying GitHub API...");
                    match self.fetch_from_github().await {
                        Ok(release) => {
                            info!("✅ GitHub API success");
                            return Ok(release);
                        }
                        Err(e) => {
                            warn!("⚠️ GitHub API failed: {error}", error = e.to_string());
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

        info!("📡 Checking latest version: {url}", url = url.as_str());

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

        info!("📡 Checking latest version: {url}", url = url.as_str());

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
    info!("🔍 Starting nuwax-cli update check...");

    let current_version = get_current_version();
    info!("📋 Current detected version: {version}", version = current_version);
    info!("🌐 Fetching latest version info...");

    let latest_release = fetch_latest_version_multi_source().await?;
    let latest_version = latest_release.tag_name.clone();
    info!("📋 Server latest version: {version}", version = latest_version);

    // 版本比较
    let comparison = compare_versions(&current_version, &latest_version);
    info!("📊 Version comparison result: {result} ({current} vs {latest})", result = format!("{:?}", comparison), current = current_version, latest = latest_version);

    let is_update_available = comparison == std::cmp::Ordering::Less;
    if is_update_available {
        info!("✅ New version available! Need to upgrade from {current} to {latest}", current = current_version, latest = latest_version);
    } else {
        info!("✅ Current version is latest: {current} = {latest}", current = current_version, latest = latest_version);
    }

    // 查找适合当前平台的下载链接
    debug!("🔍 Finding platform-specific download package...");
    let download_url = find_platform_asset(&latest_release.assets);
    if let Some(url) = &download_url {
        debug!("📦 Found platform-specific package: {url}", url = url);
    } else {
        warn!("⚠️ No platform-specific download package found");
    }

    let version_info = VersionInfo {
        current_version,
        latest_version,
        is_update_available,
        release_notes: latest_release.body,
        download_url,
        published_at: latest_release.published_at,
    };

    info!("✅ Version check complete, update available: {available}", available = is_update_available);
    Ok(version_info)
}

/// 查找适合当前平台的资源
fn find_platform_asset(assets: &[GitHubAsset]) -> Option<String> {
    let os = std::env::consts::OS;
    let arch = std::env::consts::ARCH;

    info!("🖥️ Detected platform: OS={os}, ARCH={arch}", os = os, arch = arch);
    info!("📦 Available assets: {count}", count = assets.len());

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

    info!("🎯 Target platform key: {platform}", platform = target_platform);

    // 首先尝试精确匹配平台键
    for (index, asset) in assets.iter().enumerate() {
        info!("📋 Checking asset[{index}]: name={name}, size={size}, url={url}", index = index, name = asset.name, size = asset.size, url = asset.browser_download_url);

        // 检查是否包含平台键
        if asset.name.contains(target_platform) {
            info!("✅ Found exact platform match: {name}", name = asset.name);
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

    info!("🎯 Platform match patterns: {patterns}", patterns = format!("{:?}", platform_patterns));

    // 查找匹配的资源
    for (index, asset) in assets.iter().enumerate() {
        let name_lower = asset.name.to_lowercase();
        let url_lower = asset.browser_download_url.to_lowercase();

        info!("🔍 Pattern match check[{index}]: name={name}, url={url}", index = index, name = asset.name, url = asset.browser_download_url);

        // 检查名称或URL是否包含平台模式
        if platform_patterns
            .iter()
            .any(|pattern| name_lower.contains(pattern) || url_lower.contains(pattern))
        {
            info!("✅ Found pattern match: {name}", name = asset.name);
            // 优先选择可执行文件
            if name_lower.contains("nuwax-cli")
                || name_lower.ends_with(".exe")
                || name_lower.ends_with(".tar.gz")
                || name_lower.ends_with(".msi")
                || name_lower.ends_with(".appimage")
            {
                info!("🎯 Selected asset: {name}", name = asset.name);
                return Some(asset.browser_download_url.clone());
            }
        }
    }

    warn!("⚠️ No matching assets found, trying to find executable...");
    // 如果没找到精确匹配，返回第一个看起来像可执行文件的资源
    for (index, asset) in assets.iter().enumerate() {
        let name = asset.name.to_lowercase();
        let is_executable = name.contains("nuwax-cli")
            || name.ends_with(".exe")
            || name.ends_with(".tar.gz")
            || name.ends_with(".msi")
            || name.ends_with(".appimage");

        info!("🔍 Checking executable[{index}]: {name} -> executable: {is_executable}", index = index, name = asset.name, is_executable = is_executable);

        if is_executable {
            info!("✅ Found executable: {name}", name = asset.name);
            return Some(asset.browser_download_url.clone());
        }
    }

    warn!("❌ No executable files found");
    None
}

/// 显示版本检查结果
pub fn display_version_info(version_info: &VersionInfo) {
    info!("🦆 Nuwax CLI Version Info");
    info!("Current version: {version}", version = version_info.current_version);
    info!("Latest version: {version}", version = version_info.latest_version);

    if version_info.is_update_available {
        info!("✅ New version available!");
        if let Some(ref url) = version_info.download_url {
            info!("Download URL: {url}", url = url);
        }

        // 显示发布说明（截取前500字符）
        if !version_info.release_notes.is_empty() {
            let notes = if version_info.release_notes.len() > 500 {
                format!("{}...", &version_info.release_notes[..500])
            } else {
                version_info.release_notes.clone()
            };
            info!("Release notes:
{notes}", notes = notes);
        }

        // 解析并显示发布时间
        if let Ok(published_time) = DateTime::parse_from_rfc3339(&version_info.published_at) {
            info!("Published at: {time}", time = published_time.format("%Y-%m-%d %H:%M:%S"));
        }

        info!("💡 Use the following command to install update:");
    } else {
        info!("✅ You are using the latest version!");
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

    info!("📥 Downloading version {version}: {url}", version = version, url = url);
    info!("💾 Temp save to: {path}", path = download_path.display());

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
    info!("📦 File size: {size} bytes", size = total_size);

    let bytes = response.bytes().await?;
    std::fs::write(&download_path, bytes)?;

    info!("✅ Download complete, starting installation...");

    // 获取当前可执行文件路径
    let current_exe = std::env::current_exe().context(t!("check_update.cannot_get_exe_path"))?;

    info!("🔧 Current executable: {path}", path = current_exe.display());

    // 处理不同文件类型的安装
    install_downloaded_file(&download_path, &current_exe, version).await?;

    // 清理临时文件
    if let Err(e) = std::fs::remove_file(&download_path) {
        warn!("Failed to cleanup temp file: {error}", error = e.to_string());
    }

    info!("🎉 Installation complete! Nuwax CLI has been updated to version {version}, run 'nuwax-cli --version' to verify", version = version);
    info!("💡 Please run 'nuwax-cli auto-upgrade-deploy run' again to complete final deployment");

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
        warn!("Failed to create backup: {error}", error = e.to_string());
    } else {
        info!("✅ Backup created: {path}", path = backup_path.display());
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
    info!("🔧 Replacing executable...");
    match self_replace::self_replace(download_path) {
        Ok(()) => {
            info!("✅ Executable replaced successfully");
            Ok(())
        }
        Err(e) => {
            warn!("❌ File replacement failed: {error}", error = e.to_string());

            // 尝试恢复备份
            if backup_path.exists() {
                info!("🔄 Trying to restore from backup...");
                match std::fs::copy(&backup_path, current_exe) {
                    Ok(_) => {
                        warn!("✅ Restored from backup");
                        return Err(anyhow::anyhow!("{}", t!("check_update.replace_failed_restored", error = e.to_string())));
                    }
                    Err(restore_err) => {
                        error!("❌ Backup restore also failed: {error}", error = restore_err.to_string());
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

    info!("📦 Extracting archive...");

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
        warn!("Failed to cleanup extract directory: {error}", error = e.to_string());
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
            info!("🔍 Checking Nuwax CLI updates...");

            match check_for_updates().await {
                Ok(version_info) => {
                    display_version_info(&version_info);
                }
                Err(e) => {
                    warn!("❌ Check update failed: {error}", error = e.to_string());
                    info!("Current version: {version}", version = get_current_version());
                    info!("💡 Possible reasons:");
                    info!("   - Network connection issue");
                    info!("   - Version check server temporarily unavailable");
                    info!("   - GitHub API temporarily unavailable");
                    info!("   - Project has not released any version yet");
                    return Err(e);
                }
            }
        }

        CheckUpdateCommand::Install { version, force } => {
            info!("🚀 Starting Nuwax CLI installation...");

            // 检查是否需要安装
            let (current_version, target_version) =
                match should_install(version.as_deref(), force).await {
                    Ok(versions) => versions,
                    Err(e) => {
                        if force {
                            warn!("⚠️  {}", e);
                            info!("🔧 Continuing installation with --force flag...");
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

            info!("Preparing to update from {current} to {target}", current = current_version, target = target_version);

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

            info!("📥 Starting download and install version {version}...", version = target_version);

            match install_release(&download_url, &target_version).await {
                Ok(_) => {
                    info!("🎉 Installation successful!");
                    info!("Please restart terminal to verify installation");
                }
                Err(e) => {
                    warn!("❌ Installation failed: {error}", error = e.to_string());
                    info!("💡 Possible solutions:");
                    info!("   - Check network connection");
                    info!("   - Ensure sufficient disk space");
                    info!("   - Run with administrator privileges");
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
