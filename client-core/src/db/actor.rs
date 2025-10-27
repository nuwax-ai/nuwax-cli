use anyhow::Result;
use chrono::{DateTime, Utc};
use duckdb::{Connection, params};
use serde_json;
use std::path::PathBuf;
use tokio::sync::mpsc;
use tracing::{debug, info};

use super::messages::{AppStateRecord, DbMessage, DownloadTaskRecord, UserActionRecord};
use super::models::{BackupRecord, ScheduledTask};

/// DuckDB Actor - 确保单线程访问DuckDB
pub struct DuckDbActor {
    connection: Connection,
}

impl DuckDbActor {
    /// 创建新的DuckDB Actor
    pub fn new(db_path: PathBuf) -> Result<Self> {
        let connection = Connection::open(db_path)?;
        Ok(Self { connection })
    }

    /// 创建内存DuckDB Actor
    pub fn new_memory() -> Result<Self> {
        let connection = Connection::open_in_memory()?;
        Ok(Self { connection })
    }

    /// 运行Actor消息循环
    pub async fn run(mut self, mut receiver: mpsc::Receiver<DbMessage>) {
        debug!("DuckDB Actor 已启动");

        while let Some(message) = receiver.recv().await {
            self.handle_message(message).await;
        }

        debug!("DuckDB Actor 已关闭");
    }

    /// 处理数据库消息
    async fn handle_message(&mut self, message: DbMessage) {
        match message {
            DbMessage::InitTables { respond_to } => {
                let result = self.init_tables();
                let _ = respond_to.send(result);
            }
            DbMessage::GetConfig { key, respond_to } => {
                let result = self.get_config(&key);
                let _ = respond_to.send(result);
            }
            DbMessage::SetConfig {
                key,
                value,
                respond_to,
            } => {
                let result = self.set_config(&key, &value);
                let _ = respond_to.send(result);
            }
            DbMessage::CreateBackupRecord {
                file_path,
                service_version,
                backup_type,
                status,
                respond_to,
            } => {
                let result =
                    self.create_backup_record(&file_path, &service_version, &backup_type, &status);
                let _ = respond_to.send(result);
            }
            DbMessage::GetAllBackups { respond_to } => {
                let result = self.get_all_backups();
                let _ = respond_to.send(result);
            }
            DbMessage::GetBackupById { id, respond_to } => {
                let result = self.get_backup_by_id(id);
                let _ = respond_to.send(result);
            }
            DbMessage::DeleteBackupRecord {
                backup_id,
                respond_to,
            } => {
                let result = self.delete_backup_record(backup_id);
                let _ = respond_to.send(result);
            }
            DbMessage::UpdateBackupFilePath {
                backup_id,
                new_path,
                respond_to,
            } => {
                let result = self.update_backup_file_path(backup_id, &new_path);
                let _ = respond_to.send(result);
            }
            DbMessage::CreateScheduledTask {
                task_type,
                target_version,
                scheduled_at,
                respond_to,
            } => {
                let result = self.create_scheduled_task(
                    &task_type,
                    &target_version,
                    scheduled_at,
                    "PENDING",
                );
                let _ = respond_to.send(result);
            }
            DbMessage::GetPendingTasks { respond_to } => {
                let result = self.get_pending_tasks();
                let _ = respond_to.send(result);
            }
            DbMessage::UpdateTaskStatus {
                task_id,
                status,
                details,
                respond_to,
            } => {
                let result = self.update_task_status(task_id, &status, details.as_deref());
                let _ = respond_to.send(result);
            }
            DbMessage::CancelPendingTasks {
                task_type,
                respond_to,
            } => {
                let result = self.cancel_pending_tasks(&task_type);
                let _ = respond_to.send(result);
            }

            // ========== 下载任务管理 ==========
            DbMessage::CreateDownloadTask {
                task_name,
                download_url,
                total_size,
                target_path,
                file_hash,
                respond_to,
            } => {
                let result = self.create_download_task(
                    &task_name,
                    &download_url,
                    total_size,
                    &target_path,
                    file_hash.as_deref(),
                );
                let _ = respond_to.send(result);
            }
            DbMessage::UpdateDownloadTaskStatus {
                task_id,
                status,
                downloaded_size,
                error_message,
                respond_to,
            } => {
                let result = self.update_download_task_status(
                    task_id,
                    &status,
                    downloaded_size,
                    error_message.as_deref(),
                );
                let _ = respond_to.send(result);
            }
            DbMessage::CompleteDownloadTask {
                task_id,
                average_speed,
                total_duration,
                respond_to,
            } => {
                let result = self.complete_download_task(task_id, average_speed, total_duration);
                let _ = respond_to.send(result);
            }
            DbMessage::GetDownloadTask {
                task_id,
                respond_to,
            } => {
                let result = self.get_download_task(task_id);
                let _ = respond_to.send(result);
            }
            DbMessage::GetActiveDownloadTasks { respond_to } => {
                let result = self.get_active_download_tasks();
                let _ = respond_to.send(result);
            }

            // ========== 应用状态管理 ==========
            DbMessage::UpdateAppState {
                state,
                state_data,
                error_message,
                respond_to,
            } => {
                let result =
                    self.update_app_state(&state, state_data.as_deref(), error_message.as_deref());
                let _ = respond_to.send(result);
            }
            DbMessage::GetAppState { respond_to } => {
                let result = self.get_app_state();
                let _ = respond_to.send(result);
            }

            // ========== 用户操作历史 ==========
            DbMessage::RecordUserAction {
                action_type,
                action_description,
                action_params,
                respond_to,
            } => {
                let result = self.record_user_action(
                    &action_type,
                    &action_description,
                    action_params.as_deref(),
                );
                let _ = respond_to.send(result);
            }
            DbMessage::CompleteUserAction {
                action_id,
                status,
                result_message,
                duration_seconds,
                respond_to,
            } => {
                let result = self.complete_user_action(
                    action_id,
                    &status,
                    result_message.as_deref(),
                    duration_seconds,
                );
                let _ = respond_to.send(result);
            }
            DbMessage::GetUserActions { limit, respond_to } => {
                let result = self.get_user_actions(limit);
                let _ = respond_to.send(result);
            }
        }
    }

