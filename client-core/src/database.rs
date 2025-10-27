use crate::db::DuckDbManager;
use anyhow::Result;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::{path::Path, sync::Arc};
use uuid::Uuid;

/// 数据库管理器 - DuckDB适配器
#[derive(Debug, Clone)]
pub struct Database {
    manager: Arc<DuckDbManager>,
}

/// 客户端身份信息
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ClientIdentity {
    pub id: i64,
    pub client_uuid: Uuid,
    pub created_at: DateTime<Utc>,
}

/// 备份记录
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BackupRecord {
    pub id: i64,
    pub file_path: String,
    pub service_version: String,
    pub backup_type: BackupType,
    pub status: BackupStatus,
    pub created_at: DateTime<Utc>,
}

/// 备份类型
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum BackupType {
    Manual,
    PreUpgrade,
}

/// 备份状态
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum BackupStatus {
    Completed,
    Failed,
}

/// 计划任务
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScheduledTask {
    pub id: i64,
    pub task_type: TaskType,
    pub target_version: String,
    pub scheduled_at: DateTime<Utc>,
    pub status: TaskStatus,
    pub details: Option<String>,
    pub created_at: DateTime<Utc>,
    pub completed_at: Option<DateTime<Utc>>,
}

/// 任务类型
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum TaskType {
    ServiceUpgrade,
}

/// 任务状态
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum TaskStatus {
    Pending,
    InProgress,
    Completed,
    Failed,
    Cancelled,
}

impl Database {
    /// 连接到数据库
    pub async fn connect<P: AsRef<Path>>(db_path: P) -> Result<Self> {
        let manager = DuckDbManager::new(db_path).await?;
        Ok(Database {
            manager: Arc::new(manager),
        })
    }

    /// 连接到内存数据库 (主要用于测试，生产环境建议使用connect()以确保数据持久化)
    pub async fn connect_memory() -> Result<Self> {
        let manager = DuckDbManager::new_memory().await?;
        Ok(Database {
            manager: Arc::new(manager),
        })
    }

    /// 显式初始化数据库（只应在 nuwax-cli init 时调用）
    pub async fn init_database(&self) -> Result<()> {
        self.manager.init_database().await?;
        // 标记数据库为已初始化
        self.manager.mark_database_initialized().await?;
        Ok(())
    }

    /// 检查数据库是否已经初始化
    pub async fn is_database_initialized(&self) -> Result<bool> {
        self.manager.is_database_initialized().await
    }

    /// 运行数据库迁移 (DuckDB版本中此方法为空操作，因为表在初始化时自动创建)
    pub async fn run_migrations(&self) -> Result<()> {
        // DuckDB版本中，表结构在初始化时自动创建，所以这里不需要做任何事情
        Ok(())
    }

    /// 获取或创建客户端 UUID
    pub async fn get_or_create_client_uuid(&self) -> Result<Uuid> {
        self.manager.get_or_create_client_uuid().await
    }

    /// 从数据库获取客户端UUID
    pub async fn get_client_uuid(&self) -> Result<Option<Uuid>> {
        if let Some(uuid_str) = self.manager.get_config("client_uuid").await? {
            Ok(Some(Uuid::parse_str(&uuid_str)?))
        } else {
            Ok(None)
        }
    }

    /// 设置客户端UUID
    pub async fn set_client_uuid(&self, uuid: &Uuid) -> Result<()> {
        self.manager
            .set_config("client_uuid", &uuid.to_string())
            .await
    }

    /// 更新客户端ID（服务端返回的ID）
    pub async fn update_client_id(&self, client_id: &str) -> Result<()> {
        self.manager.set_config("client_id", client_id).await
    }

    /// 获取客户端ID（服务端返回的ID）
    pub async fn get_client_id(&self) -> Result<Option<String>> {
        self.manager.get_config("client_id").await
    }

    /// 获取用于API请求的客户端标识（只使用服务端返回的client_id）
    pub async fn get_api_client_id(&self) -> Result<Option<String>> {
        // 只使用服务端返回的client_id，不使用本地UUID
        // 因为服务端不认识本地UUID，会导致401错误
        self.get_client_id().await
    }

    /// 通用配置项获取
    pub async fn get_config(&self, key: &str) -> Result<Option<String>> {
        self.manager.get_config(key).await
    }

    /// 通用配置项设置
    pub async fn set_config(&self, key: &str, value: &str) -> Result<()> {
        self.manager.set_config(key, value).await
    }

    /// 获取客户端身份信息 (兼容性方法，DuckDB版本中简化实现)
    pub async fn get_client_identity(&self) -> Result<Option<ClientIdentity>> {
        if let Some(uuid) = self.get_client_uuid().await? {
            // 从配置中获取创建时间，如果不存在则使用当前时间
            let created_at =
                if let Some(created_at_str) = self.get_config("client_created_at").await? {
                    DateTime::parse_from_rfc3339(&created_at_str)
                        .map(|dt| dt.with_timezone(&Utc))
                        .unwrap_or_else(|_| Utc::now())
                } else {
                    let now = Utc::now();
                    // 保存创建时间
                    let _ = self
                        .set_config("client_created_at", &now.to_rfc3339())
                        .await;
                    now
                };

            Ok(Some(ClientIdentity {
                id: 1, // 固定ID，因为只有一个客户端身份
                client_uuid: uuid,
                created_at,
            }))
        } else {
            Ok(None)
        }
    }

