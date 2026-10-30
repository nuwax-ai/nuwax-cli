use super::parser::parse_sql_tables;
use super::*;

#[test]
fn changed_index_is_manual_only() {
    let old = "CREATE TABLE users (id INT, name VARCHAR(20), KEY idx_user (id));";
    let new = "CREATE TABLE users (id INT, name VARCHAR(20), KEY idx_user (name));";
    let (diff, _) =
        generate_schema_diff(Some(old), new, None, "new").expect("index diff should be generated");
    assert!(diff.contains("manually drop old index"));
    assert!(!diff.contains("ADD KEY `idx_user`"));
}

#[test]
fn strict_parser_rejects_invalid_table() {
    let invalid = "CREATE TABLE users (id INT, missing_definition);";
    assert!(parse_sql_tables_strict(invalid).is_err());
}

#[test]
fn test_simple_diff() {
    let from_sql = r#"
-- 平台使用,定义mysql单独一个数据库
CREATE DATABASE IF NOT EXISTS test_platform;
-- 数据表组件使用,定义mysql单独一个数据库
CREATE DATABASE IF NOT EXISTS test_custom_table;

GRANT ALL PRIVILEGES ON test_platform.* TO 'test_user'@'%';
GRANT ALL PRIVILEGES ON test_custom_table.* TO 'test_user'@'%';
FLUSH PRIVILEGES;

USE test_platform;

CREATE TABLE users (
    id INT NOT NULL AUTO_INCREMENT,
    name VARCHAR(255) NOT NULL,
    PRIMARY KEY (id)
) ENGINE=InnoDB;
    "#;

    let to_sql = r#"
-- 平台使用,定义mysql单独一个数据库
CREATE DATABASE IF NOT EXISTS test_platform;
-- 数据表组件使用,定义mysql单独一个数据库
CREATE DATABASE IF NOT EXISTS test_custom_table;

GRANT ALL PRIVILEGES ON test_platform.* TO 'test_user'@'%';
GRANT ALL PRIVILEGES ON test_custom_table.* TO 'test_user'@'%';
FLUSH PRIVILEGES;

USE test_platform;

CREATE TABLE users (
    id INT NOT NULL AUTO_INCREMENT,
    name VARCHAR(255) NOT NULL,
    email VARCHAR(255) DEFAULT 'unknown',
    PRIMARY KEY (id)
) ENGINE=InnoDB;
    "#;

    let (diff_sql, description) =
        generate_schema_diff(Some(from_sql), to_sql, Some("1.0.0"), "1.1.0").unwrap();
    println!("Diff SQL: {diff_sql}");
    println!("Description: {description}");

    assert!(diff_sql.contains("ALTER TABLE") && diff_sql.contains("ADD COLUMN"));
    assert!(diff_sql.contains("`email`") && diff_sql.contains("VARCHAR(255)"));
}

#[test]
fn test_parse_table() {
    let sql = r#"
-- 这是一个测试MySQL初始化文件
CREATE DATABASE IF NOT EXISTS test_db;
CREATE USER IF NOT EXISTS 'test_user'@'%' IDENTIFIED BY 'password123';
GRANT ALL PRIVILEGES ON test_db.* TO 'test_user'@'%';
FLUSH PRIVILEGES;

-- 这些语句应该被忽略，因为在USE语句之前
CREATE TABLE should_be_ignored (
    id INT PRIMARY KEY
);

USE test_db;

-- 从这里开始才是我们要解析的内容
CREATE TABLE users (
    id INT NOT NULL AUTO_INCREMENT,
    name VARCHAR(255) NOT NULL,
    PRIMARY KEY (id)
) ENGINE=InnoDB;
    "#;

    let tables = parse_sql_tables(sql).unwrap();
    assert_eq!(tables.len(), 1);

    let users_table = tables.get("users").unwrap();
    assert_eq!(users_table.name, "users");
    assert_eq!(users_table.columns.len(), 2);
    assert_eq!(users_table.indexes.len(), 1);

    // 确保被忽略的表没有被解析
    assert!(!tables.contains_key("should_be_ignored"));
}

#[test]
fn test_add_table() {
    let from_sql = r#"
-- 初始化数据库
CREATE DATABASE IF NOT EXISTS app_db;
GRANT ALL PRIVILEGES ON app_db.* TO 'app_user'@'%';
FLUSH PRIVILEGES;

USE app_db;

CREATE TABLE users (
    id INT NOT NULL AUTO_INCREMENT,
    PRIMARY KEY (id)
) ENGINE=InnoDB;
    "#;

    let to_sql = r#"
-- 初始化数据库
CREATE DATABASE IF NOT EXISTS app_db;
GRANT ALL PRIVILEGES ON app_db.* TO 'app_user'@'%';
FLUSH PRIVILEGES;

USE app_db;

CREATE TABLE users (
    id INT NOT NULL AUTO_INCREMENT,
    PRIMARY KEY (id)
) ENGINE=InnoDB;

CREATE TABLE posts (
    id INT NOT NULL AUTO_INCREMENT,
    title VARCHAR(255) NOT NULL,
    user_id INT,
    PRIMARY KEY (id)
) ENGINE=InnoDB;
    "#;

    let (diff_sql, description) =
        generate_schema_diff(Some(from_sql), to_sql, Some("1.0.0"), "1.1.0").unwrap();

    assert!(diff_sql.contains("CREATE TABLE `posts`"));
    assert!(description.contains("new tables"));
}

#[test]
fn test_drop_table() {
    let from_sql = r#"
CREATE TABLE users (
    id INT NOT NULL AUTO_INCREMENT,
    PRIMARY KEY (id)
) ENGINE=InnoDB;

CREATE TABLE posts (
    id INT NOT NULL AUTO_INCREMENT,
    title VARCHAR(255) NOT NULL,
    PRIMARY KEY (id)
) ENGINE=InnoDB;
    "#;

    let to_sql = r#"
CREATE TABLE users (
    id INT NOT NULL AUTO_INCREMENT,
    PRIMARY KEY (id)
) ENGINE=InnoDB;
    "#;

    let (diff_sql, description) =
        generate_schema_diff(Some(from_sql), to_sql, Some("1.0.0"), "1.1.0").unwrap();

    assert!(diff_sql.contains("DROP TABLE IF EXISTS `posts`"));
    assert!(description.contains("dropped tables"));
}

#[test]
fn test_no_changes() {
    let sql = r#"
CREATE TABLE users (
    id INT NOT NULL AUTO_INCREMENT,
    name VARCHAR(255) NOT NULL,
    PRIMARY KEY (id)
) ENGINE=InnoDB;
    "#;

    let (diff_sql, description) =
        generate_schema_diff(Some(sql), sql, Some("1.0.0"), "1.0.1").unwrap();
    assert!(diff_sql.is_empty());
    assert!(description.contains("No changes"));
}

#[test]
fn test_modify_column() {
    let from_sql = r#"
CREATE TABLE users (
    id INT NOT NULL AUTO_INCREMENT,
    name VARCHAR(100) NOT NULL,
    PRIMARY KEY (id)
) ENGINE=InnoDB;
    "#;

    let to_sql = r#"
CREATE TABLE users (
    id INT NOT NULL AUTO_INCREMENT,
    name VARCHAR(255) NOT NULL,
    PRIMARY KEY (id)
) ENGINE=InnoDB;
    "#;

    let (diff_sql, _description) =
        generate_schema_diff(Some(from_sql), to_sql, Some("1.0.0"), "1.1.0").unwrap();
    println!("Modify column diff SQL: {diff_sql}");
    assert!(diff_sql.contains("ALTER TABLE"));
}

