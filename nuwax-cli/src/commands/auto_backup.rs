use std::path::PathBuf;

use crate::app::CliApp;
use crate::cli::AutoBackupCommand;
use crate::commands::{backup, docker_service};
use crate::docker_service::health_check::HealthChecker;
use crate::docker_utils;
use anyhow::Result;
use client_core::constants::timeout;
use rust_i18n::t;

use tracing::{debug, error, info, warn};

/// 处理自动备份命令
pub async fn handle_auto_backup(app: &mut CliApp, command: &AutoBackupCommand) -> Result<()> {
    match command {
        AutoBackupCommand::Run => {
            info!("Executing auto backup");
            run_auto_backup(app).await
        }
        // TODO: 未来版本实现内置定时调度器后启用这些命令
        // AutoBackupCommand::Cron { expression } => set_cron_expression(app, expression.clone()).await,
        // AutoBackupCommand::Enabled { enabled } => set_enabled(app, *enabled).await,
        AutoBackupCommand::Status => show_status(app).await,
    }
}

/// 执行自动备份流程：停止服务 -> 备份 -> 重启服务
///
/// 使用 app.config 中配置的 docker-compose 文件和项目名称
pub async fn run_auto_backup(app: &mut CliApp) -> Result<()> {
    info!("Starting auto backup process");

    let backup_start_time = chrono::Utc::now();
    let mut backup_success = false;

    // 1. 检查服务状态并停止（使用统一的公共方法）
    // 记录服务是否原本在运行，用于后续判断是否需要重启
    // 从 app.config 中获取配置
    let compose_file = Some(PathBuf::from(&app.config.docker.compose_file));
    let project_name: Option<String> = None; // 使用 docker-compose 文件中定义的项目名

    let service_running = {
        let health_checker = HealthChecker::new(app.docker_manager.clone());
        let report = health_checker.health_check().await?;
        report.get_running_count() > 0
    };

    if service_running {
        docker_service::stop_docker_services_and_wait(
            app,
            compose_file.clone(),
            project_name.clone(),
        )
        .await?;
    } else {
        info!("Docker services not running, proceeding with backup directly");
    }

    // 2. 执行备份
    info!("Starting backup operation");
    let mut backup_error_message: String = String::new();
    match backup::run_backup(app).await {
        Ok(_) => {
            backup_success = true;
            info!("Backup completed successfully");
        }
        Err(e) => {
            error!(error = %e, "Backup operation failed");
            backup_error_message = format!("{e}");
            // 记录失败但继续执行后续步骤
        }
    }

    // 记录备份执行时间和结果
    if let Err(e) = update_last_backup_time(app, backup_start_time, backup_success).await {
        warn!(error = %e, "Failed to record backup time");
    }

    if service_running {
        // 4. 重新启动Docker服务
        info!("Restarting Docker services");
        docker_service::start_docker_services(app, compose_file.clone(), project_name.clone())
            .await?;

        // 等待服务启动完成
        info!("Waiting for Docker services to fully start");
        let compose_path = compose_file.as_ref().unwrap().as_path();
        if docker_utils::wait_for_compose_services_started(
            compose_path,
            timeout::SERVICE_START_TIMEOUT,
        )
        .await?
        {
            if backup_success {
                info!("Auto backup process complete, services restarted");
            } else {
                warn!("Auto backup process complete (backup failed), services restarted");
            }
        } else {
            warn!("Service startup timeout, please check service status manually");

            // 最后再检查一次状态
            match check_docker_service_status(app).await {
                Ok(true) => {
                    debug!("Final check: Services started normally");
                }
                Ok(false) => {
                    debug!("Final check: Services not started");
                }
                Err(e) => {
                    error!(error = %e, "Final check failed");
                }
            }
        }
    } else if backup_success {
        info!("Auto backup process complete");
    } else {
        warn!("Auto backup process complete (backup failed)");
    }

    // 如果备份失败，返回错误
    if !backup_success {
        return Err(anyhow::anyhow!(
            "{}",
            t!("auto_backup.execution_failed", error = backup_error_message)
        ));
    }

    Ok(())
}

/// 显示备份状态和历史记录
pub async fn show_status(app: &mut CliApp) -> Result<()> {
    debug!("Displaying backup status and history");

    info!("📦 Backup Management");
    info!("============");

    // 显示备份历史记录（包含完整的操作列表）
    backup::run_list_backups(app).await?;

    // 添加手动备份特定的操作提示
    info!("");
    info!("🔧 Quick Actions:");
    info!("   - Run backup now: nuwax-cli auto-backup run");

    Ok(())
}

/// 更新最后备份时间
pub async fn update_last_backup_time(
    app: &CliApp,
    backup_time: chrono::DateTime<chrono::Utc>,
    success: bool,
) -> Result<()> {
    app.database
        .set_config("auto_backup_last_time", &backup_time.to_rfc3339())
        .await?;

    let status = if success { "success" } else { "failed" };
    app.database
        .set_config("auto_backup_last_status", status)
        .await?;

    Ok(())
}

/// 检查Docker服务状态（是否有服务在运行）
///
/// 返回 true 表示有服务在运行，false 表示没有服务在运行
async fn check_docker_service_status(app: &mut CliApp) -> Result<bool> {
    let health_checker = HealthChecker::new(app.docker_manager.clone());
    let report = health_checker.health_check().await?;

    // 检查是否有运行中的容器（而不是检查是否所有服务都健康）
    let running_count = report.get_running_count();

    if running_count > 0 {
        info!("🔍 Found {count} running services", count = running_count);
        Ok(true)
    } else {
        info!("🔍 No running services found");
        Ok(false)
    }
}