    /// 初始化数据库表
    fn init_tables(&mut self) -> Result<()> {
        debug!("正在初始化DuckDB表...");

        // 读取并执行SQL初始化脚本
        let sql_content = include_str!("../../migrations/init_duckdb.sql");

        // 按分号分割SQL语句并执行
        for statement in sql_content.split(';').filter(|s| !s.trim().is_empty()) {
            let trimmed = statement.trim();
            if !trimmed.is_empty() {
                self.connection.execute(trimmed, [])?;
            }
        }

        // 设置数据库初始化标记
        self.connection.execute(
            "INSERT OR REPLACE INTO app_config (config_key, config_value, config_type, category, description, is_system_config, is_user_editable, default_value) VALUES ('db_initialized', '\"true\"', 'BOOLEAN', 'system', '数据库初始化状态标记', TRUE, FALSE, '\"false\"')",
            []
        )?;

        info!("DuckDB表初始化完成");
        Ok(())
    }

    /// 获取配置值
    fn get_config(&mut self, key: &str) -> Result<Option<String>> {
        let mut stmt = self
            .connection
            .prepare("SELECT config_value FROM app_config WHERE config_key = ?")?;
        let mut rows = stmt.query(params![key])?;

        if let Some(row) = rows.next()? {
            let json_value: String = row.get(0)?;
            // 尝试解析JSON，如果是字符串则去掉引号
            if let Ok(parsed) = serde_json::from_str::<serde_json::Value>(&json_value) {
                match parsed {
                    serde_json::Value::String(s) => Ok(Some(s)),
                    _ => Ok(Some(json_value)), // 非字符串类型直接返回JSON
                }
            } else {
                Ok(Some(json_value)) // JSON解析失败，返回原始值
            }
        } else {
            Ok(None)
        }
    }

