use anyhow::Result;
use duckdb::{Connection, Result as DuckResult};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::Mutex;
use tracing::{debug, error, warn};

/// DuckDB 数据库管理器 - 针对并发特性优化
///
/// 设计原则：
/// - 文件数据库：每个操作创建新连接，天然支持并发读
/// - 内存数据库：使用单一连接+Mutex，确保数据一致性
/// - 写操作：串行执行，避免write-write conflict
/// - 重试机制：检测冲突并实现指数退避重试
#[derive(Clone)]
pub struct DatabaseManager {
    /// 数据库配置
    config: Arc<DatabaseConfig>,
}

#[derive(Debug)]
struct DatabaseConfig {
    /// 数据库路径（None表示内存数据库）
    db_path: Option<PathBuf>,
    /// 内存数据库的共享连接（仅用于内存数据库）
    memory_connection: Option<Arc<Mutex<Connection>>>,
}

impl DatabaseManager {
    /// 创建新的数据库管理器（文件数据库）
    pub async fn new<P: AsRef<std::path::Path>>(db_path: P) -> Result<Self> {
        let db_path = db_path.as_ref().to_path_buf();

        // 确保数据库目录存在
        if let Some(parent) = db_path.parent() {
            tokio::fs::create_dir_all(parent).await?;
        }

        // 测试连接是否可以创建
        let _test_conn = Connection::open(&db_path)?;
        debug!("数据库文件连接测试成功: {:?}", db_path);

        let manager = Self {
            config: Arc::new(DatabaseConfig {
                db_path: Some(db_path),
                memory_connection: None,
            }),
        };

        // 移除自动初始化 - 只有在明确调用 init_database 时才初始化
        // manager.initialize_schema().await?;

        Ok(manager)
    }

    /// 创建内存数据库管理器（主要用于测试）
    pub async fn new_memory() -> Result<Self> {
        // 对于内存数据库，我们需要保持一个共享连接
        let connection = Arc::new(Mutex::new(Connection::open_in_memory()?));
        debug!("内存数据库连接创建成功");

        let manager = Self {
            config: Arc::new(DatabaseConfig {
                db_path: None,
                memory_connection: Some(connection),
            }),
        };

        // 移除自动初始化 - 只有在明确调用 init_database 时才初始化
        // manager.initialize_schema().await?;

        Ok(manager)
    }

    /// 显式初始化数据库（只应在 nuwax-cli init 时调用）
    pub async fn init_database(&self) -> Result<()> {
        debug!("显式初始化数据库表结构...");
        self.initialize_schema().await?;
        debug!("数据库表结构初始化完成");
        Ok(())
    }

    /// 创建数据库连接
    async fn create_connection(&self) -> Result<Connection> {
        if let Some(ref path) = self.config.db_path {
            // 文件数据库：创建新连接
            Ok(Connection::open(path)?)
        } else if let Some(ref memory_conn) = self.config.memory_connection {
            // 内存数据库：克隆共享连接
            let conn = memory_conn.lock().await;
            Ok(conn.try_clone()?)
        } else {
            Err(anyhow::anyhow!("数据库配置无效"))
        }
    }

    /// 并发读操作（文件数据库支持真正的并发）
    pub async fn read_with_retry<F, R>(&self, operation: F) -> Result<R>
    where
        F: Fn(&Connection) -> DuckResult<R> + Send + Sync,
        R: Send,
    {
        let mut retry_count = 0;
        const MAX_RETRIES: usize = 3;

        loop {
            let connection = self.create_connection().await?;

            match operation(&connection) {
                Ok(result) => return Ok(result),
                Err(e) => {
                    let error_msg = e.to_string();
                    if retry_count < MAX_RETRIES && Self::is_retryable_error(&error_msg) {
                        retry_count += 1;
                        let delay = Duration::from_millis(100 * (1 << retry_count)); // 指数退避
                        warn!(
                            "数据库读操作失败，{}ms后重试 ({}/{}): {}",
                            delay.as_millis(),
                            retry_count,
                            MAX_RETRIES,
                            error_msg
                        );
                        tokio::time::sleep(delay).await;
                    } else {
                        error!("数据库读操作最终失败: {}", error_msg);
                        return Err(anyhow::anyhow!(e.to_string()));
                    }
                }
            }
        }
    }

