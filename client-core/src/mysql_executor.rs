use crate::container::{DockerManager, MissingVariables, interpolate_env};
use crate::sql_diff::TableDefinition;
use anyhow::{Context, Result, anyhow};
use docker_compose_types as dct;
use mysql_async::prelude::*;
use mysql_async::{OptsBuilder, Pool, Row, from_row};
use std::collections::HashMap;
use std::path::Path;

/// MySQL容器异步差异SQL执行器
/// 专为Duck Client自动升级部署设计
pub struct MySqlExecutor {
    pool: Pool,
    /// 应用账号（管理连接捕获的 MYSQL_USER；manifest permissions 授权对象）
    app_user: Option<String>,
}

/// MySQL配置适配现有系统
#[derive(Debug, Clone)]
pub struct MySqlConfig {
    pub host: String,
    pub port: u16,
    pub user: String,
    pub password: String,
    /// 默认库：应用连接为 `MYSQL_DATABASE`；root 管理连接为 `None`
    /// （多库迁移时库切换由 diff 内的 `USE` 段完成）
    pub database: Option<String>,
    /// 应用账号（root 管理连接捕获 `MYSQL_USER`，供 manifest permissions 授权）
    pub app_user: Option<String>,
}

/// 从 compose 解析出的 mysql 服务连接要素（与凭据用途无关）
struct MysqlServiceEndpoint {
    port: u16,
    env: HashMap<String, String>,
}

fn parse_short_published_port(port: &str, target_port: u16) -> Option<u16> {
    let mut segments = port.rsplit(':');
    let mapped_target = segments.next()?.parse::<u16>().ok()?;
    if mapped_target != target_port {
        return None;
    }
    segments.next()?.parse::<u16>().ok()
}

/// 解析 mysql 服务：插值引用校验 + 端口映射 + environment 键值
async fn resolve_mysql_service_endpoint(
    compose_file: Option<&str>,
    env_file: Option<&str>,
) -> Result<MysqlServiceEndpoint> {
    let docker_manager = match (compose_file, env_file) {
        (Some(c), Some(e)) => DockerManager::with_project(c, e, None)?,
        _ => {
            return Err(anyhow!(
                "docker-compose.yml and .env paths are required to load Docker Compose configuration"
            ));
        }
    };
    let compose_config = docker_manager
        .load_compose_config()
        .context("Failed to load Docker Compose configuration")?;

    let mysql_service = compose_config
        .services
        .0
        .get("mysql")
        .and_then(|s| s.as_ref())
        .ok_or_else(|| anyhow!("'mysql' service not found in docker-compose.yml"))?;

    // The shared Compose loader has already resolved these scalars. A
    // second interpolation would corrupt literal dollars in credentials.
    validate_mysql_references(
        docker_manager.get_compose_file(),
        docker_manager.get_env_file(),
    )?;
    let mut config_map = HashMap::new();
    match &mysql_service.environment {
        dct::Environment::List(env_list) => {
            for item in env_list {
                if let Some((key, value)) = item.split_once('=') {
                    config_map.insert(key.to_string(), value.to_string());
                }
            }
        }
        dct::Environment::KvPair(values) => {
            for (key, value) in values {
                if let Some(value) = value {
                    config_map.insert(key.to_string(), value.to_string());
                } else if let Ok(value) = std::env::var(key) {
                    config_map.insert(key.to_string(), value);
                }
            }
        }
    }

    let port = match &mysql_service.ports {
        dct::Ports::Short(ports_list) => ports_list
            .iter()
            .find_map(|p| parse_short_published_port(p, 3306))
            .ok_or_else(|| anyhow!("No mapping to container port 3306 found in 'mysql' service"))?,
        dct::Ports::Long(ports_list) => ports_list
            .iter()
            .find_map(|p| {
                if p.target == 3306 {
                    match &p.published {
                        Some(dct::PublishedPort::Single(port_num)) => Some(*port_num),
                        Some(dct::PublishedPort::Range(port_str)) => port_str.parse::<u16>().ok(),
                        None => None,
                    }
                } else {
                    None
                }
            })
            .ok_or_else(|| anyhow!("No mapping to container port 3306 found in 'mysql' service"))?,
    };

    Ok(MysqlServiceEndpoint {
        port,
        env: config_map,
    })
}

