use crate::app::CliApp;
use crate::cli::AutoUpgradeDeployCommand;
use crate::commands::{backup, docker_service, update};
use crate::docker_service::health_check::HealthChecker;
use anyhow::{Context, Result};
use client_core::constants::sql;
use client_core::container::DockerManager;
use client_core::mysql_executor::{MySqlConfig, MySqlExecutor};
use client_core::sql_diff::generate_live_schema_diff;
use client_core::sql_diff::parse_sql_tables_strict;
use client_core::upgrade_strategy::UpgradeStrategy;
use client_core::utils::archive::{self, ArchiveFormat};
use rust_i18n::t;
use std::fs;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use tokio::time::sleep;
use tracing::{debug, error, info, warn};

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

fn validate_target_sql(sql_content: &str) -> Result<()> {
    let tables =
        parse_sql_tables_strict(sql_content).context("Failed to parse target MySQL DDL")?;
    if tables.is_empty() {
        return Err(anyhow::anyhow!(
            "Target init_mysql.sql contains no CREATE TABLE statements"
        ));
    }
    Ok(())
}

/// 离线完整包在停止旧服务前必须能提供完整目标 DDL。
fn validate_offline_archive_sql(archive_path: &Path) -> Result<()> {
    fn is_target(path: &Path) -> bool {
        let normalized = path.strip_prefix("docker").unwrap_or(path);
        normalized == Path::new("config/init_mysql.sql")
    }

    let mut target_sql = None;
    match archive::detect_format_by_magic(archive_path)? {
        ArchiveFormat::Zip => {
            let file = fs::File::open(archive_path)?;
            let mut zip = zip::ZipArchive::new(file)?;
            for idx in 0..zip.len() {
                let mut entry = zip.by_index(idx)?;
                let path = entry
                    .enclosed_name()
                    .ok_or_else(|| anyhow::anyhow!("Unsafe archive entry: {}", entry.name()))?;
                if is_target(&path) {
                    if target_sql.is_some() {
                        return Err(anyhow::anyhow!(
                            "Duplicate init_mysql.sql in offline archive"
                        ));
                    }
                    let mut content = String::new();
                    entry.read_to_string(&mut content)?;
                    target_sql = Some(content);
                }
            }
        }
        ArchiveFormat::TarGz => {
            let file = fs::File::open(archive_path)?;
            let decoder = flate2::read::GzDecoder::new(file);
            let mut tar = tar::Archive::new(decoder);
            for entry in tar.entries()? {
                let mut entry = entry?;
                if is_target(&entry.path()?) {
                    if target_sql.is_some() {
                        return Err(anyhow::anyhow!(
                            "Duplicate init_mysql.sql in offline archive"
                        ));
                    }
                    let mut content = String::new();
                    entry.read_to_string(&mut content)?;
                    target_sql = Some(content);
                }
            }
        }
    }

    let content = target_sql
        .ok_or_else(|| anyhow::anyhow!("Offline archive is missing config/init_mysql.sql"))?;
    validate_target_sql(&content)
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
    config_path: &Path,
    version: &str,
) -> Result<()> {
    let config_mut = Arc::make_mut(config);
    config_mut.write_docker_versions(version.to_string());

    config_mut
        .save_to_file(config_path)
        .context(t!("auto_upgrade_deploy.save_config_failed"))?;

    info!(
        version = version,
        "✅ Updated and saved the version in configuration file"
    );
    Ok(())
}

async fn wait_for_mysql_connection(compose_path: &Path, env_path: &Path) -> Result<MySqlExecutor> {
    let compose = compose_path
        .to_str()
        .ok_or_else(|| anyhow::anyhow!("Compose path is not valid UTF-8"))?;
    let env = env_path
        .to_str()
        .ok_or_else(|| anyhow::anyhow!("Compose env path is not valid UTF-8"))?;
    let config = MySqlConfig::for_container(Some(compose), Some(env))
        .await
        .context("Failed to resolve MySQL connection from Compose")?;
    let executor = MySqlExecutor::new(config);
    let timeout = Duration::from_secs(sql::MYSQL_READY_TIMEOUT);
    let deadline = tokio::time::Instant::now() + timeout;
    let mut last_error = "no connection attempt completed".to_string();
    loop {
        let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
        if remaining.is_zero() {
            return Err(anyhow::anyhow!(
                "MySQL was not connectable within {}s: {last_error}",
                timeout.as_secs()
            ));
        }
        let attempt_timeout = remaining.min(Duration::from_secs(10));
        match tokio::time::timeout(attempt_timeout, executor.test_connection()).await {
            Ok(Ok(())) => return Ok(executor),
            Ok(Err(error)) => last_error = error.to_string(),
            Err(_) => last_error = "connection attempt timed out".to_string(),
        }
        debug!(error = %last_error, "Waiting for MySQL to accept SQL connections");
        let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
        sleep(remaining.min(Duration::from_secs(2))).await;
    }
}