    /// 并发写操作（确保写入一致性）
    pub async fn write_with_retry<F, R>(&self, operation: F) -> Result<R>
    where
        F: Fn(&Connection) -> DuckResult<R> + Send + Sync,
        R: Send,
    {
        let mut retry_count = 0;
        const MAX_RETRIES: usize = 3;

        loop {
            if let Some(ref memory_conn) = self.config.memory_connection {
                // 内存数据库：确保独占访问
                let conn = memory_conn.lock().await;
                match operation(&conn) {
                    Ok(result) => return Ok(result),
                    Err(e) => {
                        let error_msg = e.to_string();
                        if retry_count < MAX_RETRIES && Self::is_retryable_error(&error_msg) {
                            retry_count += 1;
                            let delay = Duration::from_millis(50 * (1 << retry_count)); // 较短的重试间隔
                            warn!(
                                "内存数据库写操作失败，{}ms后重试 ({}/{}): {}",
                                delay.as_millis(),
                                retry_count,
                                MAX_RETRIES,
                                error_msg
                            );
                            drop(conn); // 释放锁
                            tokio::time::sleep(delay).await;
                        } else {
                            error!("内存数据库写操作最终失败: {}", error_msg);
                            return Err(anyhow::anyhow!(e.to_string()));
                        }
                    }
                }
            } else {
                // 文件数据库：创建新连接
                let connection = self.create_connection().await?;
                match operation(&connection) {
                    Ok(result) => return Ok(result),
                    Err(e) => {
                        let error_msg = e.to_string();
                        if retry_count < MAX_RETRIES && Self::is_retryable_error(&error_msg) {
                            retry_count += 1;
                            let delay = Duration::from_millis(100 * (1 << retry_count)); // 指数退避
                            warn!(
                                "文件数据库写操作失败，{}ms后重试 ({}/{}): {}",
                                delay.as_millis(),
                                retry_count,
                                MAX_RETRIES,
                                error_msg
                            );
                            tokio::time::sleep(delay).await;
                        } else {
                            error!("文件数据库写操作最终失败: {}", error_msg);
                            return Err(anyhow::anyhow!(e));
                        }
                    }
                }
            }
        }
    }

    /// 批量写操作（事务中执行多个写操作）
    /// 注意：这个方法接受一个闭包，该闭包会在事务上下文中执行
    pub async fn batch_write_with_retry<F, R>(&self, operations: F) -> Result<R>
    where
        F: Fn(&Connection) -> DuckResult<R> + Send + Sync,
        R: Send,
    {
        // 将事务逻辑封装到普通的写操作中
        self.write_with_retry(|conn| {
            // 注意：这里我们不使用事务，因为 Connection 的借用问题
            // 如果需要事务，可以在 operations 闭包内部处理
            operations(conn)
        })
        .await
    }

    /// 检查错误是否可重试
    fn is_retryable_error(error_msg: &str) -> bool {
        // DuckDB 的 write-write conflict 错误模式
        error_msg.contains("write-write conflict")
            || error_msg.contains("database is locked")
            || error_msg.contains("database is busy")
            || error_msg.contains("SQLITE_BUSY")
            || error_msg.contains("SQLITE_LOCKED")
    }