    /// 设置配置值
    fn set_config(&mut self, key: &str, value: &str) -> Result<()> {
        // 首先尝试更新现有配置
        let updated = self.connection.execute(
            "UPDATE app_config SET config_value = ?, updated_at = CURRENT_TIMESTAMP WHERE config_key = ?",
            params![format!("\"{}\"", value), key],
        )?;

        // 如果没有更新任何行，则插入新配置
        if updated == 0 {
            self.connection.execute(
                "INSERT INTO app_config (config_key, config_value, config_type, category, is_system_config, is_user_editable) VALUES (?, ?, 'STRING', 'system', TRUE, TRUE)",
                params![key, format!("\"{}\"", value)],
        )?;
        }
        Ok(())
    }

    /// 创建备份记录
    fn create_backup_record(
        &mut self,
        file_path: &str,
        service_version: &str,
        backup_type: &str,
        _status: &str,
    ) -> Result<i64> {
        // 生成唯一的备份名称
        let backup_name = format!("backup_{}", chrono::Utc::now().format("%Y%m%d_%H%M%S"));

        // 插入记录，让数据库自动生成ID
        self.connection.execute(
            "INSERT INTO backup_records (backup_name, backup_type, source_version, backup_path) 
             VALUES (?, ?, ?, ?)",
            params![backup_name, backup_type, service_version, file_path],
        )?;

        // 获取最后插入的ID
        let id: i64 =
            self.connection
                .query_row("SELECT currval('backup_records_seq')", [], |row| row.get(0))?;

        Ok(id)
    }

    /// 获取所有备份记录
    fn get_all_backups(&mut self) -> Result<Vec<BackupRecord>> {
        let mut stmt = self.connection.prepare(
            "SELECT id, backup_path, source_version, backup_type, created_at 
             FROM backup_records ORDER BY created_at DESC",
        )?;

        let backup_iter = stmt.query_map([], |row| {
            Ok(BackupRecord {
                id: row.get(0)?,
                file_path: row.get(1)?,
                service_version: row.get(2)?,
                backup_type: row.get(3)?,
                status: "completed".to_string(), // 新表架构没有status字段，默认为completed
                created_at: row.get(4)?,
            })
        })?;

        let mut backups = Vec::new();
        for backup in backup_iter {
            backups.push(backup?);
        }

        Ok(backups)
    }

    /// 根据ID获取备份记录
    fn get_backup_by_id(&mut self, id: i64) -> Result<Option<BackupRecord>> {
        let mut stmt = self.connection.prepare(
            "SELECT id, backup_path, source_version, backup_type, created_at 
             FROM backup_records WHERE id = ?",
        )?;

        let mut rows = stmt.query(params![id])?;

        if let Some(row) = rows.next()? {
            Ok(Some(BackupRecord {
                id: row.get(0)?,
                file_path: row.get(1)?,
                service_version: row.get(2)?,
                backup_type: row.get(3)?,
                status: "completed".to_string(),
                created_at: row.get(4)?,
            }))
        } else {
            Ok(None)
        }
    }

    /// 删除备份记录
    fn delete_backup_record(&mut self, backup_id: i64) -> Result<()> {
        self.connection.execute(
            "DELETE FROM backup_records WHERE id = ?",
            params![backup_id],
        )?;
        Ok(())
    }

    /// 更新备份文件路径
    fn update_backup_file_path(&mut self, backup_id: i64, new_path: &str) -> Result<()> {
        self.connection.execute(
            "UPDATE backup_records SET backup_path = ? WHERE id = ?",
            params![new_path, backup_id],
        )?;
        Ok(())
    }