impl MySqlConfig {
    /// 通过解析 docker-compose.yml 文件为容器环境适配配置（应用账号 + 默认库）
    pub async fn for_container(compose_file: Option<&str>, env_file: Option<&str>) -> Result<Self> {
        let endpoint = resolve_mysql_service_endpoint(compose_file, env_file).await?;
        let user = endpoint
            .env
            .get("MYSQL_USER")
            .cloned()
            .unwrap_or_else(|| "root".to_string());
        let password = endpoint
            .env
            .get("MYSQL_PASSWORD")
            .cloned()
            .unwrap_or_else(|| "root".to_string());
        let database = endpoint
            .env
            .get("MYSQL_DATABASE")
            .cloned()
            .unwrap_or_else(|| "agent_platform".to_string());

        Ok(MySqlConfig {
            host: "127.0.0.1".to_string(),
            port: endpoint.port,
            user,
            password,
            database: Some(database),
            app_user: None,
        })
    }

    /// root 管理连接：多库 schema 迁移（Live Diff）专用。
    /// 建库（CREATE DATABASE）、授权（GRANT）与跨库 DDL 均超出应用账号权限；
    /// 不设默认库，库切换由生成的 diff 内 `USE` 段完成。
    /// `app_user` 捕获官方镜像创建的应用账号（manifest permissions 授权对象）。
    /// 缺少 `MYSQL_ROOT_PASSWORD` 时 Fail Fast，错误信息不回显任何凭据。
    pub async fn for_container_admin(
        compose_file: Option<&str>,
        env_file: Option<&str>,
    ) -> Result<Self> {
        let endpoint = resolve_mysql_service_endpoint(compose_file, env_file).await?;
        let password = endpoint
            .env
            .get("MYSQL_ROOT_PASSWORD")
            .cloned()
            .ok_or_else(|| {
                anyhow!(
                    "'mysql' service environment must define MYSQL_ROOT_PASSWORD for schema migration"
                )
            })?;
        let app_user = endpoint.env.get("MYSQL_USER").cloned();

        Ok(MySqlConfig {
            host: "127.0.0.1".to_string(),
            port: endpoint.port,
            user: "root".to_string(),
            password,
            database: None,
            app_user,
        })
    }
}

/// Validate references in the original credential expressions, never in the
/// resolved credentials themselves (which may legally contain $ or ${...}).
fn validate_mysql_references(compose_path: &Path, env_path: &Path) -> Result<()> {
    let source = std::fs::read_to_string(compose_path)?;
    let compose: serde_yaml::Value = serde_yaml::from_str(&source)?;
    let values = crate::container::load_env_values(env_path)?;
    let environment = &compose["services"]["mysql"]["environment"];
    let validate = |key: &str, expression: &str| -> Result<()> {
        if ![
            "MYSQL_USER",
            "MYSQL_PASSWORD",
            "MYSQL_DATABASE",
            "MYSQL_ROOT_PASSWORD",
        ]
        .contains(&key)
        {
            return Ok(());
        }
        interpolate_env(
            expression,
            &|name| {
                std::env::var(name)
                    .ok()
                    .or_else(|| values.get(name).cloned())
            },
            MissingVariables::Error,
        )
        .map_err(|_| {
            anyhow!(
                "MySQL {key} references an unavailable environment variable; define it in {}",
                env_path.display()
            )
        })?;
        Ok(())
    };
    match environment {
        serde_yaml::Value::Sequence(entries) => {
            for entry in entries {
                if let Some((key, value)) = entry.as_str().and_then(|entry| entry.split_once('=')) {
                    validate(key, value)?;
                }
            }
        }
        serde_yaml::Value::Mapping(entries) => {
            for (key, value) in entries {
                if let (Some(key), Some(value)) = (key.as_str(), value.as_str()) {
                    validate(key, value)?;
                }
            }
        }
        _ => {}
    }
    Ok(())
}

impl MySqlExecutor {
    /// 创建新的执行器
    pub fn new(config: MySqlConfig) -> Self {
        let opts = OptsBuilder::default()
            .ip_or_hostname(config.host.clone())
            .tcp_port(config.port)
            .user(Some(config.user.clone()))
            .pass(Some(config.password.clone()))
            .db_name(config.database.clone());
        let pool = Pool::new(opts);
        Self {
            pool,
            app_user: config.app_user.clone(),
        }
    }

    /// 应用账号（管理连接捕获的 `MYSQL_USER`）
    pub fn app_user(&self) -> Option<&str> {
        self.app_user.as_deref()
    }

    /// 测试连接是否可用
    pub async fn test_connection(&self) -> Result<(), mysql_async::Error> {
        let mut conn = self.pool.get_conn().await?;
        conn.query_drop("SELECT 1").await?;
        Ok(())
    }