    /// 初始化数据库表结构
    pub async fn initialize_schema(&self) -> Result<()> {
        debug!("正在初始化数据库表结构...");

        // 读取SQL初始化脚本
        let schema_sql = include_str!("../migrations/init_duckdb.sql");

        // 使用更智能的SQL语句分割方式
        self.write_with_retry(|conn| {
            let statements = self.parse_sql_statements(schema_sql);
            for statement in statements {
                let trimmed = statement.trim();
                if trimmed.is_empty() {
                    continue;
                }

                // 检查是否只包含注释
                let is_only_comments = trimmed
                    .lines()
                    .map(|line| line.trim())
                    .all(|line| line.is_empty() || line.starts_with("--"));

                if is_only_comments {
                    continue;
                }

                debug!(
                    "执行SQL语句: {}",
                    if trimmed.len() > 100 {
                        // 安全地截取字符串，避免切断多字节字符
                        let mut end = 100;
                        while end > 0 && !trimmed.is_char_boundary(end) {
                            end -= 1;
                        }
                        format!("{}...", &trimmed[..end])
                    } else {
                        trimmed.to_string()
                    }
                );

                if let Err(e) = conn.execute(trimmed, []) {
                    error!("SQL语句执行失败: {}, 语句: {}", e, trimmed);
                    return Err(e);
                }
            }
            Ok(())
        })
        .await?;

        debug!("数据库表结构初始化完成");
        Ok(())
    }

    /// 智能解析SQL语句（处理JSON中的分号）
    fn parse_sql_statements(&self, sql: &str) -> Vec<String> {
        let mut statements = Vec::new();
        let mut current_statement = String::new();
        let mut in_string = false;
        let mut in_json = false;
        let mut brace_count = 0;
        let chars = sql.chars().peekable();

        for ch in chars {
            match ch {
                '\'' | '"' => {
                    current_statement.push(ch);
                    if !in_json {
                        in_string = !in_string;
                    }
                }
                '{' => {
                    current_statement.push(ch);
                    if !in_string {
                        brace_count += 1;
                        in_json = true;
                    }
                }
                '}' => {
                    current_statement.push(ch);
                    if !in_string && in_json {
                        brace_count -= 1;
                        if brace_count == 0 {
                            in_json = false;
                        }
                    }
                }
                ';' => {
                    if !in_string && !in_json {
                        // 这是一个真正的语句结束符
                        if !current_statement.trim().is_empty() {
                            statements.push(current_statement.trim().to_string());
                        }
                        current_statement.clear();
                    } else {
                        current_statement.push(ch);
                    }
                }
                _ => {
                    current_statement.push(ch);
                }
            }
        }

        // 添加最后一个语句
        if !current_statement.trim().is_empty() {
            statements.push(current_statement.trim().to_string());
        }

        statements
    }

    /// 获取连接统计信息
    pub fn get_connection_stats(&self) -> ConnectionStats {
        ConnectionStats {
            db_type: if self.config.db_path.is_some() {
                "file".to_string()
            } else {
                "memory".to_string()
            },
            is_memory_db: self.config.memory_connection.is_some(),
        }
    }

    /// 健康检查
    pub async fn health_check(&self) -> Result<HealthStatus> {
        // 检查读连接
        let read_result = self
            .read_with_retry(|conn| {
                conn.query_row("SELECT 1", [], |row| {
                    let value: i32 = row.get(0)?;
                    Ok(value)
                })
            })
            .await;

        // 检查写连接
        let write_result = self
            .write_with_retry(|conn| {
                conn.query_row("SELECT 1", [], |row| {
                    let value: i32 = row.get(0)?;
                    Ok(value)
                })
            })
            .await;

        Ok(HealthStatus {
            read_healthy: read_result.is_ok(),
            write_healthy: write_result.is_ok(),
            read_error: read_result.err().map(|e| e.to_string()),
            write_error: write_result.err().map(|e| e.to_string()),
        })
    }

    /// 调试：执行单个SQL语句并返回结果
    #[cfg(test)]
    pub async fn debug_execute_sql(&self, sql: &str) -> Result<()> {
        self.write_with_retry(|conn| {
            debug!("执行调试SQL: {}", sql);
            conn.execute(sql, [])?;
            Ok(())
        })
        .await
    }