async fn run_staged_deployment(
    app: &mut CliApp,
    frontend_port: Option<u16>,
    config_file: Option<PathBuf>,
    project_name: Option<String>,
    is_first_deployment: bool,
    target_version: &str,
) -> Result<()> {
    let target_sql_path = Path::new(sql::CURRENT_SQL_PATH);
    let target_sql = fs::read_to_string(target_sql_path)
        .with_context(|| format!("Target MySQL DDL is missing: {}", target_sql_path.display()))?;
    validate_target_sql(&target_sql)?;

    let docker_manager = create_docker_manager(&config_file, &project_name)?;
    docker_manager.invalidate_compose_config_cache();
    if !is_first_deployment {
        // stop_docker_services_and_wait 在没有运行容器时可能跳过 down；这里清理旧的已停止容器。
        docker_manager
            .stop_services()
            .await
            .context("Failed to remove stopped containers from the previous deployment")?;
    }
    docker_service::prepare_docker_services(app, frontend_port, config_file.clone(), project_name)
        .await?;
    // prepare_docker_services may update .env (frontend port).
    docker_manager.invalidate_compose_config_cache();

    let mysql_stage = docker_manager.get_service_dependency_closure("mysql")?;
    let all_services = docker_manager.get_compose_service_names().await?;
    info!("▶️ Starting MySQL and its Compose dependencies...");
    docker_manager
        .up_services(&["mysql".to_string()], false)
        .await?;
    let mysql_id_before = docker_manager.get_service_container_id("mysql").await?;
    let executor = wait_for_mysql_connection(
        docker_manager.get_compose_file(),
        docker_manager.get_env_file(),
    )
    .await?;

    if is_first_deployment {
        info!("🆕 MySQL initialization completed; Live Diff is not required");
    } else {
        info!(
            "🔄 MySQL is connectable; applying live schema differences before applications start"
        );
        execute_sql_diff_upgrade(&executor).await?;
    }

    let startup_layers = docker_manager.get_compose_startup_layers(&mysql_stage)?;
    for layer in startup_layers {
        info!(services = %layer.join(", "), "▶️ Starting next Compose dependency layer...");
        docker_manager
            .up_services_without_dependencies(&layer, true)
            .await?;
        docker_manager
            .wait_for_compose_services_ready(
                &layer,
                Duration::from_secs(client_core::constants::timeout::HEALTH_CHECK_TIMEOUT),
            )
            .await?;
    }

    let mysql_id_after = docker_manager.get_service_container_id("mysql").await?;
    if mysql_id_before != mysql_id_after {
        return Err(anyhow::anyhow!(
            "MySQL container changed while starting application services; refusing to mark deployment successful"
        ));
    }

    let mut all_service_names: Vec<String> = all_services.into_iter().collect();
    all_service_names.sort();
    docker_manager
        .wait_for_compose_services_ready(
            &all_service_names,
            Duration::from_secs(client_core::constants::timeout::HEALTH_CHECK_TIMEOUT),
        )
        .await?;
    let health_checker = HealthChecker::new(docker_manager);
    health_checker
        .wait_for_services_ready(Duration::from_secs(
            client_core::constants::timeout::HEALTH_CHECK_INTERVAL,
        ))
        .await
        .context("Docker services did not become healthy after MySQL migration")?;

    let app_config_path = app.config_path.clone();
    update_config_version(&mut app.config, &app_config_path, target_version)?;
    info!("✅ Deployment completed after MySQL migration and service health checks");
    Ok(())
}

fn create_docker_backup_path() -> PathBuf {
    let timestamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs())
        .unwrap_or(0);
    PathBuf::from(format!(
        ".docker.offline-backup-{}-{timestamp}",
        std::process::id()
    ))
}

fn restore_docker_backup(backup_dir: &Path, docker_dir: &Path) -> Result<()> {
    if docker_dir.exists() {
        fs::remove_dir_all(docker_dir).context("Failed to remove partial docker directory")?;
    }

    if backup_dir.exists() {
        fs::rename(backup_dir, docker_dir)
            .context("Failed to restore previous docker directory")?;
    }

    Ok(())
}

fn restore_preserved_docker_dirs(backup_dir: &Path, docker_dir: &Path) -> Result<()> {
    if !backup_dir.exists() {
        return Ok(());
    }

    fs::create_dir_all(docker_dir).context("Failed to create docker directory")?;

    for dir_name in client_core::constants::docker::EXCLUDE_DIRS {
        let old_path = backup_dir.join(dir_name);
        if !old_path.exists() {
            continue;
        }

        let new_path = docker_dir.join(dir_name);
        if dir_name == ".env" && old_path.is_file() && new_path.is_file() {
            merge_preserved_env_file(&old_path, &new_path)?;
            info!("🛡️ Preserved existing .env values and added missing package defaults");
            continue;
        }

        if new_path.exists() {
            if new_path.is_dir() {
                fs::remove_dir_all(&new_path).with_context(|| {
                    format!("Failed to replace preserved directory: {dir_name}")
                })?;
            } else {
                fs::remove_file(&new_path)
                    .with_context(|| format!("Failed to replace preserved file: {dir_name}"))?;
            }
        }

        fs::rename(&old_path, &new_path)
            .with_context(|| format!("Failed to restore preserved directory: {dir_name}"))?;
        info!("🛡️ Restored preserved docker directory: {dir_name}");
    }

    Ok(())
}

fn merge_preserved_env_file(preserved_path: &Path, package_path: &Path) -> Result<()> {
    let preserved = fs::read_to_string(preserved_path).with_context(|| {
        format!(
            "Failed to read existing environment file: {}",
            preserved_path.display()
        )
    })?;
    let package = fs::read_to_string(package_path).with_context(|| {
        format!(
            "Failed to read package environment file: {}",
            package_path.display()
        )
    })?;
    let merged = merge_env_contents(&preserved, &package);
    let permissions = fs::metadata(preserved_path)
        .with_context(|| {
            format!(
                "Failed to inspect existing environment file: {}",
                preserved_path.display()
            )
        })?
        .permissions();
    let mut temp_file = tempfile::NamedTempFile::new_in(
        package_path
            .parent()
            .context("Package environment file has no parent directory")?,
    )
    .context("Failed to create temporary merged environment file")?;
    temp_file
        .write_all(merged.as_bytes())
        .context("Failed to write merged environment file")?;
    temp_file
        .as_file()
        .sync_all()
        .context("Failed to flush merged environment file")?;
    fs::set_permissions(temp_file.path(), permissions)
        .context("Failed to preserve environment file permissions")?;

    fs::remove_file(package_path).with_context(|| {
        format!(
            "Failed to replace package environment file: {}",
            package_path.display()
        )
    })?;
    temp_file
        .persist(package_path)
        .map_err(|error| error.error)
        .with_context(|| {
            format!(
                "Failed to install merged environment file: {}",
                package_path.display()
            )
        })?;
    fs::remove_file(preserved_path).with_context(|| {
        format!(
            "Failed to remove backed-up environment file: {}",
            preserved_path.display()
        )
    })?;
    Ok(())
}

