use crate::app::CliApp;
use crate::docker_service::DockerService;
use anyhow::Result;
use client_core::backup::{BackupManager, BackupOptions};
use client_core::constants::docker;
use client_core::database::BackupType;
use rust_i18n::t;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tracing::{error, info, warn};

/// JSON 格式的备份信息（用于 GUI 集成）
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JsonBackupInfo {
    pub id: i64,
    pub backup_type: String,
    pub created_at: String,
    pub service_version: String,
    pub file_path: String,
    pub file_size: Option<u64>,
    pub file_exists: bool,
}

/// JSON 格式的备份列表响应
#[derive(Debug, Serialize, Deserialize)]
pub struct JsonBackupListResponse {
    pub success: bool,
    pub backups: Vec<JsonBackupInfo>,
    pub error: Option<String>,
}

/// 创建备份
pub async fn run_backup(app: &CliApp) -> Result<()> {
    // 1. 检查Docker环境
    let compose_path = Path::new(&app.config.docker.compose_file);

    if !compose_path.exists() {
        error!("❌ Docker Compose file does not exist: {path}", path = compose_path.display());
        info!("💡 Please ensure Docker services are properly deployed");
        return Ok(());
    }

    // 2. 使用 DockerService 的 health_check 进行智能状态检查
    info!("🔍 Checking Docker service status...");

    let docker_service = DockerService::new(app.config.clone(), app.docker_manager.clone())?;
    match docker_service.health_check().await {
        Ok(report) => {
            info!("📊 Service status: {status}", status = report.get_status_summary());

            // 智能分析服务状态
            let running_containers = report.get_running_containers();
            let completed_containers = report.get_completed_containers();
            let failed_containers = report.get_failed_containers();

            // 🔧 改进：使用restart字段智能判断一次性任务和持续服务
            let persistent_running_services: Vec<_> = running_containers
                .iter()
                .filter(|c| c.is_persistent_service())
                .collect();

            if !persistent_running_services.is_empty() {
                warn!("⚠️  Persistent services are still running!");
                error!("❌ Cold backup requires persistent services to be stopped");

                info!("📝 Found {count} running persistent services:", count = persistent_running_services.len());
                for container in &persistent_running_services {
                    info!("   - {name} (status: {status}, restart: {restart})", name = container.name, status = container.status.display_name(), restart = container.get_restart_display());
                }

                // 显示被忽略的一次性任务
                let oneshot_running_services: Vec<_> = running_containers
                    .iter()
                    .filter(|c| c.is_oneshot())
                    .collect();

                if !oneshot_running_services.is_empty() {
                    info!("📝 Found {count} running one-shot tasks (ignored):", count = oneshot_running_services.len());
                    for container in oneshot_running_services {
                        info!("   - {name} (one-shot task, restart: {restart}, does not affect backup)", name = container.name, restart = container.get_restart_display());
                    }
                }

                info!("💡 Please stop persistent services before backup");
                return Ok(());
            }

            // 成功：所有持续服务已停止
            info!("✅ All persistent services stopped, ready for backup");

            // 显示已完成和被忽略的容器信息
            if !completed_containers.is_empty() {
                let oneshot_completed: Vec<_> = completed_containers
                    .iter()
                    .filter(|c| c.is_oneshot())
                    .collect();

                let other_completed: Vec<_> = completed_containers
                    .iter()
                    .filter(|c| !c.is_oneshot())
                    .collect();

                if !oneshot_completed.is_empty() {
                    info!("🔄 Ignoring {count} one-shot task containers:", count = oneshot_completed.len());
                    for container in oneshot_completed {
                        info!("   - {name} (status: {status}, restart: {restart})", name = container.name, status = container.status.display_name(), restart = container.get_restart_display());
                    }
                }

                if !other_completed.is_empty() {
                    info!("📝 Found {count} other completed containers:", count = other_completed.len());
                    for container in other_completed {
                        info!("   - {name} (status: {status}, restart: {restart})", name = container.name, status = container.status.display_name(), restart = container.get_restart_display());
                    }
                }
            }

            if !failed_containers.is_empty() {
                warn!("⚠️  Found {count} failed containers (does not affect backup):", count = failed_containers.len());
                for container in failed_containers {
                    warn!("   - {name} (status: {status}, restart: {restart})", name = container.name, status = container.status.display_name(), restart = container.get_restart_display());
                }
            }
        }
        Err(e) => {
            error!("❌ Docker service status check failed: {error}", error = e.to_string());
            info!("💡 Cannot confirm service status, suggest manual check before backup");
            return Ok(());
        }
    }

    // 3. 执行备份
    info!("🔄 Starting backup creation...");

    // 执行需要备份的目录: app, data 目录
    let source_paths = vec![docker::get_data_dir_path(), docker::get_app_dir_path()];

    let backup_options = BackupOptions {
        backup_type: BackupType::Manual,
        service_version: app.config.get_docker_versions(),
        work_dir: PathBuf::from("./docker"),
        source_paths,
        compression_level: 6, // 平衡压缩率和速度
    };

    // 使用 BackupManager 创建备份
    let backup_manager = BackupManager::new(
        app.config.get_backup_dir(),
        app.database.clone(),
        app.docker_manager.clone(),
    )?;

    match backup_manager.create_backup(backup_options).await {
        Ok(backup_record) => {
            info!("✅ Backup created successfully: {path}", path = backup_record.file_path);
            info!("📝 Backup ID: {id}", id = backup_record.id);
            info!("📏 Backup service version: {version}", version = backup_record.service_version);
        }
        Err(e) => {
            error!("❌ Backup creation failed: {error}", error = e.to_string());
            return Err(e);
        }
    }

    Ok(())
}