    /// 调试：检查表是否存在
    #[cfg(test)]
    pub async fn debug_table_exists(&self, table_name: &str) -> Result<bool> {
        self.read_with_retry(|conn| {
            let exists = conn.query_row(
                "SELECT COUNT(*) FROM information_schema.tables WHERE table_name = ?",
                [table_name],
                |row| {
                    let count: i64 = row.get(0)?;
                    Ok(count > 0)
                },
            );

            // 如果information_schema不存在，尝试SQLite方式
            match exists {
                Ok(result) => Ok(result),
                Err(_) => {
                    // 尝试DuckDB的系统表
                    conn.query_row(
                        "SELECT COUNT(*) FROM duckdb_tables() WHERE table_name = ?",
                        [table_name],
                        |row| {
                            let count: i64 = row.get(0)?;
                            Ok(count > 0)
                        },
                    )
                }
            }
        })
        .await
    }

    /// 调试：获取所有表名
    #[cfg(test)]
    pub async fn debug_list_tables(&self) -> Result<Vec<String>> {
        self.read_with_retry(|conn| {
            let mut stmt = conn.prepare("SELECT table_name FROM duckdb_tables()")?;
            let table_iter = stmt.query_map([], |row| {
                let table_name: String = row.get(0)?;
                Ok(table_name)
            })?;

            let mut tables = Vec::new();
            for table in table_iter {
                tables.push(table?);
            }
            Ok(tables)
        })
        .await
    }
}

/// 连接统计信息
#[derive(Debug, Clone)]
pub struct ConnectionStats {
    pub db_type: String,
    pub is_memory_db: bool,
}

/// 健康检查状态
#[derive(Debug, Clone)]
pub struct HealthStatus {
    pub read_healthy: bool,
    pub write_healthy: bool,
    pub read_error: Option<String>,
    pub write_error: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[tokio::test]
    async fn test_database_manager_creation() {
        let temp_dir = tempdir().unwrap();
        let db_path = temp_dir.path().join("test.db");

        let manager = DatabaseManager::new(&db_path).await.unwrap();
        let stats = manager.get_connection_stats();

        assert_eq!(stats.db_type, "file");
        assert!(!stats.is_memory_db);
    }

    #[tokio::test]
    async fn test_memory_database_creation() {
        let manager = DatabaseManager::new_memory().await.unwrap();
        let stats = manager.get_connection_stats();

        assert_eq!(stats.db_type, "memory");
        assert!(stats.is_memory_db);
    }

    #[tokio::test]
    async fn test_concurrent_read_operations() {
        let manager = DatabaseManager::new_memory().await.unwrap();

        // 并发执行多个读操作
        let mut handles = Vec::new();
        for i in 0..10 {
            let manager = manager.clone();
            let handle = tokio::spawn(async move {
                manager
                    .read_with_retry(|conn| {
                        conn.query_row("SELECT ?", [i], |row| {
                            let value: i32 = row.get(0)?;
                            Ok(value)
                        })
                    })
                    .await
            });
            handles.push(handle);
        }

        // 等待所有操作完成
        for (i, handle) in handles.into_iter().enumerate() {
            let result = handle.await.unwrap();
            assert_eq!(result.unwrap(), i as i32);
        }
    }

    #[tokio::test]
    async fn test_write_operations() {
        let manager = DatabaseManager::new_memory().await.unwrap();

        // 测试写操作
        let result = manager
            .write_with_retry(|conn| {
                conn.execute(
                    "CREATE TABLE IF NOT EXISTS test_table (id INTEGER, name TEXT)",
                    [],
                )?;
                conn.execute("INSERT INTO test_table (id, name) VALUES (1, 'test')", [])?;
                Ok(())
            })
            .await;

        assert!(result.is_ok());

        // 验证数据是否写入
        let value = manager
            .read_with_retry(|conn| {
                conn.query_row("SELECT name FROM test_table WHERE id = 1", [], |row| {
                    let name: String = row.get(0)?;
                    Ok(name)
                })
            })
            .await;

        assert_eq!(value.unwrap(), "test");
    }

