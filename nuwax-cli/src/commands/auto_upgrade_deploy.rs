use crate::app::CliApp;
use crate::cli::AutoUpgradeDeployCommand;
use crate::commands::{auto_backup, backup, check_update, docker_service, update};
use crate::docker_service::health_check::HealthChecker;
use crate::{DockerService, docker_utils};
use anyhow::Result;
use client_core::constants::timeout;
use client_core::container::DockerManager;
use client_core::mysql_executor::{MySqlConfig, MySqlExecutor};
use client_core::sql_diff::generate_schema_diff;
use client_core::upgrade_strategy::UpgradeStrategy;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;
use tokio::time::sleep;
use tracing::{error, info, warn};

/// 获取docker-compose文件路径
fn get_compose_file_path(config_file: &Option<PathBuf>) -> PathBuf {
    match config_file {
        Some(path) => path.clone(),
        None => client_core::constants::docker::get_compose_file_path(),
    }
}

/// 运行自动升级部署相关命令的统一入口
pub async fn handle_auto_upgrade_deploy_command(
    app: &mut CliApp,
    cmd: AutoUpgradeDeployCommand,
) -> Result<()> {
    match cmd {
        AutoUpgradeDeployCommand::Run {
            port,
            config,
            project,
        } => {
            info!("🚀 开始自动升级部署流程...");
            run_auto_upgrade_deploy(app, port, config, project).await
        }
        AutoUpgradeDeployCommand::Status => {
            info!("显示自动升级部署状态");
            show_status(app).await
        }
    }
}