/// 列出备份
pub async fn run_list_backups(app: &CliApp) -> Result<()> {
    let backups = app.backup_manager.list_backups().await?;

    if backups.is_empty() {
        info!("📦 No backup records");
        info!("💡 Use the following command to create backup:");
        info!("   nuwax-cli backup");
        return Ok(());
    }

    info!("📦 Backup List");
    info!("============");

    // 统计信息
    let total_backups = backups.len();
    let mut valid_backups = 0;
    let mut invalid_backups = 0;
    let mut total_size = 0u64;

    // 详细信息表头
    info!(
        "{:<4} {:<12} {:<20} {:<10} {:<8} {:<12} {}",
        t!("backup_cmd.header_id"),
        t!("backup_cmd.header_type"),
        t!("backup_cmd.header_created_at"),
        t!("backup_cmd.header_version"),
        t!("backup_cmd.header_status"),
        t!("backup_cmd.header_size"),
        t!("backup_cmd.header_file_path")
    );
    info!("----------------------------------------------------------------------------------------------------");

    for backup in &backups {
        let backup_path = std::path::Path::new(&backup.file_path);
        let file_exists = backup_path.exists();

        // 文件状态和大小信息
        let (status_display, size_display) = if file_exists {
            valid_backups += 1;

            // 获取文件大小
            let size = if let Ok(metadata) = std::fs::metadata(&backup.file_path) {
                let file_size = metadata.len();
                total_size += file_size;
                if file_size > 1024 * 1024 * 1024 {
                    format!("{:.1}GB", file_size as f64 / (1024.0 * 1024.0 * 1024.0))
                } else if file_size > 1024 * 1024 {
                    format!("{:.1}MB", file_size as f64 / (1024.0 * 1024.0))
                } else if file_size > 1024 {
                    format!("{:.1}KB", file_size as f64 / 1024.0)
                } else {
                    format!("{file_size}B")
                }
            } else {
                t!("backup_cmd.size_unknown").to_string()
            };

            (t!("backup_cmd.status_available").to_string(), size)
        } else {
            invalid_backups += 1;
            (t!("backup_cmd.status_file_missing").to_string(), "---".to_string())
        };

        // 备份类型显示
        let backup_type_display = match backup.backup_type {
            client_core::database::BackupType::Manual => t!("backup_cmd.type_manual"),
            client_core::database::BackupType::PreUpgrade => t!("backup_cmd.type_pre_upgrade"),
        };

        // 获取文件名而不是完整路径用于显示
        let filename = backup_path
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_else(|| backup.file_path.clone());

        info!(
            "{:<4} {:<12} {:<20} {:<10} {:<8} {:<12} {}",
            backup.id,
            backup_type_display,
            backup.created_at.format("%Y-%m-%d %H:%M:%S"),
            backup.service_version,
            status_display,
            size_display,
            filename
        );

        // 如果文件不存在，显示警告信息
        if !file_exists {
            warn!("     ⚠️  Warning: Backup file does not exist, cannot be used for rollback!");
            warn!("         Expected path: {path}", path = backup.file_path);
        }
    }

    info!("----------------------------------------------------------------------------------------------------");

    // 统计摘要
    info!("📊 Backup Statistics:");
    info!("   Total backups: {count}", count = total_backups);
    info!("   Valid backups: {count} ✅", count = valid_backups);
    if invalid_backups > 0 {
        warn!("   Invalid backups: {count} ❌", count = invalid_backups);
    }

    if total_size > 0 {
        let total_size_display = if total_size > 1024 * 1024 * 1024 {
            format!("{:.2} GB", total_size as f64 / (1024.0 * 1024.0 * 1024.0))
        } else if total_size > 1024 * 1024 {
            format!("{:.2} MB", total_size as f64 / (1024.0 * 1024.0))
        } else {
            format!("{:.2} KB", total_size as f64 / 1024.0)
        };
        info!("   Total size: {size}", size = total_size_display);
    }

    // 操作提示
    if valid_backups > 0 {
        info!("💡 Available operations:");
        info!("   - Interactive rollback: nuwax-cli rollback");
        info!("   - Rollback by ID: nuwax-cli rollback <backup_id>");
        info!("   - Create new backup: nuwax-cli backup");
    }

    if invalid_backups > 0 {
        warn!("⚠️  Found {count} invalid backups (file missing)", count = invalid_backups);
        info!("💡 Suggestions:");
        info!("   - Check backup directory settings: {dir}", dir = app.config.get_backup_dir().display());
        info!("   - If backup files were deleted, these records cannot be used for recovery");
        info!("   - Consider manually cleaning up these invalid records");
    }

    Ok(())
}