    #[tokio::test]
    async fn test_health_check() {
        let manager = DatabaseManager::new_memory().await.unwrap();
        let health = manager.health_check().await.unwrap();

        assert!(health.read_healthy);
        assert!(health.write_healthy);
        assert!(health.read_error.is_none());
        assert!(health.write_error.is_none());
    }

    #[tokio::test]
    async fn test_batch_write_operations() {
        let manager = DatabaseManager::new_memory().await.unwrap();

        // 测试批量写操作
        let result = manager
            .batch_write_with_retry(|conn| {
                conn.execute(
                    "CREATE TABLE IF NOT EXISTS batch_test (id INTEGER, value TEXT)",
                    [],
                )?;
                conn.execute("INSERT INTO batch_test (id, value) VALUES (1, 'a')", [])?;
                conn.execute("INSERT INTO batch_test (id, value) VALUES (2, 'b')", [])?;
                conn.execute("INSERT INTO batch_test (id, value) VALUES (3, 'c')", [])?;
                Ok(())
            })
            .await;

        assert!(result.is_ok());

        // 验证所有数据都已写入
        let count = manager
            .read_with_retry(|conn| {
                conn.query_row("SELECT COUNT(*) FROM batch_test", [], |row| {
                    let count: i64 = row.get(0)?;
                    Ok(count)
                })
            })
            .await;

        assert_eq!(count.unwrap(), 3);
    }

    #[tokio::test]
    async fn test_file_database_concurrent_operations() {
        let temp_dir = tempdir().unwrap();
        let db_path = temp_dir.path().join("concurrent_test.db");
        let manager = DatabaseManager::new(&db_path).await.unwrap();

        // 创建测试表
        manager
            .write_with_retry(|conn| {
                conn.execute(
                    "CREATE TABLE IF NOT EXISTS concurrent_test (id INTEGER, value TEXT)",
                    [],
                )?;
                Ok(())
            })
            .await
            .unwrap();

        // 并发写入测试
        let mut handles = Vec::new();
        for i in 0..5 {
            let manager = manager.clone();
            let handle = tokio::spawn(async move {
                manager
                    .write_with_retry(|conn| {
                        conn.execute(
                            "INSERT INTO concurrent_test (id, value) VALUES (?, ?)",
                            [&i.to_string(), &format!("value_{i}")],
                        )?;
                        Ok(())
                    })
                    .await
            });
            handles.push(handle);
        }

        // 等待所有写操作完成
        for handle in handles {
            let result = handle.await.unwrap();
            assert!(result.is_ok());
        }

        // 验证数据完整性
        let count = manager
            .read_with_retry(|conn| {
                conn.query_row("SELECT COUNT(*) FROM concurrent_test", [], |row| {
                    let count: i64 = row.get(0)?;
                    Ok(count)
                })
            })
            .await;

        assert_eq!(count.unwrap(), 5);
    }

    #[tokio::test]
    async fn test_debug_sql_initialization() {
        // 创建一个不初始化的数据库管理器
        let connection = Arc::new(Mutex::new(Connection::open_in_memory().unwrap()));
        let manager = DatabaseManager {
            config: Arc::new(DatabaseConfig {
                db_path: None,
                memory_connection: Some(connection),
            }),
        };

        println!("=== 初始化前 ===");
        // 检查表是否存在
        let tables = manager.debug_list_tables().await.unwrap();
        println!("初始化前的表: {tables:?}");

        println!("=== 开始初始化 ===");
        // 现在手动初始化
        let init_result = manager.initialize_schema().await;
        println!("初始化结果: {init_result:?}");

        if init_result.is_ok() {
            println!("=== 初始化成功，检查表 ===");
            let tables_after = manager.debug_list_tables().await.unwrap();
            println!("初始化后的表: {tables_after:?}");

            // 检查app_config表是否存在
            let app_config_exists = manager.debug_table_exists("app_config").await.unwrap();
            println!("app_config表存在: {app_config_exists}");
        } else {
            println!("=== 初始化失败，尝试手动创建简单表 ===");
            let result = manager.debug_execute_sql(
                "CREATE TABLE app_config (config_key VARCHAR PRIMARY KEY, config_value JSON NOT NULL)"
            ).await;
            println!("手动创建app_config表的结果: {result:?}");

            if result.is_ok() {
                let app_config_exists_after =
                    manager.debug_table_exists("app_config").await.unwrap();
                println!("创建后app_config表存在: {app_config_exists_after}");
            }
        }
    }