/// 执行自动升级部署流程
pub async fn run_auto_upgrade_deploy(
    app: &mut CliApp,
    frontend_port: Option<u16>,
    config_file: Option<PathBuf>,
    project_name: Option<String>,
) -> Result<()> {
    info!("🚀 开始自动升级部署流程...");

    // 如果指定了端口，显示端口信息
    if let Some(port) = frontend_port {
        info!("🔌 自定义frontend端口: {}", port);
    }

    // 如果指定了配置文件，显示配置文件信息
    if let Some(config_path) = &config_file {
        info!("📄 自定义docker-compose配置文件: {}", config_path.display());
    }

    // 0. 🔍 检查并安装 nuwax-cli 更新（如果需要）
    info!("🔍 检查 nuwax-cli 版本更新...");
    if let Err(e) = check_and_install_nuwax_cli_update().await {
        error!("❌ 检查 nuwax-cli 更新失败: {}", e);
        error!("   升级部署需要确保使用最新版本的 nuwax-cli");
        error!("   请检查网络连接或手动运行: nuwax-cli check-update install");
        return Err(e);
    }

    info!("✅ nuwax-cli 版本检查完成，继续执行升级部署流程");

    // 1. 获取最新版本信息并下载
    info!("📥 正在下载最新的Docker服务版本...");

    // 获取最新版本信息
    let latest_version = match app.api_client.get_enhanced_service_manifest().await {
        Ok(enhanced_service_manifest) => {
            let lastest_version = enhanced_service_manifest.version.to_string();

            info!(
                "📋 版本信息: {} -> {}",
                app.config.get_docker_versions(),
                lastest_version
            );
            lastest_version
        }
        Err(e) => {
            warn!("⚠️ 获取版本信息失败，使用配置版本: {}", e);
            app.config.get_docker_versions()
        }
    };

    // 下载服务包，但先不解压
    let upgrade_args = crate::cli::UpgradeArgs {
        force: false,
        check: false,
    };
    let upgrade_strategy = update::run_upgrade(app, upgrade_args).await?;

    // 2. 🔍 检查部署类型：第一次部署 vs 升级部署
    let is_first_deployment = is_first_deployment().await;

    if is_first_deployment {
        info!("🆕 检测到第一次部署，使用全新初始化");
    } else {
        info!("🔄 检测到升级部署，需要先停止服务");

        // 3. 🛑 先检查并停止服务
        info!("🔍 检查Docker服务状态...");

        // 🔧 修复：根据config_file参数创建使用正确路径的DockerService
        let docker_service = if let Some(config_file_path) = &config_file {
            let custom_docker_manager = Arc::new(DockerManager::with_project(
                config_file_path.clone(),
                client_core::constants::docker::get_env_file_path(),
                project_name.clone(),
            )?);
            DockerService::new(app.config.clone(), custom_docker_manager)?
        } else {
            // 如果没有指定config文件，但有project name，创建带project name的DockerManager
            if let Some(project_name) = &project_name {
                let custom_docker_manager = Arc::new(DockerManager::with_project(
                    client_core::constants::docker::get_compose_file_path(),
                    client_core::constants::docker::get_env_file_path(),
                    Some(project_name.clone()),
                )?);
                DockerService::new(app.config.clone(), custom_docker_manager)?
            } else {
                DockerService::new(app.config.clone(), app.docker_manager.clone())?
            }
        };
        let health_report = docker_service.health_check().await?;

        if health_report.get_running_count() > 0 {
            info!(
                "Docker服务正在运行,运行容器数量:{},准备停止服务...",
                health_report.get_running_count()
            );
            // 等待服务完全停止
            info!("⏳ 等待Docker服务完全停止...");
            let compose_path = get_compose_file_path(&config_file);
            if !docker_utils::wait_for_compose_services_stopped(
                &compose_path,
                timeout::SERVICE_STOP_TIMEOUT,
            )
            .await?
            {
                warn!("⚠️ 等待服务停止超时，但继续进行升级");
            } else {
                info!("✅ Docker服务已成功停止");
            }
        } else {
            info!("ℹ️ Docker服务未运行，跳过停止步骤");
        }

        // 4. 📄 备份当前版本的SQL文件（用于后续差异比较）
        backup_sql_file_before_upgrade().await?;
    }

    // 5. 📦 解压新的Docker服务包（在服务停止后）
    info!("📦 正在解压Docker服务包...");

    // 清理现有的docker目录以避免路径冲突
    let docker_dir = std::path::Path::new("docker");
    if docker_dir.exists() {
        // 增量升级/全量升级
        match upgrade_strategy.clone() {
            UpgradeStrategy::PatchUpgrade { patch_info, .. } => {
                // 增量升级逻辑
                let changed_files = patch_info.get_changed_files();
                //基于 docker_dir 目录下, 清理 changed_files 的相对路径的文件/目录

                let remove_file_or_dir = changed_files
                    .iter()
                    .map(|path| PathBuf::from(docker_dir).join(path))
                    .collect::<Vec<_>>();

                let remove_file_or_dir: Vec<&Path> =
                    remove_file_or_dir.iter().map(|p| p.as_path()).collect();
                match safe_remove_file_or_dir(&remove_file_or_dir).await {
                    Ok(_) => info!(
                        "✅ 清理文件/目录成功: {}",
                        &remove_file_or_dir
                            .iter()
                            .map(|p| p.to_string_lossy())
                            .collect::<Vec<_>>()
                            .join(", ")
                    ),
                    Err(e) => warn!("⚠️ 清理文件/目录失败: {}, 尝试继续解压", e),
                }
            }
            UpgradeStrategy::FullUpgrade { .. } => {
                // 全量升级逻辑
                info!("🧹 清理现有docker目录以避免文件冲突...");
                match safe_remove_docker_directory(docker_dir).await {
                    Ok(_) => info!("✅ docker目录清理完成"),
                    Err(e) => {
                        warn!("⚠️ 清理docker目录失败: {}, 尝试继续解压", e);
                        return Err(anyhow::anyhow!(format!("清理docker目录失败: {e}")));
                    }
                }
            }
            UpgradeStrategy::NoUpgrade { .. } => {
                //do nothing
                info!("版本一致,无需升级更新")
            }
        }
    }

    // 解压新的Docker服务包（使用最新版本）
    match docker_service::extract_docker_service_with_upgrade_strategy(app, upgrade_strategy).await
    {
        Ok(_) => {
            info!("✅ Docker服务包解压完成");

            // 🔧 自动修复关键脚本文件权限
            fix_script_permissions().await?;

            // 📝 更新配置文件中的Docker服务版本
            if latest_version != app.config.get_docker_versions() {
                info!(
                    "📝 更新Docker服务版本: {} -> {}",
                    app.config.get_docker_versions(),
                    latest_version
                );

                // 持久化到配置文件,这里修改docker应用版本,然后保存更新到toml配置里
                let mut config = app.config.as_ref().clone();
                //TODO: 以后需要优化这里的逻辑
                config.write_docker_versions(latest_version.clone());

                match config.save_to_file("config.toml") {
                    Ok(_) => {
                        info!("✅ 配置文件版本号已更新并保存");
                    }
                    Err(e) => {
                        warn!("⚠️ 保存配置文件失败: {}", e);
                        warn!("   版本号已在内存中更新，但配置文件未同步");
                    }
                }
            } else {
                info!("📝 版本号无需更新 (已是最新版本: {})", latest_version);
            }

            // 📊 生成SQL差异文件（仅在升级部署时）
            if !is_first_deployment {
                generate_and_save_sql_diff(&app.config.get_docker_versions(), &latest_version)
                    .await?;
            }
        }
        Err(e) => {
            error!("❌ Docker服务包解压失败: {}", e);
            return Err(e);
        }
    }

    // 6. 🔄 自动部署服务
    info!("🔄 正在部署Docker服务...");
    docker_service::deploy_docker_services(
        app,
        frontend_port,
        config_file.clone(),
        project_name.clone(),
    )
    .await?;

    // 7. ▶️ 启动服务
    info!("▶️ 正在启动Docker服务...");
    docker_service::start_docker_services(app, config_file.clone(), project_name.clone()).await?;

    // 等待服务启动完成（最多等待90秒，因为部署后启动可能需要更长时间）
    info!("⏳ 等待Docker服务完全启动...");
    let compose_path = get_compose_file_path(&config_file);
    if docker_utils::wait_for_compose_services_started(&compose_path, timeout::DEPLOY_START_TIMEOUT)
        .await?
    {
        info!("✅ 自动升级部署完成，服务已成功启动");

        // 🔄 执行数据库升级（仅在升级部署时）
        if !is_first_deployment {
            execute_sql_diff_upgrade(&config_file).await?;
        }

        info!("🎉 自动升级部署流程成功完成");
    } else {
        warn!("⚠️ 等待服务启动超时，请手动检查服务状态");

        // 最后再检查一次状态
        match check_docker_service_status(app, &config_file, &project_name).await {
            Ok(true) => {
                info!("🔍 最终检查：服务似乎已正常启动");

                // 🔄 如果服务正常，尝试执行数据库升级
                if !is_first_deployment {
                    execute_sql_diff_upgrade(&config_file).await?;
                }
            }
            Ok(false) => {
                info!("🔍 最终检查：服务可能未正常启动");
                info!("📊 详细状态检查:");
                let _ = docker_service::check_docker_services_status(app).await;
            }
            Err(e) => warn!("🔍 最终检查失败: {}", e),
        }
    }

    Ok(())
}

