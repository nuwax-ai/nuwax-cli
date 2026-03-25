use crate::app::CliApp;
use crate::cli::AutoUpgradeDeployCommand;
use crate::commands::{backup, docker_service, update};
use crate::docker_service::health_check::HealthChecker;
use crate::docker_utils;
use anyhow::{Context, Result};
use client_core::constants::sql;
use client_core::container::DockerManager;
use client_core::mysql_executor::{MySqlConfig, MySqlExecutor};
use client_core::sql_diff::generate_live_schema_diff;
use client_core::upgrade_strategy::UpgradeStrategy;
use rust_i18n::t;
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

/// 创建 DockerManager（统一处理 config_file 和 project_name）
///
/// # 参数
/// - `config_file`: 可选的自定义 docker-compose 配置文件路径
/// - `project_name`: 可选的 docker-compose 项目名称
///
/// # 返回
/// 返回配置好的 DockerManager Arc 引用
fn create_docker_manager(
    config_file: &Option<PathBuf>,
    project_name: &Option<String>,
) -> Result<Arc<DockerManager>> {
    let compose_path = get_compose_file_path(config_file);
    let env_path = client_core::constants::docker::get_env_file_path();

    Ok(Arc::new(DockerManager::with_project(
        compose_path,
        env_path,
        project_name.clone(),
    )?))
}