/// 从备份恢复
pub async fn run_rollback(
    app: &CliApp,
    backup_id: Option<i64>,
    force: bool,
    list_json: bool,
    auto_start_service: bool,
    rollback_data: bool,
) -> Result<()> {
    // 如果指定了 --list-json，禁用日志输出并输出 JSON 格式的备份列表
    if list_json {
        // 临时设置日志级别为OFF，避免污染JSON输出
        tracing::subscriber::set_global_default(
            tracing_subscriber::FmtSubscriber::builder()
                .with_max_level(tracing::Level::ERROR)
                .finish(),
        )
        .ok();

        return output_backups_as_json(app).await;
    }

    // 如果没有提供backup_id，启动交互式选择
    let selected_backup_id = if let Some(id) = backup_id {
        id
    } else {
        match interactive_backup_selection(app).await? {
            Some(id) => id,
            None => {
                info!("Operation cancelled");
                return Ok(());
            }
        }
    };

    if !force {
        if rollback_data {
            warn!("⚠️  Warning: This operation will overwrite current data directory, Mysql, Redis etc. data will also be rolled back!");
        } else {
            warn!("⚠️  Warning: This operation will rollback backend and frontend application versions, but not Mysql, Redis etc. data!");
        }

        use std::io::{self, Write};
        print!("{}", t!("backup_cmd.confirm_restore", id = selected_backup_id));
        io::stdout().flush()?;

        let mut input = String::new();
        std::io::stdin().read_line(&mut input)?;

        if input.trim().to_lowercase() != "y" {
            warn!("Operation cancelled");
            return Ok(());
        }
    }

    info!("Starting data rollback operation...");

    // 🔧 智能回滚
    if rollback_data {
        //data,app 等目录,全部恢复
        run_rollback_with_exculde(app, selected_backup_id, auto_start_service, &[]).await?;
    } else {
        info!("rollback_data is false, not rolling back data directory (mysql, redis etc. data will not be rolled back)");
        //data 数据目录不用恢复,回滚应用业务逻辑, 考虑改写: perform_selective_restore ,增加参数,用于排除 data 目录
        run_rollback_with_exculde(app, selected_backup_id, auto_start_service, &["data"]).await?;
    }

    info!("✅ Data rollback complete");
    Ok(())
}