    /// 执行单个SQL语句。
    /// 显式排空剩余结果集：若调用方传入多条语句（分号分隔），
    /// 未排空会把池化连接留在协议失步状态，后续查询会读到残留结果集。
    pub async fn execute_single(&self, sql: &str) -> Result<u64, mysql_async::Error> {
        let mut conn = self.pool.get_conn().await?;
        let result = conn.query_iter(sql).await?;
        let affected = result.affected_rows();
        result.drop_result().await?;
        Ok(affected)
    }

    /// 在同一连接上顺序执行差异 SQL。MySQL DDL 不支持整批事务回滚。
    pub async fn execute_diff_sql(&self, sql_content: &str) -> Result<Vec<String>, anyhow::Error> {
        self.execute_diff_sql_once(sql_content).await
    }

    /// 保留旧接口签名；DDL 无法安全地整批重试，重试参数不再生效。
    #[deprecated(note = "DDL batches are not safely retryable; use execute_diff_sql_once")]
    pub async fn execute_diff_sql_with_retry(
        &self,
        sql_content: &str,
        _max_retries: u8,
    ) -> Result<Vec<String>, anyhow::Error> {
        self.execute_diff_sql_once(sql_content).await
    }

    /// 出错即停，错误中包含已完成数量与失败语句；重跑时应先重新生成 Live Diff。
    pub async fn execute_diff_sql_once(
        &self,
        sql_content: &str,
    ) -> Result<Vec<String>, anyhow::Error> {
        let statements = self.parse_sql_commands(sql_content);
        let mut conn = self.pool.get_conn().await?;
        let mut results = Vec::new();
        for (idx, sql) in statements.iter().enumerate() {
            if sql.starts_with("--") || sql.trim().is_empty() {
                continue;
            }
            conn.query_drop(sql).await.with_context(|| {
                format!(
                    "Diff SQL failed at statement {} after {} successful statements",
                    idx + 1,
                    results.len()
                )
            })?;
            tracing::info!(statement_index = idx + 1, statement = %sql, "Diff SQL statement applied");
            results.push(format!("[{}] ✅ {}", idx + 1, sql));
        }
        Ok(results)
    }

    /// 解析SQL内容为可执行的命令列表
    fn parse_sql_commands(&self, sql_content: &str) -> Vec<String> {
        let mut commands = Vec::new();
        let mut current_command = String::new();

        for line in sql_content.lines() {
            let line = line.trim();

            if line.starts_with("--") || line.is_empty() {
                continue;
            }

            current_command.push_str(line);
            current_command.push(' ');

            // 如果行的末尾是分号SQL结束
            if line.ends_with(';') || line.ends_with("ENGINE=InnoDB;") || line.ends_with(");") {
                commands.push(current_command.trim().to_string());
                current_command.clear();
            }
        }

        if !current_command.trim().is_empty() {
            commands.push(current_command.trim().to_string());
        }

        commands
    }

    /// 获取数据库表结构信息
    pub async fn get_table_info(&self, table_name: &str) -> Result<(), mysql_async::Error> {
        let mut conn = self.pool.get_conn().await?;
        let results: Vec<Row> = conn.query(format!("DESCRIBE {table_name}")).await?;

        for row in results {
            println!("{row:?}");
        }
        Ok(())
    }

    /// 抓取指定库的在线数据库架构：通过 SHOW CREATE TABLE 获取真实DDL，再用 sqlparser 解析为内部类型。
    /// 库不存在或为空库时返回空表集（多库 Live Diff 据此生成"建库+全量建表"）
    pub async fn fetch_live_schema(
        &self,
        schema: &str,
    ) -> Result<std::collections::HashMap<String, TableDefinition>, anyhow::Error> {
        let (tables, _sql) = self.fetch_live_schema_with_sql(schema).await?;
        Ok(tables)
    }

