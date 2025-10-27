use crate::DuckError;
use anyhow::Result;
use chrono::{DateTime, Utc};
use std::path::Path;
use tokio::sync::{mpsc, oneshot};
use tracing::debug;
use uuid::Uuid;

use super::actor::DuckDbActor;
use super::messages::{AppStateRecord, DbMessage, DownloadTaskRecord, UserActionRecord};
use super::models::{BackupRecord, ScheduledTask};

/// DuckDB数据库管理器
#[derive(Debug, Clone)]
pub struct DuckDbManager {
    sender: mpsc::Sender<DbMessage>,
}

impl DuckDbManager {
    /// 创建新的DuckDB管理器（不自动初始化表）
    pub async fn new<P: AsRef<Path>>(db_path: P) -> Result<Self> {
        let db_path = db_path.as_ref().to_path_buf();

        // 确保数据库文件的父目录存在
        if let Some(parent) = db_path.parent() {
            tokio::fs::create_dir_all(parent).await?;
        }

        let (sender, receiver) = mpsc::channel(100);

        // 启动DuckDB Actor
        let actor = DuckDbActor::new(db_path)?;
        tokio::spawn(actor.run(receiver));

        let manager = Self { sender };

        // 移除自动初始化 - 只有在明确调用 init_database 时才初始化
        // manager.init_tables().await?;

        Ok(manager)
    }

    /// 创建内存数据库管理器（不自动初始化表）
    pub async fn new_memory() -> Result<Self> {
        let (sender, receiver) = mpsc::channel(100);

        // 启动DuckDB Actor（内存模式）
        let actor = DuckDbActor::new_memory()?;
        tokio::spawn(actor.run(receiver));

        let manager = Self { sender };

        // 移除自动初始化 - 只有在明确调用 init_database 时才初始化
        // manager.init_tables().await?;

        Ok(manager)
    }

    /// 显式初始化数据库（只应在 nuwax-cli init 时调用）
    pub async fn init_database(&self) -> Result<()> {
        debug!("显式初始化数据库表...");
        self.init_tables().await?;
        debug!("数据库初始化完成");
        Ok(())
    }

    /// 检查数据库是否已经初始化
    pub async fn is_database_initialized(&self) -> Result<bool> {
        // 检查关键配置表是否存在且有数据
        match self.get_config("db_initialized").await {
            Ok(Some(value)) => Ok(value == "true"),
            Ok(None) => Ok(false),
            Err(_) => Ok(false), // 如果出错，认为未初始化
        }
    }

    /// 标记数据库为已初始化
    pub async fn mark_database_initialized(&self) -> Result<()> {
        self.set_config("db_initialized", "true").await
    }

    /// 初始化数据库表（私有方法）
    async fn init_tables(&self) -> Result<()> {
        let (respond_to, receiver) = oneshot::channel();

        self.sender
            .send(DbMessage::InitTables { respond_to })
            .await
            .map_err(|_| DuckError::Custom("数据库Actor已关闭".to_string()))?;

        receiver
            .await
            .map_err(|_| DuckError::Custom("等待数据库响应超时".to_string()))?
    }

    /// 获取配置值
    pub async fn get_config(&self, key: &str) -> Result<Option<String>> {
        let (respond_to, receiver) = oneshot::channel();

        self.sender
            .send(DbMessage::GetConfig {
                key: key.to_string(),
                respond_to,
            })
            .await
            .map_err(|_| DuckError::Custom("数据库Actor已关闭".to_string()))?;

        receiver
            .await
            .map_err(|_| DuckError::Custom("等待数据库响应超时".to_string()))?
    }

    /// 设置配置值
    pub async fn set_config(&self, key: &str, value: &str) -> Result<()> {
        let (respond_to, receiver) = oneshot::channel();

        self.sender
            .send(DbMessage::SetConfig {
                key: key.to_string(),
                value: value.to_string(),
                respond_to,
            })
            .await
            .map_err(|_| DuckError::Custom("数据库Actor已关闭".to_string()))?;

        receiver
            .await
            .map_err(|_| DuckError::Custom("等待数据库响应超时".to_string()))?
    }

