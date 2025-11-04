use crate::app::CliApp;
use crate::docker_service::health_check::ContainerInfo;
use crate::docker_service::{DockerService, HealthReport};
use anyhow::Result;
use anyhow::anyhow;
use client_core::backup::{BackupManager, BackupOptions};
use client_core::config::AppConfig;
use client_core::constants::docker;
use client_core::container::DockerManager;
use client_core::database::BackupType;
use client_core::upgrade_strategy::UpgradeStrategy;
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

///创建备份,根据升级策略,做不同的备份逻辑
/// 检查Docker Compose 文件是否存在
pub(crate) fn validate_docker_compose_file(compose_path: &Path) -> Result<()> {
    if !compose_path.exists() {
        error!("❌ Docker Compose文件不存在: {}", compose_path.display());
        info!("💡 请先确保Docker服务已正确部署");
    }
    Ok(())
}

/// 展示运行状态的容器信息
fn display_running_containers(containers: &Vec<&&ContainerInfo>) {
    for container in containers {
        info!(
            "   - {} (状态: {}, restart: {})",
            container.name,
            container.status.display_name(),
            container.get_restart_display()
        );
    }
}

/// 展示容器状态摘要信息
fn display_container_summary(report: &HealthReport) {
    let running_containers = report.get_running_containers();
    let completed_containers = report.get_completed_containers();
    let failed_containers = report.get_failed_containers();

    let failed_containers: Vec<_> = failed_containers.iter().collect();

    // 分离不同类型的容器
    let persistent_running: Vec<_> = running_containers
        .iter()
        .filter(|c| c.is_persistent_service())
        .collect();
    let oneshot_running: Vec<_> = running_containers
        .iter()
        .filter(|c| c.is_oneshot())
        .collect();

    let oneshot_completed: Vec<_> = completed_containers
        .iter()
        .filter(|c| c.is_oneshot())
        .collect();
    let other_completed: Vec<_> = completed_containers
        .iter()
        .filter(|c| !c.is_oneshot())
        .collect();
    // 如果有持续运行的服务，显示警告
    if !persistent_running.is_empty() {
        warn!("⚠️  持续服务仍在运行中！");
        error!("❌ 冷备份要求持续运行的服务必须处于停止状态");

        info!("📝 发现 {} 个持续运行的服务:", persistent_running.len());
        display_running_containers(&persistent_running);

        if !oneshot_running.is_empty() {
            info!("🔄 忽略 {} 个运行中的一次性任务:", oneshot_running.len());
            display_running_containers(&oneshot_running);
        }

        info!("💡 请先停止持续运行的服务后再进行备份");
    } else {
        info!("✅ 所有持续服务已停止，可以进行备份");
    }

    // 显示其他已完成的容器
    if !oneshot_completed.is_empty() {
        info!(
            "🔄 忽略 {} 个已完成的一次性任务容器:",
            oneshot_completed.len()
        );
        display_running_containers(&oneshot_completed);
    }

    if !other_completed.is_empty() {
        info!("📝 发现 {} 个其他已完成容器:", other_completed.len());
        display_running_containers(&other_completed);
    }

    if !failed_containers.is_empty() {
        warn!(
            "⚠️  发现 {} 个失败的容器（不影响备份）:",
            failed_containers.len()
        );
        display_running_containers(&failed_containers);
    }
}
/// 检查Docker服务状态
pub(crate) async fn check_docker_service_running(
    app_config: Arc<AppConfig>,
    docker_manager: Arc<DockerManager>,
) -> Result<bool> {
    let docker_service = DockerService::new(app_config.clone(), docker_manager)?;
    let report = docker_service
        .health_check()
        .await
        .map_err(|e| anyhow!("检查Docker服务状态失败: {}", e))?;

    info!("📊 服务状态: {}", report.get_status_summary());
    display_container_summary(&report);

    // 检查是否有持续运行的服务
    let running_containers = report.get_running_containers();
    let persistent_running: Vec<_> = running_containers
        .iter()
        .filter(|c| c.is_persistent_service())
        .collect();

    let running_flag = !persistent_running.is_empty();

    Ok(running_flag)
}

/// 检查Docker服务状态
pub(crate) async fn check_docker_service_status(
    app_config: Arc<AppConfig>,
    docker_manager: Arc<DockerManager>,
) -> Result<()> {
    info!("🔍 检查Docker服务状态...");

    let runing_flag = check_docker_service_running(app_config, docker_manager).await?;

    if runing_flag {
        error!("有持续运行的服务，无法进行冷备份");
        return Err(anyhow!("有持续运行的服务，无法进行冷备份"));
    }

    Ok(())
}

