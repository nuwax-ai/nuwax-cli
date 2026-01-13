use crate::app::CliApp;
use crate::cli::AutoUpgradeDeployCommand;
use crate::commands::{backup, docker_service, update};
use crate::docker_service::health_check::HealthChecker;
use crate::docker_utils;
use anyhow::{Context, Result};
use client_core::constants::{sql, timeout};
use client_core::container::DockerManager;
use client_core::mysql_executor::{MySqlConfig, MySqlExecutor};
use client_core::sql_diff::generate_live_schema_diff;
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
        .context("保存配置文件失败")?;

    info!(version = version, "✅ 配置文件版本号已更新并保存");
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

    // 注意：CLI版本检查已经在 main.rs 中优先处理，这里不再重复检查
    info!("✅ CLI 版本已完成预检查，开始执行升级部署流程");

    // 1. 获取最新版本信息并下载
    info!("📥 正在下载最新的Docker服务版本...");

    // 获取最新版本信息
    let latest_version = match app.api_client.get_enhanced_service_manifest().await {
        Ok(enhanced_service_manifest) => {
            let latest_version = enhanced_service_manifest.version.to_string();

            info!(
                current_version = %app.config.get_docker_versions(),
                target_version = %latest_version,
                "📋 检测到版本信息"
            );
            latest_version
        }
        Err(e) => {
            warn!(
                error = %e,
                fallback_version = %app.config.get_docker_versions(),
                "⚠️ 获取版本信息失败，使用配置版本"
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
        info!("🆕 检测到第一次部署，使用全新初始化");
    } else {
        info!("🔄 检测到升级部署，需要先停止服务");

        // 3. 🛑 停止服务并等待（使用统一的公共方法）
        docker_service::stop_docker_services_and_wait(
            app,
            config_file.clone(),
            project_name.clone(),
        )
        .await?;
    }

    // 5. 🔍 提前检查并创建挂载目录（重要：Windows Podman Desktop 需要）
    info!("🔍 检查并创建挂载目录...");

    let docker_manager = create_docker_manager(&config_file, &project_name)?;

    // 使用新的环境检测机制
    let runtime_env = docker_manager.get_runtime_environment();

    if runtime_env.needs_special_handling() {
        info!("⚠️ 检测到 Windows Podman Desktop 环境，提前创建挂载目录");
        info!("   环境信息: {}", runtime_env.summary());
        info!("   Podman Desktop 不会自动创建挂载目录，需要手动创建");

        if let Err(e) = docker_manager.ensure_host_volumes_exist().await {
            warn!("⚠️ 挂载目录检查/创建失败: {}", e);
            warn!("   继续执行，但可能会遇到容器启动失败");
        } else {
            info!("✅ 挂载目录检查完成");
        }
    } else {
        info!("ℹ️ 当前环境: {} (无需特殊处理)", runtime_env.summary());
    }

    // 6. 📦 解压新的Docker服务包（在服务停止后）
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
                    from_version = %app.config.get_docker_versions(),
                    to_version = %latest_version,
                    "📝 更新Docker服务版本"
                );

                // 更新版本号并保存到配置文件
                update_config_version(&mut app.config, &latest_version)?;
            } else {
                info!(
                    version = %latest_version,
                    "📝 版本号无需更新（已是最新版本）"
                );
            }

            // 📊 SQL差异将在服务启动后通过 Live Diff 生成和执行
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
#[allow(dead_code)]
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
        info!("📝 docker-compose.yml文件不存在，服务未运行");
        return Ok(false);
    }

    // 使用统一的 DockerManager 创建逻辑
    let docker_manager = create_docker_manager(config_file, project_name)?;
    let health_checker = HealthChecker::new(docker_manager);
    let report = health_checker.health_check().await?;

    // 检查是否有运行中的容器（而不是检查是否所有服务都健康）
    let running_count = report.get_running_count();

    if running_count > 0 {
        info!("🔍 发现 {} 个运行中的服务", running_count);
        Ok(true)
    } else {
        info!("🔍 没有发现运行中的服务");
        Ok(false)
    }
}

