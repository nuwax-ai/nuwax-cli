use anyhow::{Context, Result, anyhow};
use client_core::mysql_executor::{MySqlConfig, MySqlExecutor};
use client_core::sql_diff::generate_live_schema_diff_multi;
use client_core::sql_diff::parse_schema_template;
use sqlx::mysql::{MySqlConnectOptions, MySqlPoolOptions};
use std::path::Path;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};
use url::Url;

const TEST_DB: &str = "executor_integration_test";
const TEST_COMPOSE_PROJECT: &str = "nuwax_mysql_integration";
const TEST_MYSQL_SERVICE: &str = "mysql";
const TEST_MYSQL_TIMEOUT: Duration = Duration::from_secs(90);
const TEST_MYSQL_RETRY_INTERVAL: Duration = Duration::from_secs(2);

#[tokio::test]
#[ignore = "requires TEST_MYSQL_URL pointing to a disposable MySQL instance"]
async fn test_partial_ddl_failure_is_reconciled_by_live_rediff() -> Result<()> {
    let url = std::env::var("TEST_MYSQL_URL").context("TEST_MYSQL_URL is required")?;
    let config = mysql_config_from_url(&url)?;
    let database = config
        .database
        .clone()
        .ok_or_else(|| anyhow!("TEST_MYSQL_URL must name a database"))?;
    let executor = MySqlExecutor::new(config);
    let table = "nuwax_partial_ddl_recovery";
    executor
        .execute_single(&format!("DROP TABLE IF EXISTS `{table}`"))
        .await?;

    let attempted = format!(
        "CREATE TABLE `{table}` (id INT NOT NULL, PRIMARY KEY (id));\n\
         ALTER TABLE `{table}` ADD COLUMN id INT;"
    );
    let error = executor
        .execute_diff_sql_once(&attempted)
        .await
        .err()
        .ok_or_else(|| anyhow!("duplicate column should fail after the first DDL"))?;
    assert!(error.to_string().contains("after 1 successful statements"));

    let live_templates = |ddl: String| -> Result<Vec<client_core::sql_diff::SchemaTemplate>> {
        Ok(vec![parse_schema_template(&format!(
            "USE `{database}`;\n{ddl}"
        ))?])
    };
    let target =
        format!("CREATE TABLE `{table}` (id INT NOT NULL, name VARCHAR(20), PRIMARY KEY (id));");
    let remaining =
        generate_live_schema_diff_multi(&executor, &live_templates(target.clone())?, "recovery")
            .await?;
    assert!(remaining.diff_sql.contains("ADD COLUMN `name`"));
    executor.execute_diff_sql_once(&remaining.diff_sql).await?;

    let final_diff =
        generate_live_schema_diff_multi(&executor, &live_templates(target.clone())?, "recovery")
            .await?;
    assert!(!final_diff.has_executable_sql);

    let manual_target = format!("{} ENGINE=MyISAM;", target.trim_end_matches(';'));
    let manual_diff =
        generate_live_schema_diff_multi(&executor, &live_templates(manual_target)?, "manual")
            .await?;
    assert!(!manual_diff.has_executable_sql);
    assert!(manual_diff.has_warnings);
    assert!(manual_diff.diff_sql.contains("table option ENGINE differs"));

    executor
        .execute_single(&format!("DROP TABLE IF EXISTS `{table}`"))
        .await?;
    Ok(())
}

/// 多库 Live Diff 真实 MySQL 场景验收（隔离库名，用后即删，不触碰现有部署）：
///
/// 1. 平台库已存在且含业务数据、IM 库不存在 → 一次迁移补建 IM 库，平台数据保留；
/// 2. 两库各自漂移（平台加列、IM 加表）→ 再次迁移全部应用，数据保留；
/// 3. IM 库与目标约束冲突 → 整体报错停止；解除冲突后重试按实际状态收敛。
#[tokio::test]
#[ignore = "requires TEST_MYSQL_URL pointing to a disposable MySQL instance"]
async fn multi_database_live_diff_scenarios() -> Result<()> {
    let url = std::env::var("TEST_MYSQL_URL").context("TEST_MYSQL_URL is required")?;
    let mut config = mysql_config_from_url(&url)?;
    // 管理连接：无默认库（库切换由 diff 的 USE 段完成）
    config.database = None;
    let executor = MySqlExecutor::new(config.clone());
    executor.test_connection().await?;

    let suffix = uuid::Uuid::now_v7().simple();
    let platform_db = format!("nuwax_mdp_{suffix}");
    let im_db = format!("nuwax_mdi_{suffix}");

    let outcome = multi_database_scenario_body(&executor, &config, &platform_db, &im_db).await;

    // 无论成败都清理隔离库
    for database in [&platform_db, &im_db] {
        executor
            .execute_single(&format!("DROP DATABASE IF EXISTS `{database}`"))
            .await
            .ok();
    }
    outcome
}

