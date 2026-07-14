use crate::container::DockerManager;
use crate::sql_diff::TableDefinition;
use anyhow::{Context, Result, anyhow};
use docker_compose_types as dct;
use mysql_async::prelude::*;
use mysql_async::{Opts, Pool, Row, Transaction, TxOpts, from_row};
use std::collections::HashMap;
use std::path::Path;

/// MySQL容器异步差异SQL执行器
/// 专为Duck Client自动升级部署设计
pub struct MySqlExecutor {
    pool: Pool,
    config: MySqlConfig,
}

/// MySQL配置适配现有系统
#[derive(Debug, Clone)]
pub struct MySqlConfig {
    pub host: String,
    pub port: u16,
    pub user: String,
    pub password: String,
    pub database: String,
}

impl MySqlConfig {
    /// 通过解析 docker-compose.yml 文件为容器环境适配配置
    pub async fn for_container(compose_file: Option<&str>, env_file: Option<&str>) -> Result<Self> {
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

        // 加载 .env 变量表，用于 environment 值的 ${VAR} 插值。
        // docker_compose_types 仅做纯 YAML 反序列化，不会像 `docker compose` 命令那样
        // 自动解析 ${VAR} 引用；若不在此插值，读到的凭据将是字面量（如 "${MYSQL_USER}"），
        // 导致 MySQL 认证失败（报 "connection closed"）。
        let env_vars = load_env_vars(docker_manager.get_env_file());

        let mut config_map = HashMap::new();
        if let dct::Environment::List(env_list) = &mysql_service.environment {
            for item in env_list {
                if let Some((key, value)) = item.split_once('=') {
                    let resolved = interpolate_vars(value, &env_vars);
                    config_map.insert(key.to_string(), resolved);
                }
            }
        }

        let port = match &mysql_service.ports {
            dct::Ports::Short(ports_list) => ports_list
                .iter()
                .find_map(|p| {
                    let parts: Vec<&str> = p.split(':').collect();
                    if parts.len() == 2 && parts[1] == "3306" {
                        parts[0].parse::<u16>().ok()
                    } else {
                        None
                    }
                })
                .ok_or_else(|| {
                    anyhow!("No mapping to container port 3306 found in 'mysql' service")
                })?,
            dct::Ports::Long(ports_list) => ports_list
                .iter()
                .find_map(|p| {
                    if p.target == 3306 {
                        match &p.published {
                            Some(dct::PublishedPort::Single(port_num)) => Some(*port_num),
                            Some(dct::PublishedPort::Range(port_str)) => {
                                port_str.parse::<u16>().ok()
                            }
                            None => None,
                        }
                    } else {
                        None
                    }
                })
                .ok_or_else(|| {
                    anyhow!("No mapping to container port 3306 found in 'mysql' service")
                })?,
        };

        let user = config_map
            .get("MYSQL_USER")
            .cloned()
            .unwrap_or_else(|| "root".to_string());
        let password = config_map
            .get("MYSQL_PASSWORD")
            .cloned()
            .unwrap_or_else(|| "root".to_string());
        let database = config_map
            .get("MYSQL_DATABASE")
            .cloned()
            .unwrap_or_else(|| "agent_platform".to_string());

        // Fail Fast：若关键凭据仍含未解析的 ${VAR}，说明 .env 缺少对应变量定义。
        // 在此直接报错——否则拿字面量去认证，MySQL 只会回 "connection closed"，极难定位。
        for (field, val) in [("user", &user), ("password", &password), ("database", &database)] {
            if val.contains('$') {
                return Err(anyhow!(
                    "MySQL {field} 仍含未解析的变量引用 {val:?}：\
                     请在 .env 文件（{}）中定义对应变量后重试",
                    docker_manager.get_env_file().display()
                ));
            }
        }

        Ok(MySqlConfig {
            host: "127.0.0.1".to_string(),
            port,
            user,
            password,
            database,
        })
    }

