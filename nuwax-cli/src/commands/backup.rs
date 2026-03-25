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
        error!("{}", t!("backup_cmd.compose_not_exists", path = compose_path.display()));
        info!("{}", t!("backup_cmd.ensure_docker_deployed"));
        return Ok(());
    }

    // 2. 使用 DockerService 的 health_check 进行智能状态检查
    info!("{}", t!("backup_cmd.checking_docker_status"));

    let docker_service = DockerService::new(app.config.clone(), app.docker_manager.clone())?;
    match docker_service.health_check().await {
        Ok(report) => {
            info!("{}", t!("backup_cmd.service_status", status = report.get_status_summary()));

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
                warn!("{}", t!("backup_cmd.persistent_services_running"));
                error!("{}", t!("backup_cmd.cold_backup_requires_stop"));

                info!(
                    "{}",
                    t!("backup_cmd.found_running_services", count = persistent_running_services.len())
                );
                for container in &persistent_running_services {
                    info!(
                        "{}",
                        t!("backup_cmd.container_info_with_restart",
                            name = container.name,
                            status = container.status.display_name(),
                            restart = container.get_restart_display())
                    );
                }

                // 显示被忽略的一次性任务
                let oneshot_running_services: Vec<_> = running_containers
                    .iter()
                    .filter(|c| c.is_oneshot())
                    .collect();

                if !oneshot_running_services.is_empty() {
                    info!(
                        "{}",
                        t!("backup_cmd.found_oneshot_running", count = oneshot_running_services.len())
                    );
                    for container in oneshot_running_services {
                        info!(
                            "{}",
                            t!("backup_cmd.oneshot_container_info",
                                name = container.name,
                                restart = container.get_restart_display())
                        );
                    }
                }

                info!("{}", t!("backup_cmd.stop_services_first"));
                return Ok(());
            }

            // 成功：所有持续服务已停止
            info!("{}", t!("backup_cmd.all_services_stopped"));

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
                    info!("{}", t!("backup_cmd.ignoring_oneshot", count = oneshot_completed.len()));
                    for container in oneshot_completed {
                        info!(
                            "{}",
                            t!("backup_cmd.container_info_with_restart",
                                name = container.name,
                                status = container.status.display_name(),
                                restart = container.get_restart_display())
                        );
                    }
                }

                if !other_completed.is_empty() {
                    info!("{}", t!("backup_cmd.found_other_completed", count = other_completed.len()));
                    for container in other_completed {
                        info!(
                            "{}",
                            t!("backup_cmd.container_info_with_restart",
                                name = container.name,
                                status = container.status.display_name(),
                                restart = container.get_restart_display())
                        );
                    }
                }
            }

            if !failed_containers.is_empty() {
                warn!(
                    "{}",
                    t!("backup_cmd.found_failed_containers", count = failed_containers.len())
                );
                for container in failed_containers {
                    warn!(
                        "{}",
                        t!("backup_cmd.container_info_with_restart",
                            name = container.name,
                            status = container.status.display_name(),
                            restart = container.get_restart_display())
                    );
                }
            }
        }
        Err(e) => {
            error!("{}", t!("backup_cmd.check_status_failed", error = e.to_string()));
            info!("{}", t!("backup_cmd.suggest_manual_check"));
            return Ok(());
        }
    }

    // 3. 执行备份
    info!("{}", t!("backup_cmd.starting_backup"));

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
            info!("{}", t!("backup_cmd.backup_created", path = backup_record.file_path));
            info!("{}", t!("backup_cmd.backup_id", id = backup_record.id));
            info!("{}", t!("backup_cmd.service_version", version = backup_record.service_version));
        }
        Err(e) => {
            error!("{}", t!("backup_cmd.backup_failed", error = e.to_string()));
            return Err(e);
        }
    }

    Ok(())
}