fn merge_env_contents(preserved: &str, package: &str) -> String {
    let mut keys = preserved
        .lines()
        .filter_map(env_assignment_key)
        .map(str::to_owned)
        .collect::<std::collections::HashSet<_>>();
    let mut merged = preserved.to_owned();

    for line in package.lines() {
        let Some(key) = env_assignment_key(line) else {
            continue;
        };
        if keys.insert(key.to_owned()) {
            if !merged.is_empty() && !merged.ends_with('\n') {
                merged.push('\n');
            }
            merged.push_str(line);
            merged.push('\n');
        }
    }

    merged
}

fn env_assignment_key(line: &str) -> Option<&str> {
    let line = line.trim();
    if line.is_empty() || line.starts_with('#') {
        return None;
    }
    let assignment = line.strip_prefix("export ").unwrap_or(line);
    let (key, _) = assignment.split_once('=')?;
    let key = key.trim();
    let mut characters = key.chars();
    let first = characters.next()?;
    if !(first == '_' || first.is_ascii_alphabetic())
        || !characters.all(|character| character == '_' || character.is_ascii_alphanumeric())
    {
        return None;
    }
    Some(key)
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
            info!("🚀 Starting auto-upgrade deployment...");
            run_auto_upgrade_deploy(app, port, config, project).await
        }
        AutoUpgradeDeployCommand::Status => {
            info!("Show auto-upgrade deployment status");
            show_status(app).await
        }
        AutoUpgradeDeployCommand::OfflineDeploy {
            archive,
            version,
            port,
            config,
            project,
        } => {
            info!("📦 Starting offline deployment...");
            run_offline_deploy(app, archive, version, port, config, project).await
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
    info!("🚀 Starting auto-upgrade deployment...");

    // 如果指定了端口，显示端口信息
    if let Some(port) = frontend_port {
        info!("🔌 Custom frontend port: {port}", port = port);
    }

    // 如果指定了配置文件，显示配置文件信息
    if let Some(config_path) = &config_file {
        info!(
            "📄 Custom docker-compose configuration file: {path}",
            path = config_path.display()
        );
    }

    // 注意：CLI版本检查已经在 main.rs 中优先处理，这里不再重复检查
    info!("✅ CLI version pre-check complete, starting upgrade deployment");

    // 1. 获取最新版本信息并下载
    info!("📥 Downloading the latest Docker service version...");

    // 下载策略只读取一次 manifest；部署版本从同一策略中取得，避免版本与包不一致。
    let upgrade_args = crate::cli::UpgradeArgs {
        force: false,
        check: false,
    };
    let upgrade_strategy = update::run_upgrade(app, upgrade_args).await?;
    let target_version = match &upgrade_strategy {
        UpgradeStrategy::FullUpgrade { target_version, .. }
        | UpgradeStrategy::PatchUpgrade { target_version, .. }
        | UpgradeStrategy::NoUpgrade { target_version } => target_version.to_string(),
    };

    // 2. 🔍 检查部署类型：第一次部署 vs 升级部署
    let is_first_deployment = is_first_deployment().await;

    if is_first_deployment {
        info!("🆕 First deployment detected, using fresh initialization");
    } else {
        info!("🔄 Upgrade deployment detected, services will be stopped first");

        // 3. 🛑 停止服务并等待（使用统一的公共方法）
        let stopped = docker_service::stop_docker_services_and_wait(
            app,
            config_file.clone(),
            project_name.clone(),
        )
        .await?;
        if !stopped {
            return Err(anyhow::anyhow!(
                "Timed out stopping old services; refusing to upgrade MySQL"
            ));
        }
    }

    // 5. 🔍 提前检查并创建挂载目录（重要：Windows Podman Desktop 需要）
    info!("🔍 Checking and creating mount directories...");

    let docker_manager = create_docker_manager(&config_file, &project_name)?;

    // 使用新的环境检测机制
    let runtime_env = docker_manager.get_runtime_environment();

    if runtime_env.needs_special_handling() {
        info!("⚠️ Windows Podman Desktop detected, pre-creating mount directories");
        info!("Environment info: {env}", env = runtime_env.summary());
        info!("Podman Desktop does not auto-create mount directories; manual creation is required");

        if let Err(e) = docker_manager.ensure_host_volumes_exist().await {
            warn!(
                "⚠️ Mount directory check/creation failed: {error}",
                error = e.to_string()
            );
            warn!("Continuing execution, but container startup may fail");
        } else {
            info!("✅ Mount directory check complete");
        }
    } else {
        info!(
            "ℹ️ Current environment: {env} (no special handling needed)",
            env = runtime_env.summary()
        );
    }

    // 6. 📦 解压新的Docker服务包（在服务停止后）
    info!("📦 Extracting Docker service package...");

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
                        "✅ Cleaned files/directories successfully: {files}",
                        files = &remove_file_or_dir
                            .iter()
                            .map(|p| p.to_string_lossy())
                            .collect::<Vec<_>>()
                            .join(", ")
                    ),
                    Err(e) => warn!(
                        "⚠️ Failed to clean files/directories: {error}; continuing extraction",
                        error = e.to_string()
                    ),
                }
            }
            UpgradeStrategy::FullUpgrade { .. } => {
                // 全量升级逻辑
                info!("🧹 Cleaning existing docker directory to avoid file conflicts...");
                match safe_remove_docker_directory(docker_dir).await {
                    Ok(_) => info!("✅ Docker directory cleanup completed"),
                    Err(e) => {
                        warn!(
                            "⚠️ Failed to clean docker directory: {error}; continuing extraction",
                            error = e.to_string()
                        );
                        return Err(anyhow::anyhow!(t!(
                            "auto_upgrade_deploy.clean_docker_dir_error",
                            error = e.to_string()
                        )));
                    }
                }
            }
            UpgradeStrategy::NoUpgrade { .. } => {
                //do nothing
                info!("Version unchanged, no upgrade required")
            }
        }
    }

    // 解压新的Docker服务包（使用最新版本）
    match docker_service::extract_docker_service_with_upgrade_strategy(app, upgrade_strategy).await
    {
        Ok(_) => {
            info!("✅ Docker service package extracted");

            // 🔧 自动修复关键脚本文件权限
            fix_script_permissions().await?;

            // 版本号在 MySQL 迁移及全部服务健康后提交。
        }
        Err(e) => {
            error!(
                "❌ Failed to extract Docker service package: {error}",
                error = e.to_string()
            );
            return Err(e);
        }
    }

    run_staged_deployment(
        app,
        frontend_port,
        config_file,
        project_name,
        is_first_deployment,
        &target_version,
    )
    .await
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
            error!("Unsupported time unit: {unit}", unit = unit);
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

    info!("⏰ Delayed auto-upgrade deployment has been scheduled");
    info!("Task ID: {id}", id = task.task_id);
    info!("Delay: {time} {unit}", time = time, unit = unit);
    println!(
        "   {}",
        t!(
            "auto_upgrade_deploy.estimated_exec_time",
            duration = format_duration(delay_duration)
        )
    );
    info!(
        "Planned execution time: {time}",
        time = scheduled_at.format("%Y-%m-%d %H:%M:%S UTC")
    );

    info!(
        "Scheduled delayed auto-upgrade deployment: {time} {unit}, task ID: {task_id}",
        time = time,
        unit = unit,
        task_id = task.task_id
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
    info!("⏳ Waiting...");

    // 这里可以优化为后台任务，避免阻塞
    sleep(delay_duration).await;

    info!("🔔 Delay reached; starting auto-upgrade deployment");
    info!(
        "Delay reached; auto-upgrade deployment starting, task ID: {task_id}",
        task_id = task.task_id
    );

    // 执行自动升级部署
    match run_auto_upgrade_deploy(app, None, None, None).await {
        Ok(_) => {
            let config_manager =
                client_core::config_manager::ConfigManager::new_with_database(app.database.clone());
            config_manager
                .update_upgrade_task_status(&task.task_id, "completed", Some(100), None)
                .await?;
            info!("✅ Delayed upgrade deployment task completed");
        }
        Err(e) => {
            let config_manager =
                client_core::config_manager::ConfigManager::new_with_database(app.database.clone());
            config_manager
                .update_upgrade_task_status(&task.task_id, "failed", None, Some(&e.to_string()))
                .await?;
            error!(
                "Delayed upgrade deployment task failed: {error}",
                error = e.to_string()
            );
            return Err(e);
        }
    }

    Ok(())
}

