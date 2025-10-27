use crate::{
    api::ApiClient,
    config::AppConfig,
    database::Database,
    upgrade_strategy::{UpgradeStrategy, UpgradeStrategyManager},
};
use anyhow::Result;
use std::{path::PathBuf, sync::Arc};
use tracing::{debug, info};

/// 升级管理器
#[derive(Debug, Clone)]
pub struct UpgradeManager {
    config: Arc<AppConfig>,
    #[allow(dead_code)]
    config_path: PathBuf,
    api_client: Arc<ApiClient>,
    #[allow(dead_code)]
    database: Arc<Database>,
}

/// 升级选项
#[derive(Debug, Clone, Default)]
pub struct UpgradeOptions {
    pub skip_backup: bool,
    pub force: bool,
    pub use_incremental: bool,
    pub backup_dir: Option<PathBuf>,
    pub download_only: bool,
}

pub type ProgressCallback = Box<dyn Fn(UpgradeStep, &str) + Send + Sync>;

#[derive(Debug, Clone)]
pub enum UpgradeStep {
    CheckingUpdates,
    CreatingBackup,
    StoppingServices,
    DownloadingUpdate,
    ExtractingUpdate,
    LoadingImages,
    StartingServices,
    VerifyingServices,
    CleaningUp,
    Completed,
    Failed(String),
}

#[derive(Debug)]
pub struct UpgradeResult {
    pub success: bool,
    pub from_version: String,
    pub to_version: String,
    pub error: Option<String>,
    pub backup_id: Option<i64>,
}

impl UpgradeManager {
    pub fn new(
        config: Arc<AppConfig>,
        config_path: PathBuf,
        api_client: Arc<ApiClient>,
        database: Arc<Database>,
    ) -> Self {
        Self {
            config,
            config_path,
            api_client,
            database,
        }
    }

    /// 检查docker应用升级策略
    pub async fn check_for_updates(&self, force_full: bool) -> Result<UpgradeStrategy> {
        info!("检查服务更新...");
        let current_version = &self.config.get_docker_versions();
        debug!("当前版本: {}", current_version);
        let enhanced_service_manifest = self.api_client.get_enhanced_service_manifest().await?;

        let upgrade_strategy_manager = UpgradeStrategyManager::new(
            current_version.to_string(),
            force_full,
            enhanced_service_manifest,
        );
        let upgrade_strategy: UpgradeStrategy = upgrade_strategy_manager.determine_strategy()?;

        Ok(upgrade_strategy)
    }
}