async fn multi_database_scenario_body(
    executor: &MySqlExecutor,
    config: &MySqlConfig,
    platform_db: &str,
    im_db: &str,
) -> Result<()> {
    use client_core::sql_diff::{
        SchemaTemplate, generate_live_schema_diff_multi, parse_schema_template,
    };

    let templates_for = |platform_ddl: &str, im_ddl: &str| -> Result<Vec<SchemaTemplate>> {
        Ok(vec![
            parse_schema_template(&format!("USE `{platform_db}`;\n{platform_ddl}"))?,
            // IM 模板带建库前缀，与真实 init_mysql_im.sql 形态一致
            parse_schema_template(&format!(
                "CREATE DATABASE IF NOT EXISTS `{im_db}` DEFAULT CHARACTER SET utf8mb4;\nUSE `{im_db}`;\n{im_ddl}"
            ))?,
        ])
    };
    let migrate = |templates: Vec<SchemaTemplate>| async move {
        generate_live_schema_diff_multi(executor, &templates, "scenario").await
    };

    // ── 场景 1：平台库已存在含数据，IM 库不存在 ─────────────────────────
    executor
        .execute_single(&format!(
            "CREATE DATABASE `{platform_db}`; \
             CREATE TABLE `{platform_db}`.`users` \
             (`id` bigint NOT NULL, `name` varchar(64), PRIMARY KEY (`id`)); \
             INSERT INTO `{platform_db}`.`users` VALUES (1, 'must-survive')"
        ))
        .await
        .context("准备平台库失败")?;

    let platform_v1 =
        "CREATE TABLE `users` (`id` bigint NOT NULL, `name` varchar(64), PRIMARY KEY (`id`));";
    let im_v1 = "CREATE TABLE `im_msg` (`id` bigint NOT NULL, PRIMARY KEY (`id`));";

    let result = migrate(templates_for(platform_v1, im_v1)?).await?;
    assert!(
        result.has_executable_sql,
        "IM 库缺失应产生可执行差异: {}",
        result.description
    );
    assert!(
        result
            .diff_sql
            .contains(&format!("CREATE DATABASE IF NOT EXISTS `{im_db}`"))
    );
    assert!(result.diff_sql.contains(&format!("USE `{im_db}`;")));
    executor
        .execute_diff_sql_once(&result.diff_sql)
        .await
        .context("场景1 迁移失败")?;

    // 两库就绪后再建立断言用连接（按库连接 + 静态 SQL，动态值走 bind）
    let connect = |database: &str| {
        MySqlPoolOptions::new().max_connections(1).connect_with(
            MySqlConnectOptions::new()
                .host(&config.host)
                .port(config.port)
                .username(&config.user)
                .password(&config.password)
                .database(database),
        )
    };
    let platform_pool = connect(platform_db).await.context("无法连接平台测试库")?;
    let im_pool = connect(im_db).await.context("无法连接 IM 测试库")?;
    let im_schema_table_count = || async {
        let count: (i64,) =
            sqlx::query_as("SELECT COUNT(*) FROM information_schema.tables WHERE table_schema = ?")
                .bind(im_db)
                .fetch_one(&im_pool)
                .await
                .context("查询 IM 库表数失败")?;
        Ok::<_, anyhow::Error>(count)
    };

    let kept: (i64,) = sqlx::query_as("SELECT COUNT(*) FROM `users` WHERE `name` = 'must-survive'")
        .fetch_one(&platform_pool)
        .await?;
    assert_eq!(kept, (1,), "平台库业务数据必须保留");
    assert_eq!(im_schema_table_count().await?, (1,), "IM 库应补建 1 张表");

    // ── 场景 2：两库各自漂移（平台加列、IM 加表） ───────────────────────
    let platform_v2 = "CREATE TABLE `users` (`id` bigint NOT NULL, `name` varchar(64), `email` varchar(255), PRIMARY KEY (`id`));";
    let im_v2 = "CREATE TABLE `im_msg` (`id` bigint NOT NULL, PRIMARY KEY (`id`));\n\
                 CREATE TABLE `im_conv` (`id` bigint NOT NULL, PRIMARY KEY (`id`));";

    let result = migrate(templates_for(platform_v2, im_v2)?).await?;
    assert!(
        result.has_executable_sql,
        "两库漂移都应被检出: {}",
        result.description
    );
    executor
        .execute_diff_sql_once(&result.diff_sql)
        .await
        .context("场景2 迁移失败")?;

    let kept: (i64,) = sqlx::query_as("SELECT COUNT(*) FROM `users` WHERE `name` = 'must-survive'")
        .fetch_one(&platform_pool)
        .await?;
    assert_eq!(kept, (1,), "加列后平台数据仍保留");
    let email_column: Vec<(String,)> =
        sqlx::query_as("SHOW COLUMNS FROM `users` WHERE Field = 'email'")
            .fetch_all(&platform_pool)
            .await?;
    assert_eq!(email_column.len(), 1, "平台库新列已应用");
    assert_eq!(im_schema_table_count().await?, (2,), "IM 库新表已应用");

    // ── 场景 3：IM 库约束冲突 → 停止；解除后重试收敛 ────────────────────
    executor
        .execute_single(&format!(
            "CREATE TABLE `{im_db}`.`im_tag` (`id` bigint NOT NULL, `tag` varchar(64), PRIMARY KEY (`id`)); \
             INSERT INTO `{im_db}`.`im_tag` VALUES (1, 'dup'), (2, 'dup')"
        ))
        .await
        .context("准备冲突数据失败")?;

    let im_v3 = "CREATE TABLE `im_msg` (`id` bigint NOT NULL, PRIMARY KEY (`id`));\n\
                 CREATE TABLE `im_conv` (`id` bigint NOT NULL, PRIMARY KEY (`id`));\n\
                 CREATE TABLE `im_tag` (`id` bigint NOT NULL, `tag` varchar(64), PRIMARY KEY (`id`), UNIQUE KEY `uk_tag` (`tag`));";

    let result = migrate(templates_for(platform_v2, im_v3)?).await?;
    assert!(result.has_executable_sql, "唯一键差异应被检出");
    let failure = executor
        .execute_diff_sql_once(&result.diff_sql)
        .await
        .err()
        .context("重复值与 UNIQUE 约束冲突必须让迁移整体失败")?;
    assert!(
        format!("{failure:#}").contains("Diff SQL failed"),
        "失败语义应为出错即停: {failure:#}"
    );

    // 解除冲突后重试：差异按数据库实际状态重算，只补未完成的约束
    executor
        .execute_single(&format!("DELETE FROM `{im_db}`.`im_tag` WHERE `id` = 2"))
        .await?;
    let result = migrate(templates_for(platform_v2, im_v3)?).await?;
    executor
        .execute_diff_sql_once(&result.diff_sql)
        .await
        .context("解除冲突后的重试必须成功")?;

    let dup_count: (i64,) = sqlx::query_as("SELECT COUNT(*) FROM `im_tag` WHERE `tag` = 'dup'")
        .fetch_one(&im_pool)
        .await?;
    assert_eq!(dup_count, (1,), "冲突数据按人工处理保留一行");

    // 幂等：无变更再跑一次不执行任何语句
    let result = migrate(templates_for(platform_v2, im_v3)?).await?;
    assert!(
        !result.has_executable_sql,
        "收敛后应无差异: {}",
        result.description
    );
    Ok(())
}

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
    if root_config.user != "root" {
        root_config.user = "root".to_string();
        root_config.password = if std::env::var_os("TEST_MYSQL_URL").is_some() {
            std::env::var("TEST_MYSQL_ROOT_PASSWORD").context(
                "TEST_MYSQL_ROOT_PASSWORD is required for an external non-root test user",
            )?
        } else {
            "root".to_string()
        };
    }

    let root_executor = MySqlExecutor::new(root_config);
    let grant_sql = format!("GRANT ALL PRIVILEGES ON *.* TO '{}'@'%'", config.user);
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
    let connection_options = MySqlConnectOptions::new()
        .host(&config.host)
        .port(config.port)
        .username(&config.user)
        .password(&config.password)
        .database(TEST_DB);

    let pool = MySqlPoolOptions::new()
        .max_connections(1)
        .connect_with(connection_options)
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
    let options = mysql_async::Opts::from_url(raw_url)
        .map_err(|_| anyhow!("TEST_MYSQL_URL 不是合法 MySQL 连接 URL"))?;
    let user = options.user().unwrap_or_default().to_string();
    if user.is_empty() {
        return Err(anyhow!("TEST_MYSQL_URL 缺少用户名"));
    }
    let password = options.pass().unwrap_or_default().to_string();
    let database = options.db_name().unwrap_or_default().to_string();
    if database.is_empty() {
        return Err(anyhow!("TEST_MYSQL_URL 缺少数据库名"));
    }

    Ok(MySqlConfig {
        host,
        port,
        user,
        password,
        database: Some(database),
        app_user: None,
    })
}