/// 显示自动升级部署状态
pub async fn show_status(app: &mut CliApp) -> Result<()> {
    let config_manager =
        client_core::config_manager::ConfigManager::new_with_database(app.database.clone());

    info!("📊 Auto-upgrade deployment status:");
    info!("Feature status: implemented");
    info!("Process: download latest version -> smart backup -> deploy services -> start services");

    // 显示待执行的升级任务
    match config_manager.get_pending_upgrade_tasks().await {
        Ok(tasks) => {
            if tasks.is_empty() {
                info!("📋 Upgrade tasks: no pending upgrade tasks");
            } else {
                info!("📋 Pending upgrade tasks:");
                for task in tasks {
                    info!("- Task ID: {id}", id = task.task_id);
                    info!("Name: {name}", name = task.task_name);
                    info!("Type: {type_name}", type_name = task.upgrade_type);
                    info!("Status: {status}", status = task.status);
                    info!(
                        "Planned execution time: {time}",
                        time = task.schedule_time.format("%Y-%m-%d %H:%M:%S UTC")
                    );
                    if let Some(target_version) = &task.target_version {
                        info!("Target version: {version}", version = target_version);
                    }
                    if let Some(progress) = task.progress {
                        info!("Progress: {progress}%", progress = progress);
                    }
                    if let Some(error) = &task.error_message {
                        warn!("Error message: {error}", error = error);
                    }
                }
            }
        }
        Err(e) => {
            warn!(
                "⚠️ Failed to obtain upgrade task information: {error}",
                error = e.to_string()
            );
            info!("Note: Task query capability is limited in this version");
        }
    }

    // 显示当前Docker服务状态
    info!("🐳 Current Docker service status:");
    docker_service::check_docker_services_status(app).await?;

    // 显示最近的备份
    info!("📝 Recent backups:");
    backup::run_list_backups(app).await?;

    Ok(())
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
        info!("📝 docker-compose.yml not found; treated as first deployment");
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
                "⚠️ Safe directory deletion failed (attempt {attempts}/{max}): {error}",
                attempts = attempts,
                max = MAX_ATTEMPTS,
                error = e.to_string()
            );

            if attempts >= MAX_ATTEMPTS {
                return Err(anyhow::anyhow!(t!(
                    "auto_upgrade_deploy.safe_delete_max_attempts",
                    max = MAX_ATTEMPTS,
                    path = path.display(),
                    error = e.to_string()
                )));
            }
        } else {
            info!("✅ Directory safely deleted: {path}", path = path.display());
            return Ok(());
        }
    }

    unreachable!()
}