/// 列出备份
pub async fn run_list_backups(app: &CliApp) -> Result<()> {
    let backups = app.backup_manager.list_backups().await?;

    if backups.is_empty() {
        info!("{}", t!("backup_cmd.no_backups"));
        info!("{}", t!("backup_cmd.hint_create_backup"));
        info!("{}", t!("backup_cmd.hint_backup_command"));
        return Ok(());
    }

    info!("{}", t!("backup_cmd.backup_list_title"));
    info!("{}", t!("backup_cmd.separator"));

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
    info!("{}", t!("backup_cmd.list_separator"));

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
            warn!("{}", t!("backup_cmd.warning_file_missing"));
            warn!("{}", t!("backup_cmd.expected_path", path = backup.file_path));
        }
    }

    info!("{}", t!("backup_cmd.list_separator"));

    // 统计摘要
    info!("{}", t!("backup_cmd.backup_statistics"));
    info!("{}", t!("backup_cmd.total_backups", count = total_backups));
    info!("{}", t!("backup_cmd.valid_backups", count = valid_backups));
    if invalid_backups > 0 {
        warn!("{}", t!("backup_cmd.invalid_backups", count = invalid_backups));
    }

    if total_size > 0 {
        let total_size_display = if total_size > 1024 * 1024 * 1024 {
            format!("{:.2} GB", total_size as f64 / (1024.0 * 1024.0 * 1024.0))
        } else if total_size > 1024 * 1024 {
            format!("{:.2} MB", total_size as f64 / (1024.0 * 1024.0))
        } else {
            format!("{:.2} KB", total_size as f64 / 1024.0)
        };
        info!("{}", t!("backup_cmd.total_size", size = total_size_display));
    }

    // 操作提示
    if valid_backups > 0 {
        info!("{}", t!("backup_cmd.available_operations"));
        info!("{}", t!("backup_cmd.operation_interactive_rollback"));
        info!("{}", t!("backup_cmd.operation_rollback_by_id"));
        info!("{}", t!("backup_cmd.operation_create_backup"));
    }

    if invalid_backups > 0 {
        warn!("{}", t!("backup_cmd.found_invalid_backups", count = invalid_backups));
        info!("{}", t!("backup_cmd.suggestions"));
        info!(
            "{}",
            t!("backup_cmd.check_backup_dir", dir = app.config.get_backup_dir().display())
        );
        info!("{}", t!("backup_cmd.file_deleted_hint"));
        info!("{}", t!("backup_cmd.cleanup_hint"));
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
                info!("{}", t!("backup_cmd.operation_cancelled"));
                return Ok(());
            }
        }
    };

    if !force {
        if rollback_data {
            warn!("{}", t!("backup_cmd.warn_rollback_data_overwrite"));
        } else {
            warn!("{}", t!("backup_cmd.warn_rollback_app_only"));
        }

        use std::io::{self, Write};
        print!("{}", t!("backup_cmd.confirm_restore", id = selected_backup_id));
        io::stdout().flush()?;

        let mut input = String::new();
        std::io::stdin().read_line(&mut input)?;

        if input.trim().to_lowercase() != "y" {
            warn!("{}", t!("backup_cmd.operation_cancelled"));
            return Ok(());
        }
    }

    info!("{}", t!("backup_cmd.starting_rollback"));

    // 🔧 智能回滚
    if rollback_data {
        //data,app 等目录,全部恢复
        run_rollback_with_exculde(app, selected_backup_id, auto_start_service, &[]).await?;
    } else {
        info!("{}", t!("backup_cmd.rollback_data_false_hint"));
        //data 数据目录不用恢复,回滚应用业务逻辑, 考虑改写: perform_selective_restore ,增加参数,用于排除 data 目录
        run_rollback_with_exculde(app, selected_backup_id, auto_start_service, &["data"]).await?;
    }

    info!("{}", t!("backup_cmd.rollback_complete"));
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
                info!("{}", t!("backup_cmd.operation_cancelled"));
                return Ok(());
            }
        }
    };

    if !force {
        warn!("{}", t!("backup_cmd.warn_overwrite_data"));
        warn!("{}", t!("backup_cmd.warn_only_data_restore"));

        use std::io::{self, Write};
        print!("{}", t!("backup_cmd.confirm_restore_data", id = selected_backup_id));
        io::stdout().flush()?;

        let mut input = String::new();
        std::io::stdin().read_line(&mut input)?;

        if input.trim().to_lowercase() != "y" {
            warn!("{}", t!("backup_cmd.operation_cancelled"));
            return Ok(());
        }
    }

    info!("{}", t!("backup_cmd.starting_data_rollback"));

    // 🔧 只回滚 data 目录：只恢复 data 目录，保留 app 目录和配置文件
    run_data_directory_only_rollback(app, selected_backup_id, auto_start_service, config_file)
        .await?;

    info!("{}", t!("backup_cmd.data_rollback_complete"));
    Ok(())
}

