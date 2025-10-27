use crate::app::CliApp;
use crate::cli::UpgradeArgs;
use anyhow::Result;
use client_core::{architecture::Architecture, upgrade_strategy::UpgradeStrategy};
use std::{fs, path::PathBuf};
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

    // 检查文件是否已存在（智能下载会处理这个检查）
    info!("   文件路径: {}", version_download_dir.to_string_lossy());

    //根据当前架构获取docker文件名
    let docker_file_name = Architecture::detect().get_docker_file_name();

    let download_path = version_download_dir.join(docker_file_name);

    let download_result = app
        .api_client
        .download_service_update_optimized(&download_path, Some(version_str), url)
        .await;

    match download_result {
        Ok(_) => {
            info!("✅ 服务包已准备就绪!");
            info!("   文件位置: {}", download_path.display());
            info!("   下载版本: {}", target_version.to_string());
            info!("   当前部署版本: {}", app.config.get_docker_versions());
            info!("📝 下一步: 运行 'nuwax-cli docker-service deploy' 来部署服务");
            Ok(())
        }
        Err(e) => {
            error!("❌ 操作失败: {}", e);
            info!("💡 请检查网络连接或稍后重试");
            Err(e)
        }
    }
}

/// 下载Docker服务升级文件
pub async fn run_upgrade(app: &mut CliApp, args: UpgradeArgs) -> Result<UpgradeStrategy> {
    if args.check {
        info!("🔍 检查Docker服务升级版本");
        info!("========================");
    } else {
        info!("📦 下载Docker服务文件");
        info!("=====================");
    }

    // 检查是否是首次使用（docker目录为空或不存在docker-compose.yml）
    let docker_compose_path = std::path::Path::new(&app.config.docker.compose_file);
    let is_first_time = !docker_compose_path.exists();

    if is_first_time {
        info!("🆕 检测到这是您的首次部署");
        info!("   将下载完整的Docker服务包");
    } else if args.force {
        info!("🔧 强制重新下载模式");
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
            info!("🔄 全量升级");
            info!("   目标版本: {}", target_version);
            info!("   下载路径: {}", url);
            info!("   当前版本: {}", current_version_str);
            info!("   最新版本: {}", target_version);

            if args.check {
                //检测升级版本是否存在
                info!("🔍 检查升级版本执行完毕");
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
            info!("🔄 增量升级");
            info!("   当前版本: {}", current_version_str);
            info!("   最新版本: {}", target_version);

            if args.check {
                info!("🔍 检查升级版本执行完毕");
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
            info!("   当前版本: {}", current_version_str);
            info!("   最新版本: {}", target_version);
            info!("✅ 当前已是最新版本");
        }
    }

    Ok(upgrade_strategy)
}