/// 强制清理目录内容（保留upload目录）
async fn force_cleanup_directory(path: &Path) -> Result<()> {
    info!(path = %path.display(), "🧹 Attempting forced cleanup of directory contents");

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
            for entry in entries.flatten() {
                let entry_path = entry.path();
                let file_name = entry.file_name();
                let file_name_str = file_name.to_string_lossy();

                // 排除指定目录，不进行删除
                if client_core::constants::docker::EXCLUDE_DIRS.contains(&file_name_str.as_ref())
                    && entry_path.is_dir()
                {
                    info!(path = %entry_path.display(), "📁 Skip protected directory");
                    skipped_count += 1;
                    continue;
                }

                if entry_path.is_dir() {
                    // 递归删除子目录
                    if let Err(e) = Box::pin(force_cleanup_directory(&entry_path)).await {
                        warn!(path = %entry_path.display(), error = %e, "📁 Failed to delete subdirectory");
                        failed_items.push((entry_path.clone(), e.to_string()));
                    }

                    // 尝试删除空目录
                    if let Err(e) = std::fs::remove_dir(&entry_path) {
                        if e.kind() != std::io::ErrorKind::NotFound {
                            warn!(path = %entry_path.display(), error = %e, "📁 Failed to delete empty directory");
                            failed_items.push((entry_path, e.to_string()));
                        }
                    } else {
                        deleted_count += 1;
                    }
                } else if let Err(e) = std::fs::remove_file(&entry_path) {
                    warn!(path = %entry_path.display(), error = %e, "📄 Failed to delete file");
                    failed_items.push((entry_path, e.to_string()));
                } else {
                    deleted_count += 1;
                }
            }
        }
        Err(e) => {
            warn!(path = %path.display(), error = %e, "📂 Failed to read directory content");
            return Err(e.into());
        }
    }

    // 报告清理结果
    if !failed_items.is_empty() {
        warn!(
            failed_count = failed_items.len(),
            deleted_count = deleted_count,
            skipped_count = skipped_count,
            "⚠️ Directory cleanup completed, but some parts failed"
        );
        for (path, error) in failed_items.iter().take(5) {
            warn!("  - {}: {}", path.display(), error);
        }
        if failed_items.len() > 5 {
            warn!(
                "  ... and {count} more failed items",
                count = failed_items.len() - 5
            );
        }
    } else {
        info!(
            deleted_count = deleted_count,
            skipped_count = skipped_count,
            "✅ Directory cleanup successful"
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
    let timestamp = chrono::Local::now().format("%Y%m%d_%H%M%S_%f");
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
            info!(
                "📝 {status} diff SQL file: {path}",
                status = status_desc,
                path = new_path.display()
            );
            Ok(())
        }
        Err(e) => {
            warn!(
                "⚠️ Failed to archive diff SQL file: {error}",
                error = e.to_string()
            );
            Ok(()) // 归档失败不影响主流程
        }
    }
}