/// 只回滚 data 目录，保留 app 目录和配置文件
pub async fn run_rollback_data_only(
    app: &CliApp,
    backup_id: Option<i64>,
    force: bool,
    auto_start_service: bool,
    config_file: Option<&std::path::PathBuf>,
) -> Result<()> {
    // 如果没有提供backup_id，启动交互式选择
    let selected_backup_id = if let Some(id) = backup_id {
        id
    } else {
        match interactive_backup_selection(app).await? {
            Some(id) => id,
            None => {
                info!("Operation cancelled");
                return Ok(());
            }
        }
    };

    if !force {
        warn!("⚠️  Warning: This operation will overwrite current data directory!");
        warn!("⚠️  Note: This operation only restores data directory, app directory and config files will remain unchanged");

        use std::io::{self, Write};
        print!("{}", t!("backup_cmd.confirm_restore_data", id = selected_backup_id));
        io::stdout().flush()?;

        let mut input = String::new();
        std::io::stdin().read_line(&mut input)?;

        if input.trim().to_lowercase() != "y" {
            warn!("Operation cancelled");
            return Ok(());
        }
    }

    info!("Starting data directory rollback operation...");

    // 🔧 只回滚 data 目录：只恢复 data 目录，保留 app 目录和配置文件
    run_data_directory_only_rollback(app, selected_backup_id, auto_start_service, config_file)
        .await?;

    info!("✅ Data directory rollback complete");
    Ok(())
}