#[test]
fn test_add_index() {
    let from_sql = r#"
CREATE TABLE users (
    id INT NOT NULL AUTO_INCREMENT,
    name VARCHAR(255) NOT NULL,
    email VARCHAR(255),
    PRIMARY KEY (id)
) ENGINE=InnoDB;
    "#;

    let to_sql = r#"
CREATE TABLE users (
    id INT NOT NULL AUTO_INCREMENT,
    name VARCHAR(255) NOT NULL,
    email VARCHAR(255),
    PRIMARY KEY (id),
    UNIQUE KEY uk_email (email),
    KEY idx_name (name)
) ENGINE=InnoDB;
    "#;

    let (diff_sql, _description) =
        generate_schema_diff(Some(from_sql), to_sql, Some("1.0.0"), "1.1.0").unwrap();
    println!("Index diff SQL: {diff_sql}");

    assert!(diff_sql.contains("ALTER TABLE") && diff_sql.contains("ADD"));
    assert!(diff_sql.contains("KEY") || diff_sql.contains("INDEX"));
}

#[test]
fn test_tenant_unique_index_constraint() {
    // 测试你提供的具体场景：tenant表的domain列新增唯一索引
    let from_sql = r#"
CREATE TABLE tenant (
    id bigint auto_increment primary key,
    name varchar(255) not null comment '商户名称',
    description text null comment '商户介绍',
    status enum ('Pending', 'Enabled', 'Disabled') not null comment '商户状态',
    domain varchar(64) default '' not null,
    modified datetime default CURRENT_TIMESTAMP not null on update CURRENT_TIMESTAMP comment '更新时间',
    created datetime default CURRENT_TIMESTAMP not null comment '创建时间'
) ENGINE=InnoDB;
    "#;

    let to_sql = r#"
CREATE TABLE tenant (
    id bigint auto_increment primary key,
    name varchar(255) not null comment '商户名称',
    description text null comment '商户介绍',
    status enum ('Pending', 'Enabled', 'Disabled') not null comment '商户状态',
    domain varchar(64) default '' not null,
    modified datetime default CURRENT_TIMESTAMP not null on update CURRENT_TIMESTAMP comment '更新时间',
    created datetime default CURRENT_TIMESTAMP not null comment '创建时间',
    constraint uk_domain unique (domain)
) ENGINE=InnoDB;
    "#;

    let (diff_sql, description) =
        generate_schema_diff(Some(from_sql), to_sql, Some("1.0.0"), "1.0.1").unwrap();

    println!("Tenant unique index diff SQL:");
    println!("{diff_sql}");
    println!("Description: {description}");

    // 验证是否能正确检测到唯一索引的变化
    assert!(
        diff_sql.contains("ALTER TABLE `tenant`"),
        "应该包含 ALTER TABLE tenant"
    );
    assert!(diff_sql.contains("ADD UNIQUE"), "应该包含 ADD UNIQUE");
    assert!(diff_sql.contains("uk_domain"), "应该包含索引名 uk_domain");
    assert!(diff_sql.contains("`domain`"), "应该包含列名 domain");
}

#[test]
fn test_tenant_unique_index_removal() {
    // 测试相反的场景：删除唯一索引
    let from_sql = r#"
CREATE TABLE tenant (
    id bigint auto_increment primary key,
    name varchar(255) not null comment '商户名称',
    description text null comment '商户介绍',
    status enum ('Pending', 'Enabled', 'Disabled') not null comment '商户状态',
    domain varchar(64) default '' not null,
    modified datetime default CURRENT_TIMESTAMP not null on update CURRENT_TIMESTAMP comment '更新时间',
    created datetime default CURRENT_TIMESTAMP not null comment '创建时间',
    constraint uk_domain unique (domain)
) ENGINE=InnoDB;
    "#;

    let to_sql = r#"
CREATE TABLE tenant (
    id bigint auto_increment primary key,
    name varchar(255) not null comment '商户名称',
    description text null comment '商户介绍',
    status enum ('Pending', 'Enabled', 'Disabled') not null comment '商户状态',
    domain varchar(64) default '' not null,
    modified datetime default CURRENT_TIMESTAMP not null on update CURRENT_TIMESTAMP comment '更新时间',
    created datetime default CURRENT_TIMESTAMP not null comment '创建时间'
) ENGINE=InnoDB;
    "#;

    let (diff_sql, description) =
        generate_schema_diff(Some(from_sql), to_sql, Some("1.0.1"), "1.0.2").unwrap();

    println!("Tenant unique index removal diff SQL:");
    println!("{diff_sql}");
    println!("Description: {description}");

    // 验证是否能正确检测到唯一索引的删除
    assert!(
        diff_sql.contains("ALTER TABLE `tenant`"),
        "应该包含 ALTER TABLE tenant"
    );
    assert!(diff_sql.contains("DROP KEY"), "应该包含 DROP KEY");
    assert!(diff_sql.contains("uk_domain"), "应该包含索引名 uk_domain");
}

#[test]
fn test_modify_varchar_length() {
    let from_sql = r#"
CREATE TABLE users (
    id INT NOT NULL AUTO_INCREMENT,
    name VARCHAR(50) NOT NULL COMMENT '用户名',
    email VARCHAR(100) DEFAULT 'unknown@example.com',
    phone VARCHAR(15),
    PRIMARY KEY (id)
) ENGINE=InnoDB;
    "#;

    let to_sql = r#"
CREATE TABLE users (
    id INT NOT NULL AUTO_INCREMENT,
    name VARCHAR(128) NOT NULL COMMENT '用户名',
    email VARCHAR(255) DEFAULT 'unknown@example.com',
    phone VARCHAR(20),
    PRIMARY KEY (id)
) ENGINE=InnoDB;
    "#;

    let (diff_sql, description) =
        generate_schema_diff(Some(from_sql), to_sql, Some("1.0.0"), "1.1.0").unwrap();
    println!("VARCHAR长度修改差异:");
    println!("Description: {description}");
    println!("Diff SQL:");
    println!("{diff_sql}");

    assert!(diff_sql.contains("ALTER TABLE"));
    assert!(diff_sql.contains("MODIFY COLUMN") || diff_sql.contains("CHANGE COLUMN"));

    assert!(diff_sql.contains("`name`"));
    assert!(diff_sql.contains("`email`"));
    assert!(diff_sql.contains("`phone`"));

    assert!(
        diff_sql.contains("VARCHAR(128)")
            || diff_sql.contains("VARCHAR(255)")
            || diff_sql.contains("VARCHAR(20)")
    );
}

#[test]
fn test_modify_default_value() {
    let from_sql = r#"
CREATE TABLE products (
    id INT NOT NULL AUTO_INCREMENT,
    name VARCHAR(255) NOT NULL,
    status TINYINT DEFAULT 0 COMMENT '状态: 0=禁用, 1=启用',
    price DECIMAL(10,2) DEFAULT 0.00,
    created_at DATETIME DEFAULT CURRENT_TIMESTAMP,
    PRIMARY KEY (id)
) ENGINE=InnoDB;
    "#;

    let to_sql = r#"
CREATE TABLE products (
    id INT NOT NULL AUTO_INCREMENT,
    name VARCHAR(255) NOT NULL,
    status TINYINT DEFAULT 1 COMMENT '状态: 0=禁用, 1=启用',
    price DECIMAL(10,2) DEFAULT 9.99,
    created_at DATETIME DEFAULT CURRENT_TIMESTAMP,
    PRIMARY KEY (id)
) ENGINE=InnoDB;
    "#;

    let (diff_sql, description) =
        generate_schema_diff(Some(from_sql), to_sql, Some("1.0.0"), "1.1.0").unwrap();
    println!("默认值修改差异:");
    println!("Description: {description}");
    println!("Diff SQL:");
    println!("{diff_sql}");

    assert!(diff_sql.contains("ALTER TABLE"));

    assert!(diff_sql.contains("`status`") || diff_sql.contains("`price`"));
}