/// 预约延迟执行自动升级部署
pub async fn schedule_delayed_deploy(app: &mut CliApp, time: u32, unit: &str) -> Result<()> {
    // 计算延迟时间（转换为秒）
    let delay_seconds = match unit.to_lowercase().as_str() {
        "minutes" | "minute" | "min" => time * 60,
        "hours" | "hour" | "h" => time * 3600,
        "days" | "day" | "d" => time * 86400,
        _ => {
            error!("不支持的时间单位: {}", unit);
            return Err(anyhow::anyhow!(format!(
                "不支持的时间单位: {unit}，支持的单位: hours, minutes, days"
            )));
        }
    };

    let delay_duration = Duration::from_secs(delay_seconds as u64);
    let scheduled_at = chrono::Utc::now() + chrono::Duration::seconds(delay_seconds as i64);

    // 创建升级任务记录
    let task = client_core::config_manager::AutoUpgradeTask {
        task_id: uuid::Uuid::new_v4().to_string(),
        task_name: format!("delayed_upgrade_{time}"),
        schedule_time: scheduled_at,
        upgrade_type: "delayed".to_string(),
        target_version: None, // 最新版本
        status: "pending".to_string(),
        progress: Some(0),
        error_message: None,
        created_at: chrono::Utc::now(),
        updated_at: chrono::Utc::now(),
    };

    {
        let config_manager =
            client_core::config_manager::ConfigManager::new_with_database(app.database.clone());
        config_manager.create_auto_upgrade_task(&task).await?
    };

    info!("⏰ 已安排延迟执行自动升级部署");
    info!("   任务ID: {}", task.task_id);
    info!("   延迟时间: {} {}", time, unit);
    println!("   预计执行时间: {} 后", format_duration(delay_duration));
    info!(
        "   计划执行时间: {}",
        scheduled_at.format("%Y-%m-%d %H:%M:%S UTC")
    );

    info!(
        "安排延迟执行自动升级部署: {} {}，任务ID: {}",
        time, unit, task.task_id
    );

    // 更新任务状态为进行中
    {
        let config_manager =
            client_core::config_manager::ConfigManager::new_with_database(app.database.clone());
        config_manager
            .update_upgrade_task_status(&task.task_id, "in_progress", Some(0), None)
            .await?;
    }

    // 开始延迟等待
    info!("⏳ 等待中...");

    // 这里可以优化为后台任务，避免阻塞
    sleep(delay_duration).await;

    info!("🔔 延迟时间到，开始执行自动升级部署");
    info!("延迟时间到，开始执行自动升级部署，任务ID: {}", task.task_id);

    // 执行自动升级部署
    match run_auto_upgrade_deploy(app, None, None, None).await {
        Ok(_) => {
            let config_manager =
                client_core::config_manager::ConfigManager::new_with_database(app.database.clone());
            config_manager
                .update_upgrade_task_status(&task.task_id, "completed", Some(100), None)
                .await?;
            info!("✅ 延迟升级部署任务完成");
        }
        Err(e) => {
            let config_manager =
                client_core::config_manager::ConfigManager::new_with_database(app.database.clone());
            config_manager
                .update_upgrade_task_status(&task.task_id, "failed", None, Some(&e.to_string()))
                .await?;
            error!("延迟升级部署任务失败: {}", e);
            return Err(e);
        }
    }

    Ok(())
}

/// 显示自动升级部署状态
pub async fn show_status(app: &mut CliApp) -> Result<()> {
    let config_manager =
        client_core::config_manager::ConfigManager::new_with_database(app.database.clone());

    info!("📊 自动升级部署状态信息:");
    info!("   功能状态: 已实现");
    info!("   流程说明: 下载最新版本 -> 智能备份 -> 部署服务 -> 启动服务");

    // 显示待执行的升级任务
    match config_manager.get_pending_upgrade_tasks().await {
        Ok(tasks) => {
            if tasks.is_empty() {
                info!("📋 升级任务: 当前没有待执行的升级任务");
            } else {
                info!("📋 待执行的升级任务:");
                for task in tasks {
                    info!("   - 任务ID: {}", task.task_id);
                    info!("     名称: {}", task.task_name);
                    info!("     类型: {}", task.upgrade_type);
                    info!("     状态: {}", task.status);
                    info!(
                        "     计划执行时间: {}",
                        task.schedule_time.format("%Y-%m-%d %H:%M:%S UTC")
                    );
                    if let Some(target_version) = &task.target_version {
                        info!("     目标版本: {}", target_version);
                    }
                    if let Some(progress) = task.progress {
                        info!("     进度: {}%", progress);
                    }
                    if let Some(error) = &task.error_message {
                        warn!("     错误信息: {}", error);
                    }
                }
            }
        }
        Err(e) => {
            warn!("⚠️  获取升级任务信息失败: {}", e);
            info!("   注意: 当前版本的任务查询功能有限");
        }
    }

    // 显示当前Docker服务状态
    info!("🐳 当前Docker服务状态:");
    docker_service::check_docker_services_status(app).await?;

    // 显示最近的备份
    info!("📝 最近的备份:");
    backup::run_list_backups(app).await?;

    Ok(())
}