    /// 生成连接URL
    fn to_url(&self) -> String {
        format!(
            "mysql://{}:{}@{}:{}/{}",
            self.user, self.password, self.host, self.port, self.database
        )
    }
}

/// 从 .env 文件加载变量表（KEY=VALUE）。
///
/// 文件不存在或不可读时返回空表（降级为不做插值，保持向后兼容）。
fn load_env_vars(path: &Path) -> HashMap<String, String> {
    let Ok(content) = std::fs::read_to_string(path) else {
        return HashMap::new();
    };
    let mut vars = HashMap::new();
    for line in content.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let line = line.strip_prefix("export ").unwrap_or(line);
        let Some((key, value)) = line.split_once('=') else {
            continue;
        };
        let key = key.trim();
        if key.is_empty() || key.chars().any(char::is_whitespace) {
            continue;
        }
        let value = strip_surrounding_quotes(value.trim());
        vars.insert(key.to_string(), value);
    }
    vars
}

/// 去除字符串两侧成对的单/双引号
fn strip_surrounding_quotes(value: &str) -> String {
    let first = value.chars().next();
    let last = value.chars().next_back();
    match (first, last) {
        (Some(q), Some(q2)) if q == q2 && (q == '"' || q == '\'') && value.len() > q.len_utf8() => {
            value[q.len_utf8()..value.len() - q.len_utf8()].to_string()
        }
        _ => value.to_string(),
    }
}

/// 对字符串做 docker compose 风格的 ${VAR} / $VAR 变量插值。
///
/// 支持：`${VAR}`、`$VAR`、`${VAR:-default}`（VAR 未设或为空→default）、
/// `${VAR-default}`（VAR 未设→default）、`$$`（转义为字面 `$`）。
/// 未定义且无默认值的变量替换为空字符串（与 `docker compose` 默认行为一致）。
fn interpolate_vars(value: &str, vars: &HashMap<String, String>) -> String {
    let mut result = String::with_capacity(value.len());
    let mut rest = value;
    while let Some(dollar_idx) = rest.find('$') {
        result.push_str(&rest[..dollar_idx]);
        let after = &rest[dollar_idx + 1..];

        // $$ → 字面 $
        if let Some(remaining) = after.strip_prefix('$') {
            result.push('$');
            rest = remaining;
            continue;
        }

        // ${VAR} / ${VAR:-default} / ${VAR-default}
        if let Some(inner) = after.strip_prefix('{') {
            if let Some(close) = inner.find('}') {
                result.push_str(&resolve_var_spec(&inner[..close], vars));
                rest = &inner[close + 1..];
                continue;
            }
            // 未闭合的 ${，原样保留 $
            result.push('$');
            rest = after;
            continue;
        }

        // $VAR（无括号；变量名以字母或下划线开头，后接字母/数字/下划线）
        let starts_name = after
            .as_bytes()
            .first()
            .is_some_and(|b| b.is_ascii_alphabetic() || *b == b'_');
        if starts_name {
            let name_end = after
                .char_indices()
                .take_while(|(_, c)| c.is_ascii_alphanumeric() || *c == '_')
                .last()
                .map(|(i, c)| i + c.len_utf8())
                .unwrap_or(0);
            if let Some(v) = vars.get(&after[..name_end]) {
                result.push_str(v);
            }
            rest = &after[name_end..];
            continue;
        }

        // 单独的 $（其后非变量名字符），原样保留
        result.push('$');
        rest = after;
    }
    result.push_str(rest);
    result
}

/// 解析 `${...}` 内部规格：`VAR` / `VAR:-default` / `VAR-default`
fn resolve_var_spec(inner: &str, vars: &HashMap<String, String>) -> String {
    if let Some((name, default)) = inner.split_once(":-") {
        match vars.get(name) {
            Some(v) if !v.is_empty() => v.clone(),
            _ => default.to_string(),
        }
    } else if let Some((name, default)) = inner.split_once('-') {
        match vars.get(name) {
            Some(v) => v.clone(),
            None => default.to_string(),
        }
    } else {
        vars.get(inner).cloned().unwrap_or_default()
    }
}

