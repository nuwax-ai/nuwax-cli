use crate::app::CliApp;
use crate::cli::UpgradeArgs;
use anyhow::Result;
use client_core::{
    api::ApiClient, config::AppConfig, package_cache::PackageIdentity,
    upgrade_strategy::UpgradeStrategy,
};
use std::{
    fs,
    path::{Path, PathBuf},
};
use tracing::info;

/// 获取指定版本的全量下载目录路径,并创建目录
pub fn create_version_download_dir(
    download_dir: PathBuf,
    version: &str,
    download_type: &str,
) -> Result<PathBuf> {
    let dir = download_dir.join(version).join(download_type);

    fs::create_dir_all(&dir)?;
    Ok(dir)
}

/// 处理下载服务包并显示相关信息
async fn handle_service_download(
    app: &mut CliApp,
    identity: &PackageIdentity,
    download_dir: PathBuf,
    version_directory: &str,
    download_directory: &str,
) -> Result<()> {
    let directory =
        create_version_download_dir(download_dir, version_directory, download_directory)?;
    let final_path = app
        .api_client
        .download_service_package(&directory, identity)
        .await?;
    info!("✅ Service package ready!");
    info!("   File location: {}", final_path.display());
    info!("   Download version: {}", identity.version);
    info!(
        "   Current deployed version: {}",
        app.config.get_docker_versions()
    );
    info!("📝 Next step: Run 'nuwax-cli docker-service deploy' to deploy services");
    Ok(())
}

/// 下载Docker服务升级文件
pub async fn run_upgrade(app: &mut CliApp, args: UpgradeArgs) -> Result<UpgradeStrategy> {
    if args.check {
        info!("🔍 Checking Docker service upgrade versions");
        info!("========================");
    } else {
        info!("📦 Downloading Docker service files");
        info!("=====================");
    }

    // 检查是否是首次使用（docker目录为空或不存在docker-compose.yml）
    let docker_compose_path = std::path::Path::new(&app.config.docker.compose_file);
    let is_first_time = !docker_compose_path.exists();

    if is_first_time {
        info!("🆕 Detected first deployment");
        info!("   Will download full Docker service package");
    } else if args.force {
        info!("🔧 Force redownload mode");
    }

    // 2. 获取当前版本信息
    let current_version_str = app.config.get_docker_versions();

    let upgrade_strategy = app.upgrade_manager.check_for_updates(args.force).await?;

    let download_dir: PathBuf = app.config.get_download_dir();

    match &upgrade_strategy {
        UpgradeStrategy::FullUpgrade {
            url,
            hash,
            signature: _,
            target_version,
            download_type,
        } => {
            info!("🔄 Full upgrade");
            info!("   Target version: {version}", version = target_version);
            info!("   Download path: {path}", path = url);
            info!(
                "   Current version: {version}",
                version = current_version_str
            );
            info!("   Latest version: {version}", version = target_version);

            if args.check {
                //检测升级版本是否存在
                info!("🔍 Check upgrade version done");
                return Ok(upgrade_strategy);
            }

            //获取主版本号，不包含补丁版本号
            let version_str = target_version.base_version_string();
            let download_type_str = download_type.to_string();

            let identity = PackageIdentity::new(
                &target_version.to_string(),
                client_core::architecture::Architecture::detect().as_str(),
                "full",
                url,
                Some(hash),
            )?;
            handle_service_download(
                app,
                &identity,
                download_dir,
                &version_str,
                &download_type_str,
            )
            .await?;
        }
        UpgradeStrategy::PatchUpgrade {
            patch_info,
            target_version,
            download_type: _,
        } => {
            info!("🔄 Incremental upgrade");
            info!(
                "   Current version: {version}",
                version = current_version_str
            );
            info!("   Latest version: {version}", version = target_version);

            if args.check {
                info!("🔍 Check upgrade version done");
                return Ok(upgrade_strategy);
            }

            //获取主版本号，不包含补丁版本号
            let base_version = target_version.base_version_string();
            let version_str = target_version.to_string();

            let identity = PackageIdentity::new(
                &target_version.to_string(),
                client_core::architecture::Architecture::detect().as_str(),
                "patch",
                &patch_info.url,
                patch_info.hash.as_deref(),
            )?;
            handle_service_download(app, &identity, download_dir, &base_version, &version_str)
                .await?;
        }
        UpgradeStrategy::NoUpgrade { target_version } => {
            info!(
                "   Current version: {version}",
                version = current_version_str
            );
            info!("   Latest version: {version}", version = target_version);
            info!("✅ Current version is latest");
        }
    }

    Ok(upgrade_strategy)
}