#[test]
fn test_modify_comment() {
    let from_sql = r#"
-- 系统数据库初始化
CREATE DATABASE IF NOT EXISTS user_system;
CREATE DATABASE IF NOT EXISTS log_system;

GRANT ALL PRIVILEGES ON user_system.* TO 'admin'@'%';
GRANT ALL PRIVILEGES ON log_system.* TO 'admin'@'%';
FLUSH PRIVILEGES;

-- 切换到用户系统数据库
USE user_system;

CREATE TABLE users (
    id INT NOT NULL AUTO_INCREMENT,
    name VARCHAR(255) NOT NULL COMMENT '姓名',
    email VARCHAR(255) COMMENT '电子邮箱',
    age INT COMMENT '年龄',
    PRIMARY KEY (id)
) ENGINE=InnoDB;
    "#;

    let to_sql = r#"
-- 系统数据库初始化
CREATE DATABASE IF NOT EXISTS user_system;
CREATE DATABASE IF NOT EXISTS log_system;

GRANT ALL PRIVILEGES ON user_system.* TO 'admin'@'%';
GRANT ALL PRIVILEGES ON log_system.* TO 'admin'@'%';
FLUSH PRIVILEGES;

-- 切换到用户系统数据库
USE user_system;

CREATE TABLE users (
    id INT NOT NULL AUTO_INCREMENT,
    name VARCHAR(255) NOT NULL COMMENT '用户姓名',
    email VARCHAR(255) COMMENT '用户电子邮箱地址',
    age INT COMMENT '用户年龄（周岁）',
    PRIMARY KEY (id)
) ENGINE=InnoDB;
    "#;

    let (diff_sql, description) =
        generate_schema_diff(Some(from_sql), to_sql, Some("1.0.0"), "1.1.0").unwrap();
    println!("注释修改差异:");
    println!("Description: {description}");
    println!("Diff SQL:");
    println!("{diff_sql}");

    if !diff_sql.is_empty() {
        assert!(diff_sql.contains("ALTER TABLE"));
        assert!(diff_sql.contains("MODIFY COLUMN") || diff_sql.contains("CHANGE COLUMN"));
    }
}

#[test]
fn test_modify_nullable() {
    let from_sql = r#"
CREATE TABLE orders (
    id INT NOT NULL AUTO_INCREMENT,
    customer_name VARCHAR(255) NOT NULL,
    customer_email VARCHAR(255),
    notes TEXT,
    PRIMARY KEY (id)
) ENGINE=InnoDB;
    "#;

    let to_sql = r#"
CREATE TABLE orders (
    id INT NOT NULL AUTO_INCREMENT,
    customer_name VARCHAR(255) NOT NULL,
    customer_email VARCHAR(255) NOT NULL,
    notes TEXT NOT NULL,
    PRIMARY KEY (id)
) ENGINE=InnoDB;
    "#;

    let (diff_sql, description) =
        generate_schema_diff(Some(from_sql), to_sql, Some("1.0.0"), "1.1.0").unwrap();
    println!("NULL约束修改差异:");
    println!("Description: {description}");
    println!("Diff SQL:");
    println!("{diff_sql}");

    if !diff_sql.is_empty() {
        assert!(diff_sql.contains("ALTER TABLE"));
        assert!(diff_sql.contains("`customer_email`") || diff_sql.contains("`notes`"));
        assert!(diff_sql.contains("NOT NULL"));
    }
}

#[test]
fn test_modify_data_type() {
    let from_sql = r#"
CREATE TABLE analytics (
    id INT NOT NULL AUTO_INCREMENT,
    user_id INT NOT NULL,
    view_count INT DEFAULT 0,
    score FLOAT DEFAULT 0.0,
    created_date DATE,
    PRIMARY KEY (id)
) ENGINE=InnoDB;
    "#;

    let to_sql = r#"
CREATE TABLE analytics (
    id INT NOT NULL AUTO_INCREMENT,
    user_id BIGINT NOT NULL,
    view_count BIGINT DEFAULT 0,
    score DOUBLE DEFAULT 0.0,
    created_date DATETIME,
    PRIMARY KEY (id)
) ENGINE=InnoDB;
    "#;

    let (diff_sql, description) =
        generate_schema_diff(Some(from_sql), to_sql, Some("1.0.0"), "1.1.0").unwrap();
    println!("数据类型修改差异:");
    println!("Description: {description}");
    println!("Diff SQL:");
    println!("{diff_sql}");

    assert!(diff_sql.contains("ALTER TABLE"));

    assert!(
        diff_sql.contains("`user_id`")
            || diff_sql.contains("`view_count`")
            || diff_sql.contains("`score`")
            || diff_sql.contains("`created_date`")
    );

    assert!(
        diff_sql.contains("BIGINT") || diff_sql.contains("DOUBLE") || diff_sql.contains("DATETIME")
    );
}

#[test]
fn test_complex_column_modifications() {
    let from_sql = r#"
CREATE TABLE user_profiles (
    id INT NOT NULL AUTO_INCREMENT,
    username VARCHAR(50) NOT NULL COMMENT '用户名',
    bio TEXT COMMENT '个人简介',
    status ENUM('active', 'inactive') DEFAULT 'inactive' COMMENT '账户状态',
    avatar_url VARCHAR(200) DEFAULT '/default.jpg' COMMENT '头像地址',
    PRIMARY KEY (id)
) ENGINE=InnoDB;
    "#;

    let to_sql = r#"
CREATE TABLE user_profiles (
    id INT NOT NULL AUTO_INCREMENT,
    username VARCHAR(100) NOT NULL COMMENT '用户登录名',
    bio TEXT COMMENT '用户个人简介描述',
    status ENUM('active', 'inactive', 'suspended') DEFAULT 'active' COMMENT '用户账户状态',
    avatar_url VARCHAR(500) DEFAULT '/assets/default-avatar.png' COMMENT '用户头像图片地址',
    PRIMARY KEY (id)
) ENGINE=InnoDB;
    "#;

    let (diff_sql, description) =
        generate_schema_diff(Some(from_sql), to_sql, Some("1.0.0"), "1.1.0").unwrap();
    println!("复合字段修改差异:");
    println!("Description: {description}");
    println!("Diff SQL:");
    println!("{diff_sql}");

    assert!(diff_sql.contains("ALTER TABLE"));

    assert!(
        diff_sql.contains("`username`")
            || diff_sql.contains("`bio`")
            || diff_sql.contains("`status`")
            || diff_sql.contains("`avatar_url`")
    );
}