/// 交互式备份选择
async fn interactive_backup_selection(app: &CliApp) -> Result<Option<i64>> {
    info!("🗂️  Backup Selection");
    info!("============");

    let backups = app.backup_manager.list_backups().await?;

    if backups.is_empty() {
        warn!("❌ No available backups");
        info!("💡 Use the following command to create backup:");
        info!("   nuwax-cli backup");
        return Ok(None);
    }

    // 筛选可用的备份（文件存在且有效）
    let mut valid_backups = Vec::new();
    for backup in &backups {
        let backup_path = std::path::Path::new(&backup.file_path);
        if backup_path.exists() {
            valid_backups.push(backup);
        }
    }

    if valid_backups.is_empty() {
        warn!("❌ No available backup files");
        info!("💡 All backup files are lost or corrupted");
        return Ok(None);
    }

    // 显示备份选择列表
    info!("📋 Available backup list:");
    info!(
        "{:<4} {:<12} {:<20} {:<10} {:<12} {}",
        t!("backup_cmd.header_index"),
        t!("backup_cmd.header_type"),
        t!("backup_cmd.header_created_at"),
        t!("backup_cmd.header_version"),
        t!("backup_cmd.header_size"),
        t!("backup_cmd.header_filename")
    );
    info!("--------------------------------------------------------------------------------");

    for (index, backup) in valid_backups.iter().enumerate() {
        let backup_path = std::path::Path::new(&backup.file_path);

        // 获取文件大小
        let size_display = if let Ok(metadata) = std::fs::metadata(&backup.file_path) {
            let file_size = metadata.len();
            if file_size > 1024 * 1024 * 1024 {
                format!("{:.1}GB", file_size as f64 / (1024.0 * 1024.0 * 1024.0))
            } else if file_size > 1024 * 1024 {
                format!("{:.1}MB", file_size as f64 / (1024.0 * 1024.0))
            } else if file_size > 1024 {
                format!("{:.1}KB", file_size as f64 / 1024.0)
            } else {
                format!("{file_size}B")
            }
        } else {
            t!("backup_cmd.size_unknown").to_string()
        };

        // 备份类型显示
        let backup_type_display = match backup.backup_type {
            client_core::database::BackupType::Manual => t!("backup_cmd.type_manual"),
            client_core::database::BackupType::PreUpgrade => t!("backup_cmd.type_pre_upgrade"),
        };

        // 获取文件名
        let filename = backup_path
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_else(|| backup.file_path.clone());

        info!(
            "{:<4} {:<12} {:<20} {:<10} {:<12} {}",
            index + 1,
            backup_type_display,
            backup.created_at.format("%Y-%m-%d %H:%M:%S"),
            backup.service_version,
            size_display,
            filename
        );
    }

    info!("--------------------------------------------------------------------------------");
    info!("💡 Input instructions:");
    info!("   - Enter index (1-{count}) to select backup to restore", count = valid_backups.len());
    info!("   - Enter 'q' or 'quit' to exit");
    info!("   - Enter 'l' or 'list' to redisplay list");

    // 交互式选择循环
    use std::io::{self, Write};
    loop {
        print!("\n{}", t!("backup_cmd.select_prompt", count = valid_backups.len()));
        io::stdout().flush()?;

        let mut input = String::new();
        io::stdin().read_line(&mut input)?;
        let input = input.trim();

        // 处理退出命令
        if input.is_empty() || input.eq_ignore_ascii_case("q") || input.eq_ignore_ascii_case("quit")
        {
            info!("👋 Operation cancelled");
            return Ok(None);
        }

        // 处理重新显示列表
        if input.eq_ignore_ascii_case("l") || input.eq_ignore_ascii_case("list") {
            info!("\n📋 Redisplaying backup list:");
            info!(
                "{:<4} {:<12} {:<20} {:<10} {:<12} {}",
                t!("backup_cmd.header_index"),
                t!("backup_cmd.header_type"),
                t!("backup_cmd.header_created_at"),
                t!("backup_cmd.header_version"),
                t!("backup_cmd.header_size"),
                t!("backup_cmd.header_filename")
            );
            info!("--------------------------------------------------------------------------------");

            for (index, backup) in valid_backups.iter().enumerate() {
                let backup_path = std::path::Path::new(&backup.file_path);

                let size_display = if let Ok(metadata) = std::fs::metadata(&backup.file_path) {
                    let file_size = metadata.len();
                    if file_size > 1024 * 1024 * 1024 {
                        format!("{:.1}GB", file_size as f64 / (1024.0 * 1024.0 * 1024.0))
                    } else if file_size > 1024 * 1024 {
                        format!("{:.1}MB", file_size as f64 / (1024.0 * 1024.0))
                    } else if file_size > 1024 {
                        format!("{:.1}KB", file_size as f64 / 1024.0)
                    } else {
                        format!("{file_size}B")
                    }
                } else {
                    t!("backup_cmd.size_unknown").to_string()
                };

                let backup_type_display = match backup.backup_type {
                    client_core::database::BackupType::Manual => t!("backup_cmd.type_manual"),
                    client_core::database::BackupType::PreUpgrade => t!("backup_cmd.type_pre_upgrade"),
                };

                let filename = backup_path
                    .file_name()
                    .map(|n| n.to_string_lossy().to_string())
                    .unwrap_or_else(|| backup.file_path.clone());

                info!(
                    "{:<4} {:<12} {:<20} {:<10} {:<12} {}",
                    index + 1,
                    backup_type_display,
                    backup.created_at.format("%Y-%m-%d %H:%M:%S"),
                    backup.service_version,
                    size_display,
                    filename
                );
            }
            info!("--------------------------------------------------------------------------------");
            continue;
        }

        // 处理数字选择
        match input.parse::<usize>() {
            Ok(selection) => {
                if selection >= 1 && selection <= valid_backups.len() {
                    let selected_backup = valid_backups[selection - 1];

                    // 显示选择确认
                    info!("✅ You selected backup:");
                    info!("   Backup ID: {id}", id = selected_backup.id);
                    info!("   Type: {backup_type}", backup_type = match selected_backup.backup_type {
                                client_core::database::BackupType::Manual => t!("backup_cmd.type_manual"),
                                client_core::database::BackupType::PreUpgrade => t!("backup_cmd.type_pre_upgrade"),
                            });
                    info!("   Created at: {time}", time = selected_backup.created_at.format("%Y-%m-%d %H:%M:%S"));
                    info!("   Service version: {version}", version = selected_backup.service_version);
                    info!("   File path: {path}", path = selected_backup.file_path);

                    return Ok(Some(selected_backup.id));
                } else {
                    warn!("❌ Invalid selection, please enter a number between 1-{count}", count = valid_backups.len());
                }
            }
            Err(_) => {
                warn!("❌ Invalid input, please enter a number, 'q' (quit) or 'l' (redisplay list)");
            }
        }
    }
}