/// 创建新的备份
async fn create_new_backup(app: &CliApp, change_files: Vec<PathBuf>) -> Result<()> {
    info!("🔄 开始创建备份...");

    //change_files 需要拼接 ./docker 目录的路径
    let work_dir = docker::get_docker_work_dir();
    let change_file_or_dir = change_files
        .iter()
        .map(|path| work_dir.join(path))
        .collect::<Vec<PathBuf>>();

    let mut need_backup_paths = vec![docker::get_data_dir_path(), docker::get_app_dir_path()];
    need_backup_paths.extend(change_file_or_dir);

    let backup_options = BackupOptions {
        backup_type: BackupType::Manual,
        service_version: app.config.get_docker_versions(),
        work_dir,
        source_paths: need_backup_paths,
        compression_level: 6,
    };

    let backup_manager = BackupManager::new(
        app.config.get_backup_dir(),
        app.database.clone(),
        app.docker_manager.clone(),
    )?;

    let backup_record = backup_manager.create_backup(backup_options).await?;
    info!("✅ 备份创建成功: {}", backup_record.file_path);
    info!("📝 备份ID: {}", backup_record.id);
    info!("📏 备份服务版本: {}", backup_record.service_version);

    Ok(())
}

/// 执行带升级策略的备份
pub async fn run_backup_with_upgrade_strategy(
    app: &CliApp,
    upgrade_strategy: UpgradeStrategy,
) -> Result<()> {
    // 验证Docker环境
    validate_docker_compose_file(Path::new(&app.config.docker.compose_file))?;

    // 检查服务状态
    check_docker_service_status(app.config.clone(), app.docker_manager.clone()).await?;

    // 创建备份
    let change_files = upgrade_strategy.get_changed_files();

    create_new_backup(app, change_files).await?;

    Ok(())
}

/// 创建备份
pub async fn run_backup(app: &CliApp) -> Result<()> {
    // 1. 检查Docker环境
    let compose_path = Path::new(&app.config.docker.compose_file);

    if !compose_path.exists() {
        error!("❌ Docker Compose文件不存在: {}", compose_path.display());
        info!("💡 请先确保Docker服务已正确部署");
        return Ok(());
    }

    // 2. 使用 DockerService 的 health_check 进行智能状态检查
    info!("🔍 检查Docker服务状态...");

    let docker_service = DockerService::new(app.config.clone(), app.docker_manager.clone())?;
    match docker_service.health_check().await {
        Ok(report) => {
            info!("📊 服务状态: {}", report.get_status_summary());

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
                warn!("⚠️  持续服务仍在运行中！");
                error!("❌ 冷备份要求持续运行的服务必须处于停止状态");

                info!(
                    "📝 发现 {} 个持续运行的服务:",
                    persistent_running_services.len()
                );
                for container in &persistent_running_services {
                    info!(
                        "   - {} (状态: {}, restart: {})",
                        container.name,
                        container.status.display_name(),
                        container.get_restart_display()
                    );
                }

                // 显示被忽略的一次性任务
                let oneshot_running_services: Vec<_> = running_containers
                    .iter()
                    .filter(|c| c.is_oneshot())
                    .collect();

                if !oneshot_running_services.is_empty() {
                    info!(
                        "📝 发现 {} 个运行中的一次性任务（已忽略）:",
                        oneshot_running_services.len()
                    );
                    for container in oneshot_running_services {
                        info!(
                            "   - {} (一次性任务，restart: {}, 不影响备份)",
                            container.name,
                            container.get_restart_display()
                        );
                    }
                }

                info!("💡 请先停止持续运行的服务后再进行备份");
                return Ok(());
            }

            // 成功：所有持续服务已停止
            info!("✅ 所有持续服务已停止，可以进行备份");

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
                    info!("🔄 忽略 {} 个一次性任务容器:", oneshot_completed.len());
                    for container in oneshot_completed {
                        info!(
                            "   - {} (状态: {}, restart: {})",
                            container.name,
                            container.status.display_name(),
                            container.get_restart_display()
                        );
                    }
                }

                if !other_completed.is_empty() {
                    info!("📝 发现 {} 个其他已完成容器:", other_completed.len());
                    for container in other_completed {
                        info!(
                            "   - {} (状态: {}, restart: {})",
                            container.name,
                            container.status.display_name(),
                            container.get_restart_display()
                        );
                    }
                }
            }

            if !failed_containers.is_empty() {
                warn!(
                    "⚠️  发现 {} 个失败的容器（不影响备份）:",
                    failed_containers.len()
                );
                for container in failed_containers {
                    warn!(
                        "   - {} (状态: {}, restart: {})",
                        container.name,
                        container.status.display_name(),
                        container.get_restart_display()
                    );
                }
            }
        }
        Err(e) => {
            error!("❌ 检查Docker服务状态失败: {}", e);
            info!("💡 无法确认服务状态，建议手动检查后再进行备份");
            return Ok(());
        }
    }

    // 3. 执行备份
    info!("🔄 开始创建备份...");

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
            info!("✅ 备份创建成功: {}", backup_record.file_path);
            info!("📝 备份ID: {}", backup_record.id);
            info!("📏 备份服务版本: {}", backup_record.service_version);
        }
        Err(e) => {
            error!("❌ 备份创建失败: {}", e);
            return Err(e);
        }
    }

    Ok(())
}

