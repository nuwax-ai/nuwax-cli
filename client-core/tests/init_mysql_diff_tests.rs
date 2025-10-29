// 专门测试 init_mysql_old.sql 和 init_mysql_new.sql 差异的测试用例
// 验证 SQL 解析器和差异生成功能的正确性

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
fn test_parse_old_mysql_tables() {
    println!("🔍 测试解析 init_mysql_old.sql");

    let old_sql = read_fixture_file("init_mysql_old.sql");

    // 测试解析功能
    let result = parse_sql_tables(&old_sql);
    assert!(result.is_ok(), "解析旧SQL文件失败: {:?}", result.err());

    let tables = result.unwrap();
    println!("✅ 成功解析 {} 个表", tables.len());

    // 验证一些预期的表是否存在
    assert!(
        tables.contains_key("agent_config"),
        "应该包含 agent_config 表"
    );
    assert!(
        tables.contains_key("model_config"),
        "应该包含 model_config 表"
    );

    // 打印所有表名
    println!("📋 解析到的表:");
    for table_name in tables.keys() {
        println!("  - {}", table_name);
    }
}

#[test]
fn test_parse_new_mysql_tables() {
    println!("🔍 测试解析 init_mysql_new.sql");

    let new_sql = read_fixture_file("init_mysql_new.sql");

    // 测试解析功能
    let result = parse_sql_tables(&new_sql);
    assert!(result.is_ok(), "解析新SQL文件失败: {:?}", result.err());

    let tables = result.unwrap();
    println!("✅ 成功解析 {} 个表", tables.len());

    // 验证新表是否存在
    assert!(
        tables.contains_key("custom_page_config"),
        "应该包含 custom_page_config 表"
    );
    assert!(
        tables.contains_key("custom_page_conversation"),
        "应该包含 custom_page_conversation 表"
    );
    assert!(
        tables.contains_key("custom_page_build"),
        "应该包含 custom_page_build 表"
    );

    // 打印所有表名
    println!("📋 解析到的表:");
    for table_name in tables.keys() {
        println!("  - {}", table_name);
    }
}