/// 格式化时间间隔为可读字符串
#[allow(dead_code)]
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
    let docker_data_dir = docker_dir.join("data/mysql");

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
    info!(
        path = %path.display(),
        "🧹 尝试强制清理目录内容"
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
                            "📁 跳过保护目录"
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
                                "📁 删除子目录失败"
                            );
                            failed_items.push((entry_path.clone(), e.to_string()));
                        }

                        // 尝试删除空目录
                        if let Err(e) = std::fs::remove_dir(&entry_path) {
                            if e.kind() != std::io::ErrorKind::NotFound {
                                warn!(
                                    path = %entry_path.display(),
                                    error = %e,
                                    "📁 删除空目录失败"
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
                                "📄 删除文件失败"
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
                "📂 读取目录内容失败"
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
            "⚠️ 目录清理完成，但有部分失败"
        );
        for (path, error) in failed_items.iter().take(5) {
            warn!("  - {}: {}", path.display(), error);
        }
        if failed_items.len() > 5 {
            warn!("  ... 还有 {} 个失败项", failed_items.len() - 5);
        }
    } else {
        info!(
            deleted_count = deleted_count,
            skipped_count = skipped_count,
            "✅ 目录清理成功"
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
                "executed" => "已执行",
                "failed" => "执行失败",
                "no_exec" => "未执行",
                _ => "已归档",
            };
            info!("📝 {} 差异SQL文件: {}", status_desc, new_path.display());
            Ok(())
        }
        Err(e) => {
            warn!("⚠️ 归档差异SQL文件失败: {}", e);
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
            info!("📁 创建历史SQL目录: {}", history_dir.display());
        }

        // 生成带时间戳的目录名
        let timestamp = chrono::Local::now().format("%Y%m%d_%H%M%S");
        let archive_name = format!("temp_sql_{}", timestamp);
        let archive_path = history_dir.join(&archive_name);

        // 移动 temp_sql 目录到 history_sql（带降级处理）
        match fs::rename(&temp_sql_dir, &archive_path) {
            Ok(_) => {
                info!("📦 已归档旧的temp_sql目录到: {}", archive_path.display());
            }
            Err(e) => {
                warn!("⚠️ 归档temp_sql目录失败: {}, 尝试直接清理", e);
                // 降级：直接删除旧目录
                if let Err(e2) = fs::remove_dir_all(&temp_sql_dir) {
                    warn!("⚠️ 清理temp_sql目录失败: {}, 继续执行", e2);
                } else {
                    info!("✅ 已清理旧的temp_sql目录");
                }
            }
        }
    }

    // 创建临时SQL目录
    fs::create_dir_all(temp_sql_dir)?;
    info!("📁 创建临时SQL目录: {}", temp_sql_dir.display());

    // 复制新版本的SQL文件（使用常量路径）
    let current_sql_path = Path::new(sql::CURRENT_SQL_PATH);
    if current_sql_path.exists() {
        // 先删除目标文件（如果存在），确保复制操作成功
        if new_sql_path.exists() {
            fs::remove_file(&new_sql_path)?;
            info!("🗑️ 已删除旧的SQL文件: {}", new_sql_path.display());
        }

        fs::copy(current_sql_path, &new_sql_path)
            .context(format!("复制SQL文件失败: {} -> {}", current_sql_path.display(), new_sql_path.display()))?;

        // 验证文件复制成功
        if !new_sql_path.exists() {
            return Err(anyhow::anyhow!(
                "SQL文件复制后不存在: {} (源文件: {})",
                new_sql_path.display(),
                current_sql_path.display()
            ));
        }

        info!("📄 已复制新版本SQL文件: {}", new_sql_path.display());
    } else {
        info!("📄 新版本没有SQL文件，跳过差异生成");
        return Ok(());
    }

    // 读取模板SQL（严格失败策略）
    if !new_sql_path.exists() {
        return Err(anyhow::anyhow!(
            "未找到模板SQL文件: {}",
            new_sql_path.display()
        ));
    }
    let new_sql_content = fs::read_to_string(&new_sql_path)?;

    // 注意：parse_sql_tables 内部的 extract_create_table_statements_with_regex
    // 会自动处理 USE 语句的查找和提取，无需手动处理

    // 从App配置中动态获取MySQL端口并建立连接
    let compose_file = get_compose_file_path(&config_file);
    let env_file = client_core::constants::docker::get_env_file_path();
    let compose_file_str = compose_file
        .to_str()
        .ok_or_else(|| anyhow::anyhow!("无法将 docker-compose.yml 路径转换为字符串"))?;
    let env_file_str = env_file
        .to_str()
        .ok_or_else(|| anyhow::anyhow!("无法将 .env 文件路径转换为字符串"))?;

    let config = MySqlConfig::for_container(Some(compose_file_str), Some(env_file_str))
        .await
        .context("创建 MySQL 配置失败")?;
    let executor = MySqlExecutor::new(config);

    info!("🔌 正在连接到MySQL数据库...");
    if let Err(e) = executor.test_connection().await {
        error!(
            error = %e,
            port = sql::DEFAULT_MYSQL_CONTAINER_PORT,
            "❌ 数据库连接失败"
        );
        error!(
            "🏃 请确保MySQL容器正在运行并且端口 {} 可访问",
            sql::DEFAULT_MYSQL_CONTAINER_PORT
        );
        return Err(e.into());
    }

    // 基于在线架构与模板生成差异SQL
    info!("📊 正在基于在线架构生成SQL差异...");
    let diff_result = generate_live_schema_diff(&executor, &new_sql_content, "目标版本")
        .await
        .context("生成在线差异SQL失败")?;

    info!(
        description = %diff_result.description,
        has_executable_sql = diff_result.has_executable_sql,
        has_warnings = diff_result.has_warnings,
        "📋 差异生成完成"
    );

    // 保存从 MySQL 读取的原始 CREATE TABLE 语句到 init_mysql_old.sql
    if let Some(live_sql) = &diff_result.live_sql {
        let old_sql_path = temp_sql_dir.join(sql::OLD_SQL_FILE);
        fs::write(&old_sql_path, live_sql)?;
        info!("📄 已保存在线架构SQL文件: {}", old_sql_path.display());
    }

    // 保存差异SQL文件（无论是否有可执行SQL，都保存以便查看）
    fs::write(&diff_sql_path, &diff_result.diff_sql).context("保存差异SQL文件失败")?;
    info!("📄 差异SQL文件已保存: {}", diff_sql_path.display());

    // 判断差异类型并输出相应提示
    if !diff_result.has_executable_sql {
        // 没有可执行SQL（可能有警告，也可能完全无差异）
        if diff_result.has_warnings {
            // 情况1：只有删除操作警告，没有可执行的新增/修改SQL
            info!("⚠️  检测到架构差异：仅包含删除操作（已跳过）");
            info!("💡 删除操作需要手动执行，请查看差异文件中的说明");
        } else {
            // 情况2：完全没有差异（既没有可执行SQL，也没有警告）
            info!("📄 数据库架构无差异，无需执行升级");
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
        warn!("⚠️  检测到差异但没有实际可执行的SQL语句");
        archive_diff_sql_file(&diff_sql_path, "no_exec").await?;
        return Ok(());
    }

    info!(
        sql_lines = executable_lines.len(),
        has_warnings = diff_result.has_warnings,
        "🔄 开始执行数据库升级"
    );

    // 如果同时包含警告，提示用户注意（这是混合场景：既有新增/修改，又有删除）
    if diff_result.has_warnings {
        warn!("⚠️  注意: 差异中同时包含可执行SQL和删除操作警告");
        warn!("   ✓ 新增/修改操作将正常执行");
        warn!("   ✗ 删除操作已跳过，需手动执行");
        warn!("   📄 详情请查看: {}", diff_sql_path.display());
    }

    info!(retry_count = sql::DEFAULT_RETRY_COUNT, "🚀 开始执行差异SQL");
    match executor
        .execute_diff_sql_with_retry(&diff_result.diff_sql, sql::DEFAULT_RETRY_COUNT)
        .await
    {
        Ok(results) => {
            info!(executed_statements = results.len(), "✅ 数据库升级成功");
            for result in results {
                info!("  {}", result);
            }

            // 归档已执行的差异SQL文件
            archive_diff_sql_file(&diff_sql_path, "executed").await?;
        }
        Err(e) => {
            error!(
                error = %e,
                "❌ 数据库升级失败"
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

/// 检查并安装 nuwax-cli 更新（独立函数，用于早期检查）
/// 这个函数可以在数据库初始化之前调用，避免数据库锁冲突
pub async fn check_and_install_nuwax_cli_update_early() -> Result<()> {
    use crate::commands::check_update::{check_for_updates, install_release};

    info!("🔍 优先检查 nuwax-cli 版本更新（在任何数据库初始化之前）...");

    // 检查更新
    let version_info = match check_for_updates().await {
        Ok(info) => {
            info!(
                "✅ 版本检查完成: 当前={}, 最新={}",
                info.current_version, info.latest_version
            );
            info
        }
        Err(e) => {
            error!("❌ 检查更新失败: {}", e);
            return Err(e);
        }
    };

    // 如果有更新，进行安装
    if version_info.is_update_available {
        info!(
            "🚀 发现 nuwax-cli 新版本: {} -> {}",
            version_info.current_version, version_info.latest_version
        );
        info!("📥 开始自动安装更新...");

        match install_release(
            &version_info.download_url.unwrap_or_default(),
            &version_info.latest_version,
        )
        .await
        {
            Ok(_) => {
                info!("✅ nuwax-cli 更新成功！程序将重启以使用新版本");
                std::process::exit(0);
            }
            Err(e) => {
                error!("❌ nuwax-cli 自动更新失败: {}", e);
                error!("请检查网络连接或手动运行: nuwax-cli check-update install");
                return Err(e);
            }
        }
    } else {
        info!("✅ nuwax-cli 已是最新版本，无需更新");
    }

    Ok(())
}