    /// 创建备份记录
    pub async fn create_backup_record(
        &self,
        file_path: String,
        service_version: String,
        backup_type: BackupType,
        status: BackupStatus,
    ) -> Result<i64> {
        let backup_type_str = match backup_type {
            BackupType::Manual => "manual",
            BackupType::PreUpgrade => "pre-upgrade",
        };

        let status_str = match status {
            BackupStatus::Completed => "completed",
            BackupStatus::Failed => "failed",
        };

        self.manager
            .create_backup_record(file_path, service_version, backup_type_str, status_str)
            .await
    }

    /// 获取所有备份记录
    pub async fn get_all_backups(&self) -> Result<Vec<BackupRecord>> {
        let duckdb_backups = self.manager.get_all_backups().await?;

        let mut backups = Vec::new();
        for backup in duckdb_backups {
            let backup_type = match backup.backup_type.as_str() {
                "manual" => BackupType::Manual,
                "pre-upgrade" => BackupType::PreUpgrade,
                _ => BackupType::Manual,
            };

            let status = match backup.status.as_str() {
                "completed" => BackupStatus::Completed,
                "failed" => BackupStatus::Failed,
                _ => BackupStatus::Failed,
            };

            backups.push(BackupRecord {
                id: backup.id,
                file_path: backup.file_path,
                service_version: backup.service_version,
                backup_type,
                status,
                created_at: backup.created_at,
            });
        }

        Ok(backups)
    }

    /// 根据 ID 获取备份记录
    pub async fn get_backup_by_id(&self, id: i64) -> Result<Option<BackupRecord>> {
        if let Some(backup) = self.manager.get_backup_by_id(id).await? {
            let backup_type = match backup.backup_type.as_str() {
                "manual" => BackupType::Manual,
                "pre-upgrade" => BackupType::PreUpgrade,
                _ => BackupType::Manual,
            };

            let status = match backup.status.as_str() {
                "completed" => BackupStatus::Completed,
                "failed" => BackupStatus::Failed,
                _ => BackupStatus::Failed,
            };

            Ok(Some(BackupRecord {
                id: backup.id,
                file_path: backup.file_path,
                service_version: backup.service_version,
                backup_type,
                status,
                created_at: backup.created_at,
            }))
        } else {
            Ok(None)
        }
    }

    /// 创建计划任务
    pub async fn create_scheduled_task(
        &self,
        task_type: TaskType,
        target_version: String,
        scheduled_at: DateTime<Utc>,
    ) -> Result<i64> {
        let task_type_str = match task_type {
            TaskType::ServiceUpgrade => "SERVICE_UPGRADE",
        };

        let id = self
            .manager
            .create_scheduled_task(task_type_str, target_version, scheduled_at)
            .await?;

        Ok(id)
    }

    /// 获取待执行的任务
    pub async fn get_pending_tasks(&self) -> Result<Vec<ScheduledTask>> {
        let duckdb_tasks = self.manager.get_pending_tasks().await?;

        let mut tasks = Vec::new();
        for task in duckdb_tasks {
            let task_type = match task.task_type.as_str() {
                "SERVICE_UPGRADE" => TaskType::ServiceUpgrade,
                _ => TaskType::ServiceUpgrade,
            };

            let status = match task.status.as_str() {
                "PENDING" => TaskStatus::Pending,
                "IN_PROGRESS" => TaskStatus::InProgress,
                "COMPLETED" => TaskStatus::Completed,
                "FAILED" => TaskStatus::Failed,
                "CANCELLED" => TaskStatus::Cancelled,
                _ => TaskStatus::Pending,
            };

            tasks.push(ScheduledTask {
                id: task.id,
                task_type,
                target_version: task.target_version,
                scheduled_at: task.scheduled_at,
                status,
                details: task.details,
                created_at: task.created_at,
                completed_at: task.completed_at,
            });
        }

        Ok(tasks)
    }

    /// 更新任务状态
    pub async fn update_task_status(
        &self,
        task_id: i64,
        status: TaskStatus,
        details: Option<String>,
    ) -> Result<()> {
        let status_str = match status {
            TaskStatus::Pending => "PENDING",
            TaskStatus::InProgress => "IN_PROGRESS",
            TaskStatus::Completed => "COMPLETED",
            TaskStatus::Failed => "FAILED",
            TaskStatus::Cancelled => "CANCELLED",
        };

        self.manager
            .update_task_status(task_id, status_str, details)
            .await
    }

    /// 删除备份记录
    pub async fn delete_backup_record(&self, backup_id: i64) -> Result<()> {
        self.manager.delete_backup_record(backup_id).await
    }

    /// 更新备份文件路径
    pub async fn update_backup_file_path(&self, backup_id: i64, new_path: String) -> Result<()> {
        self.manager
            .update_backup_file_path(backup_id, new_path)
            .await
    }

    /// 批量更新备份文件路径（用于存储目录迁移）
    pub async fn update_all_backup_paths(&self, old_prefix: &str, new_prefix: &str) -> Result<()> {
        let backups = self.get_all_backups().await?;

        for backup in backups {
            if backup.file_path.starts_with(old_prefix) {
                let new_path = backup.file_path.replace(old_prefix, new_prefix);
                self.update_backup_file_path(backup.id, new_path).await?;
            }
        }

        Ok(())
    }
}
