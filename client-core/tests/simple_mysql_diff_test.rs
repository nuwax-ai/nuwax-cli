// 简化的 MySQL 差异测试，专注于验证核心功能

use client_core::sql_diff::generate_schema_diff;
use std::fs;
use std::path::Path;

/// 读取指定的 fixture 文件
fn read_fixture_file(filename: &str) -> String {
    let project_root = std::env::var("CARGO_MANIFEST_DIR").unwrap_or_else(|_| ".".to_string());
    let fixture_path = Path::new(&project_root).join("fixtures").join(filename);
    fs::read_to_string(&fixture_path)
        .unwrap_or_else(|e| panic!("无法读取文件: {:?}, 错误: {}", fixture_path, e))
}

#[test]
fn test_generate_mysql_diff_sql_basic() {
    println!("🚀 测试生成 MySQL 差异 SQL - 基础功能");

    let old_sql = read_fixture_file("init_mysql_old.sql");
    let new_sql = read_fixture_file("init_mysql_new.sql");

    // 生成差异SQL
    let start_time = std::time::Instant::now();
    let result = generate_schema_diff(Some(&old_sql), &new_sql, Some("1.0.0"), "2.0.0");
    let duration = start_time.elapsed();

    assert!(result.is_ok(), "生成差异SQL失败: {:?}", result.err());

    let (diff_sql, description) = result.unwrap();

    println!("✅ 差异生成完成，耗时: {:?}", duration);
    println!("📝 描述: {}", description);

    // 验证差异SQL不为空
    assert!(!diff_sql.trim().is_empty(), "差异SQL不应该为空");

    // 验证包含预期的变更
    assert!(
        diff_sql.contains("CREATE TABLE `custom_page_config`"),
        "应该包含创建 custom_page_config 表"
    );
    assert!(
        diff_sql.contains("CREATE TABLE `custom_page_conversation`"),
        "应该包含创建 custom_page_conversation 表"
    );
    assert!(
        diff_sql.contains("CREATE TABLE `custom_page_build`"),
        "应该包含创建 custom_page_build 表"
    );

    // 验证包含ALTER TABLE语句
    assert!(
        diff_sql.contains("ALTER TABLE `agent_config`"),
        "应该包含修改 agent_config 表"
    );
    assert!(
        diff_sql.contains("ALTER TABLE `model_config`"),
        "应该包含修改 model_config 表"
    );

    // 验证不包含格式错误
    assert!(
        !diff_sql.contains("Enum([Name("),
        "不应该包含 Rust 枚举格式"
    );
    assert!(
        !diff_sql.contains("'CURRENT_TIMESTAMP'"),
        "CURRENT_TIMESTAMP 不应该有引号"
    );

    println!("✅ 基础验证通过");

    // 打印生成的SQL（截取前1000字符避免输出过长）
    let sql_preview = if diff_sql.len() > 1000 {
        format!("{}...(截取前1000字符)", &diff_sql[..1000])
    } else {
        diff_sql.clone()
    };

    println!("\n📄 生成的差异SQL预览:");
    println!("{}", "=".repeat(80));
    println!("{}", sql_preview);
    println!("{}", "=".repeat(80));
}

#[test]
fn test_specific_table_structures() {
    println!("🔍 测试特定表结构的正确性");

    let old_sql = read_fixture_file("init_mysql_old.sql");
    let new_sql = read_fixture_file("init_mysql_new.sql");

    let (diff_sql, _) = generate_schema_diff(Some(&old_sql), &new_sql, Some("v1.0.0"), "v2.0.0")
        .expect("生成差异SQL失败");

    // 验证 custom_page_config 表包含正确的列定义
    assert!(
        diff_sql.contains("`publish_type`"),
        "应该包含 publish_type 列"
    );
    assert!(
        diff_sql.contains("`project_type`"),
        "应该包含 project_type 列"
    );
    assert!(
        diff_sql.contains("`created` DATETIME NOT NULL DEFAULT CURRENT_TIMESTAMP"),
        "应该包含正确的 created 列定义"
    );
    assert!(
        diff_sql.contains("`modified` DATETIME DEFAULT CURRENT_TIMESTAMP"),
        "应该包含正确的 modified 列定义"
    );

    // 验证 ENUM 类型格式正确
    assert!(
        diff_sql.contains("ENUM('AGENT', 'PAGE')"),
        "应该包含正确的 ENUM 格式"
    );
    assert!(
        diff_sql.contains("ENUM('ONLINE_DEPLOY', 'REVERSE_PROXY')"),
        "应该包含正确的 project_type ENUM 格式"
    );

    // 验证新表的列定义
    assert!(
        diff_sql.contains("`id` BIGINT NOT NULL AUTO_INCREMENT"),
        "应该包含正确的 id 列"
    );
    assert!(
        diff_sql.contains("`name` VARCHAR(255) NOT NULL"),
        "应该包含正确的 name 列"
    );
    assert!(
        diff_sql.contains("`base_path` VARCHAR(255) NOT NULL"),
        "应该包含正确的 base_path 列"
    );

    println!("✅ 表结构验证通过");
}