    /// 抓取指定库的在线数据库架构并返回原始 SQL
    /// 返回：(解析后的表定义, 原始 CREATE TABLE SQL)
    pub async fn fetch_live_schema_with_sql(
        &self,
        schema: &str,
    ) -> Result<(std::collections::HashMap<String, TableDefinition>, String), anyhow::Error> {
        use crate::sql_diff::parse_sql_tables_strict;

        let mut conn = self.pool.get_conn().await?;

        // 获取指定库的所有表名
        let table_names: Vec<String> = conn
            .exec(
                r#"SELECT TABLE_NAME
                    FROM INFORMATION_SCHEMA.TABLES
                    WHERE TABLE_SCHEMA = ?
                    ORDER BY TABLE_NAME"#,
                (schema,),
            )
            .await?
            .into_iter()
            .map(|row| {
                let (name,): (String,) = from_row(row);
                name
            })
            .collect();

        // 拼接所有表的 CREATE 语句（全限定名，不依赖连接的默认库）
        let mut create_sqls = String::new();
        for table in &table_names {
            let query = format!("SHOW CREATE TABLE `{}`.`{}`", schema, table);
            let row: Row = conn.exec_first(query, ()).await?.ok_or_else(|| {
                anyhow::anyhow!(format!(
                    "Failed to get CREATE statement for table: {}",
                    table
                ))
            })?;
            // MySQL返回两列：Table, Create Table
            let (_tbl_name, create_stmt): (String, String) = from_row(row);
            create_sqls.push_str(&create_stmt);

            // 确保每个 CREATE TABLE 语句以分号结尾
            if !create_stmt.trim().ends_with(';') {
                create_sqls.push(';');
            }
            create_sqls.push_str("\n\n");
        }

        // 使用 sqlparser 解析 DDL，严格避免正则
        let tables = parse_sql_tables_strict(&create_sqls)
            .map_err(|e| anyhow::anyhow!(format!("Failed to parse online DDL: {}", e)))?;

        Ok((tables, create_sqls))
    }

    /// 判断库是否存在（仅用于日志与状态展示）
    pub async fn schema_exists(&self, schema: &str) -> Result<bool, mysql_async::Error> {
        let mut conn = self.pool.get_conn().await?;
        let exists: Option<(i64,)> = conn
            .exec_first(
                r#"SELECT COUNT(*) FROM INFORMATION_SCHEMA.SCHEMATA WHERE SCHEMA_NAME = ?"#,
                (schema,),
            )
            .await?;
        Ok(exists.is_some_and(|(count,)| count > 0))
    }

    /// 验证执行结果
    pub async fn verify_execution(
        &self,
        _expected_changes: &str,
    ) -> Result<bool, mysql_async::Error> {
        let mut conn = self.pool.get_conn().await?;

        // 简单的执行确认
        let result: Option<(i32,)> = conn.query_first("SELECT 1 as verification_status").await?;
        if let Some((1,)) = result {
            Ok(true)
        } else {
            Ok(false)
        }
    }

    /// 检查数据库连接健康
    pub async fn health_check(&self) -> HealthStatus {
        match self.test_connection().await {
            Ok(_) => HealthStatus::Healthy,
            Err(e) => HealthStatus::Failed(e.to_string()),
        }
    }
}

/// 健康状态枚举
#[derive(Debug, Clone)]
pub enum HealthStatus {
    Healthy,
    Failed(String),
}

/// 执行结果记录
#[derive(Debug, Clone)]
pub struct ExecutionResult {
    pub sql: String,
    pub status: bool,
    pub rows_affected: Option<u64>,
    pub error: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_short_mysql_port_with_bind_address() {
        assert_eq!(parse_short_published_port("13306:3306", 3306), Some(13306));
        assert_eq!(
            parse_short_published_port("127.0.0.1:23306:3306", 3306),
            Some(23306)
        );
        assert_eq!(parse_short_published_port("16379:6379", 3306), None);
    }

    #[tokio::test]
    async fn compose_credentials_preserve_literal_dollars_quotes_hashes_and_colons() -> Result<()> {
        let directory = tempfile::tempdir()?;
        let compose = directory.path().join("compose.yml");
        let env_path = directory.path().join("credentials.env");
        std::fs::write(
            &compose,
            "services:\n  mysql:\n    image: mysql:8.0\n    ports: [\"13306:3306\"]\n    environment:\n      - MYSQL_USER=${NUWAX_TEST_DB_USER}\n      - MYSQL_PASSWORD=${NUWAX_TEST_DB_PASSWORD}\n      - MYSQL_DATABASE=${NUWAX_TEST_DB_NAME}\n",
        )?;
        let password = "synthetic $word ${word} $$ # literal: value \"quote\"";
        std::fs::write(
            &env_path,
            format!(
                "NUWAX_TEST_DB_USER=synthetic\nNUWAX_TEST_DB_PASSWORD='{password}'\nNUWAX_TEST_DB_NAME=synthetic_db\n"
            ),
        )?;
        let config = MySqlConfig::for_container(
            Some(compose.to_str().unwrap()),
            Some(env_path.to_str().unwrap()),
        )
        .await?;
        assert_eq!(config.password, password);
        assert_eq!(config.user, "synthetic");
        assert_eq!(config.database.as_deref(), Some("synthetic_db"));
        assert_eq!(config.port, 13306);
        Ok(())
    }

