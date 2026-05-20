use std::sync::Arc;

use crate::{app::CliApp, docker_service::health_check::HealthChecker};
use anyhow::Result;
use client_core::container::DockerManager;
use tracing::{error, info, warn};

/// 显示客户端版本信息（标题和基本信息）
pub fn show_client_version() {
    info!("🦆 Nuwax Cli ent Status");
    info!("==================");
    info!("📋 Basic Information:");
    info!("   Client version: v{version}", version = env!("CARGO_PKG_VERSION"));
}

/// 显示服务状态（完整版本，包含基本信息）
pub async fn run_status(app: &CliApp) -> Result<()> {
    show_client_version();
    run_status_details(app).await
}

/// 显示详细状态信息（不包含基本信息标题）
pub async fn run_status_details(app: &CliApp) -> Result<()> {
    // 继续显示其他基本信息
    info!("   Docker service version: {version}", version = app.config.get_docker_versions());
    info!("   Configuration file: {file}", file = "config.toml");

    // 显示客户端UUID
    let client_uuid = app.database.get_or_create_client_uuid().await?;
    info!("   Client UUID: {uuid}", uuid = client_uuid);

    // 检查文件状态
    info!("📁 File Status:");
    let docker_compose_path = std::path::Path::new(&app.config.docker.compose_file);
    let env_file_path = std::path::Path::new(&app.config.docker.env_file);

    // 使用新的版本化路径检查服务包文件（自动查找 .zip 或 .tar.gz）
    let current_version = &app.config.get_docker_versions();
    let download_path = app.config.get_version_download_file_path(
        current_version,
        "full",
        None, // 自动查找归档文件
    );

    if docker_compose_path.exists() {
        info!("   ✅ Docker Compose file: {file}", file = app.config.docker.compose_file);
    } else {
        info!("   ❌ Docker Compose file: {file} (not exists)", file = app.config.docker.compose_file);
    }

    match &download_path {
        Ok(path) => {
            info!("   ✅ Service package file: {path}", path = path.display());
        }
        Err(e) => {
            info!("   ❌ Service package file: (Error: {error})", error = e.to_string());
        }
    }

    // Docker服务状态
    info!("🐳 Docker Service Status:");
    if docker_compose_path.exists() {
        info!("   📋 Docker Compose file ready");

        // 检查具体的服务状态
        match check_docker_services_status(docker_compose_path, env_file_path).await {
            Ok(()) => {
                // 状态检查成功，详细信息已在函数内部显示
            }
            Err(e) => {
                warn!("   ⚠️  Service status check failed: {error}", error = e.to_string());
                info!("   💡 Suggested checks:");
                info!("      - Docker is installed and running");
                info!("      - docker-compose is available");
                info!("      - Use 'docker-compose ps' to check status manually");
            }
        }
    } else {
        warn!("   ❌ Docker Compose file does not exist, service not initialized");
    }

    // 根据状态提供建议
    info!("💡 Status Analysis and Suggestions:");

    let has_compose = docker_compose_path.exists();
    let has_package = download_path
        .as_ref()
        .ok()
        .map(|p| p.exists())
        .unwrap_or(false);

    if !has_compose && !has_package {
        info!("   🆕 You seem to be a first-time user");
        info!("   📝 Suggested steps:");
        info!("      1. nuwax-cli upgrade                  (Download Docker service package)");
        info!("      2. nuwax-cli docker-service deploy    (Deploy and start services)");
    } else if !has_compose && has_package {
        info!("   📦 Service package found, but not extracted yet");
        info!("   📝 Suggested steps:");
        info!("      - nuwax-cli docker-service deploy  (Full deployment process)");
        info!("      - nuwax-cli docker-service start   (Start services only)");
    } else {
        info!("   ✅ System files complete, all features available");
        info!("   📝 Available commands:");
        info!("      - nuwax-cli docker-service start/stop/restart  (Control services)");
        info!("      - nuwax-cli upgrade                            (Upgrade services)");
        info!("      - nuwax-cli backup                             (Create backup)");
        info!("      - nuwax-cli check-update                       (Check client updates)");
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
        info!("   ✅ Services are running");
    } else {
        warn!("   ❌ Some services are not running");
        for container in report.failed_containers().iter() {
            error!("   ❌ {name}: {status}", name = container.name, status = format!("{:?}", container.status));
        }
    }

    Ok(())
}