#[test]
fn test_agent_config_changes() {
    println!("📋 测试 agent_config 表变更");

    let old_sql = read_fixture_file("init_mysql_old.sql");
    let new_sql = read_fixture_file("init_mysql_new.sql");

    let (diff_sql, _) = generate_schema_diff(Some(&old_sql), &new_sql, Some("old"), "new")
        .expect("生成差异SQL失败");

    // 验证 agent_config 表的新增列
    assert!(
        diff_sql.contains("ALTER TABLE `agent_config`"),
        "应该包含修改 agent_config 表"
    );
    assert!(
        diff_sql.contains("`type` VARCHAR(32) NOT NULL DEFAULT 'ChatBot'"),
        "应该包含 type 列"
    );
    assert!(
        diff_sql.contains("`hide_chat_area` TINYINT NOT NULL DEFAULT '0'"),
        "应该包含 hide_chat_area 列"
    );
    assert!(
        diff_sql.contains("`expand_page_area` TINYINT NOT NULL DEFAULT '0'"),
        "应该包含 expand_page_area 列"
    );

    // 验证 model_config 表的新增列
    assert!(
        diff_sql.contains("ALTER TABLE `model_config`"),
        "应该包含修改 model_config 表"
    );
    assert!(
        diff_sql.contains("`enabled` TINYINT COMMENT '启用状态'"),
        "应该包含 enabled 列"
    );

    println!("✅ agent_config 表变更验证通过");
}

#[test]
fn test_sql_syntax_validation() {
    println!("🔧 测试 SQL 语法正确性");

    let old_sql = read_fixture_file("init_mysql_old.sql");
    let new_sql = read_fixture_file("init_mysql_new.sql");

    let (diff_sql, _) = generate_schema_diff(Some(&old_sql), &new_sql, Some("baseline"), "target")
        .expect("生成差异SQL失败");

    // 基础语法检查
    assert!(
        !diff_sql.contains("Enum([Name("),
        "不应该包含 Rust 枚举格式"
    );
    assert!(
        !diff_sql.contains("'CURRENT_TIMESTAMP'"),
        "CURRENT_TIMESTAMP 不应该有引号"
    );

    // 验证正确的 SQL 关键字格式
    assert!(
        diff_sql.contains("CREATE TABLE"),
        "应该包含正确的 CREATE TABLE 语句"
    );
    assert!(
        diff_sql.contains("ALTER TABLE"),
        "应该包含正确的 ALTER TABLE 语句"
    );
    assert!(
        diff_sql.contains("ADD COLUMN"),
        "应该包含正确的 ADD COLUMN 语句"
    );

    // 验证数据类型格式正确
    assert!(diff_sql.contains("ENUM("), "ENUM 类型应该格式正确");
    assert!(
        diff_sql.contains("DEFAULT CURRENT_TIMESTAMP"),
        "CURRENT_TIMESTAMP 默认值应该格式正确"
    );

    // 验证引号使用正确
    assert!(diff_sql.contains("COMMENT '"), "注释应该使用单引号");
    assert!(diff_sql.contains("DEFAULT '"), "字符串默认值应该使用单引号");

    println!("✅ SQL 语法验证通过");
}

#[test]
fn test_complete_migration_validation() {
    println!("🎯 完整迁移验证");

    let old_sql = read_fixture_file("init_mysql_old.sql");
    let new_sql = read_fixture_file("init_mysql_new.sql");

    let (diff_sql, description) = generate_schema_diff(
        Some(&old_sql),
        &new_sql,
        Some("production-v1.0.0"),
        "development-v2.0.0",
    )
    .expect("生成差异SQL失败");

    println!("📊 差异描述: {}", description);

    // 统计SQL语句
    let lines: Vec<&str> = diff_sql.lines().collect();
    let mut create_count = 0;
    let mut alter_count = 0;
    let mut add_column_count = 0;

    for line in &lines {
        let trimmed = line.trim();
        if trimmed.starts_with("CREATE TABLE") {
            create_count += 1;
        } else if trimmed.starts_with("ALTER TABLE") {
            alter_count += 1;
        } else if trimmed.contains("ADD COLUMN") {
            add_column_count += 1;
        }
    }

    println!("📈 SQL统计:");
    println!("  CREATE TABLE: {} 个", create_count);
    println!("  ALTER TABLE: {} 个", alter_count);
    println!("  ADD COLUMN: {} 个", add_column_count);
    println!("  总行数: {}", lines.len());

    // 基本验证
    assert!(create_count >= 3, "应该至少创建3个新表");
    assert!(alter_count >= 2, "应该至少修改2个表");
    assert!(add_column_count >= 4, "应该至少添加4个列");

    // 验证关键变更存在
    assert!(
        diff_sql.contains("custom_page_config"),
        "应该包含 custom_page_config 相关变更"
    );
    assert!(
        diff_sql.contains("custom_page_conversation"),
        "应该包含 custom_page_conversation 相关变更"
    );
    assert!(
        diff_sql.contains("custom_page_build"),
        "应该包含 custom_page_build 相关变更"
    );

    // 保存到临时文件供手动检查
    if std::env::var("SAVE_DIFF_SQL").is_ok() {
        let temp_path = Path::new("/tmp/test_mysql_migration.sql");
        fs::write(temp_path, &diff_sql).expect("写入临时文件失败");
        println!("💾 完整的迁移SQL已保存到: {:?}", temp_path);
    }

    println!("✅ 完整迁移验证通过");
}

#[test]
fn run_all_mysql_diff_tests() {
    println!("🚀 运行所有MySQL差异测试");
    println!("{}", "=".repeat(80));

    test_generate_mysql_diff_sql_basic();
    println!();

    test_specific_table_structures();
    println!();

    test_agent_config_changes();
    println!();

    test_sql_syntax_validation();
    println!();

    test_complete_migration_validation();

    println!("{}", "=".repeat(80));
    println!("🎉 所有MySQL差异测试通过！");

    // 输出最终结果摘要
    println!("\n📋 测试结果摘要:");
    println!("✅ SQL 解析功能正常");
    println!("✅ 差异生成功能正常");
    println!("✅ ENUM 类型格式化正确");
    println!("✅ CURRENT_TIMESTAMP 格式化正确");
    println!("✅ ALTER TABLE 语句正确");
    println!("✅ CREATE TABLE 语句正确");
    println!("✅ 整体迁移SQL语法正确");
}