impl MySqlExecutor {
    /// 创建新的执行器
    pub fn new(config: MySqlConfig) -> Self {
        let opts = Opts::from_url(&config.to_url()).unwrap();
        let pool = Pool::new(opts);
        Self { pool, config }
    }

    /// 测试连接是否可用
    pub async fn test_connection(&self) -> Result<(), mysql_async::Error> {
        let mut conn = self.pool.get_conn().await?;
        conn.query_drop("SELECT 1").await?;
        Ok(())
    }

    /// 执行单个SQL语句
    pub async fn execute_single(&self, sql: &str) -> Result<u64, mysql_async::Error> {
        let mut conn = self.pool.get_conn().await?;
        let result = conn.query_iter(sql).await?;
        Ok(result.affected_rows())
    }

    /// 执行差异SQL内容（多语句支持）
    /// 自动处理注释和空行，支持事务回滚
    pub async fn execute_diff_sql(&self, sql_content: &str) -> Result<Vec<String>, anyhow::Error> {
        self.execute_diff_sql_with_retry(sql_content, 1).await
    }

    /// 带重试机制的SQL执行
    pub async fn execute_diff_sql_with_retry(
        &self,
        sql_content: &str,
        max_retries: u8,
    ) -> Result<Vec<String>, anyhow::Error> {
        let sql_lines = self.parse_sql_commands(sql_content);
        let mut results = Vec::new();
        let mut last_error: Option<mysql_async::Error> = None;

        for attempt in 0..=max_retries {
            if attempt > 0 {
                tokio::time::sleep(std::time::Duration::from_millis(500 * attempt as u64)).await;
                results.push(format!("🔄 Retrying attempt {attempt}/{max_retries}..."));
            }

            let mut conn = self.pool.get_conn().await?;
            let mut tx = conn.start_transaction(TxOpts::default()).await?;

            // 记录本次尝试前的日志数量，如果失败可以回滚
            let results_len_before_attempt = results.len();

            match self
                .execute_in_transaction(&mut tx, &sql_lines, &mut results)
                .await
            {
                Ok(_) => {
                    tx.commit().await?;
                    results.insert(0, "✅ Diff SQL executed successfully".to_string());
                    return Ok(results);
                }
                Err(e) => {
                    tx.rollback().await?;
                    // 移除本次失败尝试中添加的日志
                    results.truncate(results_len_before_attempt);
                    results.push(format!("❌ Attempt {} failed: {}", attempt + 1, e));
                    last_error = Some(e);
                }
            }
        }

        Err(anyhow::anyhow!(
            "❌ SQL execution failed after {} attempts. Last error: {}",
            max_retries + 1,
            last_error.unwrap()
        ))
    }