#[test]
fn test_generate_mysql_diff_sql() {
    println!("🚀 测试生成 MySQL 差异 SQL");

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

    println!("📊 生成的差异SQL统计:");
    println!("总行数: {}", diff_sql.lines().count());
    println!(
        "CREATE TABLE 数量: {}",
        diff_sql.matches("CREATE TABLE").len()
    );
    println!(
        "ALTER TABLE 数量: {}",
        diff_sql.matches("ALTER TABLE").len()
    );

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
fn test_custom_page_config_table_structure() {
    println!("🔍 验证 custom_page_config 表结构解析");

    let new_sql = read_fixture_file("init_mysql_new.sql");
    let tables = parse_sql_tables(&new_sql).expect("解析新SQL文件失败");

    let custom_page_config = tables
        .get("custom_page_config")
        .expect("应该找到 custom_page_config 表");

    println!("📋 custom_page_config 表结构:");
    println!("  列数量: {}", custom_page_config.columns.len());
    println!("  索引数量: {}", custom_page_config.indexes.len());

    // 验证关键列是否存在
    let column_names: Vec<&str> = custom_page_config
        .columns
        .iter()
        .map(|col| col.name.as_str())
        .collect();

    assert!(column_names.contains(&"id"), "应该包含 id 列");
    assert!(column_names.contains(&"name"), "应该包含 name 列");
    assert!(column_names.contains(&"base_path"), "应该包含 base_path 列");
    assert!(
        column_names.contains(&"publish_type"),
        "应该包含 publish_type 列"
    );
    assert!(
        column_names.contains(&"project_type"),
        "应该包含 project_type 列"
    );
    assert!(column_names.contains(&"created"), "应该包含 created 列");
    assert!(column_names.contains(&"modified"), "应该包含 modified 列");

    // 验证数据类型
    for column in &custom_page_config.columns {
        match column.name.as_str() {
            "publish_type" | "project_type" => {
                assert!(
                    column.data_type.starts_with("ENUM"),
                    "{} 应该是 ENUM 类型，实际: {}",
                    column.name,
                    column.data_type
                );
            }
            "created" | "modified" => {
                assert_eq!(
                    column.data_type, "DATETIME",
                    "{} 应该是 DATETIME 类型，实际: {}",
                    column.name, column.data_type
                );
            }
            _ => {}
        }
    }

    // 打印所有列信息
    println!("📝 列详情:");
    for column in &custom_page_config.columns {
        let nullable = if column.nullable { "NULL" } else { "NOT NULL" };
        let default = column
            .default_value
            .as_ref()
            .map(|d| format!(" DEFAULT {}", d))
            .unwrap_or_default();
        let comment = column
            .comment
            .as_ref()
            .map(|c| format!(" COMMENT '{}'", c))
            .unwrap_or_default();

        println!(
            "  - {} {}{}{}{}",
            column.name, column.data_type, nullable, default, comment
        );
    }
}

#[test]
fn test_current_timestamp_formatting() {
    println!("🕐 验证 CURRENT_TIMESTAMP 格式化");

    let new_sql = read_fixture_file("init_mysql_new.sql");
    let tables = parse_sql_tables(&new_sql).expect("解析新SQL文件失败");

    // 检查 custom_page_config 表的 created 和 modified 列
    let custom_page_config = tables
        .get("custom_page_config")
        .expect("应该找到 custom_page_config 表");

    for column in &custom_page_config.columns {
        if column.name == "created" || column.name == "modified" {
            if let Some(default_value) = &column.default_value {
                // 验证 CURRENT_TIMESTAMP 没有引号
                assert_eq!(
                    default_value, "CURRENT_TIMESTAMP",
                    "{} 的默认值应该是 'CURRENT_TIMESTAMP'，实际: '{}'",
                    column.name, default_value
                );
                println!("✅ {} 列的默认值格式正确: {}", column.name, default_value);
            } else {
                panic!("{} 列应该有默认值", column.name);
            }
        }
    }
}

#[test]
fn test_enum_type_formatting() {
    println!("🔗 验证 ENUM 类型格式化");

    let new_sql = read_fixture_file("init_mysql_new.sql");
    let tables = parse_sql_tables(&new_sql).expect("解析新SQL文件失败");

    let custom_page_config = tables
        .get("custom_page_config")
        .expect("应该找到 custom_page_config 表");

    for column in &custom_page_config.columns {
        match column.name.as_str() {
            "publish_type" => {
                assert!(
                    column.data_type.contains("ENUM"),
                    "publish_type 应该是 ENUM 类型"
                );
                assert!(
                    column.data_type.contains("'AGENT'"),
                    "publish_type 应该包含 AGENT 选项"
                );
                assert!(
                    column.data_type.contains("'PAGE'"),
                    "publish_type 应该包含 PAGE 选项"
                );
                println!("✅ publish_type 类型格式正确: {}", column.data_type);
            }
            "project_type" => {
                assert!(
                    column.data_type.contains("ENUM"),
                    "project_type 应该是 ENUM 类型"
                );
                assert!(
                    column.data_type.contains("'ONLINE_DEPLOY'"),
                    "project_type 应该包含 ONLINE_DEPLOY 选项"
                );
                assert!(
                    column.data_type.contains("'REVERSE_PROXY'"),
                    "project_type 应该包含 REVERSE_PROXY 选项"
                );
                println!("✅ project_type 类型格式正确: {}", column.data_type);
            }
            _ => {}
        }
    }
}

#[test]
fn test_agent_config_table_changes() {
    println!("🔍 验证 agent_config 表变更");

    let old_sql = read_fixture_file("init_mysql_old.sql");
    let new_sql = read_fixture_file("init_mysql_new.sql");

    let old_tables = parse_sql_tables(&old_sql).expect("解析旧SQL文件失败");
    let new_tables = parse_sql_tables(&new_sql).expect("解析新SQL文件失败");

    let old_agent_config = old_tables
        .get("agent_config")
        .expect("旧SQL应该包含 agent_config 表");
    let new_agent_config = new_tables
        .get("agent_config")
        .expect("新SQL应该包含 agent_config 表");

    // 验证新增的列
    let old_column_names: std::collections::HashSet<&str> = old_agent_config
        .columns
        .iter()
        .map(|col| col.name.as_str())
        .collect();
    let new_column_names: std::collections::HashSet<&str> = new_agent_config
        .columns
        .iter()
        .map(|col| col.name.as_str())
        .collect();

    let added_columns: Vec<&str> = new_column_names
        .difference(&old_column_names)
        .cloned()
        .collect();

    assert!(!added_columns.is_empty(), "应该有新增的列");

    println!("📝 agent_config 表新增的列:");
    for column in &added_columns {
        println!("  - {}", column);
    }

    // 验证特定的预期新列
    assert!(added_columns.contains(&"type"), "应该新增 type 列");
    assert!(
        added_columns.contains(&"hide_chat_area"),
        "应该新增 hide_chat_area 列"
    );
    assert!(
        added_columns.contains(&"expand_page_area"),
        "应该新增 expand_page_area 列"
    );
}

#[test]
fn test_generate_complete_migration_sql() {
    println!("🎯 生成完整的迁移SQL并验证语法");

    let old_sql = read_fixture_file("init_mysql_old.sql");
    let new_sql = read_fixture_file("init_mysql_new.sql");

    let (diff_sql, description) =
        generate_schema_diff(Some(&old_sql), &new_sql, Some("v1.0.0"), "v2.0.0")
            .expect("生成差异SQL失败");

    println!("📊 差异描述: {}", description);

    // 验证SQL语法正确性（基本检查）
    assert!(!diff_sql.is_empty(), "差异SQL不应为空");

    // 验证不包含格式错误
    assert!(
        !diff_sql.contains("Enum([Name("),
        "不应该包含 Rust 枚举格式"
    );
    assert!(
        !diff_sql.contains("'CURRENT_TIMESTAMP'"),
        "CURRENT_TIMESTAMP 不应该有引号"
    );

    // 验证包含预期的表
    let expected_tables = vec![
        "custom_page_config",
        "custom_page_conversation",
        "custom_page_build",
    ];

    for table in &expected_tables {
        assert!(
            diff_sql.contains(&format!("CREATE TABLE `{}`", table)),
            "应该包含创建 {} 表的语句",
            table
        );
    }

    // 验证包含ALTER TABLE语句
    assert!(
        diff_sql.contains("ALTER TABLE `agent_config`"),
        "应该包含修改 agent_config 表"
    );
    assert!(
        diff_sql.contains("ALTER TABLE `model_config`"),
        "应该包含修改 model_config 表"
    );

    println!("✅ 迁移SQL语法验证通过");

    // 统计SQL语句数量
    let create_count = diff_sql.matches("CREATE TABLE").len();
    let alter_count = diff_sql.matches("ALTER TABLE").len();
    let add_column_count = diff_sql.matches("ADD COLUMN").len();

    println!("📈 SQL统计:");
    println!("  CREATE TABLE: {} 个", create_count);
    println!("  ALTER TABLE: {} 个", alter_count);
    println!("  ADD COLUMN: {} 个", add_column_count);

    // 保存到临时文件供手动检查（可选）
    if std::env::var("SAVE_DIFF_SQL").is_ok() {
        let temp_path = Path::new("/tmp/test_migration.sql");
        fs::write(temp_path, &diff_sql).expect("写入临时文件失败");
        println!("💾 完整的迁移SQL已保存到: {:?}", temp_path);
    }
}

#[test]
fn run_comprehensive_mysql_diff_test() {
    println!("🚀 运行完整的MySQL差异测试套件");
    println!("{}", "=".repeat(80));

    // 运行所有相关测试
    test_parse_old_mysql_tables();
    println!();

    test_parse_new_mysql_tables();
    println!();

    test_generate_mysql_diff_sql();
    println!();

    test_custom_page_config_table_structure();
    println!();

    test_current_timestamp_formatting();
    println!();

    test_enum_type_formatting();
    println!();

    test_agent_config_table_changes();
    println!();

    test_generate_complete_migration_sql();

    println!("{}", "=".repeat(80));
    println!("🎉 所有MySQL差异测试通过！");
}
