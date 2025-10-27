use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

/// 备份记录
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BackupRecord {
    pub id: i64,
    pub file_path: String,
    pub service_version: String,
    pub backup_type: String,
    pub status: String,
    pub created_at: DateTime<Utc>,
}

/// 计划任务
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScheduledTask {
    pub id: i64,
    pub task_type: String,
    pub target_version: String,
    pub scheduled_at: DateTime<Utc>,
    pub status: String,
    pub details: Option<String>,
    pub created_at: DateTime<Utc>,
    pub completed_at: Option<DateTime<Utc>>,
}