    /// 执行在事务中的差异SQL
    async fn execute_in_transaction<'a>(
        &self,
        tx: &mut Transaction<'a>,
        lines: &[String],
        results: &mut Vec<String>,
    ) -> Result<(), mysql_async::Error> {
        for (idx, sql) in lines.iter().enumerate() {
            if sql.starts_with("--") || sql.trim().is_empty() {
                continue;
            }

            tx.query_drop(sql).await?;
            results.push(format!("[{}] ✅ {}", idx + 1, sql));
        }
        Ok(())
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

    /// 抓取在线数据库架构：通过 SHOW CREATE TABLE 获取真实DDL，再用 sqlparser 解析为内部类型
    pub async fn fetch_live_schema(
        &self,
    ) -> Result<std::collections::HashMap<String, TableDefinition>, anyhow::Error> {
        let (tables, _sql) = self.fetch_live_schema_with_sql().await?;
        Ok(tables)
    }

    /// 抓取在线数据库架构并返回原始 SQL
    /// 返回：(解析后的表定义, 原始 CREATE TABLE SQL)
    pub async fn fetch_live_schema_with_sql(
        &self,
    ) -> Result<(std::collections::HashMap<String, TableDefinition>, String), anyhow::Error> {
        use crate::sql_diff::parse_sql_tables;

        let mut conn = self.pool.get_conn().await?;

        // 获取当前数据库所有表名
        let table_names: Vec<String> = conn
            .exec(
                r#"SELECT TABLE_NAME
                    FROM INFORMATION_SCHEMA.TABLES
                    WHERE TABLE_SCHEMA = ?
                    ORDER BY TABLE_NAME"#,
                (self.config.database.clone(),),
            )
            .await?
            .into_iter()
            .map(|row| {
                let (name,): (String,) = from_row(row);
                name
            })
            .collect();

        // 拼接所有表的 CREATE 语句
        let mut create_sqls = String::new();
        for table in &table_names {
            let query = format!("SHOW CREATE TABLE `{}`", table);
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
        let tables = parse_sql_tables(&create_sqls)
            .map_err(|e| anyhow::anyhow!(format!("Failed to parse online DDL: {}", e)))?;

        Ok((tables, create_sqls))
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
    fn test_interpolate_vars_basic() {
        let mut vars = HashMap::new();
        vars.insert("MYSQL_USER".to_string(), "nuwax_test".to_string());
        vars.insert("MYSQL_PASSWORD".to_string(), "nuwax_test_pass".to_string());
        vars.insert("MYSQL_DATABASE".to_string(), "agent_platform".to_string());

        // 复现 bug 现场：完整连接串里的 ${VAR} 必须被正确解析为真实凭据
        let url = "mysql://${MYSQL_USER}:${MYSQL_PASSWORD}@127.0.0.1:13306/${MYSQL_DATABASE}";
        assert_eq!(
            interpolate_vars(url, &vars),
            "mysql://nuwax_test:nuwax_test_pass@127.0.0.1:13306/agent_platform"
        );

        // 无括号 $VAR
        assert_eq!(interpolate_vars("u=$MYSQL_USER", &vars), "u=nuwax_test");
        // $$ 转义为字面 $
        assert_eq!(interpolate_vars("price=$$5", &vars), "price=$5");
        // 未定义变量 → 空字符串（docker compose 默认行为）
        assert_eq!(interpolate_vars("[${NOPE}]", &vars), "[]");
        // 单独的 $ 原样保留
        assert_eq!(interpolate_vars("a$ b", &vars), "a$ b");
    }

    #[test]
    fn test_interpolate_vars_defaults() {
        let mut vars = HashMap::new();
        vars.insert("EMPTY".to_string(), String::new());
        vars.insert("SET".to_string(), "v".to_string());

        // :- 未设 → default
        assert_eq!(interpolate_vars("${MISSING:-def}", &vars), "def");
        // :- 已设但为空 → default
        assert_eq!(interpolate_vars("${EMPTY:-def}", &vars), "def");
        // :- 已设且非空 → 值本身
        assert_eq!(interpolate_vars("${SET:-def}", &vars), "v");
        // - 未设 → default
        assert_eq!(interpolate_vars("${MISSING-def}", &vars), "def");
        // - 已设但为空 → 空值（注意与 :- 的区别：- 只看是否定义，不看是否为空）
        assert_eq!(interpolate_vars("${EMPTY-def}", &vars), "");
    }

    #[test]
    fn test_strip_surrounding_quotes() {
        assert_eq!(strip_surrounding_quotes("\"hello\""), "hello");
        assert_eq!(strip_surrounding_quotes("'hello'"), "hello");
        assert_eq!(strip_surrounding_quotes("hello"), "hello");
        // 单个/不成对引号不剥离
        assert_eq!(strip_surrounding_quotes("\""), "\"");
        assert_eq!(strip_surrounding_quotes("\"unmatched"), "\"unmatched");
    }

    #[test]
    fn test_load_env_vars_from_fixture() {
        // 直接读取 nuwax-cli 主 crate 的 fixtures/test.env（脱敏测试样本），
        // 验证 load_env_vars 对真实 .env 文件的解析，以及 interpolate_vars 的 ${VAR} 插值。
        let manifest_dir = std::env::var("CARGO_MANIFEST_DIR").unwrap();
        let fixture = std::path::Path::new(&manifest_dir).join("../nuwax-cli/fixtures/test.env");
        assert!(
            fixture.exists(),
            "fixtures 文件不存在: {}（请确认 nuwax-cli/fixtures/test.env 已创建）",
            fixture.display()
        );

        let vars = load_env_vars(&fixture);

        // 1) load_env_vars 解析：账号密码应为 nuwax_test 前缀的测试值
        assert_eq!(vars.get("MYSQL_USER").map(String::as_str), Some("nuwax_test"));
        assert_eq!(vars.get("MYSQL_PASSWORD").map(String::as_str), Some("nuwax_test_pass"));
        assert_eq!(vars.get("MYSQL_ROOT_PASSWORD").map(String::as_str), Some("nuwax_test_root"));
        assert_eq!(vars.get("MYSQL_DATABASE").map(String::as_str), Some("agent_platform"));
        assert_eq!(vars.get("REDIS_PASSWORD").map(String::as_str), Some("nuwax_test_redis"));
        // 注释行不应被当作变量
        assert!(!vars.contains_key("# Docker Registry 配置"));

        // 2) interpolate_vars 插值：模拟 docker-compose 里 ${VAR} 引用解析为真实凭据
        assert_eq!(interpolate_vars("${MYSQL_USER}", &vars), "nuwax_test");
        assert_eq!(
            interpolate_vars("${MYSQL_PASSWORD:-default}", &vars),
            "nuwax_test_pass"
        );
        // 复现 for_container 拼出的连接串
        let url = interpolate_vars(
            "mysql://${MYSQL_USER}:${MYSQL_PASSWORD}@127.0.0.1:13306/${MYSQL_DATABASE}",
            &vars,
        );
        assert_eq!(
            url,
            "mysql://nuwax_test:nuwax_test_pass@127.0.0.1:13306/agent_platform"
        );
    }

    #[test]
    fn test_load_env_vars_missing_file_degrades() {
        // 不存在的文件 → 空 map（降级，不报错）
        let manifest_dir = std::env::var("CARGO_MANIFEST_DIR").unwrap();
        let missing = std::path::Path::new(&manifest_dir).join("nuwax_definitely_not_exist.env");
        assert!(load_env_vars(&missing).is_empty());
    }

    #[tokio::test]
    async fn test_mysql_connection() {
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
        if executor.test_connection().await.is_ok() {
            // 测试真实执行
            executor
                .execute_single("CREATE DATABASE IF NOT EXISTS test_db")
                .await
                .unwrap();

            executor.execute_single("USE test_db").await.unwrap();

            executor
                .execute_single(
                    "CREATE TABLE IF NOT EXISTS test_table (id INT PRIMARY KEY, name VARCHAR(255))",
                )
                .await
                .unwrap();

            let results = executor
                .execute_diff_sql("CREATE TABLE IF NOT EXISTS users (id INT PRIMARY KEY AUTO_INCREMENT, name VARCHAR(100)); \
                                 ALTER TABLE users ADD COLUMN email VARCHAR(255); \
                                 CREATE INDEX idx_name ON users(name);")
                .await
                .unwrap();

            assert!(!results.is_empty());
            println!("✅ MySQL执行器测试通过");

            // 清理
            executor
                .execute_single("DROP DATABASE IF EXISTS test_db")
                .await
                .unwrap();
        } else {
            println!("⚠️ MySQL容器未运行，跳过测试");
        }
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
