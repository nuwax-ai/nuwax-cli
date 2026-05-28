use crate::app::CliApp;
use crate::cli::UpgradeArgs;
use anyhow::Result;
use client_core::{
    api::ApiClient, config::AppConfig, upgrade_strategy::UpgradeStrategy, utils::archive,
};
use std::{
    fs,
    path::{Path, PathBuf},
};
use tracing::{error, info};

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
    url: &str,
    target_version: &client_core::version::Version,
    download_dir: PathBuf,
    version_str: &str,
    download_type: &str,
) -> Result<()> {
    // 确保下载目录存在
    let version_download_dir =
        create_version_download_dir(download_dir, version_str, download_type)?;

    // 总是先下载到临时文件
    let temp_path = version_download_dir.join("temp_download");

    info!(
        "   Downloading to temp file: {path}",
        path = temp_path.display()
    );

    let download_result = app
        .api_client
        .download_service_update_optimized(&temp_path, Some(version_str), url)
        .await;

    match download_result {
        Ok(_) => {
            // 魔数检测格式
            let format = archive::detect_format_by_magic(&temp_path)?;
            info!(
                "   Detected file format: {format}",
                format = format!("{:?}", format)
            );

            // 获取架构
            let arch = client_core::architecture::Architecture::detect();
            let arch_str = match arch {
                client_core::architecture::Architecture::Aarch64 => "aarch64",
                client_core::architecture::Architecture::X86_64 => "x86_64",
                _ => "unknown",
            };

            // 生成正确文件名
            let filename = archive::generate_docker_filename(arch_str, format);
            info!("   Renaming to: {filename}", filename = filename);

            let final_path = version_download_dir.join(&filename);

            // 重命名
            std::fs::rename(&temp_path, &final_path)?;

            info!("✅ Service package ready!");
            info!("   File location: {path}", path = final_path.display());
            info!(
                "   Download version: {version}",
                version = target_version.to_string()
            );
            info!(
                "   Current deployed version: {version}",
                version = app.config.get_docker_versions()
            );
            info!("📝 Next step: Run 'nuwax-cli docker-service deploy' to deploy services");
            Ok(())
        }
        Err(e) => {
            error!("❌ Operation failed: {error}", error = e.to_string());
            info!("💡 Please check network connection or try again later");
            Err(e)
        }
    }
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
            hash: _,
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

            handle_service_download(
                app,
                url,
                target_version,
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

            handle_service_download(
                app,
                &patch_info.url,
                target_version,
                download_dir,
                &base_version,
                &version_str,
            )
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
    let download_url = if let Some(ref platforms) = manifest.platforms {
        let platform_info = match arch_str {
            "x86_64" => platforms.x86_64.as_ref(),
            "aarch64" => platforms.aarch64.as_ref(),
            _ => None,
        };
        platform_info
            .map(|p| p.url.clone())
            .ok_or_else(|| anyhow::anyhow!("No download URL for architecture: {}", arch_str))?
    } else {
        return Err(anyhow::anyhow!(
            "Manifest does not contain platform information"
        ));
    };

    info!("   Download URL: {url}", url = download_url);

    // 7. 下载到配置指定的缓存目录
    let download_dir = config.get_download_dir();
    let version_str = latest_version.base_version_string();

    // 创建下载目录
    let version_download_dir = create_version_download_dir(download_dir, &version_str, "full")?;

    // 下载到临时文件
    let temp_path = version_download_dir.join("temp_download");
    info!(
        "   Downloading to temp file: {path}",
        path = temp_path.display()
    );

    let download_result = api_client
        .download_service_update_optimized(&temp_path, Some(version_str.as_str()), &download_url)
        .await;

    match download_result {
        Ok(_) => {
            // 魔数检测格式
            let format = archive::detect_format_by_magic(&temp_path)?;
            info!(
                "   Detected file format: {format}",
                format = format!("{:?}", format)
            );

            // 生成正确文件名
            let filename = archive::generate_docker_filename(arch_str, format);
            info!("   Renaming to: {filename}", filename = filename);

            let final_path = version_download_dir.join(&filename);

            // 重命名
            std::fs::rename(&temp_path, &final_path)?;

            info!("✅ Service package downloaded successfully!");
            info!("   File location: {path}", path = final_path.display());
            info!("💡 Copy this file to offline server for deployment");
        }
        Err(e) => {
            error!("❌ Download failed: {error}", error = e.to_string());
            info!("💡 Please check network connection or try again later");
            return Err(e);
        }
    }

    Ok(())
}