/// 连接MySQL容器并执行差异SQL（Live Diff）
async fn execute_sql_diff_upgrade(executor: &MySqlExecutor) -> Result<()> {
    let temp_sql_dir = Path::new(sql::TEMP_SQL_DIR);
    let diff_sql_path = temp_sql_dir.join(sql::DIFF_SQL_FILE);
    let new_sql_path = temp_sql_dir.join(sql::NEW_SQL_FILE);

    // 如果 temp_sql 目录已存在，先归档到 history_sql
    if temp_sql_dir.exists() {
        let history_dir = Path::new("history_sql");
        if !history_dir.exists() {
            fs::create_dir_all(history_dir)?;
            info!(
                "📁 Creating history SQL directory: {path}",
                path = history_dir.display()
            );
        }

        // 生成带时间戳的目录名
        let timestamp = chrono::Local::now().format("%Y%m%d_%H%M%S");
        let archive_name = format!("temp_sql_{}", timestamp);
        let archive_path = history_dir.join(&archive_name);

        // 移动 temp_sql 目录到 history_sql（带降级处理）
        match fs::rename(temp_sql_dir, &archive_path) {
            Ok(_) => {
                info!(
                    "📦 Archived old temp_sql directory to: {path}",
                    path = archive_path.display()
                );
            }
            Err(e) => {
                warn!(
                    "⚠️ Failed to archive temp_sql directory: {error}, try to clean it directly",
                    error = e.to_string()
                );
                // 降级：直接删除旧目录
                if let Err(e2) = fs::remove_dir_all(temp_sql_dir) {
                    warn!(
                        "⚠️ Failed to clean temp_sql directory: {error}, continue execution",
                        error = e2.to_string()
                    );
                } else {
                    info!("✅ The old temp_sql directory has been cleaned");
                }
            }
        }
    }

    // 创建临时SQL目录
    fs::create_dir_all(temp_sql_dir)?;
    info!(
        "📁 Creating temp SQL directory: {path}",
        path = temp_sql_dir.display()
    );

    // 复制新版本的SQL文件（使用常量路径）
    let current_sql_path = Path::new(sql::CURRENT_SQL_PATH);
    if current_sql_path.exists() {
        // 先删除目标文件（如果存在），确保复制操作成功
        if new_sql_path.exists() {
            fs::remove_file(&new_sql_path)?;
            info!(
                "🗑️Old SQL file deleted: {path}",
                path = new_sql_path.display()
            );
        }

        fs::copy(current_sql_path, &new_sql_path).context(t!(
            "auto_upgrade_deploy.copy_sql_failed",
            src = current_sql_path.display(),
            dst = new_sql_path.display()
        ))?;

        // 验证文件复制成功
        if !new_sql_path.exists() {
            return Err(anyhow::anyhow!(t!(
                "auto_upgrade_deploy.sql_copy_not_found",
                dst = new_sql_path.display(),
                src = current_sql_path.display()
            )));
        }

        info!(
            "📄 Copied new version SQL file: {path}",
            path = new_sql_path.display()
        );
    } else {
        return Err(anyhow::anyhow!(
            "Target MySQL DDL is missing: {}",
            current_sql_path.display()
        ));
    }

    // 读取模板SQL（严格失败策略）
    if !new_sql_path.exists() {
        return Err(anyhow::anyhow!(t!(
            "auto_upgrade_deploy.template_sql_not_found",
            path = new_sql_path.display()
        )));
    }
    let new_sql_content = fs::read_to_string(&new_sql_path)?;
    validate_target_sql(&new_sql_content)?;

    // 注意：parse_sql_tables 内部的 extract_create_table_statements_with_regex
    // 会自动处理 USE 语句的查找和提取，无需手动处理

    // 基于在线架构与模板生成差异SQL
    info!("📊 Generating SQL differences based on online schema...");
    let diff_result = generate_live_schema_diff(executor, &new_sql_content, "target version")
        .await
        .context(t!("auto_upgrade_deploy.generate_live_diff_failed"))?;

    info!(description = %diff_result.description, has_executable_sql = diff_result.has_executable_sql, has_warnings = diff_result.has_warnings, "📋 Difference generation completed");

    // 保存从 MySQL 读取的原始 CREATE TABLE 语句到 init_mysql_old.sql
    if let Some(live_sql) = &diff_result.live_sql {
        let old_sql_path = temp_sql_dir.join(sql::OLD_SQL_FILE);
        fs::write(&old_sql_path, live_sql)?;
        info!(
            "📄 Saved online schema SQL file: {path}",
            path = old_sql_path.display()
        );
    }

    // 保存差异SQL文件（无论是否有可执行SQL，都保存以便查看）
    fs::write(&diff_sql_path, &diff_result.diff_sql)
        .context(t!("auto_upgrade_deploy.save_diff_sql_failed"))?;
    info!(
        "📄 Diff SQL file saved: {path}",
        path = diff_sql_path.display()
    );

    // 判断差异类型并输出相应提示
    if !diff_result.has_executable_sql {
        // 没有可执行SQL（可能有警告，也可能完全无差异）
        if diff_result.has_warnings {
            // 只有人工处理提示，没有可执行的新增/修改 SQL。
            info!("⚠️ Schema difference detected: only manual changes (skipped)");
            info!("💡 Manual changes are listed in the diff file");
        } else {
            // 情况2：完全没有差异（既没有可执行SQL，也没有警告）
            info!("📄 No database schema differences; no upgrade required");
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
        warn!("⚠️ Difference detected but no executable SQL statements");
        archive_diff_sql_file(&diff_sql_path, "no_exec").await?;
        return Ok(());
    }

    info!(
        sql_lines = executable_lines.len(),
        has_warnings = diff_result.has_warnings,
        "🔄 Start database upgrade"
    );

    // 如果同时包含警告，提示用户注意（这是混合场景：既有新增/修改，又有删除）
    if diff_result.has_warnings {
        warn!("⚠️ Note: diff contains executable SQL and deletion warnings");
        warn!("✓ Add/modify operations will execute normally");
        warn!("✗ Manual changes were skipped and must be reviewed separately");
        warn!("📄 See details: {path}", path = diff_sql_path.display());
    }

    info!("🚀 Starting one-pass diff SQL execution");
    match executor.execute_diff_sql_once(&diff_result.diff_sql).await {
        Ok(results) => {
            info!(
                executed_statements = results.len(),
                "✅ Database upgraded successfully"
            );
            for result in results {
                info!("  {}", result);
            }

            // 归档已执行的差异SQL文件
            archive_diff_sql_file(&diff_sql_path, "executed").await?;
        }
        Err(e) => {
            error!(error = %e, "❌ Database upgrade failed");
            // 归档执行失败的差异SQL文件
            archive_diff_sql_file(&diff_sql_path, "failed").await?;
            return Err(e);
        }
    }

    Ok(())
}

/// 自动修复关键脚本文件权限
async fn fix_script_permissions() -> Result<()> {
    info!("🔧 Fixing critical script file permissions...");

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
                                "🔒 Fixed permissions: {path} (current: {current} -> target: 755)",
                                path = path.display(),
                                current = format!("{:o}", current_mode)
                            );

                            let new_permissions = std::fs::Permissions::from_mode(0o755);
                            if let Err(e) = std::fs::set_permissions(path, new_permissions) {
                                warn!(
                                    "⚠️ Failed to fix permission {path}: {error}",
                                    path = path.display(),
                                    error = e.to_string()
                                );
                            } else {
                                fixed_count += 1;
                                info!("✅ Permission fixed: {path}", path = path.display());
                            }
                        } else {
                            info!(
                                "✓ Permission is correct: {path} ({mode})",
                                path = path.display(),
                                mode = format!("{:o}", current_mode)
                            );
                        }
                    }

                    #[cfg(not(unix))]
                    {
                        info!(
                            "ℹ️ Non-Unix systems, skip permission fix: {path}",
                            path = path.display()
                        );
                    }
                }
                Err(e) => {
                    warn!(
                        "⚠️ Unable to read file metadata {path}: {error}",
                        path = path.display(),
                        error = e.to_string()
                    );
                }
            }
        } else {
            info!(
                "📄 The script file does not exist, skip: {path}",
                path = script_path
            );
        }
    }

    if total_count > 0 {
        info!(
            "🔧 Permission fix complete: {fixed}/{total} scripts fixed",
            fixed = fixed_count,
            total = total_count
        );
    } else {
        info!("📄 No script files found that require permission fixes");
    }

    Ok(())
}

