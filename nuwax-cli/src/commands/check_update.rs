use anyhow::{Context, Result};
use chrono::DateTime;
use client_core::constants::api;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use tracing::{debug, error, info, warn};

/// GitHub 仓库常量配置
pub const GITHUB_OWNER: &str = "soddygo";
pub const GITHUB_REPO: &str = "duck_client";

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
                    info!("📡 尝试使用版本检查服务器API获取版本信息...");
                    match self.fetch_from_version_server().await {
                        Ok(release) => {
                            info!("✅ 版本检查服务器API获取成功");
                            return Ok(release);
                        }
                        Err(e) => {
                            warn!("⚠️ 版本检查服务器API获取失败: {}", e);
                            last_error = Some(e);
                        }
                    }
                }
                UpdateSource::GitHub => {
                    info!("📡 尝试使用GitHub API获取版本信息...");
                    match self.fetch_from_github().await {
                        Ok(release) => {
                            info!("✅ GitHub API获取成功");
                            return Ok(release);
                        }
                        Err(e) => {
                            warn!("⚠️ GitHub API获取失败: {}", e);
                            last_error = Some(e);
                        }
                    }
                }
            }
        }

        Err(last_error.unwrap_or_else(|| anyhow::anyhow!("所有更新源都不可用")))
    }

    /// 从版本检查服务器获取版本信息
    async fn fetch_from_version_server(&self) -> Result<GitHubRelease> {
        let client = reqwest::Client::new();
        let url = get_cli_api_url();

        info!("📡 正在检查最新版本: {}", url);

        let response = client
            .get(&url)
            .header("User-Agent", format!("nuwax-cli/{}", get_current_version()))
            .timeout(std::time::Duration::from_secs(10))
            .send()
            .await
            .context("无法连接到版本检查服务器")?;

        if !response.status().is_success() {
            let status = response.status();
            let error_text = response.text().await.unwrap_or_default();
            return Err(anyhow::anyhow!(
                "版本检查服务器API请求失败: {} - {}",
                status,
                error_text
            ));
        }

        // 先尝试解析为 Tauri updater 格式
        let tauri_response: TauriUpdaterResponse = response
            .json()
            .await
            .context("解析版本检查服务器API响应失败")?;
        let release = convert_tauri_to_github_release(tauri_response);
        Ok(release)
    }

    /// 从GitHub获取版本信息
    async fn fetch_from_github(&self) -> Result<GitHubRelease> {
        let repo = GitHubRepo::default();
        let client = reqwest::Client::new();
        let url = repo.latest_release_url();

        info!("📡 正在检查最新版本: {}", url);

        let response = client
            .get(&url)
            .header("User-Agent", format!("nuwax-cli/{}", get_current_version()))
            .timeout(std::time::Duration::from_secs(15))
            .send()
            .await
            .context("无法连接到GitHub API")?;

        if !response.status().is_success() {
            let status = response.status();
            let error_text = response.text().await.unwrap_or_default();
            return Err(anyhow::anyhow!(
                "GitHub API请求失败: {} - {}",
                status,
                error_text
            ));
        }

        let release: GitHubRelease = response.json().await.context("解析GitHub API响应失败")?;
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

    /// 创建默认的 duck_client 仓库配置
    pub fn default() -> Self {
        Self::new(GITHUB_OWNER, GITHUB_REPO)
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
    info!("🔍 开始检查 nuwax-cli 更新...");

    let current_version = get_current_version();
    info!("📋 当前检测到的版本: {}", current_version);
    info!("🌐 正在获取最新版本信息...");

    let latest_release = fetch_latest_version_multi_source().await?;
    let latest_version = latest_release.tag_name.clone();
    info!("📋 服务器最新版本: {}", latest_version);

    // 版本比较
    let comparison = compare_versions(&current_version, &latest_version);
    info!(
        "📊 版本比较结果: {:?} ({} vs {})",
        comparison, current_version, latest_version
    );

    let is_update_available = comparison == std::cmp::Ordering::Less;
    if is_update_available {
        info!(
            "✅ 发现新版本可用! 需要从 {} 升级到 {}",
            current_version, latest_version
        );
    } else {
        info!(
            "✅ 当前版本已是最新: {} = {}",
            current_version, latest_version
        );
    }

    // 查找适合当前平台的下载链接
    debug!("🔍 查找适合当前平台的下载包...");
    let download_url = find_platform_asset(&latest_release.assets);
    if let Some(url) = &download_url {
        debug!("📦 找到适合的下载包: {}", url);
    } else {
        warn!("⚠️ 未找到适合当前平台的下载包");
    }

    let version_info = VersionInfo {
        current_version,
        latest_version,
        is_update_available,
        release_notes: latest_release.body,
        download_url,
        published_at: latest_release.published_at,
    };

    info!("✅ 版本检查完成，结果: 更新可用={}", is_update_available);
    Ok(version_info)
}

/// 查找适合当前平台的资源
fn find_platform_asset(assets: &[GitHubAsset]) -> Option<String> {
    use tracing::debug;

    let os = std::env::consts::OS;
    let arch = std::env::consts::ARCH;

    info!("🖥️ 检测到平台: OS={}, ARCH={}", os, arch);
    info!("📦 可用资产数量: {}", assets.len());

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

    info!("🎯 目标平台键: {}", target_platform);

    // 首先尝试精确匹配平台键
    for (index, asset) in assets.iter().enumerate() {
        info!(
            "📋 检查资产[{}]: name={}, size={}, url={}",
            index, asset.name, asset.size, asset.browser_download_url
        );

        // 检查是否包含平台键
        if asset.name.contains(target_platform) {
            info!("✅ 找到精确匹配的平台资产: {}", asset.name);
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

    info!("🎯 平台匹配模式: {:?}", platform_patterns);

    // 查找匹配的资源
    for (index, asset) in assets.iter().enumerate() {
        let name_lower = asset.name.to_lowercase();
        let url_lower = asset.browser_download_url.to_lowercase();

        info!(
            "🔍 模式匹配检查[{}]: name={}, url={}",
            index, asset.name, asset.browser_download_url
        );

        // 检查名称或URL是否包含平台模式
        if platform_patterns
            .iter()
            .any(|pattern| name_lower.contains(pattern) || url_lower.contains(pattern))
        {
            info!("✅ 找到模式匹配的资产: {}", asset.name);
            // 优先选择可执行文件
            if name_lower.contains("nuwax-cli")
                || name_lower.ends_with(".exe")
                || name_lower.ends_with(".tar.gz")
                || name_lower.ends_with(".msi")
                || name_lower.ends_with(".appimage")
            {
                info!("🎯 选择的资产: {}", asset.name);
                return Some(asset.browser_download_url.clone());
            }
        }
    }

    warn!("⚠️ 没有找到匹配的资产，尝试查找可执行文件...");
    // 如果没找到精确匹配，返回第一个看起来像可执行文件的资源
    for (index, asset) in assets.iter().enumerate() {
        let name = asset.name.to_lowercase();
        let is_executable = name.contains("nuwax-cli")
            || name.ends_with(".exe")
            || name.ends_with(".tar.gz")
            || name.ends_with(".msi")
            || name.ends_with(".appimage");

        info!(
            "🔍 检查可执行文件[{}]: {} -> 可执行: {}",
            index, asset.name, is_executable
        );

        if is_executable {
            info!("✅ 找到可执行文件: {}", asset.name);
            return Some(asset.browser_download_url.clone());
        }
    }

    warn!("❌ 没有找到任何可执行文件");
    None
}

/// 显示版本检查结果
pub fn display_version_info(version_info: &VersionInfo) {
    info!("🦆 Nuwax Cli  版本信息");
    info!("当前版本: {}", version_info.current_version);
    info!("最新版本: {}", version_info.latest_version);

    if version_info.is_update_available {
        info!("✅ 发现新版本可用！");
        if let Some(ref url) = version_info.download_url {
            info!("下载地址: {}", url);
        }

        // 显示发布说明（截取前500字符）
        if !version_info.release_notes.is_empty() {
            let notes = if version_info.release_notes.len() > 500 {
                format!("{}...", &version_info.release_notes[..500])
            } else {
                version_info.release_notes.clone()
            };
            info!("更新说明:\n{}", notes);
        }

        // 解析并显示发布时间
        if let Ok(published_time) = DateTime::parse_from_rfc3339(&version_info.published_at) {
            info!("发布时间: {}", published_time.format("%Y-%m-%d %H:%M:%S"));
        }

        info!("💡 使用以下命令安装更新:");
        info!("   nuwax-cli check-update install");
    } else {
        info!("✅ 您已经使用最新版本！");
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
            "当前版本 {} 已是最新或更高版本 {}。使用 --force 强制重新安装",
            current_version,
            target_version
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

    info!("📥 正在下载版本 {}: {}", version, url);
    info!("💾 临时保存到: {}", download_path.display());

    // 下载文件
    let response = client
        .get(url)
        .header("User-Agent", format!("nuwax-cli/{}", get_current_version()))
        .send()
        .await
        .context("下载失败")?;

    if !response.status().is_success() {
        return Err(anyhow::anyhow!("下载失败: HTTP {}", response.status()));
    }

    let total_size = response.content_length().unwrap_or(0);
    info!("📦 文件大小: {} bytes", total_size);

    let bytes = response.bytes().await?;
    std::fs::write(&download_path, bytes)?;

    info!("✅ 下载完成，开始安装...");

    // 获取当前可执行文件路径
    let current_exe = std::env::current_exe().context("无法获取当前可执行文件路径")?;

    info!("🔧 当前可执行文件: {}", current_exe.display());

    // 处理不同文件类型的安装
    install_downloaded_file(&download_path, &current_exe, version).await?;

    // 清理临时文件
    if let Err(e) = std::fs::remove_file(&download_path) {
        warn!("清理临时文件失败: {}", e);
    }

    info!("🎉 安装完成！Nuwax Cli  已更新到版本 {}, 可运行'nuwax-cli --version' 验证安装版本", version);
    info!("💡 请再次运行命令 'nuwax-cli auto-upgrade-deploy run' 来完成最终安装部署");

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
        Err(anyhow::anyhow!("不支持的文件格式: {}", download_name))
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
        warn!("创建备份失败: {}", e);
    } else {
        info!("✅ 已创建备份文件: {}", backup_path.display());
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
    info!("🔧 正在替换可执行文件...");
    match self_replace::self_replace(download_path) {
        Ok(()) => {
            info!("✅ 可执行文件替换成功");
            Ok(())
        }
        Err(e) => {
            warn!("❌ 文件替换失败: {}", e);

            // 尝试恢复备份
            if backup_path.exists() {
                info!("🔄 尝试从备份恢复...");
                match std::fs::copy(&backup_path, current_exe) {
                    Ok(_) => {
                        warn!("✅ 已从备份恢复原文件");
                        return Err(anyhow::anyhow!("文件替换失败，已恢复备份: {}", e));
                    }
                    Err(restore_err) => {
                        error!("❌ 备份恢复也失败: {}", restore_err);
                        return Err(anyhow::anyhow!(
                            "文件替换失败且无法恢复备份: {}, 恢复错误: {}",
                            e,
                            restore_err
                        ));
                    }
                }
            }

            Err(anyhow::anyhow!("文件替换失败: {}", e))
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

    info!("📦 正在解压缩包...");

    // 解压 tar.gz 文件
    let output = Command::new("tar")
        .args([
            "-xzf",
            &archive_path.to_string_lossy(),
            "-C",
            &temp_dir.to_string_lossy(),
        ])
        .output()
        .context("解压失败，请确保系统已安装 tar 命令")?;

    if !output.status.success() {
        return Err(anyhow::anyhow!(
            "解压失败: {}",
            String::from_utf8_lossy(&output.stderr)
        ));
    }

    // 查找可执行文件
    let executable_path = find_executable_in_dir(&temp_dir)?;

    // 安装可执行文件
    install_executable(&executable_path, current_exe).await?;

    // 清理解压目录
    if let Err(e) = std::fs::remove_dir_all(&temp_dir) {
        warn!("清理解压目录失败: {}", e);
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

    Err(anyhow::anyhow!("在压缩包中未找到可执行文件"))
}

/// 处理 check-update 命令
pub async fn handle_check_update_command(command: CheckUpdateCommand) -> Result<()> {
    match command {
        CheckUpdateCommand::Check => {
            info!("🔍 正在检查 Nuwax Cli  更新...");

            match check_for_updates().await {
                Ok(version_info) => {
                    display_version_info(&version_info);
                }
                Err(e) => {
                    warn!("❌ 检查更新失败: {}", e);
                    info!("当前版本: {}", get_current_version());
                    info!("💡 可能的原因:");
                    info!("   - 网络连接问题");
                    info!("   - 版本检查服务器暂时不可用");
                    info!("   - GitHub API 暂时不可用");
                    info!("   - 项目尚未发布任何版本");
                    return Err(e);
                }
            }
        }

        CheckUpdateCommand::Install { version, force } => {
            info!("🚀 开始安装 Nuwax Cli ...");

            // 检查是否需要安装
            let (current_version, target_version) =
                match should_install(version.as_deref(), force).await {
                    Ok(versions) => versions,
                    Err(e) => {
                        if force {
                            warn!("⚠️  {}", e);
                            info!("🔧 由于使用了 --force 参数，将继续安装...");
                            // 如果强制安装但没指定版本，返回错误
                            if version.is_none() {
                                return Err(anyhow::anyhow!("强制安装时必须指定版本号"));
                            }
                            (get_current_version(), version.as_ref().unwrap().clone())
                        } else {
                            warn!("❌ {}", e);
                            return Err(e);
                        }
                    }
                };

            info!(
                "准备从版本 {} 更新到版本 {}",
                current_version, target_version
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
                    .ok_or_else(|| anyhow::anyhow!("未找到适合当前平台的下载链接"))?
            };

            info!("📥 开始下载并安装版本 {}...", target_version);

            match install_release(&download_url, &target_version).await {
                Ok(_) => {
                    info!("🎉 安装成功！");
                    info!("请重新启动命令行验证安装结果");
                }
                Err(e) => {
                    warn!("❌ 安装失败: {}", e);
                    info!("💡 可能的解决方案:");
                    info!("   - 检查网络连接");
                    info!("   - 确保有足够的磁盘空间");
                    info!("   - 以管理员权限运行");
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
        .ok_or_else(|| anyhow::anyhow!("未找到版本 {} 适合当前平台的下载链接", version))
}