/// 更新配置文件中的版本号并持久化
///
/// 使用 Arc::make_mut 来获取可变引用，如果 Arc 有多个引用会自动克隆
///
/// # 参数
/// - `config`: 应用配置的可变 Arc 引用
/// - `version`: 新的版本号字符串
///
/// # 错误
/// 如果保存配置文件失败会返回错误
fn update_config_version(
    config: &mut Arc<client_core::config::AppConfig>,
    version: &str,
) -> Result<()> {
    let config_mut = Arc::make_mut(config);
    config_mut.write_docker_versions(version.to_string());

    config_mut
        .save_to_file("config.toml")
        .context(t!("auto_upgrade_deploy.save_config_failed"))?;

    info!(version = version, "{}", t!("auto_upgrade_deploy.config_version_updated"));
    Ok(())
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
            info!("{}", t!("auto_upgrade_deploy.start_process"));
            run_auto_upgrade_deploy(app, port, config, project).await
        }
        AutoUpgradeDeployCommand::Status => {
            info!("{}", t!("auto_upgrade_deploy.show_status"));
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
    info!("{}", t!("auto_upgrade_deploy.start_process"));

    // 如果指定了端口，显示端口信息
    if let Some(port) = frontend_port {
        info!("{}", t!("auto_upgrade_deploy.custom_frontend_port", port = port));
    }

    // 如果指定了配置文件，显示配置文件信息
    if let Some(config_path) = &config_file {
        info!("{}", t!("auto_upgrade_deploy.custom_compose_file", path = config_path.display()));
    }

    // 注意：CLI版本检查已经在 main.rs 中优先处理，这里不再重复检查
    info!("{}", t!("auto_upgrade_deploy.cli_check_done"));

    // 1. 获取最新版本信息并下载
    info!("{}", t!("auto_upgrade_deploy.downloading_latest"));

    // 获取最新版本信息
    let latest_version = match app.api_client.get_enhanced_service_manifest().await {
        Ok(enhanced_service_manifest) => {
            let latest_version = enhanced_service_manifest.version.to_string();

            info!(
                current_version = %app.config.get_docker_versions(),
                target_version = %latest_version,
                "{}", t!("auto_upgrade_deploy.version_detected")
            );
            latest_version
        }
        Err(e) => {
            warn!(
                error = %e,
                fallback_version = %app.config.get_docker_versions(),
                "{}", t!("auto_upgrade_deploy.version_fetch_failed")
            );
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
        info!("{}", t!("auto_upgrade_deploy.first_deployment_detected"));
    } else {
        info!("{}", t!("auto_upgrade_deploy.upgrade_deployment_detected"));

        // 3. 🛑 停止服务并等待（使用统一的公共方法）
        docker_service::stop_docker_services_and_wait(
            app,
            config_file.clone(),
            project_name.clone(),
        )
        .await?;
    }

    // 5. 🔍 提前检查并创建挂载目录（重要：Windows Podman Desktop 需要）
    info!("{}", t!("auto_upgrade_deploy.check_mount_dirs"));

    let docker_manager = create_docker_manager(&config_file, &project_name)?;

    // 使用新的环境检测机制
    let runtime_env = docker_manager.get_runtime_environment();

    if runtime_env.needs_special_handling() {
        info!("{}", t!("auto_upgrade_deploy.windows_podman_detected"));
        info!("{}", t!("auto_upgrade_deploy.env_info", env = runtime_env.summary()));
        info!("{}", t!("auto_upgrade_deploy.podman_no_auto_create"));

        if let Err(e) = docker_manager.ensure_host_volumes_exist().await {
            warn!("{}", t!("auto_upgrade_deploy.mount_dir_check_failed", error = e.to_string()));
            warn!("{}", t!("auto_upgrade_deploy.mount_dir_check_failed_hint"));
        } else {
            info!("{}", t!("auto_upgrade_deploy.mount_dir_check_done"));
        }
    } else {
        info!("{}", t!("auto_upgrade_deploy.current_env", env = runtime_env.summary()));
    }

    // 6. 📦 解压新的Docker服务包（在服务停止后）
    info!("{}", t!("auto_upgrade_deploy.extracting_docker_package"));

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
                        "{}",
                        t!("auto_upgrade_deploy.clean_files_success",
                            files = &remove_file_or_dir
                                .iter()
                                .map(|p| p.to_string_lossy())
                                .collect::<Vec<_>>()
                                .join(", "))
                    ),
                    Err(e) => warn!("{}", t!("auto_upgrade_deploy.clean_files_failed", error = e.to_string())),
                }
            }
            UpgradeStrategy::FullUpgrade { .. } => {
                // 全量升级逻辑
                info!("{}", t!("auto_upgrade_deploy.cleaning_docker_dir"));
                match safe_remove_docker_directory(docker_dir).await {
                    Ok(_) => info!("{}", t!("auto_upgrade_deploy.docker_dir_cleaned")),
                    Err(e) => {
                        warn!("{}", t!("auto_upgrade_deploy.clean_docker_dir_failed", error = e.to_string()));
                        return Err(anyhow::anyhow!(t!("auto_upgrade_deploy.clean_docker_dir_error", error = e.to_string())));
                    }
                }
            }
            UpgradeStrategy::NoUpgrade { .. } => {
                //do nothing
                info!("{}", t!("auto_upgrade_deploy.version_unchanged"))
            }
        }
    }

    // 解压新的Docker服务包（使用最新版本）
    match docker_service::extract_docker_service_with_upgrade_strategy(app, upgrade_strategy).await
    {
        Ok(_) => {
            info!("{}", t!("auto_upgrade_deploy.docker_extract_done"));

            // 🔧 自动修复关键脚本文件权限
            fix_script_permissions().await?;

            // 📝 更新配置文件中的Docker服务版本
            if latest_version != app.config.get_docker_versions() {
                info!(
                    from_version = %app.config.get_docker_versions(),
                    to_version = %latest_version,
                    "{}", t!("auto_upgrade_deploy.updating_docker_version")
                );

                // 更新版本号并保存到配置文件
                update_config_version(&mut app.config, &latest_version)?;
            } else {
                info!(
                    version = %latest_version,
                    "{}", t!("auto_upgrade_deploy.version_no_update")
                );
            }

            // 📊 SQL差异将在服务启动后通过 Live Diff 生成和执行
        }
        Err(e) => {
            error!("{}", t!("auto_upgrade_deploy.docker_extract_failed", error = e.to_string()));
            return Err(e);
        }
    }

    // 6. 🔄 自动部署服务
    info!("{}", t!("auto_upgrade_deploy.deploying_services"));
    docker_service::deploy_docker_services(
        app,
        frontend_port,
        config_file.clone(),
        project_name.clone(),
    )
    .await?;

    // 7. ▶️ 启动服务
    info!("{}", t!("auto_upgrade_deploy.starting_services"));
    docker_service::start_docker_services(app, config_file.clone(), project_name.clone()).await?;

    // 获取 compose 文件路径
    let compose_path = get_compose_file_path(&config_file);

    // 🔄 分阶段等待服务启动（解决 MySQL-Java 死锁问题）
    // 死锁原因：Java 容器健康检查依赖 MySQL 表结构升级，但 SQL 升级要等所有服务就绪后才执行
    // 解决方案：先等待 MySQL → 执行 SQL 升级 → 再等待其他服务
    if !is_first_deployment {
        // 阶段 1/2: 仅等待 MySQL 容器就绪
        info!("{}", t!("auto_upgrade_deploy.phase1_wait_mysql"));
        let mysql_ready =
            docker_utils::wait_for_mysql_ready(&compose_path, sql::MYSQL_READY_TIMEOUT).await?;

        if mysql_ready {
            info!("{}", t!("auto_upgrade_deploy.mysql_ready"));
        } else {
            warn!("{}", t!("auto_upgrade_deploy.mysql_wait_timeout"));
        }

        // 阶段 2/2（前半部分）: 执行 SQL 升级
        info!("{}", t!("auto_upgrade_deploy.executing_db_upgrade"));
        execute_sql_diff_upgrade(&config_file).await?;
    }

    // 等待所有服务完全启动（Java 等现在可以正常启动）
    if is_first_deployment {
        info!("{}", t!("auto_upgrade_deploy.waiting_all_services"));
    } else {
        info!("{}", t!("auto_upgrade_deploy.phase2_wait_all"));
    }
    if docker_utils::wait_for_compose_services_started(&compose_path, sql::OTHER_SERVICES_TIMEOUT)
        .await?
    {
        info!("{}", t!("auto_upgrade_deploy.deploy_complete_services_started"));
        info!("{}", t!("auto_upgrade_deploy.deploy_success"));
    } else {
        warn!("{}", t!("auto_upgrade_deploy.wait_timeout"));

        // 最后再检查一次状态
        match check_docker_service_status(app, &config_file, &project_name).await {
            Ok(true) => {
                info!("{}", t!("auto_upgrade_deploy.final_check_running"));
            }
            Ok(false) => {
                info!("{}", t!("auto_upgrade_deploy.final_check_not_running"));
                info!("{}", t!("auto_upgrade_deploy.detailed_status_check"));
                let _ = docker_service::check_docker_services_status(app).await;
            }
            Err(e) => warn!("{}", t!("auto_upgrade_deploy.final_check_failed", error = e.to_string())),
        }
    }

    Ok(())
}