/// 检查Docker服务状态
async fn check_docker_service_status(
    app: &mut CliApp,
    config_file: &Option<PathBuf>,
    project_name: &Option<String>,
) -> Result<bool> {
    let compose_path = get_compose_file_path(config_file);

    // 🔧 修复：如果compose文件不存在，直接返回false（服务未运行）
    if !compose_path.exists() {
        info!("📝 docker-compose.yml文件不存在，服务未运行");
        return Ok(false);
    }

    // 🔧 修复：根据config_file参数创建使用正确路径的DockerManager
    if let Some(config_file_path) = config_file {
        let custom_docker_manager = Arc::new(DockerManager::with_project(
            config_file_path.clone(),
            client_core::constants::docker::get_env_file_path(),
            project_name.clone(),
        )?);
        let health_checker = HealthChecker::new(custom_docker_manager);
        let report = health_checker.health_check().await?;
        Ok(report.is_all_healthy())
    } else {
        // 如果没有指定config文件，但有project name，创建带project name的DockerManager
        if let Some(project_name) = project_name {
            let custom_docker_manager = Arc::new(DockerManager::with_project(
                client_core::constants::docker::get_compose_file_path(),
                client_core::constants::docker::get_env_file_path(),
                Some(project_name.clone()),
            )?);
            let health_checker = HealthChecker::new(custom_docker_manager);
            let report = health_checker.health_check().await?;
            Ok(report.is_all_healthy())
        } else {
            let health_checker = HealthChecker::new(app.docker_manager.clone());
            let report = health_checker.health_check().await?;
            Ok(report.is_all_healthy())
        }
    }
}

/// 检查docker目录是否存在且有文件需要备份
async fn check_docker_files_exist() -> Result<bool> {
    let docker_dir = Path::new("./docker");

    if !docker_dir.exists() {
        info!("docker目录不存在，无需备份");
        return Ok(false);
    }

    // 检查是否有重要文件需要备份
    let important_files = [
        client_core::constants::docker::COMPOSE_FILE_NAME, // docker-compose.yml
        "docker-compose.yaml",
        ".env",
        "data",
        "config",
    ];

    for file_name in important_files.iter() {
        let file_path = docker_dir.join(file_name);
        if file_path.exists() {
            info!("发现需要备份的文件: {}", file_path.display());
            return Ok(true);
        }
    }

    info!("docker目录存在但没有需要备份的重要文件");
    Ok(false)
}

/// 格式化时间间隔为可读字符串
fn format_duration(duration: Duration) -> String {
    let seconds = duration.as_secs();

    if seconds >= 86400 {
        format!("{} 天", seconds / 86400)
    } else if seconds >= 3600 {
        format!("{} 小时", seconds / 3600)
    } else if seconds >= 60 {
        format!("{} 分钟", seconds / 60)
    } else {
        format!("{seconds} 秒")
    }
}

/// 检测是否为第一次部署
async fn is_first_deployment() -> bool {
    let docker_dir = std::path::Path::new("docker");
    let docker_compose_file = docker_dir.join("docker-compose.yml");
    let docker_data_dir = docker_dir.join("data");

    // 如果docker目录不存在，肯定是第一次部署
    if !docker_dir.exists() {
        return true;
    }

    // 🔧 关键修复：如果docker-compose.yml文件不存在，视为首次部署
    // 因为没有compose文件就无法管理现有服务
    if !docker_compose_file.exists() {
        info!("📝 未找到docker-compose.yml文件，视为首次部署");
        return true;
    }

    // 如果docker/data目录不存在，也是第一次部署
    if !docker_data_dir.exists() {
        return true;
    }

    false
}

/// 在清理docker目录前备份数据目录
async fn backup_data_before_cleanup() -> Result<Option<std::path::PathBuf>> {
    let docker_data_dir = Path::new("docker/data");

    if !docker_data_dir.exists() {
        info!("📁 无现有数据目录需要备份");
        return Ok(None);
    }

    // 创建临时备份目录
    let temp_dir = std::env::temp_dir();
    let backup_name = format!("duck_data_backup_{}", chrono::Utc::now().timestamp());
    let temp_backup_path = temp_dir.join(backup_name);

    info!(
        "🛡️ 正在备份数据目录到临时位置: {}",
        temp_backup_path.display()
    );

    // 递归复制数据目录到临时位置
    match copy_dir_recursively(docker_data_dir, &temp_backup_path) {
        Ok(_) => {
            info!("✅ 数据目录备份完成");
            Ok(Some(temp_backup_path))
        }
        Err(e) => {
            warn!("⚠️ 数据目录备份失败: {}", e);
            // 备份失败时，返回None表示没有备份
            Ok(None)
        }
    }
}