/// 列出备份
pub async fn run_list_backups(app: &CliApp) -> Result<()> {
    let backups = app.backup_manager.list_backups().await?;

    if backups.is_empty() {
        info!("📦 暂无备份记录");
        info!("💡 使用以下命令创建备份:");
        info!("   nuwax-cli backup");
        return Ok(());
    }

    info!("📦 备份列表");
    info!("============");

    // 统计信息
    let total_backups = backups.len();
    let mut valid_backups = 0;
    let mut invalid_backups = 0;
    let mut total_size = 0u64;

    // 详细信息表头
    info!(
        "{:<4} {:<12} {:<20} {:<10} {:<8} {:<12} {}",
        "ID", "类型", "创建时间", "版本", "状态", "大小", "文件路径"
    );
    info!("{}", "-".repeat(100));

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
                "未知".to_string()
            };

            ("✅ 可用", size)
        } else {
            invalid_backups += 1;
            ("❌ 文件缺失", "---".to_string())
        };

        // 备份类型显示
        let backup_type_display = match backup.backup_type {
            client_core::database::BackupType::Manual => "手动",
            client_core::database::BackupType::PreUpgrade => "升级前",
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
            warn!("     ⚠️  警告: 备份文件不存在，无法用于回滚！");
            warn!("         预期路径: {}", backup.file_path);
        }
    }

    info!("{}", "-".repeat(100));

    // 统计摘要
    info!("📊 备份统计:");
    info!("   总备份数: {}", total_backups);
    info!("   可用备份: {} ✅", valid_backups);
    if invalid_backups > 0 {
        warn!("   无效备份: {} ❌", invalid_backups);
    }

    if total_size > 0 {
        let total_size_display = if total_size > 1024 * 1024 * 1024 {
            format!("{:.2} GB", total_size as f64 / (1024.0 * 1024.0 * 1024.0))
        } else if total_size > 1024 * 1024 {
            format!("{:.2} MB", total_size as f64 / (1024.0 * 1024.0))
        } else {
            format!("{:.2} KB", total_size as f64 / 1024.0)
        };
        info!("   总大小: {}", total_size_display);
    }

    // 操作提示
    if valid_backups > 0 {
        info!("💡 可用操作:");
        info!("   - 交互式回滚: nuwax-cli rollback");
        info!("   - 指定ID回滚: nuwax-cli rollback <备份ID>");
        info!("   - 创建新备份: nuwax-cli backup");
    }

    if invalid_backups > 0 {
        warn!("⚠️  发现 {} 个无效备份（文件缺失）", invalid_backups);
        info!("💡 建议:");
        info!(
            "   - 检查备份目录设置: {}",
            app.config.get_backup_dir().display()
        );
        info!("   - 如果备份文件被误删，这些记录将无法用于恢复");
        info!("   - 可考虑手动清理这些无效记录");
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
                info!("操作已取消");
                return Ok(());
            }
        }
    };

    if !force {
        if rollback_data {
            warn!("⚠️  警告: 此操作将覆盖当前数据目录,Mysql,Redis等数据也会一起回滚!");
        } else {
            warn!("⚠️  警告: 此操作会回滚后端和前端应用版本,但不回滚Mysql,Redis等数据!");
        }

        use std::io::{self, Write};
        print!("请确认您要从备份 {selected_backup_id} 恢复数据 (y/N): ");
        io::stdout().flush()?;

        let mut input = String::new();
        std::io::stdin().read_line(&mut input)?;

        if input.trim().to_lowercase() != "y" {
            warn!("操作已取消");
            return Ok(());
        }
    }

    info!("开始数据回滚操作...");

    // 🔧 智能回滚
    if rollback_data {
        //data,app 等目录,全部恢复
        run_rollback_with_exculde(app, selected_backup_id, auto_start_service, &[]).await?;
    } else {
        info!("rollback_data 为 false, 不回滚 data 目录(mysql,redis等数据,不会回滚)");
        //data 数据目录不用恢复,回滚应用业务逻辑, 考虑改写: perform_selective_restore ,增加参数,用于排除 data 目录
        run_rollback_with_exculde(app, selected_backup_id, auto_start_service, &["data"]).await?;
    }

    info!("✅ 数据回滚完成");
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
                info!("操作已取消");
                return Ok(());
            }
        }
    };

    if !force {
        warn!("⚠️  警告: 此操作将覆盖当前 data 目录!");
        warn!("⚠️  注意: 此操作只恢复 data 目录，app 目录和配置文件将保持不变");

        use std::io::{self, Write};
        print!("请确认您要从备份 {selected_backup_id} 恢复 data 目录 (y/N): ");
        io::stdout().flush()?;

        let mut input = String::new();
        std::io::stdin().read_line(&mut input)?;

        if input.trim().to_lowercase() != "y" {
            warn!("操作已取消");
            return Ok(());
        }
    }

    info!("开始 data 目录回滚操作...");

    // 🔧 只回滚 data 目录：只恢复 data 目录，保留 app 目录和配置文件
    run_data_directory_only_rollback(app, selected_backup_id, auto_start_service, config_file)
        .await?;

    info!("✅ data 目录回滚完成");
    Ok(())
}