/// 交互式备份选择
async fn interactive_backup_selection(app: &CliApp) -> Result<Option<i64>> {
    info!("{}", t!("backup_cmd.backup_selection"));
    info!("{}", t!("backup_cmd.selection_separator"));

    let backups = app.backup_manager.list_backups().await?;

    if backups.is_empty() {
        warn!("{}", t!("backup_cmd.no_available_backups"));
        info!("{}", t!("backup_cmd.hint_create_backup"));
        info!("{}", t!("backup_cmd.hint_backup_command"));
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
        warn!("{}", t!("backup_cmd.no_available_backup_files"));
        info!("{}", t!("backup_cmd.all_backups_lost"));
        return Ok(None);
    }

    // 显示备份选择列表
    info!("{}", t!("backup_cmd.available_backup_list"));
    info!(
        "{:<4} {:<12} {:<20} {:<10} {:<12} {}",
        t!("backup_cmd.header_index"),
        t!("backup_cmd.header_type"),
        t!("backup_cmd.header_created_at"),
        t!("backup_cmd.header_version"),
        t!("backup_cmd.header_size"),
        t!("backup_cmd.header_filename")
    );
    info!("{}", t!("backup_cmd.selection_list_separator"));

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

    info!("{}", t!("backup_cmd.selection_list_separator"));
    info!("{}", t!("backup_cmd.input_instructions"));
    info!("{}", t!("backup_cmd.input_select_hint", count = valid_backups.len()));
    info!("{}", t!("backup_cmd.input_quit_hint"));
    info!("{}", t!("backup_cmd.input_list_hint"));

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
            info!("{}", t!("backup_cmd.operation_cancelled_bye"));
            return Ok(None);
        }

        // 处理重新显示列表
        if input.eq_ignore_ascii_case("l") || input.eq_ignore_ascii_case("list") {
            info!("\n{}", t!("backup_cmd.redisplay_list"));
            info!(
                "{:<4} {:<12} {:<20} {:<10} {:<12} {}",
                t!("backup_cmd.header_index"),
                t!("backup_cmd.header_type"),
                t!("backup_cmd.header_created_at"),
                t!("backup_cmd.header_version"),
                t!("backup_cmd.header_size"),
                t!("backup_cmd.header_filename")
            );
            info!("{}", t!("backup_cmd.selection_list_separator"));

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
            info!("{}", t!("backup_cmd.selection_list_separator"));
            continue;
        }

        // 处理数字选择
        match input.parse::<usize>() {
            Ok(selection) => {
                if selection >= 1 && selection <= valid_backups.len() {
                    let selected_backup = valid_backups[selection - 1];

                    // 显示选择确认
                    info!("{}", t!("backup_cmd.selected_backup"));
                    info!("{}", t!("backup_cmd.selected_backup_id", id = selected_backup.id));
                    info!(
                        "{}",
                        t!("backup_cmd.selected_backup_type",
                            backup_type = match selected_backup.backup_type {
                                client_core::database::BackupType::Manual => t!("backup_cmd.type_manual"),
                                client_core::database::BackupType::PreUpgrade => t!("backup_cmd.type_pre_upgrade"),
                            })
                    );
                    info!(
                        "{}",
                        t!("backup_cmd.selected_created_at",
                            time = selected_backup.created_at.format("%Y-%m-%d %H:%M:%S"))
                    );
                    info!("{}", t!("backup_cmd.selected_service_version", version = selected_backup.service_version));
                    info!("{}", t!("backup_cmd.selected_file_path", path = selected_backup.file_path));

                    return Ok(Some(selected_backup.id));
                } else {
                    warn!("{}", t!("backup_cmd.invalid_selection", count = valid_backups.len()));
                }
            }
            Err(_) => {
                warn!("{}", t!("backup_cmd.invalid_input"));
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
    info!("{}", t!("backup_cmd.smart_rollback_mode"));
    info!("{}", t!("backup_cmd.will_restore_data_app"));
    info!("{}", t!("backup_cmd.will_keep_config"));
    info!("{}", t!("backup_cmd.excluded_dirs", dirs = format!("{:?}", dirs_to_exculde)));

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
            info!("{}", t!("backup_cmd.smart_restore_complete"));

            // 设置正确的权限
            let mysql_data_dir = docker_dir.join("data/mysql");
            if mysql_data_dir.exists() {
                #[cfg(unix)]
                {
                    use std::os::unix::fs::PermissionsExt;
                    let permissions = std::fs::Permissions::from_mode(0o775);
                    if let Err(e) = std::fs::set_permissions(&mysql_data_dir, permissions) {
                        warn!("{}", t!("backup_cmd.set_mysql_permission_failed", error = e.to_string()));
                    } else {
                        info!("{}", t!("backup_cmd.mysql_permission_set"));
                    }
                }
            }

            info!("{}", t!("backup_cmd.restore_info_title"));
            info!("{}", t!("backup_cmd.restore_db_complete"));
            info!("{}", t!("backup_cmd.restore_app_complete"));
            info!("{}", t!("backup_cmd.restore_config_kept"));

            if auto_start_service {
                info!("{}", t!("backup_cmd.docker_service_started"));
            } else {
                info!("{}", t!("backup_cmd.docker_service_start_skipped"));
            }
        }
        Err(e) => {
            error!("{}", t!("backup_cmd.data_restore_failed", error = e.to_string()));
            warn!("{}", t!("backup_cmd.suggestions"));
            warn!("{}", t!("backup_cmd.check_backup_file"));
            warn!("{}", t!("backup_cmd.check_disk_space"));
            warn!("{}", t!("backup_cmd.manual_start_hint"));
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
    info!("{}", t!("backup_cmd.smart_data_rollback_mode"));
    info!("{}", t!("backup_cmd.will_restore_data_only"));
    info!("{}", t!("backup_cmd.will_keep_app_config"));

    // 使用 BackupManager 的智能数据恢复功能
    let docker_dir = std::path::Path::new("./docker");

    // 如果有自定义配置文件，创建新的 DockerManager
    let backup_manager = if let Some(config_path) = config_file {
        info!("{}", t!("backup_cmd.using_custom_config", path = config_path.display()));

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
            info!("{}", t!("backup_cmd.smart_data_restore_complete"));

            // 设置正确的权限
            let mysql_data_dir = docker_dir.join("data/mysql");
            if mysql_data_dir.exists() {
                #[cfg(unix)]
                {
                    use std::os::unix::fs::PermissionsExt;
                    let permissions = std::fs::Permissions::from_mode(0o775);
                    if let Err(e) = std::fs::set_permissions(&mysql_data_dir, permissions) {
                        warn!("{}", t!("backup_cmd.set_mysql_permission_failed", error = e.to_string()));
                    } else {
                        info!("{}", t!("backup_cmd.mysql_permission_set"));
                    }
                }
            }

            info!("{}", t!("backup_cmd.restore_info_title"));
            info!("{}", t!("backup_cmd.restore_db_complete"));
            info!("{}", t!("backup_cmd.app_dir_kept"));
            info!("{}", t!("backup_cmd.restore_config_kept"));

            if auto_start_service {
                info!("{}", t!("backup_cmd.docker_service_started"));
            } else {
                info!("{}", t!("backup_cmd.docker_service_start_skipped"));
            }
        }
        Err(e) => {
            error!("{}", t!("backup_cmd.data_dir_restore_failed", error = e.to_string()));
            warn!("{}", t!("backup_cmd.suggestions"));
            warn!("{}", t!("backup_cmd.check_backup_file"));
            warn!("{}", t!("backup_cmd.check_disk_space"));
            warn!("{}", t!("backup_cmd.manual_start_hint"));
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
