use anyhow::{Context, Result, anyhow};
use client_core::mysql_executor::{MySqlConfig, MySqlExecutor};
use sqlx::mysql::MySqlPoolOptions;
use std::path::Path;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};
use url::Url;

const TEST_DB: &str = "executor_integration_test";
const TEST_COMPOSE_PROJECT: &str = "nuwax_mysql_integration";
const TEST_MYSQL_SERVICE: &str = "mysql";
const TEST_MYSQL_TIMEOUT: Duration = Duration::from_secs(90);
const TEST_MYSQL_RETRY_INTERVAL: Duration = Duration::from_secs(2);

/// 测试 MySqlExecutor 的集成测试
/// 这个测试会：
/// 1. 优先使用 TEST_MYSQL_URL 连接到外部 MySQL；未设置时启动仓库内的 Docker MySQL。
/// 2. 创建一个测试数据库。
/// 3. 使用 MySqlExecutor 执行一系列的 SQL 操作（创建表、修改表、增删索引）。
/// 4. 使用 sqlx 直接连接数据库来验证 MySqlExecutor 执行的结果是否正确。
#[tokio::test]
async fn test_mysql_executor_integration() -> Result<()> {
    // 1. 设置测试环境
    println!("🔧 1. 设置测试环境...");

    // 2. 获取 MySQL 配置
    println!("🔧 2. 获取 MySQL 配置...");
    let test_env = match prepare_mysql().await {
        Ok(test_env) => test_env,
        Err(err) if !is_mysql_required() => {
            eprintln!("⚠️ 无法启动测试 MySQL，跳过集成测试: {err:#}");
            return Ok(());
        }
        Err(err) => return Err(err),
    };
    let config = test_env.config.clone();

    // 2.1. 使用 root 用户确保测试用户拥有所需权限
    println!("🔧 2.1. 使用 root 用户确保测试用户拥有权限...");
    let mut root_config = config.clone();
    root_config.user = "root".to_string();
    // 注意：这里我们假设 root 密码也是 'root'，这通常在 .env 文件中配置
    root_config.password = "root".to_string();

    let root_executor = MySqlExecutor::new(root_config);
    let grant_sql = format!("GRANT ALL PRIVILEGES ON *.* TO '{}'@'%'", &config.user);
    root_executor
        .execute_single(&grant_sql)
        .await
        .context("使用 root 用户授权失败")?;

    let flush_sql = "FLUSH PRIVILEGES";
    root_executor
        .execute_single(flush_sql)
        .await
        .context("刷新权限失败")?;

    println!("✅ 权限已自动授予。");

    let executor = MySqlExecutor::new(config.clone());

    // 3. 清理并创建测试数据库
    println!("🧹 3. 清理并创建测试数据库 '{TEST_DB}'...");
    let drop_db_sql = format!("DROP DATABASE IF EXISTS `{TEST_DB}`");
    executor.execute_single(&drop_db_sql).await.ok();

    let create_db_sql = format!("CREATE DATABASE `{TEST_DB}`");
    executor
        .execute_single(&create_db_sql)
        .await
        .context("创建测试数据库失败")?;

    // 4. 执行 SQL 脚本
    println!("🔧 4. 在 '{TEST_DB}' 数据库中执行 SQL 脚本...");
    let sql_script = format!(
        "USE `{TEST_DB}`;
        {SQL_CREATE_TABLE}\n{SQL_ADD_COLUMN_AND_INDEX}\n{SQL_INSERT_DATA}\n{SQL_DROP_INDEX_AND_COLUMN}"
    );
    executor
        .execute_diff_sql(&sql_script)
        .await
        .context("执行 SQL 脚本失败")?;

    // 5. 连接数据库并验证结果
    println!("🔧 5. 连接数据库并验证结果...");
    let db_url = format!(
        "mysql://{}:{}@{}:{}/{}",
        config.user, config.password, config.host, config.port, TEST_DB
    );

    let pool = MySqlPoolOptions::new()
        .max_connections(1)
        .connect(&db_url)
        .await
        .context("无法连接到测试数据库")?;

    // 验证最终的表结构
    let columns: Vec<(String,)> = sqlx::query_as("SHOW COLUMNS FROM users WHERE Field = 'status'")
        .fetch_all(&pool)
        .await
        .context("查询表结构失败")?;
    assert!(columns.is_empty(), "'status' 列未被成功删除");

    let indexes: Vec<(String,)> =
        sqlx::query_as("SHOW INDEX FROM users WHERE Key_name = 'idx_email'")
            .fetch_all(&pool)
            .await
            .context("查询索引失败")?;
    assert!(indexes.is_empty(), "'idx_email' 索引未被成功删除");

    // 验证数据
    let count: (i64,) = sqlx::query_as("SELECT COUNT(*) FROM users")
        .fetch_one(&pool)
        .await
        .context("查询数据失败")?;
    assert_eq!(count.0, 1, "数据插入验证失败");

    drop(pool);

    if test_env.started_by_test && should_cleanup_mysql() {
        stop_mysql(&test_env.compose)?;
    }

    println!("✅ 集成测试成功!");
    Ok(())
}