/// 预约延迟执行自动升级部署
#[allow(dead_code)]
pub async fn schedule_delayed_deploy(app: &mut CliApp, time: u32, unit: &str) -> Result<()> {
    // 计算延迟时间（转换为秒）
    let delay_seconds = match unit.to_lowercase().as_str() {
        "minutes" | "minute" | "min" => time * 60,
        "hours" | "hour" | "h" => time * 3600,
        "days" | "day" | "d" => time * 86400,
        _ => {
            error!("{}", t!("auto_upgrade_deploy.unsupported_time_unit", unit = unit));
            return Err(anyhow::anyhow!(t!(
                "auto_upgrade_deploy.unsupported_time_unit_error",
                unit = unit
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

    info!("{}", t!("auto_upgrade_deploy.delayed_scheduled"));
    info!("{}", t!("auto_upgrade_deploy.task_id", id = task.task_id));
    info!("{}", t!("auto_upgrade_deploy.delay_time", time = time, unit = unit));
    println!("   {}", t!("auto_upgrade_deploy.estimated_exec_time", duration = format_duration(delay_duration)));
    info!(
        "{}",
        t!("auto_upgrade_deploy.scheduled_exec_time",
            time = scheduled_at.format("%Y-%m-%d %H:%M:%S UTC"))
    );

    info!(
        "{}",
        t!("auto_upgrade_deploy.delayed_deploy_scheduled",
            time = time, unit = unit, task_id = task.task_id)
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
    info!("{}", t!("auto_upgrade_deploy.waiting"));

    // 这里可以优化为后台任务，避免阻塞
    sleep(delay_duration).await;

    info!("{}", t!("auto_upgrade_deploy.delay_time_reached"));
    info!("{}", t!("auto_upgrade_deploy.delayed_deploy_starting", task_id = task.task_id));

    // 执行自动升级部署
    match run_auto_upgrade_deploy(app, None, None, None).await {
        Ok(_) => {
            let config_manager =
                client_core::config_manager::ConfigManager::new_with_database(app.database.clone());
            config_manager
                .update_upgrade_task_status(&task.task_id, "completed", Some(100), None)
                .await?;
            info!("{}", t!("auto_upgrade_deploy.delayed_task_completed"));
        }
        Err(e) => {
            let config_manager =
                client_core::config_manager::ConfigManager::new_with_database(app.database.clone());
            config_manager
                .update_upgrade_task_status(&task.task_id, "failed", None, Some(&e.to_string()))
                .await?;
            error!("{}", t!("auto_upgrade_deploy.delayed_task_failed", error = e.to_string()));
            return Err(e);
        }
    }

    Ok(())
}

/// 显示自动升级部署状态
pub async fn show_status(app: &mut CliApp) -> Result<()> {
    let config_manager =
        client_core::config_manager::ConfigManager::new_with_database(app.database.clone());

    info!("{}", t!("auto_upgrade_deploy.status_title"));
    info!("{}", t!("auto_upgrade_deploy.feature_status"));
    info!("{}", t!("auto_upgrade_deploy.process_desc"));

    // 显示待执行的升级任务
    match config_manager.get_pending_upgrade_tasks().await {
        Ok(tasks) => {
            if tasks.is_empty() {
                info!("{}", t!("auto_upgrade_deploy.no_pending_tasks"));
            } else {
                info!("{}", t!("auto_upgrade_deploy.pending_tasks_title"));
                for task in tasks {
                    info!("{}", t!("auto_upgrade_deploy.task_id_item", id = task.task_id));
                    info!("{}", t!("auto_upgrade_deploy.task_name_item", name = task.task_name));
                    info!("{}", t!("auto_upgrade_deploy.task_type_item", type_name = task.upgrade_type));
                    info!("{}", t!("auto_upgrade_deploy.task_status_item", status = task.status));
                    info!(
                        "{}",
                        t!("auto_upgrade_deploy.task_schedule_time",
                            time = task.schedule_time.format("%Y-%m-%d %H:%M:%S UTC"))
                    );
                    if let Some(target_version) = &task.target_version {
                        info!("{}", t!("auto_upgrade_deploy.task_target_version", version = target_version));
                    }
                    if let Some(progress) = task.progress {
                        info!("{}", t!("auto_upgrade_deploy.task_progress", progress = progress));
                    }
                    if let Some(error) = &task.error_message {
                        warn!("{}", t!("auto_upgrade_deploy.task_error_message", error = error));
                    }
                }
            }
        }
        Err(e) => {
            warn!("{}", t!("auto_upgrade_deploy.get_tasks_failed", error = e.to_string()));
            info!("{}", t!("auto_upgrade_deploy.task_query_limited"));
        }
    }

    // 显示当前Docker服务状态
    info!("{}", t!("auto_upgrade_deploy.docker_status_title"));
    docker_service::check_docker_services_status(app).await?;

    // 显示最近的备份
    info!("{}", t!("auto_upgrade_deploy.recent_backups_title"));
    backup::run_list_backups(app).await?;

    Ok(())
}

/// 检查Docker服务状态（是否有服务在运行）
///
/// 返回 true 表示有服务在运行，false 表示没有服务在运行
async fn check_docker_service_status(
    _app: &mut CliApp,
    config_file: &Option<PathBuf>,
    project_name: &Option<String>,
) -> Result<bool> {
    let compose_path = get_compose_file_path(config_file);

    // 如果compose文件不存在，直接返回false（服务未运行）
    if !compose_path.exists() {
        info!("{}", t!("auto_upgrade_deploy.compose_not_exists"));
        return Ok(false);
    }

    // 使用统一的 DockerManager 创建逻辑
    let docker_manager = create_docker_manager(config_file, project_name)?;
    let health_checker = HealthChecker::new(docker_manager);
    let report = health_checker.health_check().await?;

    // 检查是否有运行中的容器（而不是检查是否所有服务都健康）
    let running_count = report.get_running_count();

    if running_count > 0 {
        info!("{}", t!("auto_upgrade_deploy.found_running_services", count = running_count));
        Ok(true)
    } else {
        info!("{}", t!("auto_upgrade_deploy.no_running_services"));
        Ok(false)
    }
}

/// 格式化时间间隔为可读字符串
#[allow(dead_code)]
fn format_duration(duration: Duration) -> String {
    let seconds = duration.as_secs();

    if seconds >= 86400 {
        t!("auto_upgrade_deploy.days", count = seconds / 86400).to_string()
    } else if seconds >= 3600 {
        t!("auto_upgrade_deploy.hours", count = seconds / 3600).to_string()
    } else if seconds >= 60 {
        t!("auto_upgrade_deploy.minutes", count = seconds / 60).to_string()
    } else {
        t!("auto_upgrade_deploy.seconds", count = seconds).to_string()
    }
}

/// 检测是否为第一次部署
async fn is_first_deployment() -> bool {
    let docker_dir = std::path::Path::new("docker");
    let docker_compose_file = docker_dir.join("docker-compose.yml");
    let docker_data_dir = docker_dir.join("data/mysql");

    // 如果docker目录不存在，肯定是第一次部署
    if !docker_dir.exists() {
        return true;
    }

    // 🔧 关键修复：如果docker-compose.yml文件不存在，视为首次部署
    // 因为没有compose文件就无法管理现有服务
    if !docker_compose_file.exists() {
        info!("{}", t!("auto_upgrade_deploy.compose_not_found_first_deploy"));
        return true;
    }

    // 如果docker/data目录不存在，也是第一次部署
    if !docker_data_dir.exists() {
        return true;
    }

    false
}

/// 递归复制目录
#[allow(dead_code)]
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
    const MAX_ATTEMPTS: usize = sql::MAX_CLEANUP_ATTEMPTS;

    while attempts < MAX_ATTEMPTS {
        attempts += 1;

        // 首先尝试安全删除（保留upload目录）
        if let Err(e) = force_cleanup_directory(path).await {
            warn!(
                "{}",
                t!("auto_upgrade_deploy.safe_delete_failed",
                    attempts = attempts, max = MAX_ATTEMPTS, error = e.to_string())
            );

            if attempts >= MAX_ATTEMPTS {
                return Err(anyhow::anyhow!(t!(
                    "auto_upgrade_deploy.safe_delete_max_attempts",
                    max = MAX_ATTEMPTS, path = path.display(), error = e.to_string()
                )));
            }
        } else {
            info!("{}", t!("auto_upgrade_deploy.safe_delete_success", path = path.display()));
            return Ok(());
        }
    }

    unreachable!()
}

/// 强制清理目录内容（保留upload目录）
async fn force_cleanup_directory(path: &Path) -> Result<()> {
    info!(
        path = %path.display(),
        "{}", t!("auto_upgrade_deploy.trying_force_cleanup")
    );

    if !path.exists() {
        return Ok(());
    }

    // 收集清理失败的文件列表
    let mut failed_items: Vec<(PathBuf, String)> = Vec::new();
    let mut skipped_count = 0;
    let mut deleted_count = 0;

    // 递归遍历并删除文件
    match std::fs::read_dir(path) {
        Ok(entries) => {
            for entry in entries {
                if let Ok(entry) = entry {
                    let entry_path = entry.path();
                    let file_name = entry.file_name();
                    let file_name_str = file_name.to_string_lossy();

                    // 排除指定目录，不进行删除
                    if client_core::constants::docker::EXCLUDE_DIRS
                        .contains(&file_name_str.as_ref())
                        && entry_path.is_dir()
                    {
                        info!(
                            path = %entry_path.display(),
                            "{}", t!("auto_upgrade_deploy.skipping_protected_dir")
                        );
                        skipped_count += 1;
                        continue;
                    }

                    if entry_path.is_dir() {
                        // 递归删除子目录
                        if let Err(e) = Box::pin(force_cleanup_directory(&entry_path)).await {
                            warn!(
                                path = %entry_path.display(),
                                error = %e,
                                "{}", t!("auto_upgrade_deploy.delete_subdir_failed")
                            );
                            failed_items.push((entry_path.clone(), e.to_string()));
                        }

                        // 尝试删除空目录
                        if let Err(e) = std::fs::remove_dir(&entry_path) {
                            if e.kind() != std::io::ErrorKind::NotFound {
                                warn!(
                                    path = %entry_path.display(),
                                    error = %e,
                                    "{}", t!("auto_upgrade_deploy.delete_empty_dir_failed")
                                );
                                failed_items.push((entry_path, e.to_string()));
                            }
                        } else {
                            deleted_count += 1;
                        }
                    } else {
                        if let Err(e) = std::fs::remove_file(&entry_path) {
                            warn!(
                                path = %entry_path.display(),
                                error = %e,
                                "{}", t!("auto_upgrade_deploy.delete_file_failed")
                            );
                            failed_items.push((entry_path, e.to_string()));
                        } else {
                            deleted_count += 1;
                        }
                    }
                }
            }
        }
        Err(e) => {
            warn!(
                path = %path.display(),
                error = %e,
                "{}", t!("auto_upgrade_deploy.read_dir_failed")
            );
            return Err(e.into());
        }
    }

    // 报告清理结果
    if !failed_items.is_empty() {
        warn!(
            failed_count = failed_items.len(),
            deleted_count = deleted_count,
            skipped_count = skipped_count,
            "{}", t!("auto_upgrade_deploy.dir_cleanup_partial")
        );
        for (path, error) in failed_items.iter().take(5) {
            warn!("  - {}: {}", path.display(), error);
        }
        if failed_items.len() > 5 {
            warn!("{}", t!("auto_upgrade_deploy.more_failed_items", count = failed_items.len() - 5));
        }
    } else {
        info!(
            deleted_count = deleted_count,
            skipped_count = skipped_count,
            "{}", t!("auto_upgrade_deploy.dir_cleanup_success")
        );
    }

    Ok(())
}

/// 归档差异SQL文件
///
/// # 参数
/// - `diff_sql_path`: 差异SQL文件路径
/// - `status`: 状态标识 ("executed", "failed", "no_exec")
async fn archive_diff_sql_file(diff_sql_path: &Path, status: &str) -> Result<()> {
    if !diff_sql_path.is_file() {
        return Ok(());
    }

    let parent = diff_sql_path.parent().unwrap_or(Path::new("."));
    let timestamp = chrono::Local::now().format("%Y%m%d_%H%M%S");
    let new_name = format!("diff_sql_{}_{}.sql", status, timestamp);
    let new_path = parent.join(new_name);

    match fs::rename(diff_sql_path, &new_path) {
        Ok(_) => {
            let status_desc = match status {
                "executed" => t!("auto_upgrade_deploy.diff_sql_executed"),
                "failed" => t!("auto_upgrade_deploy.diff_sql_failed"),
                "no_exec" => t!("auto_upgrade_deploy.diff_sql_no_exec"),
                _ => t!("auto_upgrade_deploy.diff_sql_archived"),
            };
            info!("{}", t!("auto_upgrade_deploy.diff_sql_file_status", status = status_desc, path = new_path.display()));
            Ok(())
        }
        Err(e) => {
            warn!("{}", t!("auto_upgrade_deploy.archive_diff_sql_failed", error = e.to_string()));
            Ok(()) // 归档失败不影响主流程
        }
    }
}

/// 连接MySQL容器并执行差异SQL（Live Diff）
async fn execute_sql_diff_upgrade(config_file: &Option<PathBuf>) -> Result<()> {
    let temp_sql_dir = Path::new(sql::TEMP_SQL_DIR);
    let diff_sql_path = temp_sql_dir.join(sql::DIFF_SQL_FILE);
    let new_sql_path = temp_sql_dir.join(sql::NEW_SQL_FILE);

    // 如果 temp_sql 目录已存在，先归档到 history_sql
    if temp_sql_dir.exists() {
        let history_dir = Path::new("history_sql");
        if !history_dir.exists() {
            fs::create_dir_all(history_dir)?;
            info!("{}", t!("auto_upgrade_deploy.create_history_sql_dir", path = history_dir.display()));
        }

        // 生成带时间戳的目录名
        let timestamp = chrono::Local::now().format("%Y%m%d_%H%M%S");
        let archive_name = format!("temp_sql_{}", timestamp);
        let archive_path = history_dir.join(&archive_name);

        // 移动 temp_sql 目录到 history_sql（带降级处理）
        match fs::rename(&temp_sql_dir, &archive_path) {
            Ok(_) => {
                info!("{}", t!("auto_upgrade_deploy.archived_temp_sql", path = archive_path.display()));
            }
            Err(e) => {
                warn!("{}", t!("auto_upgrade_deploy.archive_temp_sql_failed", error = e.to_string()));
                // 降级：直接删除旧目录
                if let Err(e2) = fs::remove_dir_all(&temp_sql_dir) {
                    warn!("{}", t!("auto_upgrade_deploy.cleanup_temp_sql_failed", error = e2.to_string()));
                } else {
                    info!("{}", t!("auto_upgrade_deploy.cleaned_old_temp_sql"));
                }
            }
        }
    }

    // 创建临时SQL目录
    fs::create_dir_all(temp_sql_dir)?;
    info!("{}", t!("auto_upgrade_deploy.create_temp_sql_dir", path = temp_sql_dir.display()));

    // 复制新版本的SQL文件（使用常量路径）
    let current_sql_path = Path::new(sql::CURRENT_SQL_PATH);
    if current_sql_path.exists() {
        // 先删除目标文件（如果存在），确保复制操作成功
        if new_sql_path.exists() {
            fs::remove_file(&new_sql_path)?;
            info!("{}", t!("auto_upgrade_deploy.deleted_old_sql_file", path = new_sql_path.display()));
        }

        fs::copy(current_sql_path, &new_sql_path)
            .context(t!("auto_upgrade_deploy.copy_sql_failed",
                src = current_sql_path.display(), dst = new_sql_path.display()))?;

        // 验证文件复制成功
        if !new_sql_path.exists() {
            return Err(anyhow::anyhow!(t!(
                "auto_upgrade_deploy.sql_copy_not_found",
                dst = new_sql_path.display(), src = current_sql_path.display()
            )));
        }

        info!("{}", t!("auto_upgrade_deploy.copied_new_sql_file", path = new_sql_path.display()));
    } else {
        info!("{}", t!("auto_upgrade_deploy.no_sql_file_skip"));
        return Ok(());
    }

    // 读取模板SQL（严格失败策略）
    if !new_sql_path.exists() {
        return Err(anyhow::anyhow!(t!(
            "auto_upgrade_deploy.template_sql_not_found",
            path = new_sql_path.display()
        )));
    }
    let new_sql_content = fs::read_to_string(&new_sql_path)?;

    // 注意：parse_sql_tables 内部的 extract_create_table_statements_with_regex
    // 会自动处理 USE 语句的查找和提取，无需手动处理

    // 从App配置中动态获取MySQL端口并建立连接
    let compose_file = get_compose_file_path(&config_file);
    let env_file = client_core::constants::docker::get_env_file_path();
    let compose_file_str = compose_file
        .to_str()
        .ok_or_else(|| anyhow::anyhow!(t!("auto_upgrade_deploy.compose_path_to_string_failed")))?;
    let env_file_str = env_file
        .to_str()
        .ok_or_else(|| anyhow::anyhow!(t!("auto_upgrade_deploy.env_path_to_string_failed")))?;

    let config = MySqlConfig::for_container(Some(compose_file_str), Some(env_file_str))
        .await
        .context(t!("auto_upgrade_deploy.create_mysql_config_failed"))?;
    let executor = MySqlExecutor::new(config);

    info!("{}", t!("auto_upgrade_deploy.connecting_mysql"));
    if let Err(e) = executor.test_connection().await {
        error!(
            error = %e,
            port = sql::DEFAULT_MYSQL_CONTAINER_PORT,
            "{}", t!("auto_upgrade_deploy.db_connection_failed")
        );
        error!(
            "{}",
            t!("auto_upgrade_deploy.ensure_mysql_running",
                port = sql::DEFAULT_MYSQL_CONTAINER_PORT)
        );
        return Err(e.into());
    }

    // 基于在线架构与模板生成差异SQL
    info!("{}", t!("auto_upgrade_deploy.generating_sql_diff"));
    let diff_result = generate_live_schema_diff(&executor, &new_sql_content, "目标版本")
        .await
        .context(t!("auto_upgrade_deploy.generate_live_diff_failed"))?;

    info!(
        description = %diff_result.description,
        has_executable_sql = diff_result.has_executable_sql,
        has_warnings = diff_result.has_warnings,
        "{}", t!("auto_upgrade_deploy.diff_generation_complete")
    );

    // 保存从 MySQL 读取的原始 CREATE TABLE 语句到 init_mysql_old.sql
    if let Some(live_sql) = &diff_result.live_sql {
        let old_sql_path = temp_sql_dir.join(sql::OLD_SQL_FILE);
        fs::write(&old_sql_path, live_sql)?;
        info!("{}", t!("auto_upgrade_deploy.saved_live_schema_sql", path = old_sql_path.display()));
    }

    // 保存差异SQL文件（无论是否有可执行SQL，都保存以便查看）
    fs::write(&diff_sql_path, &diff_result.diff_sql).context(t!("auto_upgrade_deploy.save_diff_sql_failed"))?;
    info!("{}", t!("auto_upgrade_deploy.saved_diff_sql_file", path = diff_sql_path.display()));

    // 判断差异类型并输出相应提示
    if !diff_result.has_executable_sql {
        // 没有可执行SQL（可能有警告，也可能完全无差异）
        if diff_result.has_warnings {
            // 情况1：只有删除操作警告，没有可执行的新增/修改SQL
            info!("{}", t!("auto_upgrade_deploy.diff_only_deletions"));
            info!("{}", t!("auto_upgrade_deploy.deletions_need_manual"));
        } else {
            // 情况2：完全没有差异（既没有可执行SQL，也没有警告）
            info!("{}", t!("auto_upgrade_deploy.no_db_diff"));
        }

        // 统一归档差异SQL文件
        archive_diff_sql_file(&diff_sql_path, "no_exec").await?;
        return Ok(());
    }

    // 情况3：有可执行的SQL语句（可能同时包含警告）
    // 再次确认是否真的有可执行的SQL语句（排除全是注释的情况）
    let executable_lines: Vec<&str> = diff_result
        .diff_sql
        .lines()
        .filter(|line| {
            let trimmed = line.trim();
            !trimmed.is_empty() && !trimmed.starts_with("--") && !trimmed.starts_with("/*")
        })
        .collect();

    if executable_lines.is_empty() {
        // 虽然 has_executable_sql 为 true，但实际没有可执行的SQL（可能是逻辑错误）
        warn!("{}", t!("auto_upgrade_deploy.diff_no_executable_sql"));
        archive_diff_sql_file(&diff_sql_path, "no_exec").await?;
        return Ok(());
    }

    info!(
        sql_lines = executable_lines.len(),
        has_warnings = diff_result.has_warnings,
        "{}", t!("auto_upgrade_deploy.start_db_upgrade")
    );

    // 如果同时包含警告，提示用户注意（这是混合场景：既有新增/修改，又有删除）
    if diff_result.has_warnings {
        warn!("{}", t!("auto_upgrade_deploy.diff_mixed_warnings"));
        warn!("{}", t!("auto_upgrade_deploy.add_modify_will_execute"));
        warn!("{}", t!("auto_upgrade_deploy.deletions_skipped"));
        warn!("{}", t!("auto_upgrade_deploy.see_diff_file", path = diff_sql_path.display()));
    }

    info!(retry_count = sql::DEFAULT_RETRY_COUNT, "{}", t!("auto_upgrade_deploy.start_execute_diff_sql"));
    match executor
        .execute_diff_sql_with_retry(&diff_result.diff_sql, sql::DEFAULT_RETRY_COUNT)
        .await
    {
        Ok(results) => {
            info!(executed_statements = results.len(), "{}", t!("auto_upgrade_deploy.db_upgrade_success"));
            for result in results {
                info!("  {}", result);
            }

            // 归档已执行的差异SQL文件
            archive_diff_sql_file(&diff_sql_path, "executed").await?;
        }
        Err(e) => {
            error!(
                error = %e,
                "{}", t!("auto_upgrade_deploy.db_upgrade_failed")
            );
            // 归档执行失败的差异SQL文件
            archive_diff_sql_file(&diff_sql_path, "failed").await?;
            return Err(e);
        }
    }

    Ok(())
}

/// 自动修复关键脚本文件权限
async fn fix_script_permissions() -> Result<()> {
    info!("{}", t!("auto_upgrade_deploy.fixing_script_permissions"));

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
                                "{}",
                                t!("auto_upgrade_deploy.fixing_permission",
                                    path = path.display(), current = format!("{:o}", current_mode))
                            );

                            let new_permissions = std::fs::Permissions::from_mode(0o755);
                            if let Err(e) = std::fs::set_permissions(path, new_permissions) {
                                warn!("{}", t!("auto_upgrade_deploy.fix_permission_failed", path = path.display(), error = e.to_string()));
                            } else {
                                fixed_count += 1;
                                info!("{}", t!("auto_upgrade_deploy.permission_fixed", path = path.display()));
                            }
                        } else {
                            info!("{}", t!("auto_upgrade_deploy.permission_ok", path = path.display(), mode = format!("{:o}", current_mode)));
                        }
                    }

                    #[cfg(not(unix))]
                    {
                        info!("{}", t!("auto_upgrade_deploy.non_unix_skip_permission", path = path.display()));
                    }
                }
                Err(e) => {
                    warn!("{}", t!("auto_upgrade_deploy.read_metadata_failed", path = path.display(), error = e.to_string()));
                }
            }
        } else {
            info!("{}", t!("auto_upgrade_deploy.script_not_exists_skip", path = script_path));
        }
    }

    if total_count > 0 {
        info!(
            "{}",
            t!("auto_upgrade_deploy.permission_fix_complete",
                fixed = fixed_count, total = total_count)
        );
    } else {
        info!("{}", t!("auto_upgrade_deploy.no_scripts_to_fix"));
    }

    Ok(())
}