#[test]
fn test_use_statement_splitting() {
    // 测试USE语句分割逻辑，确保只解析USE语句之后的内容
    let sql_with_interference = r#"
-- 这是MySQL初始化脚本
-- 创建数据库和用户
CREATE DATABASE IF NOT EXISTS main_app;
CREATE DATABASE IF NOT EXISTS logs_app;
CREATE DATABASE IF NOT EXISTS cache_app;

-- 创建用户并授权
CREATE USER IF NOT EXISTS 'app_user'@'%' IDENTIFIED BY 'secure_password';
GRANT ALL PRIVILEGES ON main_app.* TO 'app_user'@'%';
GRANT ALL PRIVILEGES ON logs_app.* TO 'app_user'@'%';
GRANT SELECT, INSERT ON cache_app.* TO 'app_user'@'%';
FLUSH PRIVILEGES;

-- 这些表定义应该被忽略，因为在USE语句之前
CREATE TABLE ignored_table1 (
    id INT PRIMARY KEY,
    data VARCHAR(100)
);

CREATE TABLE ignored_table2 (
    id BIGINT AUTO_INCREMENT PRIMARY KEY,
    name VARCHAR(255) NOT NULL
) ENGINE=InnoDB;

-- 一些其他的干扰语句
SET GLOBAL sql_mode = 'STRICT_TRANS_TABLES,NO_ZERO_DATE,NO_ZERO_IN_DATE,ERROR_FOR_DIVISION_BY_ZERO';
SET GLOBAL innodb_file_per_table = ON;

-- 现在切换到目标数据库
USE main_app;

-- 从这里开始才是我们要解析的表定义
CREATE TABLE users (
    id BIGINT AUTO_INCREMENT PRIMARY KEY,
    username VARCHAR(64) NOT NULL COMMENT '用户名',
    email VARCHAR(255) NOT NULL COMMENT '邮箱地址',
    password_hash VARCHAR(255) NOT NULL,
    created_at DATETIME DEFAULT CURRENT_TIMESTAMP,
    updated_at DATETIME DEFAULT CURRENT_TIMESTAMP ON UPDATE CURRENT_TIMESTAMP,
    UNIQUE KEY uk_username (username),
    UNIQUE KEY uk_email (email)
) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4 COMMENT='用户表';

CREATE TABLE posts (
    id BIGINT AUTO_INCREMENT PRIMARY KEY,
    user_id BIGINT NOT NULL,
    title VARCHAR(255) NOT NULL COMMENT '文章标题',
    content TEXT COMMENT '文章内容',
    status TINYINT DEFAULT 1 COMMENT '状态：1=发布，0=草稿',
    created_at DATETIME DEFAULT CURRENT_TIMESTAMP,
    KEY idx_user_id (user_id),
    KEY idx_status (status),
    KEY idx_created_at (created_at)
) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4 COMMENT='文章表';
    "#;

    let tables = parse_sql_tables(sql_with_interference).unwrap();

    println!("解析到的表数量: {}", tables.len());
    println!("解析到的表: {:?}", tables.keys().collect::<Vec<_>>());

    // 应该只解析到USE语句之后的表
    assert_eq!(tables.len(), 2);
    assert!(tables.contains_key("users"));
    assert!(tables.contains_key("posts"));

    // 确保被忽略的表没有被解析
    assert!(!tables.contains_key("ignored_table1"));
    assert!(!tables.contains_key("ignored_table2"));

    // 验证users表的结构
    let users_table = tables.get("users").unwrap();
    assert_eq!(users_table.name, "users");
    assert_eq!(users_table.columns.len(), 6); // id, username, email, password_hash, created_at, updated_at
    assert_eq!(users_table.indexes.len(), 3); // PRIMARY, uk_username, uk_email

    // 验证posts表的结构
    let posts_table = tables.get("posts").unwrap();
    assert_eq!(posts_table.name, "posts");
    assert_eq!(posts_table.columns.len(), 6); // id, user_id, title, content, status, created_at
    assert_eq!(posts_table.indexes.len(), 4); // PRIMARY, idx_user_id, idx_status, idx_created_at

    println!("✅ USE语句分割逻辑测试通过");
}

#[test]
fn test_parse_real_mysql_sql() {
    // 使用模拟的真实 MySQL SQL 内容进行测试，而不是依赖外部文件
    let sql_content = r#"
-- 真实的 MySQL 数据库初始化脚本示例
CREATE DATABASE IF NOT EXISTS duck_server;
CREATE USER IF NOT EXISTS 'duck_admin'@'%' IDENTIFIED BY 'duck_password';
GRANT ALL PRIVILEGES ON duck_server.* TO 'duck_admin'@'%';
FLUSH PRIVILEGES;

SET GLOBAL innodb_buffer_pool_size = 1073741824;
SET GLOBAL max_connections = 1000;

USE duck_server;

CREATE TABLE agent_component_config (
    id BIGINT AUTO_INCREMENT PRIMARY KEY,
    component_name VARCHAR(255) NOT NULL COMMENT '组件名称',
    config_json TEXT NOT NULL COMMENT '配置JSON',
    version VARCHAR(32) NOT NULL DEFAULT '1.0.0' COMMENT '版本号',
    status TINYINT DEFAULT 1 COMMENT '状态：1=启用，0=禁用',
    created_at DATETIME DEFAULT CURRENT_TIMESTAMP,
    updated_at DATETIME DEFAULT CURRENT_TIMESTAMP ON UPDATE CURRENT_TIMESTAMP,
    UNIQUE KEY uk_component_version (component_name, version),
    KEY idx_component_name (component_name),
    KEY idx_status (status),
    KEY idx_created_at (created_at)
) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4 COMMENT='组件配置表';

CREATE TABLE client_versions (
    id TEXT PRIMARY KEY,
    tag_name TEXT NOT NULL UNIQUE,
    version_name TEXT NOT NULL,
    release_notes TEXT,
    github_release_id INTEGER,
    github_created_at DATETIME,
    github_published_at DATETIME,
    sync_status TEXT NOT NULL DEFAULT 'PENDING',
    sync_started_at DATETIME,
    sync_completed_at DATETIME,
    error_message TEXT,
    created_at DATETIME DEFAULT CURRENT_TIMESTAMP,
    updated_at DATETIME DEFAULT CURRENT_TIMESTAMP
) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4 COMMENT='客户端版本表';

CREATE TABLE client_assets (
    id TEXT PRIMARY KEY,
    version_id TEXT NOT NULL,
    asset_name TEXT NOT NULL,
    platform TEXT NOT NULL,
    file_type TEXT NOT NULL,
    original_url TEXT NOT NULL,
    oss_url TEXT,
    file_size INTEGER,
    sha256_hash TEXT,
    download_status TEXT DEFAULT 'PENDING',
    created_at DATETIME DEFAULT CURRENT_TIMESTAMP,
    updated_at DATETIME DEFAULT CURRENT_TIMESTAMP,
    FOREIGN KEY (version_id) REFERENCES client_versions(id)
) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4 COMMENT='客户端构建包表';
    "#;

    println!("SQL 文件长度: {} 字符", sql_content.len());

    // 查找 USE 语句
    let lines: Vec<&str> = sql_content.lines().collect();
    for (i, line) in lines.iter().enumerate() {
        let line_trimmed = line.trim().to_uppercase();
        if line_trimmed.starts_with("USE ") {
            println!("找到 USE 语句在第 {} 行: {}", i + 1, line);
        }
    }

    let tables = parse_sql_tables(sql_content).unwrap();

    println!("解析到的表: {:?}", tables.keys().collect::<Vec<_>>());

    // 应该解析到3个表
    assert_eq!(tables.len(), 3, "应该能解析到3个表");
    assert!(tables.contains_key("agent_component_config"));
    assert!(tables.contains_key("client_versions"));
    assert!(tables.contains_key("client_assets"));

    // 检查 agent_component_config 表结构
    if let Some(agent_table) = tables.get("agent_component_config") {
        println!("agent_component_config 表结构: {agent_table:?}");
        // 验证表结构
        assert!(!agent_table.columns.is_empty());
        assert_eq!(agent_table.columns.len(), 7); // id, component_name, config_json, version, status, created_at, updated_at
        assert_eq!(agent_table.indexes.len(), 5); // PRIMARY, uk_component_version, idx_component_name, idx_status, idx_created_at
    }

    // 检查 client_versions 表结构
    if let Some(client_versions_table) = tables.get("client_versions") {
        println!("client_versions 表结构: {client_versions_table:?}");
        assert_eq!(client_versions_table.columns.len(), 13);
    }

    // 检查 client_assets 表结构
    if let Some(client_assets_table) = tables.get("client_assets") {
        println!("client_assets 表结构: {client_assets_table:?}");
        assert_eq!(client_assets_table.columns.len(), 12);
    }

    println!("✅ 真实 MySQL SQL 解析测试通过");
}