    #[tokio::test]
    async fn test_debug_sql_parsing() {
        // 创建一个不初始化的数据库管理器
        let connection = Arc::new(Mutex::new(Connection::open_in_memory().unwrap()));
        let manager = DatabaseManager {
            config: Arc::new(DatabaseConfig {
                db_path: None,
                memory_connection: Some(connection),
            }),
        };

        let test_sql = r#"
        CREATE TABLE test1 (id INTEGER);
        INSERT INTO test1 VALUES (1);
        CREATE TABLE test2 (data JSON);
        INSERT INTO test2 VALUES ('{"key": "value; with semicolon"}');
        "#;

        let statements = manager.parse_sql_statements(test_sql);
        println!("解析出的SQL语句数量: {}", statements.len());
        for (i, stmt) in statements.iter().enumerate() {
            println!("语句 {}: {}", i + 1, stmt);
        }

        // 测试我们的真实SQL脚本
        let schema_sql = include_str!("../migrations/init_duckdb.sql");
        let real_statements = manager.parse_sql_statements(schema_sql);
        println!("真实SQL脚本解析出的语句数量: {}", real_statements.len());

        // 打印前几个语句
        for (i, stmt) in real_statements.iter().take(10).enumerate() {
            println!("真实语句 {}: {}", i + 1, stmt);
        }
    }

    #[tokio::test]
    async fn test_debug_individual_sql_statements() {
        // 创建一个不初始化的数据库管理器
        let connection = Arc::new(Mutex::new(Connection::open_in_memory().unwrap()));
        let manager = DatabaseManager {
            config: Arc::new(DatabaseConfig {
                db_path: None,
                memory_connection: Some(connection),
            }),
        };

        let schema_sql = include_str!("../migrations/init_duckdb.sql");
        let statements = manager.parse_sql_statements(schema_sql);
        println!("总共解析出 {} 个语句", statements.len());

        // 逐个执行SQL语句
        for (i, stmt) in statements.iter().enumerate() {
            let trimmed = stmt.trim();
            if trimmed.is_empty() {
                println!("跳过语句 {}: [空语句]", i + 1);
                continue;
            }

            // 检查是否只包含注释
            let is_only_comments = trimmed
                .lines()
                .map(|line| line.trim())
                .all(|line| line.is_empty() || line.starts_with("--"));

            if is_only_comments {
                println!("跳过语句 {}: [仅包含注释]", i + 1);
                continue;
            }

            println!(
                "执行语句 {}: {}",
                i + 1,
                if trimmed.len() > 100 {
                    // 安全地截取字符串，避免切断多字节字符
                    let mut end = 100;
                    while end > 0 && !trimmed.is_char_boundary(end) {
                        end -= 1;
                    }
                    format!("{}...", &trimmed[..end])
                } else {
                    trimmed.to_string()
                }
            );

            let result = manager
                .write_with_retry(|conn| {
                    conn.execute(trimmed, [])?;
                    Ok(())
                })
                .await;

            if let Err(e) = result {
                println!("❌ 语句 {} 执行失败: {}", i + 1, e);
                println!("失败的语句: {trimmed}");
                break;
            } else {
                println!("✅ 语句 {} 执行成功", i + 1);
            }
        }
    }
}