/// 检查并安装 nuwax-cli 更新（独立函数，用于早期检查）
/// 这个函数可以在数据库初始化之前调用，避免数据库锁冲突
pub async fn check_and_install_nuwax_cli_update_early() -> Result<()> {
    use crate::commands::check_update::{check_for_updates, install_release};

    info!("🔍 Prioritizing nuwax-cli version check (before database initialization)...");

    // 检查更新
    let version_info = match check_for_updates().await {
        Ok(info) => {
            info!(
                "✅ Version check completed: current={current}, latest={latest}",
                current = info.current_version,
                latest = info.latest_version
            );
            info
        }
        Err(e) => {
            error!(
                "❌ Failed to check for updates: {error}",
                error = e.to_string()
            );
            return Err(e);
        }
    };

    // 如果有更新，进行安装
    if version_info.is_update_available {
        info!(
            "🚀 Found new version of nuwax-cli: {current} -> {latest}",
            current = version_info.current_version,
            latest = version_info.latest_version
        );
        info!("📥 Starting automatic update installation...");

        match install_release(
            &version_info.download_url.unwrap_or_default(),
            &version_info.latest_version,
        )
        .await
        {
            Ok(_) => {
                info!(
                    "✅ nuwax-cli updated successfully! The program will restart to use the new version"
                );
                std::process::exit(0);
            }
            Err(e) => {
                error!(
                    "❌ nuwax-cli automatic update failed: {error}",
                    error = e.to_string()
                );
                error!(
                    "Please check the network connection or run manually: nuwax-cli check-update install"
                );
                return Err(e);
            }
        }
    } else {
        info!("✅ nuwax-cli is already up to date");
    }

    Ok(())
}

/// 离线部署入口函数
pub async fn run_offline_deploy(
    app: &mut CliApp,
    archive_path: PathBuf,
    version: String,
    frontend_port: Option<u16>,
    config_file: Option<PathBuf>,
    project_name: Option<String>,
) -> Result<()> {
    info!(
        "📦 Starting offline deployment from: {path}",
        path = archive_path.display()
    );

    // 1. 验证文件存在
    if !archive_path.exists() {
        return Err(anyhow::anyhow!(
            "Archive file not found: {}. Please check the file path.",
            archive_path.display()
        ));
    }

    crate::utils::validate_archive_paths(&archive_path)
        .context("Archive package validation failed")?;
    validate_offline_archive_sql(&archive_path)
        .context("Offline package MySQL DDL preflight failed")?;

    // 2. 解析版本号
    let version: client_core::version::Version =
        version.parse().context("Invalid version format")?;

    info!(
        "   Target version: {version}",
        version = version.to_string()
    );

    // 3. 检测是否首次部署
    let is_first_deployment = is_first_deployment().await;

    if is_first_deployment {
        info!("🆕 First deployment detected, using fresh initialization");
    } else {
        info!("🔄 Upgrade deployment detected, services will be stopped first");

        // 停止服务并等待
        let stopped = docker_service::stop_docker_services_and_wait(
            app,
            config_file.clone(),
            project_name.clone(),
        )
        .await?;
        if !stopped {
            return Err(anyhow::anyhow!(
                "Timed out stopping old services; refusing to upgrade MySQL"
            ));
        }
    }

    // 4. 创建 DockerManager
    let docker_manager = create_docker_manager(&config_file, &project_name)?;

    // 5. 环境检测（Podman Desktop 需要预先创建挂载目录）
    let runtime_env = docker_manager.get_runtime_environment();
    if runtime_env.needs_special_handling() {
        info!("⚠️ Windows Podman Desktop detected, pre-creating mount directories");
        if let Err(e) = docker_manager.ensure_host_volumes_exist().await {
            warn!(
                "⚠️ Mount directory check/creation failed: {error}",
                error = e.to_string()
            );
        }
    }

    // 6. 解压（全量升级方式）
    info!("📦 Extracting Docker service package...");

    let upgrade_strategy = UpgradeStrategy::FullUpgrade {
        url: String::new(),
        hash: String::new(),
        signature: String::new(),
        target_version: version.clone(),
        download_type: client_core::upgrade_strategy::DownloadType::Full,
    };

    // 直接解压本地文件。先把旧 docker 目录改名备份，解压失败时恢复。
    let docker_dir = std::path::Path::new("docker");
    let backup_dir = create_docker_backup_path();
    let had_existing_docker_dir = docker_dir.exists();
    if had_existing_docker_dir {
        info!("🧹 Moving existing docker directory to temporary backup...");
        fs::rename(docker_dir, &backup_dir)
            .context("Failed to backup existing docker directory")?;
    }

    if let Err(e) = crate::utils::extract_docker_service(&archive_path, &upgrade_strategy).await {
        warn!("⚠️ Extract failed, restoring previous docker directory");
        restore_docker_backup(&backup_dir, docker_dir)?;
        return Err(e);
    }

    if had_existing_docker_dir {
        restore_preserved_docker_dirs(&backup_dir, docker_dir)?;
    }
    info!("✅ Docker service package extracted");

    let target_version = version.to_string();
    let deployment = async {
        fix_script_permissions().await?;
        run_staged_deployment(
            app,
            frontend_port,
            config_file,
            project_name,
            is_first_deployment,
            &target_version,
        )
        .await
    }
    .await;
    if let Err(error) = deployment {
        if had_existing_docker_dir {
            error!(backup_path = %backup_dir.display(), "Deployment failed; old package files were retained for inspection. MySQL data was not rolled back");
        }
        return Err(error);
    }

    if had_existing_docker_dir
        && backup_dir.exists()
        && let Err(error) = fs::remove_dir_all(&backup_dir)
    {
        warn!(backup_path = %backup_dir.display(), %error, "Deployment succeeded, but old package directory could not be removed");
    }

    Ok(())
}