fn is_mysql_required() -> bool {
    std::env::var("TEST_MYSQL_REQUIRED").is_ok_and(|value| value == "1" || value == "true")
}

#[test]
fn test_mysql_url_credentials_are_decoded_once() -> Result<()> {
    let config = mysql_config_from_url(
        "mysql://test%40user:p%40ss%3A%23%24%25E4@127.0.0.1:33316/disposable",
    )?;
    assert_eq!(config.user, "test@user");
    assert_eq!(config.password, "p@ss:#$%E4");
    assert_eq!(config.database.as_deref(), Some("disposable"));
    Ok(())
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

/// manifest v1 存量迁移场景（隔离库名 + 随机应用账号，用后即删）：
/// schema 文件为拆分后的纯 USE + 表结构（无 CREATE DATABASE/GRANT）。
///
/// 1. 平台库已存在含数据、IM 与 bootstrap_only 库缺失 → bootstrap 建库 +
///    应用账号授权 + IM 补建；平台数据保留；授权覆盖全部声明库；
/// 2. 零表差异但清单新增 bootstrap_only 库 → bootstrap/授权仍独立执行；
/// 3. 双库漂移 → Live Diff 应用；幂等重跑零执行。
#[tokio::test]
#[ignore = "requires TEST_MYSQL_URL pointing to a disposable MySQL instance"]
async fn manifest_migration_scenarios() -> Result<()> {
    let url = std::env::var("TEST_MYSQL_URL").context("TEST_MYSQL_URL is required")?;
    let base = mysql_config_from_url(&url)?;
    let suffix = uuid::Uuid::now_v7().simple();
    let platform_db = format!("nuwax_mmp_{suffix}");
    let im_db = format!("nuwax_mmi_{suffix}");
    let custom_db = format!("nuwax_mmc_{suffix}");
    // MySQL 用户名上限 32 字符，取 uuid 尾部 16 位保证唯一
    let app_user = format!("nuwax_app_{}", &suffix.to_string()[24..]);

    let mut admin = MySqlConfig {
        host: base.host.clone(),
        port: base.port,
        user: base.user.clone(),
        password: base.password.clone(),
        database: None,
        app_user: None,
    };
    admin.app_user = Some(app_user.clone());
    let executor = MySqlExecutor::new(admin);
    executor.test_connection().await?;

    executor
        .execute_single(&format!(
            "CREATE USER IF NOT EXISTS '{app_user}'@'%' IDENTIFIED BY 'disposable'"
        ))
        .await
        .context("创建应用账号失败")?;

    let outcome = manifest_scenario_body(
        &executor,
        &app_user,
        &platform_db,
        &im_db,
        &custom_db,
        &base,
    )
    .await;

    for database in [&platform_db, &im_db, &custom_db] {
        executor
            .execute_single(&format!("DROP DATABASE IF EXISTS `{database}`"))
            .await
            .ok();
    }
    executor
        .execute_single(&format!("DROP USER IF EXISTS '{app_user}'@'%'"))
        .await
        .ok();
    outcome
}

async fn manifest_scenario_body(
    executor: &MySqlExecutor,
    app_user: &str,
    platform_db: &str,
    im_db: &str,
    custom_db: &str,
    base: &MySqlConfig,
) -> Result<()> {
    use client_core::mysql_manifest::{
        parse_schema_manifest, run_manifest_migration, validate_manifest_files,
    };

    let workspace = tempfile::tempdir()?;
    let docker_root = workspace.path().join("docker");
    std::fs::create_dir_all(docker_root.join("config"))?;

    let bootstrap_v1 = format!(
        "CREATE DATABASE IF NOT EXISTS `{platform_db}` CHARACTER SET utf8mb4 COLLATE utf8mb4_unicode_ci;\n\
         CREATE DATABASE IF NOT EXISTS `{custom_db}` CHARACTER SET utf8mb4 COLLATE utf8mb4_unicode_ci;\n\
         CREATE DATABASE IF NOT EXISTS `{im_db}` CHARACTER SET utf8mb4 COLLATE utf8mb4_0900_ai_ci;\n"
    );
    let platform_v1 = format!(
        "USE `{platform_db}`;\nCREATE TABLE `users` (`id` bigint NOT NULL, `name` varchar(64), PRIMARY KEY (`id`));\n"
    );
    let im_v1 = format!(
        "USE `{im_db}`;\nCREATE TABLE `im_msg` (`id` bigint NOT NULL, PRIMARY KEY (`id`));\n"
    );

    std::fs::write(
        docker_root.join("config/init_mysql_databases.sql"),
        &bootstrap_v1,
    )?;
    std::fs::write(
        docker_root.join("config/init_mysql_permissions.sh"),
        "#!/bin/sh\nexit 0\n",
    )?;
    std::fs::write(docker_root.join("config/platform.sql"), &platform_v1)?;
    std::fs::write(docker_root.join("config/im.sql"), &im_v1)?;

    let manifest_json = |custom_bootstrap: bool| {
        format!(
            r#"{{
  "contract_version": 1,
  "requires": {{ "cli_capability": "mysql-schema-manifest-v1" }},
  "mysql_target": {{ "service": "mysql", "internal_port": 3306 }},
  "application_connections": [],
  "databases": [
    {{ "name": "{platform_db}", "bootstrap_only": false }},
    {{ "name": "{custom_db}", "bootstrap_only": {custom_bootstrap} }},
    {{ "name": "{im_db}", "bootstrap_only": false }}
  ],
  "bootstrap": {{ "path": "config/init_mysql_databases.sql", "idempotent": true, "initdb_target": "00_init_mysql_databases.sql" }},
  "permissions": {{ "path": "config/init_mysql_permissions.sh", "user_env": "MYSQL_USER", "databases": "bootstrap", "initdb_target": "01_init_mysql_permissions.sh" }},
  "schemas": [
    {{ "database": "{platform_db}", "path": "config/platform.sql", "initdb_target": "10_platform.sql" }},
    {{ "database": "{im_db}", "path": "config/im.sql", "initdb_target": "20_im.sql" }}
  ],
  "first_install_seeds": []
}}"#
        )
    };
    let manifest = parse_schema_manifest(&manifest_json(true)).context("清单解析失败")?;
    validate_manifest_files(&manifest, &docker_root).context("清单文件校验失败")?;

    // ── 场景 1：平台库存在含数据，IM/bootstrap_only 库缺失 ────────────────
    executor
        .execute_single(&format!(
            "CREATE DATABASE `{platform_db}`;              CREATE TABLE `{platform_db}`.`users` (`id` bigint NOT NULL, `name` varchar(64), PRIMARY KEY (`id`));              INSERT INTO `{platform_db}`.`users` VALUES (1, 'must-survive')"
        ))
        .await
        .context("准备平台库失败")?;

    let result = run_manifest_migration(executor, &manifest, &docker_root, app_user)
        .await
        .context("场景1 manifest 迁移失败")?;
    assert!(result.has_executable_sql, "IM 补建应产生可执行差异");
    // 部署命令负责执行生成的 diff；此处模拟同一顺序
    executor
        .execute_diff_sql_once(&result.diff_sql)
        .await
        .context("场景1 diff 执行失败")?;

    let pool = MySqlPoolOptions::new()
        .max_connections(1)
        .connect_with(
            MySqlConnectOptions::new()
                .host(&base.host)
                .port(base.port)
                .username(&base.user)
                .password(&base.password)
                .database(platform_db),
        )
        .await?;
    let kept: (i64,) = sqlx::query_as("SELECT COUNT(*) FROM `users` WHERE `name` = 'must-survive'")
        .fetch_one(&pool)
        .await?;
    assert_eq!(kept, (1,), "平台库业务数据必须保留");

    let dbs: Vec<(String,)> = sqlx::query_as(
        "SELECT schema_name FROM information_schema.schemata WHERE schema_name IN (?, ?) ORDER BY schema_name",
    )
    .bind(im_db)
    .bind(custom_db)
    .fetch_all(&pool)
    .await?;
    assert_eq!(dbs.len(), 2, "bootstrap 必须独立于表差异建齐全部库");

    let im_tables: (i64,) =
        sqlx::query_as("SELECT COUNT(*) FROM information_schema.tables WHERE table_schema = ?")
            .bind(im_db)
            .fetch_one(&pool)
            .await?;
    assert_eq!(im_tables, (1,), "IM schema 补建 1 张表");

    let grants: Vec<(String,)> = sqlx::query_as(
        "SELECT TABLE_SCHEMA FROM information_schema.SCHEMA_PRIVILEGES WHERE GRANTEE = ? GROUP BY TABLE_SCHEMA",
    )
    .bind(format!("'{app_user}'@'%'"))
    .fetch_all(&pool)
    .await?;
    let grants_text = grants
        .iter()
        .map(|(grant,)| grant.clone())
        .collect::<Vec<_>>()
        .join("\n");
    for database in [platform_db, im_db, custom_db] {
        assert!(
            grants_text.contains(database),
            "授权必须覆盖声明库 {database}: {grants_text}"
        );
    }

    // ── 场景 2：零表差异 + 授权独立（重跑幂等 + bootstrap/授权始终执行） ──
    let result = run_manifest_migration(executor, &manifest, &docker_root, app_user)
        .await
        .context("场景2 重跑失败")?;
    assert!(
        !result.has_executable_sql,
        "收敛后应零表差异（bootstrap/授权仍已独立执行）: {}",
        result.description
    );

    // ── 场景 3：双库漂移 → Live Diff 应用；幂等 ──────────────────────────
    let platform_v2 = format!(
        "USE `{platform_db}`;\nCREATE TABLE `users` (`id` bigint NOT NULL, `name` varchar(64), `email` varchar(255), PRIMARY KEY (`id`));\n"
    );
    let im_v2 = format!(
        "USE `{im_db}`;\nCREATE TABLE `im_msg` (`id` bigint NOT NULL, PRIMARY KEY (`id`));\n\
         CREATE TABLE `im_conv` (`id` bigint NOT NULL, PRIMARY KEY (`id`));\n"
    );
    std::fs::write(docker_root.join("config/platform.sql"), &platform_v2)?;
    std::fs::write(docker_root.join("config/im.sql"), &im_v2)?;
    validate_manifest_files(&manifest, &docker_root)?;

    let result = run_manifest_migration(executor, &manifest, &docker_root, app_user)
        .await
        .context("场景3 漂移迁移失败")?;
    assert!(result.has_executable_sql, "双库漂移都应被检出");
    executor
        .execute_diff_sql_once(&result.diff_sql)
        .await
        .context("场景3 diff 执行失败")?;
    let email_column: Vec<(String,)> =
        sqlx::query_as("SHOW COLUMNS FROM `users` WHERE Field = 'email'")
            .fetch_all(&pool)
            .await?;
    assert_eq!(email_column.len(), 1, "平台库新列已应用");
    let im_tables: (i64,) =
        sqlx::query_as("SELECT COUNT(*) FROM information_schema.tables WHERE table_schema = ?")
            .bind(im_db)
            .fetch_one(&pool)
            .await?;
    assert_eq!(im_tables, (2,), "IM 库新表已应用");

    let result = run_manifest_migration(executor, &manifest, &docker_root, app_user).await?;
    assert!(!result.has_executable_sql, "再次重跑应零差异");
    Ok(())
}