/// 交互式备份选择
async fn interactive_backup_selection(app: &CliApp) -> Result<Option<i64>> {
    info!("🗂️  备份选择");
    info!("============");

    let backups = app.backup_manager.list_backups().await?;

    if backups.is_empty() {
        warn!("❌ 没有可用的备份");
        info!("💡 请先创建备份:");
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
        warn!("❌ 没有可用的备份文件");
        info!("💡 所有备份文件都已丢失或损坏");
        return Ok(None);
    }

    // 显示备份选择列表
    info!("📋 可用备份列表:");
    info!(
        "{:<4} {:<12} {:<20} {:<10} {:<12} {}",
        "序号", "类型", "创建时间", "版本", "大小", "文件名"
    );
    info!("{}", "-".repeat(80));

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
            "未知".to_string()
        };

        // 备份类型显示
        let backup_type_display = match backup.backup_type {
            client_core::database::BackupType::Manual => "手动",
            client_core::database::BackupType::PreUpgrade => "升级前",
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

    info!("{}", "-".repeat(80));
    info!("💡 输入说明:");
    info!("   - 输入序号 (1-{}) 选择要恢复的备份", valid_backups.len());
    info!("   - 输入 'q' 或 'quit' 退出");
    info!("   - 输入 'l' 或 'list' 重新显示列表");

    // 交互式选择循环
    use std::io::{self, Write};
    loop {
        print!("\n请选择要恢复的备份 (1-{}/q/l): ", valid_backups.len());
        io::stdout().flush()?;

        let mut input = String::new();
        io::stdin().read_line(&mut input)?;
        let input = input.trim();

        // 处理退出命令
        if input.is_empty() || input.eq_ignore_ascii_case("q") || input.eq_ignore_ascii_case("quit")
        {
            info!("👋 操作已取消");
            return Ok(None);
        }

        // 处理重新显示列表
        if input.eq_ignore_ascii_case("l") || input.eq_ignore_ascii_case("list") {
            info!("\n📋 重新显示备份列表:");
            info!(
                "{:<4} {:<12} {:<20} {:<10} {:<12} {}",
                "序号", "类型", "创建时间", "版本", "大小", "文件名"
            );
            info!("{}", "-".repeat(80));

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
                    "未知".to_string()
                };

                let backup_type_display = match backup.backup_type {
                    client_core::database::BackupType::Manual => "手动",
                    client_core::database::BackupType::PreUpgrade => "升级前",
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
            info!("{}", "-".repeat(80));
            continue;
        }

        // 处理数字选择
        match input.parse::<usize>() {
            Ok(selection) => {
                if selection >= 1 && selection <= valid_backups.len() {
                    let selected_backup = valid_backups[selection - 1];

                    // 显示选择确认
                    info!("✅ 您选择了备份:");
                    info!("   备份ID: {}", selected_backup.id);
                    info!(
                        "   类型: {}",
                        match selected_backup.backup_type {
                            client_core::database::BackupType::Manual => "手动",
                            client_core::database::BackupType::PreUpgrade => "升级前",
                        }
                    );
                    info!(
                        "   创建时间: {}",
                        selected_backup.created_at.format("%Y-%m-%d %H:%M:%S")
                    );
                    info!("   服务版本: {}", selected_backup.service_version);
                    info!("   文件路径: {}", selected_backup.file_path);

                    return Ok(Some(selected_backup.id));
                } else {
                    warn!("❌ 无效的选择，请输入 1-{} 之间的数字", valid_backups.len());
                }
            }
            Err(_) => {
                warn!("❌ 无效的输入，请输入数字、'q'(退出) 或 'l'(重新显示列表)");
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
    info!("🛡️ 使用智能数据回滚模式");
    info!("   📁 将恢复: data/, app/ 目录");
    info!("   🔧 将保留: docker-compose.yml, .env 等配置文件");
    info!("   不恢复的目录:{:?}", dirs_to_exculde);

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
            info!("✅ 智能数据恢复完成");

            // 设置正确的权限
            let mysql_data_dir = docker_dir.join("data/mysql");
            if mysql_data_dir.exists() {
                #[cfg(unix)]
                {
                    use std::os::unix::fs::PermissionsExt;
                    let permissions = std::fs::Permissions::from_mode(0o775);
                    if let Err(e) = std::fs::set_permissions(&mysql_data_dir, permissions) {
                        warn!("⚠️ 设置MySQL权限失败: {}", e);
                    } else {
                        info!("🔒 已设置MySQL数据目录权限为775");
                    }
                }
            }

            info!("💡 数据恢复说明:");
            info!("   ✅ 所有数据库数据已恢复");
            info!("   ✅ 所有应用程序文件已恢复");
            info!("   ✅ 配置文件保持最新版本");

            if auto_start_service {
                info!("   ✅ Docker服务已自动启动");
            } else {
                info!("   📝 Docker服务启动已跳过（由上级流程控制）");
            }
        }
        Err(e) => {
            error!("❌ 数据恢复失败: {}", e);
            warn!("💡 建议操作:");
            warn!("   1. 检查备份文件是否存在且完整");
            warn!("   2. 确保有足够的磁盘空间");
            warn!("   3. 手动启动服务: nuwax-cli docker-service start");
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
    info!("🛡️ 使用智能 data 目录回滚模式");
    info!("   📁 将恢复: data/ 目录");
    info!("   🔧 将保留: app/ 目录, docker-compose.yml, .env 等配置文件");

    // 使用 BackupManager 的智能数据恢复功能
    let docker_dir = std::path::Path::new("./docker");

    // 如果有自定义配置文件，创建新的 DockerManager
    let backup_manager = if let Some(config_path) = config_file {
        info!("📄 使用自定义配置文件进行恢复: {}", config_path.display());

        // 获取对应的 .env 文件路径
        let env_file = config_path.with_file_name(".env");
        let custom_docker_manager = Arc::new(
            client_core::container::DockerManager::with_project(
                config_path.clone(),
                env_file.clone(),
                None,
            )
            .map_err(|e| anyhow::anyhow!("创建自定义DockerManager失败: {}", e))?,
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
            info!("✅ 智能 data 目录恢复完成");

            // 设置正确的权限
            let mysql_data_dir = docker_dir.join("data/mysql");
            if mysql_data_dir.exists() {
                #[cfg(unix)]
                {
                    use std::os::unix::fs::PermissionsExt;
                    let permissions = std::fs::Permissions::from_mode(0o775);
                    if let Err(e) = std::fs::set_permissions(&mysql_data_dir, permissions) {
                        warn!("⚠️ 设置MySQL权限失败: {}", e);
                    } else {
                        info!("🔒 已设置MySQL数据目录权限为775");
                    }
                }
            }

            info!("💡 数据恢复说明:");
            info!("   ✅ 所有数据库数据已恢复");
            info!("   ✅ app 目录保持原状");
            info!("   ✅ 配置文件保持最新版本");

            if auto_start_service {
                info!("   ✅ Docker服务已自动启动");
            } else {
                info!("   📝 Docker服务启动已跳过（由上级流程控制）");
            }
        }
        Err(e) => {
            error!("❌ data 目录恢复失败: {}", e);
            warn!("💡 建议操作:");
            warn!("   1. 检查备份文件是否存在且完整");
            warn!("   2. 确保有足够的磁盘空间");
            warn!("   3. 手动启动服务: nuwax-cli docker-service start");
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
                        error: Some(format!("JSON 序列化失败: {e}")),
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
