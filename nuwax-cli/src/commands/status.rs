use std::sync::Arc;

use crate::docker_utils;
use crate::{app::CliApp, docker_service::health_check::HealthChecker};
use anyhow::Result;
use client_core::container::{DockerManager, ServiceStatus};
use tracing::{error, info, warn};

/// 显示客户端版本信息（标题和基本信息）
pub fn show_client_version() {
    info!("🦆 Nuwax Cli ent 状态");
    info!("==================");
    info!("📋 基本信息:");
    info!("   客户端版本: v{}", env!("CARGO_PKG_VERSION"));
}

/// 显示服务状态（完整版本，包含基本信息）
pub async fn run_status(app: &CliApp) -> Result<()> {
    show_client_version();
    run_status_details(app).await
}

/// 显示详细状态信息（不包含基本信息标题）
pub async fn run_status_details(app: &CliApp) -> Result<()> {
    // 继续显示其他基本信息
    info!("   Docker服务版本: {}", app.config.get_docker_versions());
    info!("   配置文件: {}", "config.toml");

    // 显示客户端UUID
    let client_uuid = app.database.get_or_create_client_uuid().await?;
    info!("   客户端UUID: {}", client_uuid);

    // 检查文件状态
    info!("📁 文件状态:");
    let docker_compose_path = std::path::Path::new(&app.config.docker.compose_file);
    let env_file_path = std::path::Path::new(&app.config.docker.env_file);

    // 使用新的版本化路径检查服务包文件
    let current_version = &app.config.get_docker_versions();
    let download_path = app.config.get_version_download_file_path(
        current_version,
        "full",
        Some(client_core::constants::upgrade::DOCKER_SERVICE_PACKAGE),
    );

    if docker_compose_path.exists() {
        info!(
            "   ✅ Docker Compose文件: {}",
            app.config.docker.compose_file
        );
    } else {
        info!(
            "   ❌ Docker Compose文件: {} (不存在)",
            app.config.docker.compose_file
        );
    }

    if download_path.exists() {
        info!("   ✅ 服务包文件: {}", download_path.display());
    } else {
        info!("   ❌ 服务包文件: {} (不存在)", download_path.display());
    }

    // Docker服务状态
    info!("🐳 Docker服务状态:");
    if docker_compose_path.exists() {
        info!("   📋 Docker Compose文件已就绪");

        // 检查具体的服务状态
        match check_docker_services_status(docker_compose_path, env_file_path).await {
            Ok(()) => {
                // 状态检查成功，详细信息已在函数内部显示
            }
            Err(e) => {
                warn!("   ⚠️  服务状态检查失败: {}", e);
                info!("   💡 建议检查:");
                info!("      - Docker是否已安装并运行");
                info!("      - docker-compose是否可用");
                info!("      - 使用 'docker-compose ps' 手动查看状态");
            }
        }
    } else {
        warn!("   ❌ Docker Compose文件不存在，服务未初始化");
    }

    // 根据状态提供建议
    info!("💡 状态分析和建议:");

    if !docker_compose_path.exists() && !download_path.exists() {
        info!("   🆕 您似乎是首次使用");
        info!("   📝 建议执行以下步骤:");
        info!("      1. nuwax-cli upgrade                  (下载Docker服务包)");
        info!("      2. nuwax-cli docker-service deploy    (部署并启动服务)");
    } else if !docker_compose_path.exists() && download_path.exists() {
        info!("   📦 发现服务包文件，但尚未解压");
        info!("   📝 建议执行:");
        info!("      - nuwax-cli docker-service deploy  (完整部署流程)");
        info!("      - nuwax-cli docker-service start   (仅启动服务)");
    } else {
        info!("   ✅ 系统文件完整，可以正常使用所有功能");
        info!("   📝 可用命令:");
        info!("      - nuwax-cli docker-service start/stop/restart  (控制服务)");
        info!("      - nuwax-cli upgrade                            (升级服务)");
        info!("      - nuwax-cli backup                             (创建备份)");
        info!("      - nuwax-cli check-update                       (检查客户端更新)");
    }

    Ok(())
}

/// 显示API配置信息
pub async fn run_api_info(app: &CliApp) -> Result<()> {
    let api_config = app.api_client.get_config();
    info!("{}", api_config);
    Ok(())
}

/// 检查Docker服务状态的内部辅助函数
async fn check_docker_services_status(
    compose_file_path: &std::path::Path,
    env_file_path: &std::path::Path,
) -> Result<()> {
    let docker_manager = DockerManager::with_project(
        compose_file_path.to_path_buf(),
        env_file_path.to_path_buf(),
        None,
    )?;

    let health_checker = HealthChecker::new(Arc::new(docker_manager));
    let report = health_checker.health_check().await?;
    if report.is_all_healthy() {
        info!("   ✅ 服务正在运行");
    } else {
        warn!("   ❌ 存在服务未运行");
        for container in report.failed_containers().iter() {
            error!("   ❌ {}: {:?}", container.name, container.status);
        }
    }

    Ok(())
}