/// 解压完成后恢复备份的数据目录
async fn restore_data_after_cleanup(temp_backup_path: &Option<std::path::PathBuf>) -> Result<()> {
    if let Some(backup_path) = temp_backup_path {
        if backup_path.exists() {
            let docker_data_dir = Path::new("docker/data");

            info!("🔄 正在恢复数据目录从: {}", backup_path.display());

            // 确保目标目录存在
            if let Some(parent) = docker_data_dir.parent() {
                fs::create_dir_all(parent)?;
            }

            // 如果新解压的包中有data目录，先删除它
            if docker_data_dir.exists() {
                fs::remove_dir_all(docker_data_dir)?;
            }

            // 从临时备份恢复数据目录
            match copy_dir_recursively(backup_path, docker_data_dir) {
                Ok(_) => {
                    info!("✅ 数据目录恢复完成");

                    // 设置正确的权限（特别是MySQL目录需要775权限）
                    let mysql_data_dir = docker_data_dir.join("mysql");
                    if mysql_data_dir.exists() {
                        #[cfg(unix)]
                        {
                            use std::os::unix::fs::PermissionsExt;
                            let permissions = fs::Permissions::from_mode(0o775);
                            fs::set_permissions(&mysql_data_dir, permissions)?;
                            info!("🔒 已设置MySQL数据目录权限为775");
                        }
                    }
                }
                Err(e) => {
                    error!("❌ 数据目录恢复失败: {}", e);
                    return Err(anyhow::anyhow!(format!("数据目录恢复失败: {e}")));
                }
            }

            // 清理临时备份
            if let Err(e) = fs::remove_dir_all(backup_path) {
                warn!("⚠️ 清理临时备份失败: {}", e);
            } else {
                info!("🧹 临时备份已清理");
            }
        }
    } else {
        info!("📁 无备份数据需要恢复");
    }

    Ok(())
}

/// 递归复制目录
fn copy_dir_recursively(src: &Path, dst: &Path) -> std::io::Result<()> {
    if !src.exists() {
        return Ok(());
    }

    fs::create_dir_all(dst)?;

    for entry in fs::read_dir(src)? {
        let entry = entry?;
        let src_path = entry.path();
        let dst_path = dst.join(entry.file_name());

        if src_path.is_dir() {
            copy_dir_recursively(&src_path, &dst_path)?;
        } else {
            fs::copy(&src_path, &dst_path)?;
        }
    }

    Ok(())
}

/// 备份当前版本的SQL文件（用于后续差异比较）
async fn backup_sql_file_before_upgrade() -> Result<()> {
    let current_sql_path = Path::new("docker/config/init_mysql.sql");
    let temp_sql_dir = Path::new("temp_sql");
    let old_sql_path = temp_sql_dir.join("init_mysql_old.sql");

    // 创建临时SQL目录
    if !temp_sql_dir.exists() {
        fs::create_dir_all(temp_sql_dir)?;
        info!("📁 创建临时SQL目录: {}", temp_sql_dir.display());
    }

    // 🔧 关键修复：如果 upgrade_diff.sql 已存在，则不覆盖 init_mysql_old.sql
    // 避免重新部署时旧版本SQL被新版本覆盖，导致差异为空
    let diff_sql_path = temp_sql_dir.join("upgrade_diff.sql");
    if diff_sql_path.exists() {
        if old_sql_path.exists() {
            info!("✅ 检测到已有差异SQL文件和旧版本SQL文件，保持不变");
            info!("   跳过备份以保护旧版本SQL: {}", old_sql_path.display());
            return Ok(());
        } else {
            warn!("⚠️ 发现差异SQL文件但缺少旧版本SQL文件，将重新备份");
        }
    }

    // 复制当前SQL文件到临时目录
    // 注意：此函数只在非首次部署时调用，所以SQL文件应该存在
    if current_sql_path.exists() {
        fs::copy(current_sql_path, &old_sql_path)?;
        info!("📄 已备份当前版本SQL文件: {}", old_sql_path.display());
    } else {
        // 如果文件不存在，说明可能是特殊情况，记录警告但不中断流程
        warn!("⚠️ 当前版本SQL文件不存在");
        // 创建空的占位文件，后续差异生成会处理
        fs::write(&old_sql_path, "")?;
        info!("📄 创建空的旧版本SQL占位文件");
    }

    Ok(())
}