/// 只恢复数据的智能回滚
async fn run_rollback_with_exculde(
    app: &CliApp,
    backup_id: i64,
    auto_start_service: bool,
    dirs_to_exculde: &[&str],
) -> Result<()> {
    info!("🛡️ Using smart data rollback mode");
    info!("   📁 Will restore: data/, app/ directories");
    info!("   🔧 Will keep: docker-compose.yml, .env and other config files");
    info!("   Directories not restored:{dirs}", dirs = format!("{:?}", dirs_to_exculde));

    // 使用 BackupManager 的智能数据恢复功能
    let docker_dir = std::path::Path::new("./docker");
    match app
        .backup_manager
        .restore_data_from_backup_with_exculde(
            backup_id,
            docker_dir,
            auto_start_service,
            dirs_to_exculde,
        )
        .await
    {
        Ok(_) => {
            info!("✅ Smart data restore complete");

            // 设置正确的权限
            let mysql_data_dir = docker_dir.join("data/mysql");
            if mysql_data_dir.exists() {
                #[cfg(unix)]
                {
                    use std::os::unix::fs::PermissionsExt;
                    let permissions = std::fs::Permissions::from_mode(0o775);
                    if let Err(e) = std::fs::set_permissions(&mysql_data_dir, permissions) {
                        warn!("⚠️ Failed to set MySQL permission: {error}", error = e.to_string());
                    } else {
                        info!("🔒 MySQL data directory permission set to 775");
                    }
                }
            }

            info!("💡 Data restore info:");
            info!("   ✅ All database data restored");
            info!("   ✅ All application files restored");
            info!("   ✅ Config files kept at latest version");

            if auto_start_service {
                info!("   ✅ Docker services auto-started");
            } else {
                info!("   📝 Docker service start skipped (controlled by parent process)");
            }
        }
        Err(e) => {
            error!("❌ Data restore failed: {error}", error = e.to_string());
            warn!("💡 Suggestions:");
            warn!("   1. Check if backup file exists and is complete");
            warn!("   2. Ensure sufficient disk space");
            warn!("   3. Manually start services: nuwax-cli docker-service start");
            return Err(e);
        }
    }

    Ok(())
}

