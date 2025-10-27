use anyhow::Result;
use client_core::{
    api::ApiClient, authenticated_client::AuthenticatedClient, backup::BackupManager,
    config::AppConfig, constants::config, container::DockerManager, database::Database,
    upgrade::UpgradeManager,
};
use log::info;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use crate::cli::Commands;
use crate::commands;
use tracing::debug;

#[derive(Clone)]
pub struct CliApp {
    pub config: Arc<AppConfig>,
    pub database: Arc<Database>,
    pub api_client: Arc<ApiClient>,
    pub authenticated_client: Arc<AuthenticatedClient>,
    pub docker_manager: Arc<DockerManager>,
    pub backup_manager: Arc<BackupManager>,
    pub upgrade_manager: Arc<UpgradeManager>,
}

impl CliApp {
    /// 使用智能配置查找初始化CLI应用
    pub async fn new_with_auto_config() -> Result<Self> {
        let config = Arc::new(AppConfig::find_and_load_config()?);

        Self::new_with_config(config).await
    }

    /// 使用指定配置文件路径初始化CLI应用
    pub async fn new_with_config_path<P: AsRef<Path>>(config_path: P) -> Result<Self> {
        let config_path = config_path.as_ref();
        let config = if config_path.exists() {
            Arc::new(AppConfig::load_from_file(config_path)?)
        } else {
            // 如果指定的配置文件不存在，尝试智能查找
            Arc::new(AppConfig::find_and_load_config()?)
        };

        Self::new_with_config(config).await
    }

    /// 使用配置初始化CLI应用
    async fn new_with_config(config: Arc<AppConfig>) -> Result<Self> {
        // 确保缓存目录存在
        config.ensure_cache_dirs()?;

        // 初始化数据库
        let db_path = config::get_database_path();
        let database = Arc::new(Database::connect(&db_path).await?);
        debug!("数据库连接成功: {}", db_path.display());

        // 检查数据库是否已经初始化
        if !database.is_database_initialized().await? {
            return Err(anyhow::anyhow!(
                "数据库未初始化。请先运行 'nuwax-cli init' 命令来初始化数据库。".to_string(),
            ));
        }

        // 创建认证客户端（自动处理注册和认证）
        let server_base_url = client_core::constants::api::DEFAULT_BASE_URL.to_string();
        let authenticated_client =
            Arc::new(AuthenticatedClient::new(database.clone(), server_base_url).await?);

        // 获取用于API请求的客户端ID（只使用服务端返回的client_id）
        let client_id = database.get_api_client_id().await?;
        let api_client = Arc::new(ApiClient::new(
            client_id.clone(),
            Some(authenticated_client.clone()),
        ));

        // 创建其他管理器
        let docker_manager = Arc::new(DockerManager::new(
            PathBuf::from(&config.docker.compose_file),
            PathBuf::from(&config.docker.env_file),
        )?);

        let backup_manager = Arc::new(BackupManager::new(
            PathBuf::from(&config.backup.storage_dir),
            database.clone(),
            docker_manager.clone(),
        )?);
        let upgrade_manager = Arc::new(UpgradeManager::new(
            config.clone(),
            PathBuf::from("config.toml"), // 使用默认配置路径
            api_client.clone(),
            database.clone(),
        ));

        Ok(Self {
            config,
            database,
            api_client,
            authenticated_client,
            docker_manager,
            backup_manager,
            upgrade_manager,
        })
    }

    /// 运行应用命令
    pub async fn run_command(&mut self, command: Commands) -> Result<()> {
        match command {
            Commands::Status => commands::run_status(self).await,
            Commands::ApiInfo => commands::run_api_info(self).await,
            Commands::Init { .. } => unreachable!(), // 已经在 main.rs 中处理
            Commands::CheckUpdate(check_update_cmd) => {
                commands::handle_check_update_command(check_update_cmd)
                    .await
                    .map_err(|e| anyhow::anyhow!(format!("检查更新失败: {e}")))
            }
            Commands::Upgrade { args } => {
                commands::run_upgrade(self, args)
                    .await
                    .map_err(|e| client_core::error::DuckError::custom(format!("升级失败: {e}")))?;
                Ok(())
            }
            Commands::Backup => commands::run_backup(self).await,
            Commands::ListBackups => commands::run_list_backups(self).await,
            Commands::Rollback {
                backup_id,
                force,
                list_json,
                rollback_data,
            } => {
                commands::backup::run_rollback(
                    self,
                    backup_id,
                    force,
                    list_json,
                    true,
                    rollback_data,
                )
                .await
            }
            Commands::RollbackDataOnly { backup_id, force } => {
                commands::backup::run_rollback_data_only(self, backup_id, force, true, None).await
            }
            Commands::DockerService(docker_cmd) => {
                commands::run_docker_service_command(self, docker_cmd).await
            }
            Commands::Ducker { args } => commands::run_ducker(args).await,
            Commands::AutoBackup(auto_backup_cmd) => {
                commands::handle_auto_backup(self, &auto_backup_cmd).await
            }
            Commands::AutoUpgradeDeploy(auto_upgrade_deploy_cmd) => {
                commands::handle_auto_upgrade_deploy_command(self, auto_upgrade_deploy_cmd).await
            }
            Commands::Cache(cache_cmd) => commands::handle_cache_command(self, cache_cmd).await,
            Commands::DiffSql {
                old_sql,
                new_sql,
                old_version,
                new_version,
                output,
            } => commands::run_diff_sql(old_sql, new_sql, old_version, new_version, output).await,
        }
    }
}