/// 生成并保存SQL差异文件
async fn generate_and_save_sql_diff(from_version: &str, to_version: &str) -> Result<()> {
    let temp_sql_dir = Path::new("temp_sql");
    let old_sql_path = temp_sql_dir.join("init_mysql_old.sql");
    let new_sql_path = temp_sql_dir.join("init_mysql_new.sql");
    let diff_sql_path = temp_sql_dir.join("upgrade_diff.sql");

    // 复制新版本的SQL文件
    let current_sql_path = Path::new("docker/config/init_mysql.sql");
    if current_sql_path.exists() {
        fs::copy(current_sql_path, &new_sql_path)?;
        info!("📄 已复制新版本SQL文件: {}", new_sql_path.display());
    } else {
        info!("📄 新版本没有SQL文件，跳过差异生成");
        return Ok(());
    }

    // 读取旧版本SQL文件内容（前面备份函数已确保此文件存在）
    let old_sql_content = fs::read_to_string(&old_sql_path)?;
    let old_sql_content = if old_sql_content.trim().is_empty() {
        info!("📄 旧版本SQL文件为空，将生成完整的初始化脚本");
        None
    } else {
        Some(old_sql_content)
    };

    // 读取新版本SQL文件内容
    let new_sql_content = fs::read_to_string(&new_sql_path)?;

    // 生成SQL差异
    info!("🔄 正在生成SQL差异...");
    let (diff_sql, description) = generate_schema_diff(
        old_sql_content.as_deref(),
        &new_sql_content,
        Some(from_version),
        to_version,
    )
    .map_err(|e| client_core::error::DuckError::custom(format!("生成SQL差异失败: {e}")))?;

    info!("📊 SQL差异分析结果: {}", description);

    // 检查是否有实际的SQL语句需要执行
    let meaningful_lines: Vec<&str> = diff_sql
        .lines()
        .filter(|line| !line.trim().is_empty() && !line.trim().starts_with("--"))
        .collect();

    if meaningful_lines.is_empty() {
        info!("✅ 数据库架构无变化，无需执行升级脚本");

        // 🗂️ 重命名空差异文件以保留历史记录
        if diff_sql_path.exists() {
            let parent = diff_sql_path.parent().unwrap_or(Path::new("."));
            let timestamp = chrono::Local::now().format("%Y%m%d_%H%M%S");
            let new_name = format!("diff_sql_empty_{timestamp}.sql");
            let new_path = parent.join(new_name);

            match fs::rename(&diff_sql_path, &new_path) {
                Ok(_) => info!("📝 已归档空差异SQL文件: {}", new_path.display()),
                Err(e) => warn!("⚠️ 归档空差异SQL文件失败: {}", e),
            }
        }

        return Ok(());
    }

    // 保存差异SQL文件
    fs::write(&diff_sql_path, &diff_sql)?;
    info!("📄 已保存SQL差异文件: {}", diff_sql_path.display());
    info!("📋 发现 {} 行可执行的SQL语句", meaningful_lines.len());

    // 显示差异SQL内容（截取前几行）
    let diff_lines: Vec<&str> = diff_sql.lines().take(10).collect();
    info!("📋 差异SQL预览（前10行）:");
    for line in diff_lines {
        if !line.trim().is_empty() {
            info!("    {}", line);
        }
    }

    if diff_sql.lines().count() > 10 {
        info!("    ... 更多内容请查看文件: {}", diff_sql_path.display());
    }

    Ok(())
}

//批量删除文件,或者目录
async fn safe_remove_file_or_dir(paths: &[&Path]) -> Result<()> {
    for path in paths {
        if !path.exists() {
            continue;
        }

        if path.is_file() {
            fs::remove_file(path)?;
        } else if path.is_dir() {
            safe_remove_docker_directory(path).await?;
        }
    }
    Ok(())
}

/// 安全地删除目录，处理"Directory not empty"错误（保留upload目录）
async fn safe_remove_docker_directory(path: &Path) -> Result<()> {
    if !path.exists() {
        return Ok(());
    }

    let mut attempts = 0;
    const MAX_ATTEMPTS: usize = 3;

    while attempts < MAX_ATTEMPTS {
        attempts += 1;

        // 首先尝试安全删除（保留upload目录）
        if let Err(e) = force_cleanup_directory(path).await {
            warn!(
                "⚠️ 安全删除目录失败 (尝试 {}/{}): {}",
                attempts, MAX_ATTEMPTS, e
            );

            if attempts >= MAX_ATTEMPTS {
                return Err(anyhow::anyhow!(format!(
                    "在 {} 次尝试后，目录 {} 仍无法删除: {}",
                    MAX_ATTEMPTS,
                    path.display(),
                    e
                )));
            }
        } else {
            info!("✅ 成功安全删除目录: {}", path.display());
            return Ok(());
        }
    }

    unreachable!()
}

/// 强制清理目录内容（保留upload目录）
async fn force_cleanup_directory(path: &Path) -> Result<()> {
    info!("🧹 尝试强制清理目录内容: {}", path.display());

    if !path.exists() {
        return Ok(());
    }

    // 递归遍历并删除文件
    match std::fs::read_dir(path) {
        Ok(entries) => {
            for entry in entries {
                if let Ok(entry) = entry {
                    let entry_path = entry.path();
                    let file_name = entry.file_name();
                    let file_name_str = file_name.to_string_lossy();

                    // 只检查docker目录下的第一层[upload, project_workspace, project_zips, project_nginx, project_init]目录

                    // 排除指定目录，不进行删除
                    const EXCLUDE_DIRS: [&str; 7] = [
                        "upload",
                        "project_workspace",
                        "project_zips",
                        "project_nginx",
                        "project_init",
                        "uv_cache",
                        "data",
                    ];

                    if EXCLUDE_DIRS.contains(&file_name_str.as_ref()) && entry_path.is_dir() {
                        info!("📁 跳过目录: {}", entry_path.display());
                        continue;
                    }

                    if entry_path.is_dir() {
                        // 递归删除子目录
                        if let Err(e) = Box::pin(force_cleanup_directory(&entry_path)).await {
                            warn!("📁 删除子目录失败: {} - {}", entry_path.display(), e);
                        }

                        // 尝试删除空目录
                        if let Err(e) = std::fs::remove_dir(&entry_path) {
                            warn!("📁 删除空目录失败: {} - {}", entry_path.display(), e);
                        }
                    } else {
                        if let Err(e) = std::fs::remove_file(&entry_path) {
                            warn!("📄 删除文件失败: {} - {}", entry_path.display(), e);
                        }
                    }
                }
            }
        }
        Err(e) => {
            warn!("📂 读取目录内容失败: {}", e);
        }
    }

    Ok(())
}

