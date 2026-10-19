use crate::{
    api::ApiClient,
    config::{AppConfig, DockerChannel},
    database::Database,
    upgrade_strategy::{DownloadType, UpgradeStrategy, UpgradeStrategyManager},
};
use anyhow::Context as _;
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
    ///
    /// `channel` 决定读取哪条发布通道的清单：命令决定通道，不读取本机配置，
    /// 这样离线命令在 registry 部署过的机器上依然能取到离线全量清单。
    pub async fn check_for_updates(
        &self,
        force_full: bool,
        channel: DockerChannel,
    ) -> Result<UpgradeStrategy> {
        info!("Checking for service updates...");
        let current_version = &self.config.get_docker_versions();
        debug!("Current version: {}", current_version);
        let enhanced_service_manifest = self
            .api_client
            .get_service_manifest(channel)
            .await
            .with_context(|| format!("Failed to read the {} service manifest", channel.as_str()))?;

        let upgrade_strategy_manager = UpgradeStrategyManager::new(
            current_version.to_string(),
            force_full,
            enhanced_service_manifest,
            full_download_type(channel),
        );
        let upgrade_strategy: UpgradeStrategy = upgrade_strategy_manager.determine_strategy()?;

        Ok(upgrade_strategy)
    }
}

/// 全量包所属通道 → 下载类型。两条通道会为同一版本号发布不同的包，下载目录与
/// 缓存收据必须分开，否则瘦包会覆盖离线包并让另一条通道读到错误的收据。
fn full_download_type(channel: DockerChannel) -> DownloadType {
    match channel {
        DockerChannel::Images => DownloadType::Full,
        DockerChannel::Registry => DownloadType::Registry,
    }
}
