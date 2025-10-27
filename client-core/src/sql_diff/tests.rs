use super::parser::parse_sql_tables;
use super::*;

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
    assert!(tables.get("should_be_ignored").is_none());
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
    assert!(description.contains("新增表"));
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
    assert!(description.contains("删除表"));
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
    assert!(description.contains("无变化"));
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

    let (diff_sql, description) =
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
