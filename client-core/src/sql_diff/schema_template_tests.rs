//! 多库 schema 模板测试：`parse_schema_template`（解析/Fail Fast 语义）与
//! `assemble_multi_db_diff`（多库组装/段渲染规则）。

use super::generator::assemble_multi_db_diff;
use super::parser::parse_schema_template;
use super::types::{DbSectionResult, DiffStats};

const STICKER_TARGET: &str = "USE nuwax_im;

CREATE TABLE `sticker` (
  `id` bigint NOT NULL,
  `url` varchar(1024) NOT NULL,
  PRIMARY KEY (`id`),
  UNIQUE KEY `uk_user_url` (`id`,`url`(255))
) ENGINE=InnoDB;
";

const PLATFORM_TEMPLATE: &str = "-- 平台主库
CREATE DATABASE IF NOT EXISTS agent_platform CHARACTER SET utf8mb4 COLLATE utf8mb4_unicode_ci;
CREATE DATABASE IF NOT EXISTS agent_custom_table CHARACTER SET utf8mb4 COLLATE utf8mb4_unicode_ci;
GRANT ALL PRIVILEGES ON agent_platform.* TO 'agent_platform'@'%';
GRANT ALL PRIVILEGES ON agent_custom_table.* TO 'agent_platform'@'%';
FLUSH PRIVILEGES;

USE agent_platform;

CREATE TABLE `users` (
  `id` bigint NOT NULL AUTO_INCREMENT,
  PRIMARY KEY (`id`)
) ENGINE=InnoDB;
";

const IM_TEMPLATE: &str = "CREATE DATABASE IF NOT EXISTS `nuwax_im` DEFAULT CHARACTER SET utf8mb4 COLLATE utf8mb4_0900_ai_ci;
USE `nuwax_im`;

CREATE TABLE IF NOT EXISTS `im_agent_binding` (
  `id` bigint NOT NULL,
  PRIMARY KEY (`id`)
) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4 COLLATE=utf8mb4_0900_ai_ci;
";

fn platform_template() -> super::types::SchemaTemplate {
    parse_schema_template(PLATFORM_TEMPLATE).expect("platform template must parse")
}

fn im_template() -> super::types::SchemaTemplate {
    parse_schema_template(IM_TEMPLATE).expect("im template must parse")
}

#[test]
fn parse_schema_template_extracts_database_preamble_and_tables() {
    let template = platform_template();
    assert_eq!(template.database, "agent_platform");
    // 2 CREATE DATABASE + 2 GRANT + 1 FLUSH = 5，按原文件顺序透传
    assert_eq!(template.preamble_stmts.len(), 5);
    assert!(template.preamble_stmts[0].starts_with("CREATE DATABASE IF NOT EXISTS agent_platform"));
    assert!(template.preamble_stmts[3].starts_with("GRANT ALL PRIVILEGES ON agent_custom_table"));
    assert_eq!(template.preamble_stmts[4], "FLUSH PRIVILEGES;");
    assert!(template.tables.contains_key("users"));
}

#[test]
fn parse_schema_template_accepts_backticked_use() {
    let template = im_template();
    assert_eq!(template.database, "nuwax_im");
    assert_eq!(template.preamble_stmts.len(), 1);
    assert!(template.tables.contains_key("im_agent_binding"));
}

#[test]
fn parse_schema_template_requires_exactly_one_use() {
    // 无 USE：CREATE TABLE 无法归属库
    let no_use = "CREATE TABLE users (id INT);";
    assert!(parse_schema_template(no_use).is_err());

    // 多个 USE：一个模板文件只允许一个库
    let two_use = "USE db_a;\nCREATE TABLE a (id INT);\nUSE db_b;\nCREATE TABLE b (id INT);";
    assert!(parse_schema_template(two_use).is_err());
}

#[test]
fn parse_schema_template_rejects_data_only_and_tableless_files() {
    // 纯数据文件（init_mysql_data.sql 形态）误入清单时在此被拦截
    let data_only = "BEGIN;\nINSERT INTO `card` (`id`) VALUES (1);\nCOMMIT;";
    assert!(parse_schema_template(data_only).is_err());

    // 只有 USE 没有任何表
    let use_only = "USE nuwax_im;";
    assert!(parse_schema_template(use_only).is_err());
}

fn section(database: &str, stats: DiffStats, diff_sql: &str) -> DbSectionResult {
    DbSectionResult {
        database: database.to_string(),
        diff_sql: diff_sql.to_string(),
        stats,
        live_sql: String::new(),
    }
}