    #[tokio::test]
    async fn missing_mysql_reference_fails_without_printing_credentials() -> Result<()> {
        let directory = tempfile::tempdir()?;
        let compose = directory.path().join("compose.yml");
        let env_path = directory.path().join("credentials.env");
        std::fs::write(
            &compose,
            "services:\n  mysql:\n    image: mysql:8.0\n    ports: [\"13306:3306\"]\n    environment:\n      - MYSQL_PASSWORD=${NUWAX_MISSING_DB_PASSWORD}\n",
        )?;
        std::fs::write(&env_path, "UNRELATED='synthetic-sensitive-value'\n")?;
        let error = MySqlConfig::for_container(
            Some(compose.to_str().unwrap()),
            Some(env_path.to_str().unwrap()),
        )
        .await
        .unwrap_err();
        assert!(error.to_string().contains("MYSQL_PASSWORD"));
        assert!(!format!("{error:#}").contains("synthetic-sensitive-value"));
        Ok(())
    }

    #[tokio::test]
    async fn mysql_defaults_and_mapping_environment_remain_supported() -> Result<()> {
        let directory = tempfile::tempdir()?;
        let compose = directory.path().join("compose.yml");
        let env_path = directory.path().join("credentials.env");
        std::fs::write(
            &compose,
            "services:\n  mysql:\n    image: mysql:8.0\n    ports: [\"13306:3306\"]\n    environment:\n      MYSQL_PASSWORD: ${NUWAX_MISSING_PASSWORD_WITH_DEFAULT:-fallback}\n",
        )?;
        std::fs::write(&env_path, "")?;
        let config = MySqlConfig::for_container(
            Some(compose.to_str().unwrap()),
            Some(env_path.to_str().unwrap()),
        )
        .await?;
        assert_eq!(config.user, "root");
        assert_eq!(config.database.as_deref(), Some("agent_platform"));
        assert_eq!(config.password, "fallback");
        Ok(())
    }

    #[tokio::test]
    async fn mysql_defaults_distinguish_unset_and_empty_values() -> Result<()> {
        for (expression, defined, expected) in [
            ("${NUWAX_PASSWORD_FALLBACK-fallback}", false, "fallback"),
            ("${NUWAX_PASSWORD_FALLBACK-fallback}", true, ""),
            ("${NUWAX_PASSWORD_FALLBACK:-fallback}", false, "fallback"),
            ("${NUWAX_PASSWORD_FALLBACK:-fallback}", true, "fallback"),
        ] {
            let directory = tempfile::tempdir()?;
            let compose = directory.path().join("compose.yml");
            let env_path = directory.path().join("credentials.env");
            std::fs::write(
                &compose,
                format!(
                    "services:\n  mysql:\n    image: mysql:8.0\n    ports: [\"13306:3306\"]\n    environment:\n      - MYSQL_PASSWORD={expression}\n"
                ),
            )?;
            std::fs::write(
                &env_path,
                if defined {
                    "NUWAX_PASSWORD_FALLBACK=\n"
                } else {
                    ""
                },
            )?;
            let config = MySqlConfig::for_container(
                Some(compose.to_str().unwrap()),
                Some(env_path.to_str().unwrap()),
            )
            .await?;
            assert_eq!(config.password, expected);
        }
        Ok(())
    }

    #[tokio::test]
    async fn admin_config_uses_root_password_without_default_database() -> Result<()> {
        let directory = tempfile::tempdir()?;
        let compose = directory.path().join("compose.yml");
        let env_path = directory.path().join("credentials.env");
        std::fs::write(
            &compose,
            "services:\n  mysql:\n    image: mysql:8.0\n    ports: [\"13306:3306\"]\n    environment:\n      - MYSQL_ROOT_PASSWORD=${NUWAX_TEST_ROOT_PASSWORD}\n      - MYSQL_DATABASE=app_db\n",
        )?;
        let password = "synthetic $root {secret} $$ # literal";
        std::fs::write(
            &env_path,
            format!("NUWAX_TEST_ROOT_PASSWORD='{password}'\n"),
        )?;
        let config = MySqlConfig::for_container_admin(
            Some(compose.to_str().unwrap()),
            Some(env_path.to_str().unwrap()),
        )
        .await?;
        assert_eq!(config.user, "root");
        assert_eq!(config.password, password);
        assert_eq!(config.database, None);
        assert_eq!(config.port, 13306);
        Ok(())
    }