#[test]
fn test_complex_sql_diff() {
    let v1_sql = r#"
-- 应用数据库v1.0初始化脚本
CREATE DATABASE IF NOT EXISTS app_v1;
CREATE DATABASE IF NOT EXISTS app_logs;

-- 创建应用用户
CREATE USER IF NOT EXISTS 'app_admin'@'%' IDENTIFIED BY 'app_password_v1';
GRANT ALL PRIVILEGES ON app_v1.* TO 'app_admin'@'%';
GRANT INSERT, SELECT ON app_logs.* TO 'app_admin'@'%';
FLUSH PRIVILEGES;

-- 设置一些全局参数
SET GLOBAL max_connections = 1000;
SET GLOBAL innodb_buffer_pool_size = 1073741824;

USE app_v1;

CREATE TABLE users (
    id BIGINT AUTO_INCREMENT PRIMARY KEY,
    name VARCHAR(64) NOT NULL COMMENT '用户名',
    email VARCHAR(255) DEFAULT 'unknown' COMMENT '邮箱',
    created DATETIME DEFAULT CURRENT_TIMESTAMP NOT NULL
) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4;

CREATE TABLE posts (
    id BIGINT AUTO_INCREMENT PRIMARY KEY,
    title VARCHAR(255) NOT NULL,
    content TEXT,
    user_id BIGINT,
    created DATETIME DEFAULT CURRENT_TIMESTAMP NOT NULL
) ENGINE=InnoDB;
    "#;

    let v2_sql = r#"
-- 应用数据库v2.0升级脚本
CREATE DATABASE IF NOT EXISTS app_v1;
CREATE DATABASE IF NOT EXISTS app_logs;

-- 创建应用用户（密码已更新）
CREATE USER IF NOT EXISTS 'app_admin'@'%' IDENTIFIED BY 'app_password_v2';
GRANT ALL PRIVILEGES ON app_v1.* TO 'app_admin'@'%';
GRANT INSERT, SELECT ON app_logs.* TO 'app_admin'@'%';
FLUSH PRIVILEGES;

-- 设置一些全局参数（已优化）
SET GLOBAL max_connections = 2000;
SET GLOBAL innodb_buffer_pool_size = 2147483648;

USE app_v1;

CREATE TABLE users (
    id BIGINT AUTO_INCREMENT PRIMARY KEY,
    name VARCHAR(128) NOT NULL COMMENT '用户名',
    email VARCHAR(255) DEFAULT 'unknown' COMMENT '邮箱',
    phone VARCHAR(20) COMMENT '手机号',
    created DATETIME DEFAULT CURRENT_TIMESTAMP NOT NULL,
    UNIQUE KEY uk_email (email),
    KEY idx_name (name)
) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4;

CREATE TABLE posts (
    id BIGINT AUTO_INCREMENT PRIMARY KEY,
    title VARCHAR(255) NOT NULL,
    content TEXT,
    user_id BIGINT,
    status TINYINT DEFAULT 1 NOT NULL COMMENT '状态',
    created DATETIME DEFAULT CURRENT_TIMESTAMP NOT NULL,
    KEY idx_user_id (user_id)
) ENGINE=InnoDB;

CREATE TABLE comments (
    id BIGINT AUTO_INCREMENT PRIMARY KEY,
    post_id BIGINT NOT NULL,
    content TEXT NOT NULL,
    created DATETIME DEFAULT CURRENT_TIMESTAMP NOT NULL
) ENGINE=InnoDB;
    "#;

    let (diff_sql, description) =
        generate_schema_diff(Some(v1_sql), v2_sql, Some("1.0.0"), "2.0.0").unwrap();

    println!("复杂SQL差异:");
    println!("Description: {description}");
    println!("Diff SQL:");
    println!("{diff_sql}");

    assert!(diff_sql.contains("ALTER TABLE") || diff_sql.contains("CREATE TABLE"));

    assert!(diff_sql.contains("comments"));

    assert!(diff_sql.contains("users"));

    assert!(diff_sql.contains("posts"));
}

#[test]
fn test_fulltext_index_diff_standalone() {
    // 验证 `CREATE FULLTEXT INDEX` 独立语句被识别并生成 ADD FULLTEXT KEY
    let from_sql = r#"
USE test_platform;

CREATE TABLE published (
    id BIGINT NOT NULL AUTO_INCREMENT,
    name VARCHAR(255),
    description TEXT,
    PRIMARY KEY (id)
);
    "#;

    let to_sql = r#"
USE test_platform;

CREATE TABLE published (
    id BIGINT NOT NULL AUTO_INCREMENT,
    name VARCHAR(255),
    description TEXT,
    PRIMARY KEY (id)
);

CREATE FULLTEXT INDEX ft_name_desc ON published (name, description);
    "#;

    let (diff_sql, _description) =
        generate_schema_diff(Some(from_sql), to_sql, Some("1.0.0"), "2.0.0").unwrap();

    assert!(
        diff_sql.contains("ADD FULLTEXT KEY `ft_name_desc`"),
        "expected FULLTEXT KEY diff, got: {diff_sql}"
    );
    assert!(
        diff_sql.contains("`name`, `description`"),
        "expected both columns in diff, got: {diff_sql}"
    );
    // 没有 WITH PARSER 时不应凭空生成该子句
    assert!(
        !diff_sql.contains("WITH PARSER"),
        "expected no WITH PARSER clause, got: {diff_sql}"
    );
}

#[test]
fn test_fulltext_index_with_parser_diff() {
    // 验证 `CREATE FULLTEXT INDEX ... WITH PARSER ngram` 的 parser 子句被透传到 diff
    let from_sql = r#"
USE test_platform;

CREATE TABLE published (
    id BIGINT NOT NULL AUTO_INCREMENT,
    name VARCHAR(255),
    description TEXT,
    PRIMARY KEY (id)
);
    "#;

    let to_sql = r#"
USE test_platform;

CREATE TABLE published (
    id BIGINT NOT NULL AUTO_INCREMENT,
    name VARCHAR(255),
    description TEXT,
    PRIMARY KEY (id)
);

CREATE FULLTEXT INDEX ft_name_desc ON published (name, description) WITH PARSER ngram;
    "#;

    let (diff_sql, _description) =
        generate_schema_diff(Some(from_sql), to_sql, Some("1.0.0"), "2.0.0").unwrap();

    assert!(
        diff_sql
            .contains("ADD FULLTEXT KEY `ft_name_desc` (`name`, `description`) WITH PARSER ngram"),
        "expected FULLTEXT KEY diff with WITH PARSER ngram clause, got: {diff_sql}"
    );
}

#[test]
fn test_inline_fulltext_with_parser_versioned_comment() {
    // 线上 `SHOW CREATE TABLE` 的真实输出：WITH PARSER 被包在 `/*!50100 */` 版本注释里，
    // 且 FULLTEXT 以 CREATE TABLE 内联 KEY 形式存在（非 standalone CREATE INDEX）。
    // 验证 fork-sqlparser 0.63.2+ 能解析内联 FULLTEXT 的 index_options，且 nuwax-cli
    // 能提取 parser 名并透传到 diff（auto-upgrade-deploy live schema 的核心场景）。
    let from_sql = r#"
USE test_platform;

CREATE TABLE published (
    id BIGINT NOT NULL AUTO_INCREMENT,
    name VARCHAR(255),
    description TEXT,
    PRIMARY KEY (id)
);
    "#;

    let to_sql = r#"
USE test_platform;

CREATE TABLE published (
    id BIGINT NOT NULL AUTO_INCREMENT,
    name VARCHAR(255),
    description TEXT,
    PRIMARY KEY (id),
    FULLTEXT KEY ft_name_desc (name, description) /*!50100 WITH PARSER `ngram` */
);
    "#;

    let (diff_sql, _description) =
        generate_schema_diff(Some(from_sql), to_sql, Some("1.0.0"), "2.0.0").unwrap();

    assert!(
        diff_sql
            .contains("ADD FULLTEXT KEY `ft_name_desc` (`name`, `description`) WITH PARSER ngram"),
        "inline FULLTEXT KEY with versioned-comment WITH PARSER must propagate to diff, got: {diff_sql}"
    );
}