/// 连接MySQL容器并执行差异SQL
async fn execute_sql_diff_upgrade(config_file: &Option<PathBuf>) -> Result<()> {
    let temp_sql_dir = Path::new("temp_sql");
    let diff_sql_path = temp_sql_dir.join("upgrade_diff.sql");

    // 检查差异SQL文件是否存在
    if !diff_sql_path.exists() {
        info!("📄 没有发现SQL差异文件，跳过数据库升级");
        return Ok(());
    }

    // 🔄 重新生成差异SQL以确保准确性
    info!("🔄 检测到差异SQL文件，重新生成以确保准确性...");

    let old_sql_path = temp_sql_dir.join("init_mysql_old.sql");
    let new_sql_path = temp_sql_dir.join("init_mysql_new.sql");

    // 读取新旧版本SQL文件内容
    let diff_sql = if old_sql_path.exists() && new_sql_path.exists() {
        let old_sql_content = fs::read_to_string(&old_sql_path)?;
        let new_sql_content = fs::read_to_string(&new_sql_path)?;

        // 重新生成差异SQL
        info!("📊 正在基于源文件重新生成SQL差异...");
        let (regenerated_diff_sql, description) = generate_schema_diff(
            if old_sql_content.trim().is_empty() {
                None
            } else {
                Some(&old_sql_content)
            },
            &new_sql_content,
            Some("旧版本"),
            "新版本",
        )
        .map_err(|e| anyhow::anyhow!("重新生成SQL差异失败: {}", e))?;

        info!("📋 差异生成结果: {}", description);

        // 保存重新生成的差异SQL文件（覆盖旧文件）
        fs::write(&diff_sql_path, &regenerated_diff_sql)?;
        info!(
            "💾 已保存重新生成的差异SQL文件: {}",
            diff_sql_path.display()
        );

        regenerated_diff_sql
    } else {
        // 如果源文件不存在，使用已有的差异文件
        warn!("⚠️ 缺少源SQL文件，使用已存在的差异SQL文件");
        fs::read_to_string(&diff_sql_path)?
    };

    // 检查是否有实际的SQL语句需要执行
    let meaningful_lines: Vec<&str> = diff_sql
        .lines()
        .filter(|line| {
            let trimmed = line.trim();
            !trimmed.is_empty() && !trimmed.starts_with("--") && !trimmed.starts_with("/*")
        })
        .collect();

    if meaningful_lines.is_empty() {
        info!("📄 差异SQL为空，无需执行数据库升级");

        // 🗂️ 重命名空差异文件以保留历史记录
        if diff_sql_path.exists() {
            let parent = diff_sql_path.parent().unwrap_or(Path::new("."));
            let timestamp = chrono::Local::now().format("%Y%m%d_%H%M%S");
            let new_name = format!("diff_sql_empty_{timestamp}.sql");
            let new_path = parent.join(new_name);

            match fs::rename(&diff_sql_path, &new_path) {
                Ok(_) => info!("📝 已归档空差异SQL文件: {}", new_path.display()),
                Err(e) => warn!("⚠️ 归档空差异SQL文件失败: {}", e),
            }
        }

        return Ok(());
    }

    info!("🔄 开始执行数据库升级...");
    info!("📋 即将执行 {} 行SQL语句", meaningful_lines.len());

    //从App配置中动态获取MySQL端口
    let compose_file = get_compose_file_path(&config_file);
    let env_file = client_core::constants::docker::get_env_file_path();
    let compose_file_str = compose_file
        .to_str()
        .ok_or_else(|| anyhow::anyhow!("无法将 docker-compose.yml 路径转换为字符串"))?;
    let env_file_str = env_file
        .to_str()
        .ok_or_else(|| anyhow::anyhow!("无法将 .env 文件路径转换为字符串"))?;

    let config = MySqlConfig::for_container(Some(compose_file_str), Some(env_file_str)).await?;
    let executor = MySqlExecutor::new(config);

    info!("🔌 正在连接到MySQL数据库...");
    if let Err(e) = executor.test_connection().await {
        error!("❌ 数据库连接失败: {}", e);
        error!("🏃 请确保MySQL容器正在运行并且端口 13306 可访问");
        return Err(e.into());
    }

    info!("🚀 开始执行差异SQL...");
    match executor.execute_diff_sql_with_retry(&diff_sql, 3).await {
        Ok(results) => {
            for result in results {
                info!("  {}", result);
            }
            // Rename diff SQL file after successful upgrade to preserve history
            if diff_sql_path.is_file() {
                let parent = diff_sql_path.parent().unwrap_or(Path::new("."));
                let timestamp = chrono::Local::now().format("%Y%m%d_%H%M%S");
                let new_name = format!("diff_sql_executed_{timestamp}.sql");
                let new_path = parent.join(new_name);

                match fs::rename(&diff_sql_path, &new_path) {
                    Ok(_) => info!("✅ Renamed diff SQL file to: {}", new_path.display()),
                    Err(e) => warn!("⚠️ Failed to rename diff SQL file: {}", e),
                }
            }

            info!("✅ 数据库升级成功");
        }
        Err(e) => {
            error!("❌ 数据库升级失败: {}", e);
            return Err(e);
        }
    }

    Ok(())
}

