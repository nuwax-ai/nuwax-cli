use client_core::mysql_executor::{MySqlConfig, MySqlExecutor};
use sqlx::mysql::MySqlPoolOptions;
use std::path::Path;

const TEST_DB: &str = "executor_integration_test";

/// 测试 MySqlExecutor 的集成测试
/// 这个测试会：
/// 1. 使用 MySqlConfig 连接到已有的 MySQL 服务。
/// 2. 创建一个测试数据库。
/// 3. 使用 MySqlExecutor 执行一系列的 SQL 操作（创建表、修改表、增删索引）。
/// 4. 使用 sqlx 直接连接数据库来验证 MySqlExecutor 执行的结果是否正确。
/// 2. 使用 MySqlConfig::for_container 从 docker-compose.yml 获取数据库连接信息。
/// 3. 创建一个测试数据库。
/// 4. 使用 MySqlExecutor 执行一系列的 SQL 操作（创建表、修改表、增删索引）。
/// 5. 使用 sqlx 直接连接数据库来验证 MySqlExecutor 执行的结果是否正确。
/// 6. 测试结束后，自动关闭并清理容器。
#[tokio::test]
async fn test_mysql_executor_integration() {
    // 1. 设置测试环境
    println!("🔧 1. 设置测试环境...");
    let cargo_manifest_dir = std::env::var("CARGO_MANIFEST_DIR").unwrap();
    let compose_path_buf = Path::new(&cargo_manifest_dir).join("fixtures/docker-compose.yml");
    let compose_path = compose_path_buf.to_str().unwrap();
    let env_path_buf = Path::new(&cargo_manifest_dir).join("fixtures/.env");
    let env_path = env_path_buf.to_str().unwrap();

    // 2. 获取 MySQL 配置
    println!("🔧 2. 获取 MySQL 配置...");
    let config = MySqlConfig::for_container(Some(compose_path), Some(env_path))
        .await
        .expect("无法从容器获取 MySQL 配置");

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
        .expect("使用 root 用户授权失败");

    let flush_sql = "FLUSH PRIVILEGES";
    root_executor
        .execute_single(flush_sql)
        .await
        .expect("刷新权限失败");

    println!("✅ 权限已自动授予。");

    let executor = MySqlExecutor::new(config.clone());

    // 3. 清理并创建测试数据库
    println!("🧹 3. 清理并创建测试数据库 '{TEST_DB}'...");
    let drop_db_sql = format!("DROP DATABASE IF EXISTS `{TEST_DB}`");
    executor.execute_single(&drop_db_sql).await.ok(); // 忽略错误，因为数据库可能不存在

    let create_db_sql = format!("CREATE DATABASE `{TEST_DB}`");
    executor
        .execute_single(&create_db_sql)
        .await
        .expect("创建测试数据库失败");

    // 4. 执行 SQL 脚本
    println!("🔧 4. 在 '{TEST_DB}' 数据库中执行 SQL 脚本...");
    let sql_script = format!(
        "USE `{TEST_DB}`;
        {SQL_CREATE_TABLE}\n{SQL_ADD_COLUMN_AND_INDEX}\n{SQL_INSERT_DATA}\n{SQL_DROP_INDEX_AND_COLUMN}"
    );
    executor
        .execute_diff_sql(&sql_script)
        .await
        .expect("执行 SQL 脚本失败");

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
        .expect("无法连接到测试数据库");

    // 验证最终的表结构
    let columns: Vec<(String,)> = sqlx::query_as("SHOW COLUMNS FROM users WHERE Field = 'status'")
        .fetch_all(&pool)
        .await
        .expect("查询表结构失败");
    assert!(columns.is_empty(), "'status' 列未被成功删除");

    let indexes: Vec<(String,)> =
        sqlx::query_as("SHOW INDEX FROM users WHERE Key_name = 'idx_email'")
            .fetch_all(&pool)
            .await
            .expect("查询索引失败");
    assert!(indexes.is_empty(), "'idx_email' 索引未被成功删除");

    // 验证数据
    let count: (i64,) = sqlx::query_as("SELECT COUNT(*) FROM users")
        .fetch_one(&pool)
        .await
        .expect("查询数据失败");
    assert_eq!(count.0, 1, "数据插入验证失败");

    println!("✅ 集成测试成功!");
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