    #[tokio::test]
    async fn admin_config_missing_root_password_fails_without_printing_credentials() -> Result<()> {
        let directory = tempfile::tempdir()?;
        let compose = directory.path().join("compose.yml");
        let env_path = directory.path().join("credentials.env");
        // 应用账号配置完整，但缺少 MYSQL_ROOT_PASSWORD
        std::fs::write(
            &compose,
            "services:\n  mysql:\n    image: mysql:8.0\n    ports: [\"13306:3306\"]\n    environment:\n      - MYSQL_USER=app\n      - MYSQL_PASSWORD=${NUWAX_TEST_APP_PASSWORD}\n      - MYSQL_DATABASE=app_db\n",
        )?;
        std::fs::write(
            &env_path,
            "NUWAX_TEST_APP_PASSWORD='synthetic-sensitive-value'\n",
        )?;
        let error = MySqlConfig::for_container_admin(
            Some(compose.to_str().unwrap()),
            Some(env_path.to_str().unwrap()),
        )
        .await
        .unwrap_err();
        assert!(error.to_string().contains("MYSQL_ROOT_PASSWORD"));
        assert!(!format!("{error:#}").contains("synthetic-sensitive-value"));
        Ok(())
    }

    #[tokio::test]
    #[ignore = "requires TEST_MYSQL_URL pointing to a disposable MySQL database"]
    async fn test_mysql_connection() -> Result<()> {
        let url = std::env::var("TEST_MYSQL_URL").context("TEST_MYSQL_URL is required")?;
        let options = mysql_async::Opts::from_url(&url)
            .map_err(|_| anyhow::anyhow!("TEST_MYSQL_URL must be a valid MySQL URL"))?;
        let database = options
            .db_name()
            .filter(|name| !name.is_empty())
            .context("TEST_MYSQL_URL must name a disposable database")?;
        let config = MySqlConfig {
            host: options.ip_or_hostname().to_string(),
            port: options.tcp_port(),
            user: options
                .user()
                .context("TEST_MYSQL_URL requires a user")?
                .to_string(),
            password: options.pass().unwrap_or_default().to_string(),
            database: Some(database.to_string()),
            app_user: None,
        };
        let executor = MySqlExecutor::new(config);
        executor.test_connection().await?;
        let table = format!("nuwax_connection_{}", uuid::Uuid::now_v7().simple());
        let statements = format!(
            "CREATE TABLE `{table}` (id INT PRIMARY KEY AUTO_INCREMENT, name VARCHAR(100));\n\
             ALTER TABLE `{table}` ADD COLUMN email VARCHAR(255);\n\
             CREATE INDEX idx_name ON `{table}`(name);"
        );
        let execution = executor.execute_diff_sql(&statements).await;
        // Clean the unique table even when a later DDL statement fails.
        let cleanup = executor
            .execute_single(&format!("DROP TABLE IF EXISTS `{table}`"))
            .await;
        let results = execution?;
        cleanup?;
        assert_eq!(results.len(), 3);
        Ok(())
    }

    #[tokio::test]
    async fn test_parse_sql_commands() {
        let content = "-- 注释\n\
                      CREATE TABLE users (id INT);\n\
                      ALTER TABLE users ADD COLUMN name VARCHAR(100);\n\
                      CREATE INDEX idx_name ON users(name);";

        let manifest_dir = std::env::var("CARGO_MANIFEST_DIR").unwrap();
        let compose_path = std::path::Path::new(&manifest_dir).join("fixtures/docker-compose.yml");
        let env_path = std::path::Path::new(&manifest_dir).join("fixtures/.env");
        let config = MySqlConfig::for_container(
            Some(compose_path.to_str().unwrap()),
            Some(env_path.to_str().unwrap()),
        )
        .await
        .unwrap();
        let executor = MySqlExecutor::new(config);

        let commands = executor.parse_sql_commands(content);
        assert_eq!(commands.len(), 3);
        assert!(commands[0].contains("CREATE TABLE users"));
        assert!(commands[1].contains("ALTER TABLE users ADD COLUMN name"));
    }