    /// 获取或创建客户端 UUID
    pub async fn get_or_create_client_uuid(&self) -> Result<Uuid> {
        const CLIENT_UUID_KEY: &str = "client_uuid";

        // 尝试从数据库获取现有的UUID
        if let Some(uuid_str) = self.get_config(CLIENT_UUID_KEY).await? {
            if let Ok(uuid) = Uuid::parse_str(&uuid_str) {
                return Ok(uuid);
            }
        }

        // 生成新的UUID并保存
        let new_uuid = Uuid::new_v4();
        self.set_config(CLIENT_UUID_KEY, &new_uuid.to_string())
            .await?;

        Ok(new_uuid)
    }

    // ========== 下载任务管理 ==========

    /// 创建下载任务
    pub async fn create_download_task(
        &self,
        task_name: String,
        download_url: String,
        total_size: i64,
        target_path: String,
        file_hash: Option<String>,
    ) -> Result<i64> {
        let (respond_to, receiver) = oneshot::channel();

        self.sender
            .send(DbMessage::CreateDownloadTask {
                task_name,
                download_url,
                total_size,
                target_path,
                file_hash,
                respond_to,
            })
            .await
            .map_err(|_| DuckError::Custom("数据库Actor已关闭".to_string()))?;

        receiver
            .await
            .map_err(|_| DuckError::Custom("等待数据库响应超时".to_string()))?
    }

    /// 更新下载任务状态
    pub async fn update_download_task_status(
        &self,
        task_id: i64,
        status: &str,
        downloaded_size: Option<i64>,
        error_message: Option<String>,
    ) -> Result<()> {
        let (respond_to, receiver) = oneshot::channel();

        self.sender
            .send(DbMessage::UpdateDownloadTaskStatus {
                task_id,
                status: status.to_string(),
                downloaded_size,
                error_message,
                respond_to,
            })
            .await
            .map_err(|_| DuckError::Custom("数据库Actor已关闭".to_string()))?;

        receiver
            .await
            .map_err(|_| DuckError::Custom("等待数据库响应超时".to_string()))?
    }

    /// 完成下载任务
    pub async fn complete_download_task(
        &self,
        task_id: i64,
        average_speed: Option<i64>,
        total_duration: Option<i32>,
    ) -> Result<()> {
        let (respond_to, receiver) = oneshot::channel();

        self.sender
            .send(DbMessage::CompleteDownloadTask {
                task_id,
                average_speed,
                total_duration,
                respond_to,
            })
            .await
            .map_err(|_| DuckError::Custom("数据库Actor已关闭".to_string()))?;

        receiver
            .await
            .map_err(|_| DuckError::Custom("等待数据库响应超时".to_string()))?
    }

    /// 获取下载任务
    pub async fn get_download_task(&self, task_id: i64) -> Result<Option<DownloadTaskRecord>> {
        let (respond_to, receiver) = oneshot::channel();

        self.sender
            .send(DbMessage::GetDownloadTask {
                task_id,
                respond_to,
            })
            .await
            .map_err(|_| DuckError::Custom("数据库Actor已关闭".to_string()))?;

        receiver
            .await
            .map_err(|_| DuckError::Custom("等待数据库响应超时".to_string()))?
    }

    /// 获取活跃的下载任务
    pub async fn get_active_download_tasks(&self) -> Result<Vec<DownloadTaskRecord>> {
        let (respond_to, receiver) = oneshot::channel();

        self.sender
            .send(DbMessage::GetActiveDownloadTasks { respond_to })
            .await
            .map_err(|_| DuckError::Custom("数据库Actor已关闭".to_string()))?;

        receiver
            .await
            .map_err(|_| DuckError::Custom("等待数据库响应超时".to_string()))?
    }

    // ========== 应用状态管理 ==========