fn select_full_download_package<'a>(
    manifest: &'a client_core::api_types::EnhancedServiceManifest,
    architecture: &str,
) -> Result<(&'a str, Option<String>)> {
    let platform = manifest
        .platforms
        .as_ref()
        .and_then(|platforms| match architecture {
            "x86_64" => platforms.x86_64.as_ref(),
            "aarch64" => platforms.aarch64.as_ref(),
            _ => None,
        });
    let legacy = manifest.packages.as_ref().map(|packages| &packages.full);
    if let Some(platform) = platform {
        Ok((platform.url.as_str(), platform.published_sha256(legacy)?))
    } else if manifest.platforms.is_none() {
        let package =
            legacy.ok_or_else(|| anyhow::anyhow!("Manifest does not contain a full package"))?;
        Ok((
            package.url.as_str(),
            client_core::package_cache::normalize_sha256(Some(&package.hash))?,
        ))
    } else {
        Err(anyhow::anyhow!(
            "No download URL for architecture: {}",
            architecture
        ))
    }
}

/// 下载最新的 Docker 服务包（全量包）用于离线部署
///
/// 此函数独立于 CliApp，不需要数据库初始化
pub async fn run_download(config_path: Option<&Path>) -> Result<()> {
    let config = match config_path {
        Some(path) if path.exists() => AppConfig::load_from_file(path)?,
        Some(_) | None => match AppConfig::find_and_load_config() {
            Ok(cfg) => cfg,
            Err(_) => {
                info!("   No config file found, using default configuration");
                AppConfig::default()
            }
        },
    };

    run_download_with_config(&config).await
}

pub async fn run_download_with_config(config: &AppConfig) -> Result<()> {
    info!("📦 Downloading latest Docker service package...");
    info!("=====================");

    // 2. 创建 API 客户端
    let api_client = ApiClient::new(Some("offline-download".to_string()), None);

    // 3. 获取最新版本信息
    let manifest = api_client.get_enhanced_service_manifest().await?;
    let latest_version = manifest.version.clone();

    info!(
        "   Latest version: {version}",
        version = latest_version.to_string()
    );

    // 4. 获取当前部署版本（如果存在）
    let current_version = config.get_docker_versions();
    if !current_version.is_empty() {
        info!(
            "   Current deployed version: {version}",
            version = current_version
        );
    } else {
        info!("   No current deployment detected");
    }

    // 5. 获取本机架构
    let arch = client_core::architecture::Architecture::detect();
    let arch_str = match arch {
        client_core::architecture::Architecture::Aarch64 => "aarch64",
        client_core::architecture::Architecture::X86_64 => "x86_64",
        _ => {
            return Err(anyhow::anyhow!("Unsupported architecture"));
        }
    };
    info!("   Target architecture: {arch}", arch = arch_str);

    // 6. 从 manifest 中获取下载 URL
    let (download_url, expected_hash) = select_full_download_package(&manifest, arch_str)?;

    info!("   Download URL: {url}", url = download_url);

    // 7. 下载到配置指定的缓存目录
    let download_dir = config.get_download_dir();
    let version_str = latest_version.base_version_string();

    // 创建下载目录
    let version_download_dir = create_version_download_dir(download_dir, &version_str, "full")?;

    let identity = PackageIdentity::new(
        &latest_version.to_string(),
        arch_str,
        "full",
        download_url,
        expected_hash.as_deref(),
    )?;
    let final_path = api_client
        .download_service_package(&version_download_dir, &identity)
        .await?;
    info!("✅ Service package downloaded successfully!");
    info!("   File location: {}", final_path.display());
    info!("💡 Copy this file to offline server for deployment");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn standalone_download_normalizes_sentinels_before_same_url_digest_fallback() {
        for placeholder in [
            serde_json::Value::Null,
            serde_json::json!(""),
            serde_json::json!("external"),
        ] {
            let manifest: client_core::api_types::EnhancedServiceManifest = serde_json::from_value(serde_json::json!({
                "version": "0.0.108", "release_date": "2026-10-10T00:00:00Z", "release_notes": "fixture",
                "packages": {"full": {"url": "https://example.com/x86.zip", "hash": "a".repeat(64), "signature": "", "size": 1}},
                "platforms": {"x86_64": {"url": "https://example.com/x86.zip", "signature": "", "hash": placeholder},
                    "aarch64": {"url": "https://example.com/arm.zip", "signature": "", "hash": "external"}}
            })).unwrap();
            let (url, hash) = select_full_download_package(&manifest, "x86_64").unwrap();
            assert_eq!(url, "https://example.com/x86.zip");
            assert_eq!(hash, Some("a".repeat(64)));
            assert_eq!(
                select_full_download_package(&manifest, "aarch64")
                    .unwrap()
                    .1,
                None
            );
        }
    }

    #[test]
    fn standalone_download_rejects_malformed_platform_digest_despite_valid_generic_digest() {
        let manifest: client_core::api_types::EnhancedServiceManifest = serde_json::from_value(serde_json::json!({
            "version": "0.0.108", "release_date": "2026-10-10T00:00:00Z", "release_notes": "fixture",
            "packages": {"full": {"url": "https://example.com/x86.zip", "hash": "a".repeat(64), "signature": "", "size": 1}},
            "platforms": {"x86_64": {"url": "https://example.com/x86.zip", "signature": "", "hash": "broken-published-hash"}}
        })).unwrap();
        assert!(select_full_download_package(&manifest, "x86_64").is_err());
    }
}