    #[tokio::test]
    async fn test_empty_and_comments() {
        let content = "-- This is a comment\n\nCREATE TABLE test (id INT);\n-- Another comment";
        let manifest_dir = std::env::var("CARGO_MANIFEST_DIR").unwrap();
        let compose_path = std::path::Path::new(&manifest_dir).join("fixtures/docker-compose.yml");
        let env_path = std::path::Path::new(&manifest_dir).join("fixtures/.env");
        let config = MySqlConfig::for_container(
            Some(compose_path.to_str().unwrap()),
            Some(env_path.to_str().unwrap()),
        )
        .await
        .unwrap();
        let executor = MySqlExecutor::new(config);

        let commands = executor.parse_sql_commands(content);
        assert_eq!(commands.len(), 1);
        assert_eq!(commands[0], "CREATE TABLE test (id INT);");
    }

    #[tokio::test]
    async fn test_multi_db_preamble_and_use_statements_split_correctly() {
        // 多库 Live Diff 组装结果：建库原句 / USE / DDL 必须各自成为独立语句
        let content = "-- ===== Database: `nuwax_im` =====\n\
                      CREATE DATABASE IF NOT EXISTS `nuwax_im` DEFAULT CHARACTER SET utf8mb4 COLLATE utf8mb4_0900_ai_ci;\n\
                      GRANT ALL PRIVILEGES ON nuwax_im.* TO 'agent_platform'@'%';\n\
                      FLUSH PRIVILEGES;\n\
                      USE `nuwax_im`;\n\
                      CREATE TABLE `im_agent_binding` (id bigint NOT NULL, PRIMARY KEY (`id`)) ENGINE=InnoDB;\n\
                      ALTER TABLE `im_agent_binding` ADD COLUMN mode tinyint NOT NULL DEFAULT '1';";

        let manifest_dir = std::env::var("CARGO_MANIFEST_DIR").unwrap();
        let compose_path = std::path::Path::new(&manifest_dir).join("fixtures/docker-compose.yml");
        let env_path = std::path::Path::new(&manifest_dir).join("fixtures/.env");
        let config = MySqlConfig::for_container(
            Some(compose_path.to_str().unwrap()),
            Some(env_path.to_str().unwrap()),
        )
        .await
        .unwrap();
        let executor = MySqlExecutor::new(config);

        let commands = executor.parse_sql_commands(content);
        assert_eq!(commands.len(), 6, "statements: {commands:?}");
        assert!(commands[0].starts_with("CREATE DATABASE IF NOT EXISTS"));
        assert!(commands[1].starts_with("GRANT ALL PRIVILEGES"));
        assert_eq!(commands[2], "FLUSH PRIVILEGES;");
        assert_eq!(commands[3], "USE `nuwax_im`;");
        assert!(commands[4].starts_with("CREATE TABLE `im_agent_binding`"));
        assert!(commands[5].starts_with("ALTER TABLE `im_agent_binding`"));
    }

    #[test]
    fn test_table_name_normalization() {
        // 测试表名标准化：确保带反引号和不带反引号的表名被识别为同一个表
        use crate::sql_diff::parse_sql_tables;

        // SQL 1: 带反引号的表名
        let sql_with_backticks = "CREATE TABLE `test_table` (\n  `id` int NOT NULL AUTO_INCREMENT,\n  PRIMARY KEY (`id`)\n) ENGINE=InnoDB;";

        // SQL 2: 不带反引号的表名
        let sql_without_backticks = "CREATE TABLE test_table (\n  id int NOT NULL AUTO_INCREMENT,\n  PRIMARY KEY (id)\n) ENGINE=InnoDB;";

        let tables1 = parse_sql_tables(sql_with_backticks).expect("解析带反引号的 SQL 失败");
        let tables2 = parse_sql_tables(sql_without_backticks).expect("解析不带反引号的 SQL 失败");

        // 两种情况都应该解析出相同的表名（不带反引号）
        assert!(
            tables1.contains_key("test_table"),
            "带反引号的表名应该被标准化为 test_table"
        );
        assert!(
            tables2.contains_key("test_table"),
            "不带反引号的表名应该是 test_table"
        );

        // 确保不会有带反引号的 key
        assert!(
            !tables1.contains_key("`test_table`"),
            "不应该有带反引号的表名作为 key"
        );
        assert!(
            !tables2.contains_key("`test_table`"),
            "不应该有带反引号的表名作为 key"
        );

        println!("✅ 表名标准化测试通过");
    }