#[test]
fn assemble_renders_preamble_and_use_only_for_executable_sections() {
    let templates = vec![platform_template(), im_template()];
    let sections = vec![
        section(
            "agent_platform",
            DiffStats {
                tables_added: 1,
                ..Default::default()
            },
            "CREATE TABLE `users` (id bigint NOT NULL);",
        ),
        section("nuwax_im", DiffStats::default(), ""),
    ];

    let result = assemble_multi_db_diff(&templates, sections, "1.2.0");

    assert!(result.has_executable_sql);
    assert!(!result.has_warnings);
    // 干净的库不输出任何段
    assert!(
        result
            .diff_sql
            .contains("-- ===== Database: `agent_platform` =====")
    );
    assert!(
        !result
            .diff_sql
            .contains("-- ===== Database: `nuwax_im` =====")
    );

    // 段内顺序：建库/授权原句 → USE → DDL
    let flush = result
        .diff_sql
        .find("FLUSH PRIVILEGES;")
        .expect("preamble must be present");
    let use_stmt = result
        .diff_sql
        .find("USE `agent_platform`;")
        .expect("USE must be present");
    let ddl = result
        .diff_sql
        .find("CREATE TABLE `users`")
        .expect("DDL must be present");
    assert!(flush < use_stmt);
    assert!(use_stmt < ddl);
    assert_eq!(
        result.sections.len(),
        2,
        "sections keep per-database details even when clean"
    );
}

#[test]
fn assemble_renders_warning_only_sections_without_executing_anything() {
    let templates = vec![im_template()];
    let sections = vec![section(
        "nuwax_im",
        DiffStats {
            tables_dropped: 2,
            ..Default::default()
        },
        "-- Warning: table `legacy` was removed in the new version",
    )];

    let result = assemble_multi_db_diff(&templates, sections, "1.2.0");

    assert!(!result.has_executable_sql);
    assert!(result.has_warnings);
    // 仅警告的段只有注释：不带 preamble / USE，不产生任何可执行语句
    assert!(
        result
            .diff_sql
            .contains("-- ===== Database: `nuwax_im` =====")
    );
    assert!(!result.diff_sql.contains("USE `nuwax_im`;"));
    assert!(!result.diff_sql.contains("CREATE DATABASE"));
}

#[test]
fn assemble_returns_empty_diff_when_all_databases_clean() {
    let templates = vec![platform_template(), im_template()];
    let sections = vec![
        section("agent_platform", DiffStats::default(), ""),
        section("nuwax_im", DiffStats::default(), ""),
    ];

    let result = assemble_multi_db_diff(&templates, sections, "1.2.0");

    assert!(result.diff_sql.is_empty());
    assert!(!result.has_executable_sql);
    assert!(!result.has_warnings);
    assert!(result.description.contains("agent_platform"));
    assert!(result.description.contains("nuwax_im"));
}

/// 索引列前缀长度（`url`(255)）在两条渲染路径上都必须输出合法 SQL：
/// 解析后列名带 "(255)" 后缀，整体加反引号会产生非法标识符 `` `url(255)` ``
#[test]
fn regression_index_prefix_length_renders_valid_sql() {
    use super::generator::generate_schema_diff;

    // 路径1：新表 → generate_create_table_sql 渲染整条 CREATE TABLE
    let live_without_table =
        "USE nuwax_im;\nCREATE TABLE `unrelated` (`id` bigint NOT NULL, PRIMARY KEY (`id`));";
    let (diff, _) = generate_schema_diff(
        Some(live_without_table),
        STICKER_TARGET,
        Some("live"),
        "target",
    )
    .expect("diff must generate");
    assert!(diff.contains("`url`(255)"), "CREATE path diff: {diff}");
    assert!(!diff.contains("`url(255)`"), "CREATE path diff: {diff}");

    // 路径2：已存在的表新增索引 → differ 渲染 ALTER TABLE ADD UNIQUE KEY
    let live_without_index = "USE nuwax_im;\nCREATE TABLE `sticker` (\n  `id` bigint NOT NULL,\n  `url` varchar(1024) NOT NULL,\n  PRIMARY KEY (`id`)\n) ENGINE=InnoDB;";
    let (diff, _) = generate_schema_diff(
        Some(live_without_index),
        STICKER_TARGET,
        Some("live"),
        "target",
    )
    .expect("diff must generate");
    assert!(
        diff.contains("ADD UNIQUE KEY `uk_user_url` (`id`, `url`(255))"),
        "ALTER path diff: {diff}"
    );
}