#[derive(Debug)]
struct TestMySqlEnv {
    config: MySqlConfig,
    compose: Option<DockerComposeConfig>,
    started_by_test: bool,
}

#[derive(Debug)]
struct DockerComposeConfig {
    compose_file: String,
    env_file: String,
}

#[derive(Debug, Clone, Copy)]
enum DockerComposeCommand {
    DockerPlugin,
    Standalone,
}

impl DockerComposeCommand {
    fn detect() -> Result<Self> {
        if Command::new("docker")
            .args(["compose", "version"])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .is_ok_and(|status| status.success())
        {
            return Ok(Self::DockerPlugin);
        }

        if Command::new("docker-compose")
            .arg("--version")
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .is_ok_and(|status| status.success())
        {
            return Ok(Self::Standalone);
        }

        Err(anyhow!("docker compose 或 docker-compose 不可用"))
    }

    fn command(self) -> Command {
        match self {
            Self::DockerPlugin => {
                let mut command = Command::new("docker");
                command.arg("compose");
                command
            }
            Self::Standalone => Command::new("docker-compose"),
        }
    }
}

async fn prepare_mysql() -> Result<TestMySqlEnv> {
    if let Ok(url) = std::env::var("TEST_MYSQL_URL") {
        let config = mysql_config_from_url(&url)?;
        wait_for_mysql(&config, TEST_MYSQL_TIMEOUT)
            .await
            .context("TEST_MYSQL_URL 指定的 MySQL 不可用")?;

        return Ok(TestMySqlEnv {
            config,
            compose: None,
            started_by_test: false,
        });
    }

    let cargo_manifest_dir = std::env::var("CARGO_MANIFEST_DIR")?;
    let workspace_root = Path::new(&cargo_manifest_dir)
        .parent()
        .ok_or_else(|| anyhow!("无法定位 workspace 根目录"))?;
    let compose_path_buf = workspace_root.join("docker/mysql-integration/docker-compose.yml");
    let env_path_buf = workspace_root.join("docker/mysql-integration/.env");
    let compose_path = compose_path_buf
        .to_str()
        .ok_or_else(|| anyhow!("无法将测试 docker-compose.yml 路径转换为字符串"))?
        .to_string();
    let env_path = env_path_buf
        .to_str()
        .ok_or_else(|| anyhow!("无法将测试 .env 路径转换为字符串"))?
        .to_string();
    let compose = DockerComposeConfig {
        compose_file: compose_path,
        env_file: env_path,
    };

    start_mysql(&compose)?;
    let config = MySqlConfig::for_container(Some(&compose.compose_file), Some(&compose.env_file))
        .await
        .context("无法从测试 Docker Compose 配置获取 MySQL 配置")?;
    wait_for_mysql(&config, TEST_MYSQL_TIMEOUT)
        .await
        .context("Docker MySQL 启动后仍不可用")?;

    Ok(TestMySqlEnv {
        config,
        compose: Some(compose),
        started_by_test: true,
    })
}