#[test]
fn test_spatial_index_diff_standalone() {
    let from_sql = r#"
USE test_platform;

CREATE TABLE geo_data (
    id BIGINT NOT NULL AUTO_INCREMENT,
    geom GEOMETRY NOT NULL,
    PRIMARY KEY (id)
);
    "#;

    let to_sql = r#"
USE test_platform;

CREATE TABLE geo_data (
    id BIGINT NOT NULL AUTO_INCREMENT,
    geom GEOMETRY NOT NULL,
    PRIMARY KEY (id)
);

CREATE SPATIAL INDEX sp_geom ON geo_data (geom);
    "#;

    let (diff_sql, _description) =
        generate_schema_diff(Some(from_sql), to_sql, Some("1.0.0"), "2.0.0").unwrap();

    assert!(
        diff_sql.contains("ADD SPATIAL KEY `sp_geom`"),
        "expected SPATIAL KEY diff, got: {diff_sql}"
    );
}

#[test]
fn test_fulltext_index_unchanged_no_redo() {
    // 当 old 和 new 都有相同 FULLTEXT 索引时,diff 不应再次 ADD
    let from_sql = r#"
USE test_platform;

CREATE TABLE published (
    id BIGINT NOT NULL AUTO_INCREMENT,
    name VARCHAR(255),
    description TEXT,
    PRIMARY KEY (id)
);

CREATE FULLTEXT INDEX ft_name_desc ON published (name, description);
    "#;

    let to_sql = from_sql; // 完全相同

    let (diff_sql, _description) =
        generate_schema_diff(Some(from_sql), to_sql, Some("1.0.0"), "2.0.0").unwrap();

    assert!(
        diff_sql.trim().is_empty(),
        "expected no diff when FULLTEXT unchanged, got: {diff_sql}"
    );
}

#[test]
fn test_fulltext_index_in_create_table() {
    // 验证 `CREATE TABLE` 内部 FULLTEXT 约束形式也能被正确识别
    let from_sql = r#"
USE test_platform;

CREATE TABLE published (
    id BIGINT NOT NULL AUTO_INCREMENT,
    name VARCHAR(255),
    description TEXT,
    PRIMARY KEY (id)
);
    "#;

    let to_sql = r#"
USE test_platform;

CREATE TABLE published (
    id BIGINT NOT NULL AUTO_INCREMENT,
    name VARCHAR(255),
    description TEXT,
    PRIMARY KEY (id),
    FULLTEXT INDEX ft_name_desc (name, description)
);
    "#;

    let (diff_sql, _description) =
        generate_schema_diff(Some(from_sql), to_sql, Some("1.0.0"), "2.0.0").unwrap();

    assert!(
        diff_sql.contains("ADD FULLTEXT KEY `ft_name_desc`"),
        "expected in-table FULLTEXT to produce ADD FULLTEXT KEY, got: {diff_sql}"
    );
}

// ============ 六缺陷回归测试（2026-09 diff-sql 修复） ============
// 背景：用真实 schema（100→131 表）验证发现生成器存在六处缺陷，
// 以下用例逐项锁定行为；断言用精确片段而非弱 contains。

/// 公共基线：old 侧只有 base 表，保证目标表走「新表 CREATE」或「存量表 ALTER」两条路径
const BASE_OLD: &str = r#"
USE app;
CREATE TABLE base (
    id BIGINT NOT NULL AUTO_INCREMENT,
    PRIMARY KEY (`id`)
) ENGINE=InnoDB;
    "#;

#[test]
fn regression_unsigned_integer_types_render_valid_sql() {
    // 缺陷1：Unsigned 整型族此前走 Debug 兜底，把 BigIntUnsigned(None) 直接印进 DDL
    let new_sql = r#"
USE app;
CREATE TABLE base (
    id BIGINT NOT NULL AUTO_INCREMENT,
    PRIMARY KEY (`id`)
) ENGINE=InnoDB;

CREATE TABLE `repo_page` (
    `id` bigint unsigned NOT NULL AUTO_INCREMENT COMMENT '主键ID',
    `tenant_id` int unsigned NOT NULL,
    `weight` tinyint unsigned DEFAULT NULL,
    PRIMARY KEY (`id`)
) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4 COLLATE=utf8mb4_0900_ai_ci;
    "#;
    let (diff_sql, _) =
        generate_schema_diff(Some(BASE_OLD), new_sql, Some("1.0.0"), "1.1.0").unwrap();
    assert!(
        diff_sql.contains("CREATE TABLE `repo_page`"),
        "应生成新表 CREATE: {diff_sql}"
    );
    assert!(
        diff_sql.contains("`id` BIGINT UNSIGNED NOT NULL AUTO_INCREMENT"),
        "bigint unsigned 应渲染为合法 SQL: {diff_sql}"
    );
    assert!(
        diff_sql.contains("`tenant_id` INT UNSIGNED NOT NULL"),
        "{diff_sql}"
    );
    assert!(
        diff_sql.contains("`weight` TINYINT UNSIGNED DEFAULT NULL"),
        "{diff_sql}"
    );
    assert!(
        !diff_sql.contains("Unsigned("),
        "Debug 格式泄漏: {diff_sql}"
    );
}

#[test]
fn regression_mysql_style_unique_key_name_preserved() {
    // 缺陷2：MySQL 风格 `UNIQUE KEY uk_x (...)` 的名字在 UniqueConstraint.index_name，
    // 此前只读 name 字段 → 名字丢失被合成为 unique_<列名>
    let old_sql = r#"
USE app;
CREATE TABLE `user` (
    `id` BIGINT NOT NULL AUTO_INCREMENT,
    `email` VARCHAR(255) NOT NULL,
    PRIMARY KEY (`id`)
) ENGINE=InnoDB;
    "#;
    let new_sql = r#"
USE app;
CREATE TABLE `user` (
    `id` BIGINT NOT NULL AUTO_INCREMENT,
    `email` VARCHAR(255) NOT NULL,
    PRIMARY KEY (`id`),
    UNIQUE KEY `uk_email` (`email`)
) ENGINE=InnoDB;
    "#;
    let (diff_sql, _) =
        generate_schema_diff(Some(old_sql), new_sql, Some("1.0.0"), "1.1.0").unwrap();
    assert!(
        diff_sql.contains("ALTER TABLE `user` ADD UNIQUE KEY `uk_email` (`email`)"),
        "唯一索引名 uk_email 必须保留: {diff_sql}"
    );
    assert!(
        !diff_sql.contains("`unique_email`"),
        "不应出现合成名 unique_email: {diff_sql}"
    );
}

#[test]
fn regression_on_update_current_timestamp_preserved() {
    // 缺陷3：ON UPDATE CURRENT_TIMESTAMP 此前三层丢失（字段/解析/渲染）
    let new_table_sql = r#"
USE app;
CREATE TABLE base (
    id BIGINT NOT NULL AUTO_INCREMENT,
    PRIMARY KEY (`id`)
) ENGINE=InnoDB;

CREATE TABLE `audit_log` (
    `id` BIGINT NOT NULL AUTO_INCREMENT,
    `modified` DATETIME NOT NULL DEFAULT CURRENT_TIMESTAMP ON UPDATE CURRENT_TIMESTAMP COMMENT '更新时间',
    PRIMARY KEY (`id`)
) ENGINE=InnoDB;
    "#;
    let (diff_sql, _) =
        generate_schema_diff(Some(BASE_OLD), new_table_sql, Some("1.0.0"), "1.1.0").unwrap();
    assert!(
        diff_sql.contains("ON UPDATE CURRENT_TIMESTAMP"),
        "新表 CREATE 必须保留 ON UPDATE: {diff_sql}"
    );

    // 存量表：old 无 ON UPDATE、new 有 → 必须生成 MODIFY
    let old_no_ou = r#"
USE app;
CREATE TABLE `audit_log` (
    `id` BIGINT NOT NULL AUTO_INCREMENT,
    `modified` DATETIME NOT NULL DEFAULT CURRENT_TIMESTAMP,
    PRIMARY KEY (`id`)
) ENGINE=InnoDB;
    "#;
    let new_with_ou = r#"
USE app;
CREATE TABLE `audit_log` (
    `id` BIGINT NOT NULL AUTO_INCREMENT,
    `modified` DATETIME NOT NULL DEFAULT CURRENT_TIMESTAMP ON UPDATE CURRENT_TIMESTAMP,
    PRIMARY KEY (`id`)
) ENGINE=InnoDB;
    "#;
    let (diff_sql, _) =
        generate_schema_diff(Some(old_no_ou), new_with_ou, Some("1.0.0"), "1.1.0").unwrap();
    assert!(
        diff_sql.contains("ALTER TABLE `audit_log` MODIFY COLUMN `modified`")
            && diff_sql.contains("ON UPDATE CURRENT_TIMESTAMP"),
        "ON UPDATE 差异必须生成 MODIFY: {diff_sql}"
    );
}