    /// 创建计划任务
    fn create_scheduled_task(
        &mut self,
        task_type: &str,
        target_version: &str,
        scheduled_at: DateTime<Utc>,
        status: &str,
    ) -> Result<i64> {
        // 插入记录，让数据库自动生成ID
        self.connection.execute(
            "INSERT INTO scheduled_tasks (task_type, target_version, scheduled_at, status) 
             VALUES (?, ?, ?, ?)",
            params![task_type, target_version, scheduled_at, status],
        )?;

        // 获取最后插入的ID
        let id: i64 = self
            .connection
            .query_row("SELECT currval('task_id_seq')", [], |row| row.get(0))?;

        Ok(id)
    }

    /// 获取待执行任务
    fn get_pending_tasks(&mut self) -> Result<Vec<ScheduledTask>> {
        let mut stmt = self.connection.prepare(
            "SELECT id, task_type, target_version, scheduled_at, status, details, created_at, completed_at 
             FROM scheduled_tasks WHERE status = 'PENDING' ORDER BY scheduled_at"
        )?;

        let task_iter = stmt.query_map([], |row| {
            Ok(ScheduledTask {
                id: row.get(0)?,
                task_type: row.get(1)?,
                target_version: row.get(2)?,
                scheduled_at: row.get(3)?,
                status: row.get(4)?,
                details: row.get(5)?,
                created_at: row.get(6)?,
                completed_at: row.get(7)?,
            })
        })?;

        let mut tasks = Vec::new();
        for task in task_iter {
            tasks.push(task?);
        }

        Ok(tasks)
    }

    /// 更新任务状态
    fn update_task_status(
        &mut self,
        task_id: i64,
        status: &str,
        details: Option<&str>,
    ) -> Result<()> {
        if let Some(details) = details {
            self.connection.execute(
                "UPDATE scheduled_tasks SET status = ?, details = ?, completed_at = NOW() WHERE id = ?",
                params![status, details, task_id],
            )?;
        } else {
            self.connection.execute(
                "UPDATE scheduled_tasks SET status = ?, completed_at = NOW() WHERE id = ?",
                params![status, task_id],
            )?;
        }
        Ok(())
    }

    /// 取消待执行任务
    fn cancel_pending_tasks(&mut self, task_type: &str) -> Result<()> {
        self.connection.execute(
            "UPDATE scheduled_tasks SET status = 'CANCELLED', completed_at = NOW() 
             WHERE task_type = ? AND status = 'PENDING'",
            params![task_type],
        )?;
        Ok(())
    }

    // ========== 下载任务管理方法 ==========

    /// 创建下载任务
    fn create_download_task(
        &mut self,
        task_name: &str,
        download_url: &str,
        total_size: i64,
        target_path: &str,
        file_hash: Option<&str>,
    ) -> Result<i64> {
        // 使用 RETURNING 子句获取插入的ID
        let id: i64 = self
            .connection
            .query_row(
                "INSERT INTO download_tasks (task_name, download_url, total_size, target_path, file_hash) 
                 VALUES (?, ?, ?, ?, ?) RETURNING id",
                params![task_name, download_url, total_size, target_path, file_hash],
                |row| row.get(0)
            )?;

        Ok(id)
    }

    /// 更新下载任务状态
    fn update_download_task_status(
        &mut self,
        task_id: i64,
        status: &str,
        downloaded_size: Option<i64>,
        error_message: Option<&str>,
    ) -> Result<()> {
        if let Some(size) = downloaded_size {
            if let Some(error) = error_message {
                self.connection.execute(
                    "UPDATE download_tasks SET status = ?, downloaded_size = ?, error_message = ?, updated_at = CURRENT_TIMESTAMP WHERE id = ?",
                    params![status, size, error, task_id],
                )?;
            } else {
                self.connection.execute(
                    "UPDATE download_tasks SET status = ?, downloaded_size = ?, updated_at = CURRENT_TIMESTAMP WHERE id = ?",
                    params![status, size, task_id],
                )?;
            }
        } else if let Some(error) = error_message {
            self.connection.execute(
                "UPDATE download_tasks SET status = ?, error_message = ?, updated_at = CURRENT_TIMESTAMP WHERE id = ?",
                params![status, error, task_id],
            )?;
        } else {
            self.connection.execute(
                "UPDATE download_tasks SET status = ?, updated_at = CURRENT_TIMESTAMP WHERE id = ?",
                params![status, task_id],
            )?;
        };
        Ok(())
    }