/// 自动修复关键脚本文件权限
async fn fix_script_permissions() -> Result<()> {
    info!("🔧 正在修复关键脚本文件权限...");

    // 需要修复权限的脚本文件列表
    let script_files = ["docker/config/docker-entrypoint.sh"];

    let mut fixed_count = 0;
    let mut total_count = 0;

    for script_path in script_files.iter() {
        let path = std::path::Path::new(script_path);

        if path.exists() {
            total_count += 1;

            // 检查当前权限
            match std::fs::metadata(path) {
                Ok(metadata) => {
                    #[cfg(unix)]
                    {
                        use std::os::unix::fs::PermissionsExt;
                        let current_mode = metadata.permissions().mode() & 0o777;

                        // 如果没有执行权限，添加执行权限
                        if current_mode & 0o111 == 0 {
                            info!(
                                "🔒 修复权限: {} (当前: {:o} -> 目标: 755)",
                                path.display(),
                                current_mode
                            );

                            let new_permissions = std::fs::Permissions::from_mode(0o755);
                            if let Err(e) = std::fs::set_permissions(path, new_permissions) {
                                warn!("⚠️ 修复权限失败 {}: {}", path.display(), e);
                            } else {
                                fixed_count += 1;
                                info!("✅ 权限修复成功: {}", path.display());
                            }
                        } else {
                            info!("✓ 权限正常: {} ({:o})", path.display(), current_mode);
                        }
                    }

                    #[cfg(not(unix))]
                    {
                        info!("ℹ️ 非Unix系统，跳过权限修复: {}", path.display());
                    }
                }
                Err(e) => {
                    warn!("⚠️ 无法读取文件元数据 {}: {}", path.display(), e);
                }
            }
        } else {
            info!("📄 脚本文件不存在，跳过: {}", script_path);
        }
    }

    if total_count > 0 {
        info!(
            "🔧 权限修复完成: {}/{} 个脚本文件已修复",
            fixed_count, total_count
        );
    } else {
        info!("📄 未找到需要修复权限的脚本文件");
    }

    Ok(())
}

/// 获取最新备份的ID
async fn get_latest_backup_id(app: &CliApp) -> Result<Option<i64>> {
    let backup_manager = client_core::backup::BackupManager::new(
        app.config.get_backup_dir(),
        app.database.clone(),
        app.docker_manager.clone(),
    )?;

    match backup_manager.list_backups().await {
        Ok(backups) => {
            if backups.is_empty() {
                info!("📁 未找到备份记录");
                Ok(None)
            } else {
                // 获取最新的备份（按创建时间排序，取最新的）
                let latest_backup = backups
                    .iter()
                    .max_by(|a, b| a.created_at.cmp(&b.created_at));

                match latest_backup {
                    Some(backup) => {
                        info!(
                            "✅ 找到最新备份ID: {} (创建时间: {})",
                            backup.id,
                            backup.created_at.format("%Y-%m-%d %H:%M:%S")
                        );

                        //检查备份文件是否存在,
                        let backup_file = Path::new(&backup.file_path);
                        if !backup_file.exists() {
                            warn!(
                                "❌ 数据库中记录的备份文件,再磁盘上不存在: {}",
                                backup_file.display()
                            );
                            Ok(None)
                        } else {
                            Ok(Some(backup.id))
                        }
                    }
                    None => {
                        info!("📁 备份列表为空");
                        Ok(None)
                    }
                }
            }
        }
        Err(e) => {
            error!("❌ 获取备份列表失败: {}", e);
            Err(e)
        }
    }
}

/// 检查并安装 nuwax-cli 更新
///
/// 此函数直接使用现有的 check-update install 逻辑来检查和安装更新
/// 如果发现新版本，会自动下载安装并使用 self-replace 库替换当前进程
async fn check_and_install_nuwax_cli_update() -> Result<()> {
    use crate::commands::check_update::{check_for_updates, install_release};

    // 检查是否有可用更新
    match check_for_updates().await {
        Ok(version_info) => {
            if !version_info.is_update_available {
                info!(
                    "✅ nuwax-cli 已是最新版本: {}",
                    version_info.current_version
                );
                return Ok(());
            }

            info!("🆕 发现新版本可用:");
            info!("   当前版本: {}", version_info.current_version);
            info!("   最新版本: {}", version_info.latest_version);

            if let Some(download_url) = &version_info.download_url {
                info!("   下载地址: {}", download_url);

                // 执行自动安装
                info!(
                    "🚀 开始自动安装 nuwax-cli {}...",
                    version_info.latest_version
                );

                match install_release(download_url, &version_info.latest_version).await {
                    Ok(_) => {
                        info!("✅ nuwax-cli 安装完成！");
                        info!("🔄 self-replace 库将自动重启进程以使用新版本...");

                        // install_release 函数内部的 self_replace::self_replace()
                        // 会自动处理进程替换，这里不需要额外处理
                        // 但为了确保调用者知道发生了什么，我们返回一个特殊错误
                        return Err(anyhow::anyhow!("nuwax-cli 已更新并自动重启"));
                    }
                    Err(e) => {
                        error!("❌ nuwax-cli 自动安装失败: {}", e);
                        error!("   请手动运行: nuwax-cli check-update install");
                        return Err(e);
                    }
                }
            } else {
                warn!("⚠️ 未找到适合当前平台的下载包");
                warn!("   请访问 GitHub Releases 页面手动下载");
                return Err(anyhow::anyhow!("未找到适合当前平台的下载包"));
            }
        }
        Err(e) => {
            error!("❌ 检查 nuwax-cli 更新失败: {}", e);
            return Err(e);
        }
    }
}