#[test]
fn regression_on_update_now_synonym_no_false_diff() {
    // live 链路：SHOW CREATE 恒输出 CURRENT_TIMESTAMP，模板写 NOW() 语义相同，不应重复 MODIFY
    let old_sql = r#"
USE app;
CREATE TABLE `t` (
    `id` BIGINT NOT NULL AUTO_INCREMENT,
    `modified` DATETIME NOT NULL DEFAULT CURRENT_TIMESTAMP ON UPDATE CURRENT_TIMESTAMP,
    PRIMARY KEY (`id`)
) ENGINE=InnoDB;
    "#;
    let new_sql = r#"
USE app;
CREATE TABLE `t` (
    `id` BIGINT NOT NULL AUTO_INCREMENT,
    `modified` datetime NOT NULL DEFAULT CURRENT_TIMESTAMP ON UPDATE NOW(),
    PRIMARY KEY (`id`)
) ENGINE=InnoDB;
    "#;
    let (diff_sql, _) =
        generate_schema_diff(Some(old_sql), new_sql, Some("1.0.0"), "1.1.0").unwrap();
    assert!(
        !diff_sql.contains("MODIFY COLUMN"),
        "NOW() 与 CURRENT_TIMESTAMP 语义相同，不应产生 MODIFY: {diff_sql}"
    );
}

#[test]
fn regression_generated_column_preserved_and_normalized() {
    // 缺陷4：生成列此前降级为普通列，GENERATED ALWAYS AS (...) STORED 全丢
    let new_table_sql = r#"
USE app;
CREATE TABLE base (
    id BIGINT NOT NULL AUTO_INCREMENT,
    PRIMARY KEY (`id`)
) ENGINE=InnoDB;

CREATE TABLE `apply` (
    `id` BIGINT NOT NULL AUTO_INCREMENT,
    `project_id` BIGINT NOT NULL,
    `status` VARCHAR(16) NOT NULL,
    `pending_slot` BIGINT GENERATED ALWAYS AS (if((`status` = 'Pending'), `project_id`, NULL)) STORED,
    PRIMARY KEY (`id`),
    UNIQUE KEY `uk_pending_slot` (`pending_slot`)
) ENGINE=InnoDB;
    "#;
    let (diff_sql, _) =
        generate_schema_diff(Some(BASE_OLD), new_table_sql, Some("1.0.0"), "1.1.0").unwrap();
    assert!(
        diff_sql.contains("GENERATED ALWAYS AS (") && diff_sql.contains(") STORED"),
        "生成列必须渲染 GENERATED ALWAYS AS (...) STORED: {diff_sql}"
    );
    assert!(
        diff_sql.contains("UNIQUE KEY `uk_pending_slot`"),
        "生成列上的唯一索引名必须保留: {diff_sql}"
    );

    // 全写与 AS(...) 简写等价，不应产生 MODIFY
    let old_full = r#"
USE app;
CREATE TABLE `t` (
    `id` BIGINT NOT NULL AUTO_INCREMENT,
    `a` INT NOT NULL,
    `b` INT NOT NULL,
    `total` INT GENERATED ALWAYS AS (a + b) STORED,
    PRIMARY KEY (`id`)
) ENGINE=InnoDB;
    "#;
    let new_short = r#"
USE app;
CREATE TABLE `t` (
    `id` BIGINT NOT NULL AUTO_INCREMENT,
    `a` INT NOT NULL,
    `b` INT NOT NULL,
    `total` INT AS (a + b) STORED,
    PRIMARY KEY (`id`)
) ENGINE=InnoDB;
    "#;
    let (diff_sql, _) =
        generate_schema_diff(Some(old_full), new_short, Some("1.0.0"), "1.1.0").unwrap();
    assert!(
        !diff_sql.contains("MODIFY COLUMN"),
        "GENERATED ALWAYS AS 与 AS 简写等价，不应产生 MODIFY: {diff_sql}"
    );
}

#[test]
fn regression_integer_display_width_semantics() {
    // 缺陷5：tinyint(1) 宽度必须保留；int 与 int(11) 语义等价不应 MODIFY
    let new_table_sql = r#"
USE app;
CREATE TABLE base (
    id BIGINT NOT NULL AUTO_INCREMENT,
    PRIMARY KEY (`id`)
) ENGINE=InnoDB;

CREATE TABLE `flag` (
    `id` BIGINT NOT NULL AUTO_INCREMENT,
    `enabled` TINYINT(1) NOT NULL DEFAULT '1',
    PRIMARY KEY (`id`)
) ENGINE=InnoDB;
    "#;
    let (diff_sql, _) =
        generate_schema_diff(Some(BASE_OLD), new_table_sql, Some("1.0.0"), "1.1.0").unwrap();
    assert!(
        diff_sql.contains("`enabled` TINYINT(1) NOT NULL DEFAULT '1'"),
        "tinyint(1) 宽度必须保留（布尔语义）: {diff_sql}"
    );

    // tinyint(1) vs tinyint → 语义不同，必须 MODIFY
    let old_ti = r#"
USE app;
CREATE TABLE `t` (`enabled` TINYINT(1) NOT NULL DEFAULT '1') ENGINE=InnoDB;
    "#;
    let new_ti = r#"
USE app;
CREATE TABLE `t` (`enabled` TINYINT NOT NULL DEFAULT '1') ENGINE=InnoDB;
    "#;
    let (diff_sql, _) = generate_schema_diff(Some(old_ti), new_ti, Some("1.0.0"), "1.1.0").unwrap();
    assert!(
        diff_sql.contains("MODIFY COLUMN"),
        "tinyint(1) ≠ tinyint 应生成 MODIFY: {diff_sql}"
    );

    // int vs int(11) → 宽度已弃用，语义相同，不应 MODIFY
    let old_int = r#"
USE app;
CREATE TABLE `t` (`count` INT NOT NULL) ENGINE=InnoDB;
    "#;
    let new_int = r#"
USE app;
CREATE TABLE `t` (`count` INT(11) NOT NULL) ENGINE=InnoDB;
    "#;
    let (diff_sql, _) =
        generate_schema_diff(Some(old_int), new_int, Some("1.0.0"), "1.1.0").unwrap();
    assert!(
        !diff_sql.contains("MODIFY COLUMN"),
        "int 与 int(11) 宽度已弃用，不应产生 MODIFY: {diff_sql}"
    );
}

#[test]
fn regression_table_options_preserved_and_filtered() {
    // 缺陷6：新表必须携带 ENGINE/CHARSET/COLLATE；AUTO_INCREMENT dump 计数器必须过滤
    let new_table_sql = r#"
USE app;
CREATE TABLE base (
    id BIGINT NOT NULL AUTO_INCREMENT,
    PRIMARY KEY (`id`)
) ENGINE=InnoDB;

CREATE TABLE `with_options` (
    `id` BIGINT NOT NULL AUTO_INCREMENT,
    PRIMARY KEY (`id`)
) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4 COLLATE=utf8mb4_0900_ai_ci AUTO_INCREMENT=5;
    "#;
    let (diff_sql, _) =
        generate_schema_diff(Some(BASE_OLD), new_table_sql, Some("1.0.0"), "1.1.0").unwrap();
    assert!(
        diff_sql.contains(") ENGINE=InnoDB DEFAULT CHARSET=utf8mb4 COLLATE=utf8mb4_0900_ai_ci;"),
        "新表 CREATE 必须携带表选项: {diff_sql}"
    );
    assert!(
        !diff_sql.contains("AUTO_INCREMENT=5"),
        "AUTO_INCREMENT dump 计数器不属于 schema，必须过滤: {diff_sql}"
    );
}