fn start_mysql(compose: &DockerComposeConfig) -> Result<()> {
    let compose_command = DockerComposeCommand::detect()?;
    let status = compose_command
        .command()
        .args([
            "-f",
            &compose.compose_file,
            "--env-file",
            &compose.env_file,
            "-p",
            TEST_COMPOSE_PROJECT,
            "up",
            "-d",
            "--build",
            TEST_MYSQL_SERVICE,
        ])
        .status()
        .context("启动测试 MySQL 容器失败")?;

    if !status.success() {
        return Err(anyhow!("启动测试 MySQL 容器失败，退出状态: {status}"));
    }

    Ok(())
}

fn stop_mysql(compose: &Option<DockerComposeConfig>) -> Result<()> {
    let Some(compose) = compose else {
        return Ok(());
    };

    let compose_command = DockerComposeCommand::detect()?;
    let status = compose_command
        .command()
        .args([
            "-f",
            &compose.compose_file,
            "--env-file",
            &compose.env_file,
            "-p",
            TEST_COMPOSE_PROJECT,
            "down",
            "-v",
        ])
        .status()
        .context("清理测试 MySQL 容器失败")?;

    if !status.success() {
        return Err(anyhow!("清理测试 MySQL 容器失败，退出状态: {status}"));
    }

    Ok(())
}

async fn wait_for_mysql(config: &MySqlConfig, timeout: Duration) -> Result<()> {
    let started_at = Instant::now();
    let executor = MySqlExecutor::new(config.clone());
    let mut last_error = None;

    while started_at.elapsed() < timeout {
        match executor.test_connection().await {
            Ok(()) => return Ok(()),
            Err(err) => {
                last_error = Some(err.to_string());
                tokio::time::sleep(TEST_MYSQL_RETRY_INTERVAL).await;
            }
        }
    }

    Err(anyhow!(
        "等待 MySQL 就绪超时，最后错误: {}",
        last_error.unwrap_or_else(|| "unknown".to_string())
    ))
}

fn mysql_config_from_url(raw_url: &str) -> Result<MySqlConfig> {
    let url = Url::parse(raw_url).context("TEST_MYSQL_URL 不是合法 URL")?;
    if url.scheme() != "mysql" {
        return Err(anyhow!("TEST_MYSQL_URL 必须使用 mysql:// scheme"));
    }

    let host = url
        .host_str()
        .ok_or_else(|| anyhow!("TEST_MYSQL_URL 缺少 host"))?
        .to_string();
    let port = url.port().unwrap_or(3306);
    let user = url.username().to_string();
    if user.is_empty() {
        return Err(anyhow!("TEST_MYSQL_URL 缺少用户名"));
    }
    let password = url.password().unwrap_or_default().to_string();
    let database = url.path().trim_start_matches('/').to_string();
    if database.is_empty() {
        return Err(anyhow!("TEST_MYSQL_URL 缺少数据库名"));
    }

    Ok(MySqlConfig {
        host,
        port,
        user,
        password,
        database,
    })
}

fn is_mysql_required() -> bool {
    std::env::var("TEST_MYSQL_REQUIRED").is_ok_and(|value| value == "1" || value == "true")
}

fn should_cleanup_mysql() -> bool {
    std::env::var("TEST_MYSQL_CLEANUP").is_ok_and(|value| value == "1" || value == "true")
}

// --- SQL 脚本常量 ---

const SQL_CREATE_TABLE: &str = r#"
CREATE TABLE `users` (
    `id` bigint NOT NULL AUTO_INCREMENT,
    `username` varchar(50) NOT NULL,
    PRIMARY KEY (`id`)
) ENGINE=InnoDB;
"#;

const SQL_ADD_COLUMN_AND_INDEX: &str = r#"
ALTER TABLE `users`
    ADD COLUMN `email` varchar(100) NOT NULL AFTER `username`,
    ADD COLUMN `status` tinyint(1) DEFAULT 1,
    ADD INDEX `idx_email` (`email`);
"#;

const SQL_INSERT_DATA: &str = r#"
INSERT INTO `users` (username, email, status) VALUES ('test_user', 'test@example.com', 1);
"#;

const SQL_DROP_INDEX_AND_COLUMN: &str = r#"
ALTER TABLE `users`
    DROP INDEX `idx_email`,
    DROP COLUMN `status`;
"#;