    /// 更新应用状态
    pub async fn update_app_state(
        &self,
        state: &str,
        state_data: Option<String>,
        error_message: Option<String>,
    ) -> Result<()> {
        let (respond_to, receiver) = oneshot::channel();

        self.sender
            .send(DbMessage::UpdateAppState {
                state: state.to_string(),
                state_data,
                error_message,
                respond_to,
            })
            .await
            .map_err(|_| DuckError::Custom("数据库Actor已关闭".to_string()))?;

        receiver
            .await
            .map_err(|_| DuckError::Custom("等待数据库响应超时".to_string()))?
    }

    /// 获取当前应用状态
    pub async fn get_app_state(&self) -> Result<Option<AppStateRecord>> {
        let (respond_to, receiver) = oneshot::channel();

        self.sender
            .send(DbMessage::GetAppState { respond_to })
            .await
            .map_err(|_| DuckError::Custom("数据库Actor已关闭".to_string()))?;

        receiver
            .await
            .map_err(|_| DuckError::Custom("等待数据库响应超时".to_string()))?
    }

    // ========== 用户操作历史 ==========

    /// 记录用户操作
    pub async fn record_user_action(
        &self,
        action_type: &str,
        action_description: &str,
        action_params: Option<String>,
    ) -> Result<i64> {
        let (respond_to, receiver) = oneshot::channel();

        self.sender
            .send(DbMessage::RecordUserAction {
                action_type: action_type.to_string(),
                action_description: action_description.to_string(),
                action_params,
                respond_to,
            })
            .await
            .map_err(|_| DuckError::Custom("数据库Actor已关闭".to_string()))?;

        receiver
            .await
            .map_err(|_| DuckError::Custom("等待数据库响应超时".to_string()))?
    }

    /// 完成用户操作
    pub async fn complete_user_action(
        &self,
        action_id: i64,
        status: &str,
        result_message: Option<String>,
        duration_seconds: Option<i32>,
    ) -> Result<()> {
        let (respond_to, receiver) = oneshot::channel();

        self.sender
            .send(DbMessage::CompleteUserAction {
                action_id,
                status: status.to_string(),
                result_message,
                duration_seconds,
                respond_to,
            })
            .await
            .map_err(|_| DuckError::Custom("数据库Actor已关闭".to_string()))?;

        receiver
            .await
            .map_err(|_| DuckError::Custom("等待数据库响应超时".to_string()))?
    }

    /// 获取用户操作历史
    pub async fn get_user_actions(&self, limit: Option<i32>) -> Result<Vec<UserActionRecord>> {
        let (respond_to, receiver) = oneshot::channel();

        self.sender
            .send(DbMessage::GetUserActions { limit, respond_to })
            .await
            .map_err(|_| DuckError::Custom("数据库Actor已关闭".to_string()))?;

        receiver
            .await
            .map_err(|_| DuckError::Custom("等待数据库响应超时".to_string()))?
    }

    // ========== 现有的备份和任务管理 ==========

    /// 创建备份记录
    pub async fn create_backup_record(
        &self,
        file_path: String,
        service_version: String,
        backup_type: &str,
        status: &str,
    ) -> Result<i64> {
        let (respond_to, receiver) = oneshot::channel();

        self.sender
            .send(DbMessage::CreateBackupRecord {
                file_path,
                service_version,
                backup_type: backup_type.to_string(),
                status: status.to_string(),
                respond_to,
            })
            .await
            .map_err(|_| DuckError::Custom("数据库Actor已关闭".to_string()))?;

        receiver
            .await
            .map_err(|_| DuckError::Custom("等待数据库响应超时".to_string()))?
    }

    /// 获取所有备份记录
    pub async fn get_all_backups(&self) -> Result<Vec<BackupRecord>> {
        let (respond_to, receiver) = oneshot::channel();

        self.sender
            .send(DbMessage::GetAllBackups { respond_to })
            .await
            .map_err(|_| DuckError::Custom("数据库Actor已关闭".to_string()))?;

        receiver
            .await
            .map_err(|_| DuckError::Custom("等待数据库响应超时".to_string()))?
    }