#[test]
fn regression_table_option_drift_warns_without_sql() {
    // 表选项漂移：双方显式才比较；只警告不生成 SQL；单侧省略不产生任何输出
    let old_sql = r#"
USE app;
CREATE TABLE `t` (
    `id` BIGINT NOT NULL AUTO_INCREMENT,
    PRIMARY KEY (`id`)
) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4 COLLATE=utf8mb4_unicode_ci;
    "#;
    let new_sql = r#"
USE app;
CREATE TABLE `t` (
    `id` BIGINT NOT NULL AUTO_INCREMENT,
    PRIMARY KEY (`id`)
) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4 COLLATE=utf8mb4_0900_ai_ci;
    "#;
    let (diff_sql, description) =
        generate_schema_diff(Some(old_sql), new_sql, Some("1.0.0"), "1.1.0").unwrap();
    assert!(
        diff_sql.contains("table option COLLATION differs"),
        "显式声明的 collation 漂移必须给出警告: {diff_sql}"
    );
    assert!(
        description.contains("manual-change warnings"),
        "{description}"
    );
    let (_, stats) = super::differ::generate_mysql_diff(
        &parse_sql_tables(old_sql).unwrap(),
        &parse_sql_tables(new_sql).unwrap(),
    )
    .unwrap();
    assert_eq!(stats.table_options_changed, 1);
    assert!(stats.has_changes());
    assert!(stats.has_warnings());
    assert!(!stats.has_executable_operations());
    assert!(
        !diff_sql.contains("ALTER TABLE"),
        "表选项变更不自动生成 SQL（需人工 CONVERT TO）: {diff_sql}"
    );

    // 单侧省略 COLLATE：不比较、无警告
    let new_omitted = r#"
USE app;
CREATE TABLE `t` (
    `id` BIGINT NOT NULL AUTO_INCREMENT,
    PRIMARY KEY (`id`)
) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4;
    "#;
    let (diff_sql, _) =
        generate_schema_diff(Some(old_sql), new_omitted, Some("1.0.0"), "1.1.0").unwrap();
    assert!(
        diff_sql.trim().is_empty(),
        "模板侧省略 COLLATE 视为依赖默认值，不应产生任何输出: {diff_sql}"
    );
}

#[test]
fn regression_show_create_form_equivalent_to_template() {
    // live 部署链路验收：SHOW CREATE TABLE 形态 vs 手写模板形态必须零差异。
    // 覆盖：反引号、小写类型、无宽度 int、ON UPDATE 同义词、生成列的
    // 反引号/双层括号/字符集引导符、UNIQUE KEY vs CONSTRAINT ... UNIQUE、表选项。
    let show_create_form = r#"
USE app;
CREATE TABLE `order_ext` (
  `id` bigint NOT NULL AUTO_INCREMENT,
  `amount` int DEFAULT NULL,
  `flag` tinyint(1) NOT NULL DEFAULT '0',
  `full_name` varchar(255) GENERATED ALWAYS AS (concat(`first_name`, _utf8mb4' ', `last_name`)) STORED,
  `first_name` varchar(64) NOT NULL,
  `last_name` varchar(64) NOT NULL,
  `modified` datetime NOT NULL DEFAULT CURRENT_TIMESTAMP ON UPDATE CURRENT_TIMESTAMP,
  `status` enum('Pending','Active') NOT NULL DEFAULT 'Pending',
  PRIMARY KEY (`id`),
  UNIQUE KEY `uk_name` (`first_name`,`last_name`)
) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4 COLLATE=utf8mb4_0900_ai_ci;
    "#;
    let template_form = r#"
USE app;
CREATE TABLE order_ext (
    id BIGINT NOT NULL AUTO_INCREMENT,
    amount INT(11) NULL,
    flag TINYINT(1) NOT NULL DEFAULT '0',
    full_name VARCHAR(255) AS (concat(first_name, ' ', last_name)) STORED,
    first_name VARCHAR(64) NOT NULL,
    last_name VARCHAR(64) NOT NULL,
    modified DATETIME NOT NULL DEFAULT CURRENT_TIMESTAMP ON UPDATE NOW(),
    status ENUM('Pending','Active') NOT NULL DEFAULT 'Pending',
    PRIMARY KEY (id),
    CONSTRAINT uk_name UNIQUE (first_name, last_name)
) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4 COLLATE=utf8mb4_0900_ai_ci;
    "#;
    let (diff_sql, _) = generate_schema_diff(
        Some(show_create_form),
        template_form,
        Some("1.0.0"),
        "1.1.0",
    )
    .unwrap();
    assert!(
        diff_sql.trim().is_empty(),
        "SHOW CREATE 形态与模板形态语义等价，diff 必须为空:\n{diff_sql}"
    );
}

#[test]
fn regression_default_value_case_change_detected() {
    // 预存缺陷修复：默认值引号内字符串此前被整体大写比较，'Pending' ≡ 'pending' 漏检
    let old_sql = r#"
USE app;
CREATE TABLE `t` (
    `id` BIGINT NOT NULL AUTO_INCREMENT,
    `status` ENUM('Pending','Active') NOT NULL DEFAULT 'Pending',
    PRIMARY KEY (`id`)
) ENGINE=InnoDB;
    "#;
    let new_sql = r#"
USE app;
CREATE TABLE `t` (
    `id` BIGINT NOT NULL AUTO_INCREMENT,
    `status` ENUM('Pending','Active') NOT NULL DEFAULT 'pending',
    PRIMARY KEY (`id`)
) ENGINE=InnoDB;
    "#;
    let (diff_sql, _) =
        generate_schema_diff(Some(old_sql), new_sql, Some("1.0.0"), "1.1.0").unwrap();
    assert!(
        diff_sql.contains("MODIFY COLUMN") && diff_sql.contains("DEFAULT 'pending'"),
        "默认值大小写变化必须被检出: {diff_sql}"
    );
}

#[test]
fn regression_unsigned_integer_width_normalization() {
    // MySQL 5.7 风格 dump（int(10) unsigned 带宽度）vs 8.0 SHOW CREATE / 手写模板
    // （int unsigned 无宽度）必须语义等价，不产生虚假 MODIFY。
    // 此前宽度剥离把后缀直接拼接，INT(11) UNSIGNED 归一化成 INTUNSIGNED 导致误报。
    let old_sql = r#"
USE app;
CREATE TABLE `t` (`count` INT(10) UNSIGNED NOT NULL DEFAULT '0') ENGINE=InnoDB;
    "#;
    let new_sql = r#"
USE app;
CREATE TABLE `t` (`count` int unsigned NOT NULL DEFAULT '0') ENGINE=InnoDB;
    "#;
    let (diff_sql, _) =
        generate_schema_diff(Some(old_sql), new_sql, Some("1.0.0"), "1.1.0").unwrap();
    assert!(
        !diff_sql.contains("MODIFY COLUMN"),
        "int(10) unsigned 与 int unsigned 语义等价，不应产生 MODIFY: {diff_sql}"
    );

    // 宽度剥离不能把不同类型误判相等：int unsigned -> bigint unsigned 必须检出
    let new_bigint = r#"
USE app;
CREATE TABLE `t` (`count` bigint unsigned NOT NULL DEFAULT '0') ENGINE=InnoDB;
    "#;
    let (diff_sql, _) =
        generate_schema_diff(Some(old_sql), new_bigint, Some("1.0.0"), "1.1.0").unwrap();
    assert!(
        diff_sql.contains("MODIFY COLUMN"),
        "int 与 bigint 类型变化必须检出: {diff_sql}"
    );
}
