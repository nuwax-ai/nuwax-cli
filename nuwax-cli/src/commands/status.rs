use std::sync::Arc;

use crate::{app::CliApp, docker_service::health_check::HealthChecker};
use anyhow::Result;
use client_core::container::{DockerManager, ServiceStatus};
use rust_i18n::t;
use tracing::{error, info, warn};

/// 显示客户端版本信息（标题和基本信息）
pub fn show_client_version() {
    info!("{}", t!("status.title"));
    info!("{}", t!("status.separator"));
    info!("{}", t!("status.basic_info"));
    info!("{}", t!("status.client_version", version = env!("CARGO_PKG_VERSION")));
}

/// 显示服务状态（完整版本，包含基本信息）
pub async fn run_status(app: &CliApp) -> Result<()> {
    show_client_version();
    run_status_details(app).await
}

/// 显示详细状态信息（不包含基本信息标题）
pub async fn run_status_details(app: &CliApp) -> Result<()> {
    // 继续显示其他基本信息
    info!("{}", t!("status.docker_service_version", version = app.config.get_docker_versions()));
    info!("{}", t!("status.config_file", file = "config.toml"));

    // 显示客户端UUID
    let client_uuid = app.database.get_or_create_client_uuid().await?;
    info!("{}", t!("status_cmd.client_uuid", uuid = client_uuid));

    // 检查文件状态
    info!("{}", t!("status.file_status"));
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
        info!("{}", t!("status.docker_compose_exists", file = app.config.docker.compose_file));
    } else {
        info!("{}", t!("status.docker_compose_not_exists", file = app.config.docker.compose_file));
    }

    match &download_path {
        Ok(path) => {
            info!("{}", t!("status_cmd.service_package_exists", path = path.display()));
        }
        Err(e) => {
            info!("{}", t!("status_cmd.service_package_error", error = e.to_string()));
        }
    }

    // Docker服务状态
    info!("{}", t!("status.docker_status"));
    if docker_compose_path.exists() {
        info!("{}", t!("status.docker_ready"));

        // 检查具体的服务状态
        match check_docker_services_status(docker_compose_path, env_file_path).await {
            Ok(()) => {
                // 状态检查成功，详细信息已在函数内部显示
            }
            Err(e) => {
                warn!("{}", t!("status_cmd.status_check_failed", error = e.to_string()));
                info!("{}", t!("status_cmd.suggest_check_docker"));
                info!("{}", t!("status_cmd.suggest_docker_installed"));
                info!("{}", t!("status_cmd.suggest_docker_compose"));
                info!("{}", t!("status_cmd.suggest_manual_check"));
            }
        }
    } else {
        warn!("{}", t!("status_cmd.compose_not_exists"));
    }

    // 根据状态提供建议
    info!("{}", t!("status.suggestions"));

    let has_compose = docker_compose_path.exists();
    let has_package = download_path
        .as_ref()
        .ok()
        .map(|p| p.exists())
        .unwrap_or(false);

    if !has_compose && !has_package {
        info!("{}", t!("status_cmd.first_time_user"));
        info!("{}", t!("status_cmd.suggested_steps"));
        info!("{}", t!("status_cmd.step_upgrade"));
        info!("{}", t!("status_cmd.step_deploy"));
    } else if !has_compose && has_package {
        info!("{}", t!("status_cmd.package_found_not_extracted"));
        info!("{}", t!("status_cmd.suggested_steps"));
        info!("{}", t!("status_cmd.suggest_deploy"));
        info!("{}", t!("status_cmd.suggest_start"));
    } else {
        info!("{}", t!("status_cmd.system_complete"));
        info!("{}", t!("status_cmd.available_commands"));
        info!("{}", t!("status_cmd.cmd_service_control"));
        info!("{}", t!("status_cmd.cmd_upgrade"));
        info!("{}", t!("status_cmd.cmd_backup"));
        info!("{}", t!("status_cmd.cmd_check_update"));
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
        info!("{}", t!("status_cmd.service_running"));
    } else {
        warn!("{}", t!("status_cmd.service_not_running"));
        for container in report.failed_containers().iter() {
            error!("   ❌ {}: {:?}", container.name, container.status);
        }
    }

    Ok(())
}