    /// 根据ID获取备份记录
    pub async fn get_backup_by_id(&self, id: i64) -> Result<Option<BackupRecord>> {
        let (respond_to, receiver) = oneshot::channel();

        self.sender
            .send(DbMessage::GetBackupById { id, respond_to })
            .await
            .map_err(|_| DuckError::Custom("数据库Actor已关闭".to_string()))?;

        receiver
            .await
            .map_err(|_| DuckError::Custom("等待数据库响应超时".to_string()))?
    }

    /// 删除备份记录
    pub async fn delete_backup_record(&self, backup_id: i64) -> Result<()> {
        let (respond_to, receiver) = oneshot::channel();

        self.sender
            .send(DbMessage::DeleteBackupRecord {
                backup_id,
                respond_to,
            })
            .await
            .map_err(|_| DuckError::Custom("数据库Actor已关闭".to_string()))?;

        receiver
            .await
            .map_err(|_| DuckError::Custom("等待数据库响应超时".to_string()))?
    }

    /// 更新备份文件路径
    pub async fn update_backup_file_path(&self, backup_id: i64, new_path: String) -> Result<()> {
        let (respond_to, receiver) = oneshot::channel();

        self.sender
            .send(DbMessage::UpdateBackupFilePath {
                backup_id,
                new_path,
                respond_to,
            })
            .await
            .map_err(|_| DuckError::Custom("数据库Actor已关闭".to_string()))?;

        receiver
            .await
            .map_err(|_| DuckError::Custom("等待数据库响应超时".to_string()))?
    }

    /// 创建计划任务
    pub async fn create_scheduled_task(
        &self,
        task_type: &str,
        target_version: String,
        scheduled_at: DateTime<Utc>,
    ) -> Result<i64> {
        // 取消同类型的待执行任务
        self.cancel_pending_tasks(task_type).await?;

        let (respond_to, receiver) = oneshot::channel();

        self.sender
            .send(DbMessage::CreateScheduledTask {
                task_type: task_type.to_string(),
                target_version,
                scheduled_at,
                respond_to,
            })
            .await
            .map_err(|_| DuckError::Custom("数据库Actor已关闭".to_string()))?;

        receiver
            .await
            .map_err(|_| DuckError::Custom("等待数据库响应超时".to_string()))?
    }

    /// 获取待执行任务
    pub async fn get_pending_tasks(&self) -> Result<Vec<ScheduledTask>> {
        let (respond_to, receiver) = oneshot::channel();

        self.sender
            .send(DbMessage::GetPendingTasks { respond_to })
            .await
            .map_err(|_| DuckError::Custom("数据库Actor已关闭".to_string()))?;

        receiver
            .await
            .map_err(|_| DuckError::Custom("等待数据库响应超时".to_string()))?
    }

    /// 更新任务状态
    pub async fn update_task_status(
        &self,
        task_id: i64,
        status: &str,
        details: Option<String>,
    ) -> Result<()> {
        let (respond_to, receiver) = oneshot::channel();

        self.sender
            .send(DbMessage::UpdateTaskStatus {
                task_id,
                status: status.to_string(),
                details,
                respond_to,
            })
            .await
            .map_err(|_| DuckError::Custom("数据库Actor已关闭".to_string()))?;

        receiver
            .await
            .map_err(|_| DuckError::Custom("等待数据库响应超时".to_string()))?
    }

    /// 取消待执行任务
    async fn cancel_pending_tasks(&self, task_type: &str) -> Result<()> {
        let (respond_to, receiver) = oneshot::channel();

        self.sender
            .send(DbMessage::CancelPendingTasks {
                task_type: task_type.to_string(),
                respond_to,
            })
            .await
            .map_err(|_| DuckError::Custom("数据库Actor已关闭".to_string()))?;

        receiver
            .await
            .map_err(|_| DuckError::Custom("等待数据库响应超时".to_string()))?
    }
}