/// 只恢复 data 目录，保留 app 目录和配置文件
async fn run_data_directory_only_rollback(
    app: &CliApp,
    backup_id: i64,
    auto_start_service: bool,
    config_file: Option<&std::path::PathBuf>,
) -> Result<()> {
    info!("🛡️ Using smart data directory rollback mode");
    info!("   📁 Will restore: data/ directory");
    info!("   🔧 Will keep: app/ directory, docker-compose.yml, .env and other config files");

    // 使用 BackupManager 的智能数据恢复功能
    let docker_dir = std::path::Path::new("./docker");

    // 如果有自定义配置文件，创建新的 DockerManager
    let backup_manager = if let Some(config_path) = config_file {
        info!("📄 Using custom config file for restore: {path}", path = config_path.display());

        // 获取对应的 .env 文件路径
        let env_file = config_path.with_file_name(".env");
        let custom_docker_manager = Arc::new(
            client_core::container::DockerManager::with_project(
                config_path.clone(),
                env_file.clone(),
                None,
            )
            .map_err(|e| anyhow::anyhow!("{}", t!("backup_cmd.create_docker_manager_failed", error = e.to_string())))?,
        );
        Arc::new(client_core::backup::BackupManager::new(
            app.config.get_backup_dir(),
            app.database.clone(),
            custom_docker_manager,
        )?)
    } else {
        app.backup_manager.clone()
    };

    //只恢复 data 目录,其他的数据不恢复
    let dir_to_restore = vec!["data"];
    match backup_manager
        .restore_data_directory_only(backup_id, docker_dir, auto_start_service, &dir_to_restore)
        .await
    {
        Ok(_) => {
            info!("✅ Smart data directory restore complete");

            // 设置正确的权限
            let mysql_data_dir = docker_dir.join("data/mysql");
            if mysql_data_dir.exists() {
                #[cfg(unix)]
                {
                    use std::os::unix::fs::PermissionsExt;
                    let permissions = std::fs::Permissions::from_mode(0o775);
                    if let Err(e) = std::fs::set_permissions(&mysql_data_dir, permissions) {
                        warn!("⚠️ Failed to set MySQL permission: {error}", error = e.to_string());
                    } else {
                        info!("🔒 MySQL data directory permission set to 775");
                    }
                }
            }

            info!("💡 Data restore info:");
            info!("   ✅ All database data restored");
            info!("   ✅ app directory kept unchanged");
            info!("   ✅ Config files kept at latest version");

            if auto_start_service {
                info!("   ✅ Docker services auto-started");
            } else {
                info!("   📝 Docker service start skipped (controlled by parent process)");
            }
        }
        Err(e) => {
            error!("❌ data directory restore failed: {error}", error = e.to_string());
            warn!("💡 Suggestions:");
            warn!("   1. Check if backup file exists and is complete");
            warn!("   2. Ensure sufficient disk space");
            warn!("   3. Manually start services: nuwax-cli docker-service start");
            return Err(e);
        }
    }

    Ok(())
}

/// 输出 JSON 格式的备份列表（用于 GUI 集成）
async fn output_backups_as_json(app: &CliApp) -> Result<()> {
    match get_backups_as_json(app).await {
        Ok(response) => {
            // 只输出纯JSON到标准输出，不包含任何日志信息
            match serde_json::to_string(&response) {
                Ok(json_str) => {
                    // 使用 print! 而不是 println! 来避免额外的换行符
                    print!("{json_str}");
                    Ok(())
                }
                Err(e) => {
                    let error_response = JsonBackupListResponse {
                        success: false,
                        backups: vec![],
                        error: Some(t!("backup_cmd.json_serialize_failed", error = e.to_string()).to_string()),
                    };
                    if let Ok(error_json) = serde_json::to_string(&error_response) {
                        print!("{error_json}");
                    }
                    Ok(())
                }
            }
        }
        Err(e) => {
            let error_response = JsonBackupListResponse {
                success: false,
                backups: vec![],
                error: Some(e.to_string()),
            };
            if let Ok(error_json) = serde_json::to_string(&error_response) {
                print!("{error_json}");
            }
            Ok(())
        }
    }
}

/// 获取 JSON 格式的备份列表
async fn get_backups_as_json(app: &CliApp) -> Result<JsonBackupListResponse> {
    let backups = app.backup_manager.list_backups().await?;

    let mut json_backups = Vec::new();

    for backup in backups {
        let backup_path = std::path::Path::new(&backup.file_path);
        let file_exists = backup_path.exists();

        // 获取文件大小
        let file_size = if file_exists {
            std::fs::metadata(&backup.file_path).ok().map(|m| m.len())
        } else {
            None
        };

        // 备份类型转换为字符串
        let backup_type_str = match backup.backup_type {
            client_core::database::BackupType::Manual => "Manual",
            client_core::database::BackupType::PreUpgrade => "PreUpgrade",
        };

        json_backups.push(JsonBackupInfo {
            id: backup.id,
            backup_type: backup_type_str.to_string(),
            created_at: backup.created_at.format("%Y-%m-%d %H:%M:%S").to_string(),
            service_version: backup.service_version,
            file_path: backup.file_path,
            file_size,
            file_exists,
        });
    }

    Ok(JsonBackupListResponse {
        success: true,
        backups: json_backups,
        error: None,
    })
}