#[cfg(test)]
mod staged_deploy_tests {
    use super::{restore_preserved_docker_dirs, validate_offline_archive_sql};
    use anyhow::Result;
    use flate2::{Compression, write::GzEncoder};
    use std::{fs::File, io::Write, path::Path};

    fn write_archive(path: &Path, sql: Option<&str>) -> Result<()> {
        let encoder = GzEncoder::new(File::create(path)?, Compression::default());
        let mut archive = tar::Builder::new(encoder);
        if let Some(sql) = sql {
            let mut header = tar::Header::new_gnu();
            header.set_size(sql.len() as u64);
            header.set_mode(0o644);
            header.set_cksum();
            archive.append_data(&mut header, "docker/config/init_mysql.sql", sql.as_bytes())?;
        }
        archive.finish()?;
        archive.into_inner()?.finish()?.flush()?;
        Ok(())
    }

    #[test]
    fn offline_archive_requires_parseable_target_sql() -> Result<()> {
        let directory = tempfile::tempdir()?;
        let archive = directory.path().join("bundle.tar.gz");

        write_archive(&archive, None)?;
        assert!(validate_offline_archive_sql(&archive).is_err());

        write_archive(&archive, Some("CREATE TABLE users (id INT);"))?;
        validate_offline_archive_sql(&archive)?;
        Ok(())
    }

    #[test]
    fn offline_zip_archive_accepts_target_sql() -> Result<()> {
        let directory = tempfile::tempdir()?;
        let archive = directory.path().join("bundle.zip");
        let mut zip = zip::ZipWriter::new(File::create(&archive)?);
        zip.start_file(
            "docker/config/init_mysql.sql",
            zip::write::SimpleFileOptions::default(),
        )?;
        zip.write_all(b"CREATE TABLE users (id INT);")?;
        zip.finish()?;
        validate_offline_archive_sql(&archive)
    }

    #[test]
    fn offline_upgrade_preserves_existing_values_and_adds_package_env_defaults() -> Result<()> {
        let directory = tempfile::tempdir()?;
        let backup = directory.path().join("docker.previous");
        let docker = directory.path().join("docker");
        std::fs::create_dir_all(&backup)?;
        std::fs::create_dir_all(&docker)?;
        std::fs::write(
            backup.join(".env"),
            "# operator config\nMYSQL_PASSWORD=existing-test-value\nFRONTEND_HOST_PORT=8091",
        )?;
        std::fs::write(
            docker.join(".env"),
            "MYSQL_PASSWORD=package-default\nSURREALDB_USER=package-user\nSURREALDB_PASSWORD=package-password\n",
        )?;

        restore_preserved_docker_dirs(&backup, &docker)?;

        let merged = std::fs::read_to_string(docker.join(".env"))?;
        assert!(merged.contains("# operator config\n"));
        assert!(merged.contains("MYSQL_PASSWORD=existing-test-value\n"));
        assert!(merged.contains("FRONTEND_HOST_PORT=8091\n"));
        assert!(merged.contains("SURREALDB_USER=package-user\n"));
        assert!(merged.contains("SURREALDB_PASSWORD=package-password\n"));
        assert!(!merged.contains("MYSQL_PASSWORD=package-default"));
        assert!(!backup.join(".env").exists());
        Ok(())
    }

    #[test]
    fn offline_upgrade_preserves_existing_service_logs() -> Result<()> {
        let directory = tempfile::tempdir()?;
        let backup = directory.path().join("docker.previous");
        let docker = directory.path().join("docker");
        let old_logs = backup.join("logs/rcoder/project_logs/SYSTEM");
        let package_logs = docker.join("logs/rcoder");
        std::fs::create_dir_all(&old_logs)?;
        std::fs::create_dir_all(&package_logs)?;
        std::fs::write(old_logs.join("api.log"), "existing service log\n")?;
        std::fs::write(
            package_logs.join("package-placeholder.log"),
            "package file\n",
        )?;

        restore_preserved_docker_dirs(&backup, &docker)?;

        assert_eq!(
            std::fs::read_to_string(docker.join("logs/rcoder/project_logs/SYSTEM/api.log"))?,
            "existing service log\n"
        );
        assert!(!docker.join("logs/rcoder/package-placeholder.log").exists());
        assert!(!backup.join("logs").exists());
        Ok(())
    }
}
