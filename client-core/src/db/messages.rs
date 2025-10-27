use chrono::{DateTime, Utc};
use tokio::sync::oneshot;

use anyhow::Result;

use super::models::{BackupRecord, ScheduledTask};

/// DuckDB数据库操作消息
#[derive(Debug)]
pub enum DbMessage {
    /// 初始化数据库表
    InitTables {
        respond_to: oneshot::Sender<Result<()>>,
    },
    /// 获取配置值
    GetConfig {
        key: String,
        respond_to: oneshot::Sender<Result<Option<String>>>,
    },
    /// 设置配置值
    SetConfig {
        key: String,
        value: String,
        respond_to: oneshot::Sender<Result<()>>,
    },

    // ========== 下载任务管理 ==========
    /// 创建下载任务
    CreateDownloadTask {
        task_name: String,
        download_url: String,
        total_size: i64,
        target_path: String,
        file_hash: Option<String>,
        respond_to: oneshot::Sender<Result<i64>>,
    },
    /// 更新下载任务状态
    UpdateDownloadTaskStatus {
        task_id: i64,
        status: String,
        downloaded_size: Option<i64>,
        error_message: Option<String>,
        respond_to: oneshot::Sender<Result<()>>,
    },
    /// 完成下载任务
    CompleteDownloadTask {
        task_id: i64,
        average_speed: Option<i64>,
        total_duration: Option<i32>,
        respond_to: oneshot::Sender<Result<()>>,
    },
    /// 获取下载任务
    GetDownloadTask {
        task_id: i64,
        respond_to: oneshot::Sender<Result<Option<DownloadTaskRecord>>>,
    },
    /// 获取活跃的下载任务
    GetActiveDownloadTasks {
        respond_to: oneshot::Sender<Result<Vec<DownloadTaskRecord>>>,
    },

    // ========== 应用状态管理 ==========
    /// 更新应用状态
    UpdateAppState {
        state: String,
        state_data: Option<String>, // JSON格式
        error_message: Option<String>,
        respond_to: oneshot::Sender<Result<()>>,
    },
    /// 获取当前应用状态
    GetAppState {
        respond_to: oneshot::Sender<Result<Option<AppStateRecord>>>,
    },

    // ========== 用户操作历史 ==========
    /// 记录用户操作
    RecordUserAction {
        action_type: String,
        action_description: String,
        action_params: Option<String>, // JSON格式
        respond_to: oneshot::Sender<Result<i64>>,
    },
    /// 完成用户操作
    CompleteUserAction {
        action_id: i64,
        status: String,
        result_message: Option<String>,
        duration_seconds: Option<i32>,
        respond_to: oneshot::Sender<Result<()>>,
    },
    /// 获取用户操作历史
    GetUserActions {
        limit: Option<i32>,
        respond_to: oneshot::Sender<Result<Vec<UserActionRecord>>>,
    },

    // ========== 现有的备份和任务管理 ==========
    /// 创建备份记录
    CreateBackupRecord {
        file_path: String,
        service_version: String,
        backup_type: String,
        status: String,
        respond_to: oneshot::Sender<Result<i64>>,
    },
    /// 获取所有备份记录
    GetAllBackups {
        respond_to: oneshot::Sender<Result<Vec<BackupRecord>>>,
    },
    /// 根据ID获取备份记录
    GetBackupById {
        id: i64,
        respond_to: oneshot::Sender<Result<Option<BackupRecord>>>,
    },
    /// 删除备份记录
    DeleteBackupRecord {
        backup_id: i64,
        respond_to: oneshot::Sender<Result<()>>,
    },
    /// 更新备份文件路径
    UpdateBackupFilePath {
        backup_id: i64,
        new_path: String,
        respond_to: oneshot::Sender<Result<()>>,
    },
    /// 创建计划任务
    CreateScheduledTask {
        task_type: String,
        target_version: String,
        scheduled_at: DateTime<Utc>,
        respond_to: oneshot::Sender<Result<i64>>,
    },
    /// 获取待执行任务
    GetPendingTasks {
        respond_to: oneshot::Sender<Result<Vec<ScheduledTask>>>,
    },
    /// 更新任务状态
    UpdateTaskStatus {
        task_id: i64,
        status: String,
        details: Option<String>,
        respond_to: oneshot::Sender<Result<()>>,
    },
    /// 取消待执行任务
    CancelPendingTasks {
        task_type: String,
        respond_to: oneshot::Sender<Result<()>>,
    },
}

/// 下载任务记录
#[derive(Debug, Clone)]
pub struct DownloadTaskRecord {
    pub id: i64,
    pub task_name: String,
    pub download_url: String,
    pub total_size: i64,
    pub downloaded_size: i64,
    pub target_path: String,
    pub file_hash: Option<String>,
    pub status: String,
    pub error_message: Option<String>,
    pub retry_count: i32,
    pub average_speed: i64,
    pub total_duration_seconds: i32,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub completed_at: Option<DateTime<Utc>>,
}

/// 应用状态记录
#[derive(Debug, Clone)]
pub struct AppStateRecord {
    pub current_state: String,
    pub state_data: Option<String>,
    pub last_error: Option<String>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

/// 用户操作记录
#[derive(Debug, Clone)]
pub struct UserActionRecord {
    pub id: i64,
    pub action_type: String,
    pub action_description: String,
    pub action_params: Option<String>,
    pub status: String,
    pub result_message: Option<String>,
    pub started_at: DateTime<Utc>,
    pub completed_at: Option<DateTime<Utc>>,
    pub duration_seconds: Option<i32>,
    pub client_version: Option<String>,
    pub platform_info: Option<String>,
}