    /// 完成下载任务
    fn complete_download_task(
        &mut self,
        task_id: i64,
        average_speed: Option<i64>,
        total_duration: Option<i32>,
    ) -> Result<()> {
        self.connection.execute(
            "UPDATE download_tasks SET status = 'COMPLETED', average_speed = ?, total_duration_seconds = ?, 
             completed_at = CURRENT_TIMESTAMP, updated_at = CURRENT_TIMESTAMP WHERE id = ?",
            params![average_speed.unwrap_or(0), total_duration.unwrap_or(0), task_id],
        )?;
        Ok(())
    }

    /// 获取下载任务
    fn get_download_task(&mut self, task_id: i64) -> Result<Option<DownloadTaskRecord>> {
        let mut stmt = self.connection.prepare(
            "SELECT id, task_name, download_url, total_size, downloaded_size, target_path, file_hash, 
             status, error_message, retry_count, average_speed, total_duration_seconds, 
             created_at, updated_at, completed_at 
             FROM download_tasks WHERE id = ?"
        )?;

        let mut rows = stmt.query(params![task_id])?;

        if let Some(row) = rows.next()? {
            Ok(Some(DownloadTaskRecord {
                id: row.get(0)?,
                task_name: row.get(1)?,
                download_url: row.get(2)?,
                total_size: row.get(3)?,
                downloaded_size: row.get(4)?,
                target_path: row.get(5)?,
                file_hash: row.get(6)?,
                status: row.get(7)?,
                error_message: row.get(8)?,
                retry_count: row.get(9)?,
                average_speed: row.get(10)?,
                total_duration_seconds: row.get(11)?,
                created_at: row.get(12)?,
                updated_at: row.get(13)?,
                completed_at: row.get(14)?,
            }))
        } else {
            Ok(None)
        }
    }

    /// 获取活跃的下载任务
    fn get_active_download_tasks(&mut self) -> Result<Vec<DownloadTaskRecord>> {
        let mut stmt = self.connection.prepare(
            "SELECT id, task_name, download_url, total_size, downloaded_size, target_path, file_hash, 
             status, error_message, retry_count, average_speed, total_duration_seconds, 
             created_at, updated_at, completed_at 
             FROM download_tasks WHERE status IN ('PENDING', 'DOWNLOADING', 'PAUSED') 
             ORDER BY created_at DESC"
        )?;

        let task_iter = stmt.query_map([], |row| {
            Ok(DownloadTaskRecord {
                id: row.get(0)?,
                task_name: row.get(1)?,
                download_url: row.get(2)?,
                total_size: row.get(3)?,
                downloaded_size: row.get(4)?,
                target_path: row.get(5)?,
                file_hash: row.get(6)?,
                status: row.get(7)?,
                error_message: row.get(8)?,
                retry_count: row.get(9)?,
                average_speed: row.get(10)?,
                total_duration_seconds: row.get(11)?,
                created_at: row.get(12)?,
                updated_at: row.get(13)?,
                completed_at: row.get(14)?,
            })
        })?;

        let mut tasks = Vec::new();
        for task in task_iter {
            tasks.push(task?);
        }
        Ok(tasks)
    }

    // ========== 应用状态管理方法 ==========