    #[test]
    fn test_sql_diff_with_same_tables() {
        // 测试 SQL diff：模拟从 MySQL 读取的表（带反引号）与文件中的表（不带反引号）
        use crate::sql_diff::{generate_schema_diff, parse_sql_tables};

        // 模拟从 MySQL SHOW CREATE TABLE 返回的 SQL（带反引号）
        let mysql_sql = "CREATE TABLE `custom_page_config` (\n  `id` bigint NOT NULL AUTO_INCREMENT,\n  `name` varchar(255) NOT NULL,\n  PRIMARY KEY (`id`)\n) ENGINE=InnoDB;";

        // 模拟从文件读取的 SQL（不带反引号）
        let file_sql = "CREATE TABLE custom_page_config (\n  id bigint NOT NULL AUTO_INCREMENT,\n  name varchar(255) NOT NULL,\n  PRIMARY KEY (id)\n) ENGINE=InnoDB;";

        let mysql_tables = parse_sql_tables(mysql_sql).expect("解析 MySQL SQL 失败");
        let file_tables = parse_sql_tables(file_sql).expect("解析文件 SQL 失败");

        println!("MySQL 表: {:?}", mysql_tables.keys().collect::<Vec<_>>());
        println!("文件表: {:?}", file_tables.keys().collect::<Vec<_>>());

        // 生成差异 SQL（使用 SQL 字符串作为参数）
        let (diff_sql, description) =
            generate_schema_diff(Some(mysql_sql), file_sql, Some("在线架构"), "目标版本")
                .expect("生成差异 SQL 失败");

        println!("差异描述: {}", description);
        println!("差异 SQL:\n{}", diff_sql);

        // 由于两个表结构相同，不应该有任何差异
        let meaningful_lines: Vec<&str> = diff_sql
            .lines()
            .filter(|line| !line.trim().is_empty() && !line.trim().starts_with("--"))
            .collect();

        assert!(
            meaningful_lines.is_empty(),
            "相同的表不应该产生差异 SQL，但生成了: {:?}",
            meaningful_lines
        );

        println!("✅ SQL diff 测试通过：相同的表没有产生差异");
    }

    #[test]
    fn test_create_table_concatenation_with_semicolons() {
        // 模拟从 MySQL SHOW CREATE TABLE 返回的多个语句（没有分号）
        let mut create_sqls = String::new();

        // 模拟第一个表的 CREATE 语句（没有分号）
        let stmt1 = "CREATE TABLE `agent_config` (\n  `id` int NOT NULL AUTO_INCREMENT,\n  PRIMARY KEY (`id`)\n) ENGINE=InnoDB";
        create_sqls.push_str(stmt1);

        // 添加分号（这是我们的修复）
        if !stmt1.trim().ends_with(';') {
            create_sqls.push(';');
        }
        create_sqls.push_str("\n\n");

        // 模拟第二个表的 CREATE 语句（没有分号）
        let stmt2 = "CREATE TABLE `agent_component_config` (\n  `id` int NOT NULL AUTO_INCREMENT,\n  PRIMARY KEY (`id`)\n) ENGINE=InnoDB";
        create_sqls.push_str(stmt2);

        // 添加分号
        if !stmt2.trim().ends_with(';') {
            create_sqls.push(';');
        }
        create_sqls.push_str("\n\n");

        println!("拼接后的 SQL:\n{}", create_sqls);

        // 验证结果：每个 CREATE TABLE 语句都应该以分号结尾
        assert!(
            create_sqls.contains("ENGINE=InnoDB;"),
            "第一个表的语句应该以分号结尾"
        );
        assert!(
            create_sqls.matches("ENGINE=InnoDB;").count() == 2,
            "两个表的语句都应该以分号结尾"
        );

        // 验证可以被 sqlparser 正确解析
        use crate::sql_diff::parse_sql_tables;
        let result = parse_sql_tables(&create_sqls);

        if let Err(ref e) = result {
            println!("解析错误: {}", e);
        }

        assert!(
            result.is_ok(),
            "拼接后的 SQL 应该可以被正确解析: {:?}",
            result.err()
        );

        let tables = result.unwrap();
        println!("解析出的表: {:?}", tables.keys().collect::<Vec<_>>());
        assert_eq!(
            tables.len(),
            2,
            "应该解析出 2 个表，实际解析出 {} 个",
            tables.len()
        );

        // 表名可能带反引号，所以检查两种情况
        let has_agent_config =
            tables.contains_key("agent_config") || tables.contains_key("`agent_config`");
        let has_agent_component_config = tables.contains_key("agent_component_config")
            || tables.contains_key("`agent_component_config`");

        assert!(has_agent_config, "应该包含 agent_config 表");
        assert!(
            has_agent_component_config,
            "应该包含 agent_component_config 表"
        );

        println!("✅ CREATE TABLE 语句拼接测试通过");
    }
}