/// 检查并安装 nuwax-cli 更新（独立函数，用于早期检查）
/// 这个函数可以在数据库初始化之前调用，避免数据库锁冲突
pub async fn check_and_install_nuwax_cli_update_early() -> Result<()> {
    use crate::commands::check_update::{check_for_updates, install_release};

    info!("{}", t!("auto_upgrade_deploy.priority_cli_version_check"));

    // 检查更新
    let version_info = match check_for_updates().await {
        Ok(info) => {
            info!(
                "{}",
                t!("auto_upgrade_deploy.version_check_complete",
                    current = info.current_version, latest = info.latest_version)
            );
            info
        }
        Err(e) => {
            error!("{}", t!("auto_upgrade_deploy.version_check_failed", error = e.to_string()));
            return Err(e);
        }
    };

    // 如果有更新，进行安装
    if version_info.is_update_available {
        info!(
            "{}",
            t!("auto_upgrade_deploy.new_cli_version_found",
                current = version_info.current_version, latest = version_info.latest_version)
        );
        info!("{}", t!("auto_upgrade_deploy.start_auto_install"));

        match install_release(
            &version_info.download_url.unwrap_or_default(),
            &version_info.latest_version,
        )
        .await
        {
            Ok(_) => {
                info!("{}", t!("auto_upgrade_deploy.cli_update_success"));
                std::process::exit(0);
            }
            Err(e) => {
                error!("{}", t!("auto_upgrade_deploy.cli_auto_update_failed", error = e.to_string()));
                error!("{}", t!("auto_upgrade_deploy.manual_check_update_hint"));
                return Err(e);
            }
        }
    } else {
        info!("{}", t!("auto_upgrade_deploy.cli_already_latest"));
    }

    Ok(())
}