    /// 更新应用状态
    fn update_app_state(
        &mut self,
        state: &str,
        state_data: Option<&str>,
        error_message: Option<&str>,
    ) -> Result<()> {
        // 使用UPSERT（INSERT OR REPLACE）来更新唯一记录
        self.connection.execute(
            "INSERT OR REPLACE INTO app_state (id, current_state, state_data, last_error, updated_at) 
             VALUES (1, ?, ?, ?, CURRENT_TIMESTAMP)",
            params![state, state_data, error_message],
        )?;
        Ok(())
    }

    /// 获取当前应用状态
    fn get_app_state(&mut self) -> Result<Option<AppStateRecord>> {
        let mut stmt = self.connection.prepare(
            "SELECT current_state, state_data, last_error, 
             created_at, updated_at 
             FROM app_state WHERE id = 1",
        )?;

        let mut rows = stmt.query([])?;

        if let Some(row) = rows.next()? {
            Ok(Some(AppStateRecord {
                current_state: row.get(0)?,
                state_data: row.get(1)?,
                last_error: row.get(2)?,
                created_at: row.get(3)?,
                updated_at: row.get(4)?,
            }))
        } else {
            Ok(None)
        }
    }

    // ========== 用户操作历史方法 ==========

    /// 记录用户操作
    fn record_user_action(
        &mut self,
        action_type: &str,
        action_description: &str,
        action_params: Option<&str>,
    ) -> Result<i64> {
        // 获取客户端版本和平台信息
        let client_version = env!("CARGO_PKG_VERSION");
        let platform_info = format!("{}-{}", std::env::consts::OS, std::env::consts::ARCH);

        // 使用 RETURNING 子句获取插入的ID
        let id: i64 = self
            .connection
            .query_row(
                "INSERT INTO user_actions (action_type, action_description, action_params, status, client_version, platform_info) 
                 VALUES (?, ?, ?, 'SUCCESS', ?, ?) RETURNING id",
                params![action_type, action_description, action_params, client_version, platform_info],
                |row| row.get(0)
            )?;

        Ok(id)
    }

    /// 完成用户操作
    fn complete_user_action(
        &mut self,
        action_id: i64,
        status: &str,
        result_message: Option<&str>,
        duration_seconds: Option<i32>,
    ) -> Result<()> {
        self.connection.execute(
            "UPDATE user_actions SET status = ?, result_message = ?, completed_at = CURRENT_TIMESTAMP, duration_seconds = ? 
             WHERE id = ?",
            params![status, result_message, duration_seconds, action_id],
        )?;
        Ok(())
    }

    /// 获取用户操作历史
    fn get_user_actions(&mut self, limit: Option<i32>) -> Result<Vec<UserActionRecord>> {
        let sql = if let Some(limit) = limit {
            format!(
                "SELECT id, action_type, action_description, action_params, status, result_message, 
                 started_at, completed_at, duration_seconds, client_version, platform_info 
                 FROM user_actions ORDER BY started_at DESC LIMIT {limit}"
            )
        } else {
            "SELECT id, action_type, action_description, action_params, status, result_message, 
             started_at, completed_at, duration_seconds, client_version, platform_info 
             FROM user_actions ORDER BY started_at DESC"
                .to_string()
        };

        let mut stmt = self.connection.prepare(&sql)?;
        let action_iter = stmt.query_map([], |row| {
            Ok(UserActionRecord {
                id: row.get(0)?,
                action_type: row.get(1)?,
                action_description: row.get(2)?,
                action_params: row.get(3)?,
                status: row.get(4)?,
                result_message: row.get(5)?,
                started_at: row.get(6)?,
                completed_at: row.get(7)?,
                duration_seconds: row.get(8)?,
                client_version: row.get(9)?,
                platform_info: row.get(10)?,
            })
        })?;

        let mut actions = Vec::new();
        for action in action_iter {
            actions.push(action?);
        }
        Ok(actions)
    }
}
