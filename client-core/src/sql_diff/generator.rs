use super::differ::generate_mysql_diff;
use super::parser::parse_sql_tables;
use super::types::{
    DbSectionResult, MultiDbDiffResult, SchemaTemplate, TableColumn, TableDefinition, TableIndex,
};
use crate::error::DuckError;
use crate::mysql_executor::MySqlExecutor;
use tracing::info;

/// 生成SQL架构差异
/// 这个版本专注于生成实际可执行的MySQL差异SQL，而不是简单的文本比较
pub fn generate_schema_diff(
    from_sql: Option<&str>,
    to_sql: &str,
    from_version: Option<&str>,
    to_version: &str,
) -> Result<(String, String), DuckError> {
    match from_sql {
        None => {
            // 初始版本，返回完整的创建脚本
            info!(
                "Generating complete database schema for initial version {}",
                to_version
            );
            let description = format!(
                "Complete database schema for initial version {}",
                to_version
            );
            Ok((to_sql.to_string(), description))
        }
        Some(from_content) => {
            info!(
                "Starting to generate SQL diff from version {} to {}",
                from_version.unwrap_or("unknown"),
                to_version
            );

            // 如果内容完全相同，返回空差异
            if from_content.trim() == to_sql.trim() {
                info!("Version content is identical, no diff needed");
                return Ok((
                    String::new(),
                    format!(
                        "Version {} to {}: No changes",
                        from_version.unwrap_or("unknown"),
                        to_version
                    ),
                ));
            }

            // 解析两个SQL文件的表结构
            let from_tables = parse_sql_tables(from_content)?;
            let to_tables = parse_sql_tables(to_sql)?;

            // 生成差异SQL
            let (diff_sql, stats) = generate_mysql_diff(&from_tables, &to_tables)?;

            let description = if diff_sql.trim().is_empty() {
                format!(
                    "Version {} to {}: only comments or format changes, no actual schema differences",
                    from_version.unwrap_or("unknown"),
                    to_version
                )
            } else {
                let lines_count = diff_sql
                    .lines()
                    .filter(|line| !line.trim().is_empty() && !line.trim().starts_with("--"))
                    .count();

                // 分析差异类型
                let mut change_types = Vec::new();
                if stats.tables_added > 0 {
                    change_types.push("new tables");
                }
                if stats.tables_dropped > 0 {
                    change_types.push("dropped tables");
                }
                if stats.columns_added > 0 {
                    change_types.push("new columns");
                }
                if stats.columns_dropped > 0 {
                    change_types.push("dropped columns");
                }
                if stats.columns_modified > 0 {
                    change_types.push("modified columns");
                }
                if stats.indexes_added > 0 {
                    change_types.push("new indexes");
                }
                if stats.indexes_dropped > 0 {
                    change_types.push("dropped indexes");
                }
                if stats.indexes_modified > 0 {
                    change_types.push("modified indexes");
                }
                if stats.table_options_changed > 0 {
                    change_types.push("table option changes");
                }

                let change_summary = if change_types.is_empty() {
                    "schema changes".to_string()
                } else {
                    change_types.join(", ")
                };

                if !stats.has_executable_operations() && stats.has_warnings() {
                    format!(
                        "Version {} to {}: {} - only includes manual-change warnings, no executable SQL",
                        from_version.unwrap_or("unknown"),
                        to_version,
                        change_summary
                    )
                } else {
                    format!(
                        "Version {} to {}: {} - generated {} lines of executable diff SQL",
                        from_version.unwrap_or("unknown"),
                        to_version,
                        change_summary,
                        lines_count
                    )
                }
            };

            info!("Diff generation completed: {}", description);
            Ok((diff_sql, description))
        }
    }
}

/// 多库 Live Diff：逐个模板抓取在线库架构并生成差异，组装为单份 diff SQL。
///
/// 每库独立处理：库不存在或为空库时 `fetch_live_schema_with_sql` 返回空表集，
/// 自然生成"建库 + 全量建表"——即存量机器的自动补建路径。
/// 执行连接须为 root 管理连接（建库/授权/跨库 DDL 均超出应用账号权限）。
///
/// 返回 `MultiDbDiffResult`：sections 逐库明细，diff_sql 为组装结果（执行顺序
/// 与模板清单一致），描述汇总各库。
pub async fn generate_live_schema_diff_multi(
    executor: &MySqlExecutor,
    templates: &[SchemaTemplate],
    to_version: &str,
) -> Result<MultiDbDiffResult, DuckError> {
    info!(
        databases = ?templates.iter().map(|t| t.database.as_str()).collect::<Vec<_>>(),
        to_version,
        "Starting multi-database live schema diff"
    );

    let mut sections = Vec::with_capacity(templates.len());
    for template in templates {
        let database = template.database.as_str();
        info!(
            database,
            to_version, "Generating live schema diff for database"
        );

        let (live_tables, live_sql) = executor
            .fetch_live_schema_with_sql(database)
            .await
            .map_err(|e| {
                DuckError::custom(format!(
                    "Failed to fetch online schema for database `{database}`: {e}"
                ))
            })?;

        let (diff_sql, stats) = generate_mysql_diff(&live_tables, &template.tables)?;
        sections.push(DbSectionResult {
            database: database.to_string(),
            diff_sql,
            stats,
            live_sql,
        });
    }

    Ok(assemble_multi_db_diff(templates, sections, to_version))
}

/// 组装多库结果（纯函数，无 IO，便于单测）：
/// 汇总各库开关与描述，并仅对"有可执行变更"或"仅警告"的库输出段。
/// `templates` 与 `sections` 按同序一一对应（由调用方保证）。
pub(super) fn assemble_multi_db_diff(
    templates: &[SchemaTemplate],
    sections: Vec<DbSectionResult>,
    to_version: &str,
) -> MultiDbDiffResult {
    let mut diff_parts = Vec::new();
    let mut descriptions = Vec::new();
    let mut has_executable_sql = false;
    let mut has_warnings = false;

    for (template, section) in templates.iter().zip(sections.iter()) {
        let executable = section.stats.has_executable_operations();
        let warnings = section.stats.has_warnings();
        has_executable_sql |= executable;
        has_warnings |= warnings;
        descriptions.push(format!("{}: {}", section.database, section.stats.summary()));

        if executable || warnings {
            diff_parts.push(render_db_section(template, &section.diff_sql, executable));
        }
    }

    let diff_sql = if diff_parts.is_empty() {
        String::new()
    } else {
        diff_parts.join("\n")
    };
    let description = format!("Online schema to {to_version}: {}", descriptions.join("; "));

    info!(description = %description, has_executable_sql, has_warnings, "Multi-database live diff completed");
    MultiDbDiffResult {
        sections,
        diff_sql,
        description,
        has_executable_sql,
        has_warnings,
    }
}

/// 渲染单个库段。
///
/// `executable == true` 时段结构：注释头 → 建库/授权原句 → `USE` → 该库 DDL，
/// 会真正执行；`executable == false`（仅警告）时只输出注释头与警告注释，
/// 不带 preamble/USE，确保不会有任何语句被执行。
fn render_db_section(template: &SchemaTemplate, diff_sql: &str, executable: bool) -> String {
    let mut section = format!("-- ===== Database: `{}` =====\n", template.database);
    if executable {
        for stmt in &template.preamble_stmts {
            section.push_str(stmt);
            section.push('\n');
        }
        section.push_str(&format!("USE `{}`;\n", template.database));
    }
    section.push_str(diff_sql);
    section
}

/// 格式化默认值用于SQL输出，正确处理不同类型的值
fn format_default_value_for_sql(default: &str) -> String {
    // 检查是否是MySQL关键字/函数（不需要引号）
    let mysql_keywords = [
        "CURRENT_TIMESTAMP",
        "NOW()",
        "CURRENT_DATE",
        "CURRENT_TIME",
        "LOCALTIMESTAMP",
        "LOCALTIME",
        "NULL",
        "TRUE",
        "FALSE",
    ];

    let upper_default = default.to_uppercase();

    // 如果是MySQL关键字，直接返回（不加引号）
    if mysql_keywords.contains(&upper_default.as_str()) {
        return default.to_string();
    }

    // 如果是纯数字（可能包含负号和小数点），直接返回
    if default
        .chars()
        .all(|c| c.is_ascii_digit() || c == '-' || c == '.')
    {
        return default.to_string();
    }

    // 如果已经是引号包围的，直接返回
    if (default.starts_with('\'') && default.ends_with('\''))
        || (default.starts_with('"') && default.ends_with('"'))
    {
        return default.to_string();
    }

    // 其他情况作为字符串处理，添加单引号
    format!("'{}'", default)
}

/// 生成CREATE TABLE SQL
pub fn generate_create_table_sql(table: &TableDefinition) -> String {
    let mut sql = format!("CREATE TABLE `{}` (", table.name);

    // 添加列定义
    let mut parts = Vec::new();
    for column in &table.columns {
        parts.push(format!("  {}", generate_column_sql(column)));
    }

    // 添加索引定义
    for index in &table.indexes {
        parts.push(format!("  {}", generate_index_sql(index)));
    }

    sql.push_str(&parts.join(",\n"));
    sql.push_str("\n)");

    // 添加表选项
    if let Some(engine) = &table.engine {
        sql.push_str(&format!(" ENGINE={engine}"));
    }
    if let Some(charset) = &table.charset {
        sql.push_str(&format!(" DEFAULT CHARSET={charset}"));
    }
    if let Some(collation) = &table.collation {
        sql.push_str(&format!(" COLLATE={collation}"));
    }

    sql.push(';');
    sql
}

/// 生成列定义SQL
pub fn generate_column_sql(column: &TableColumn) -> String {
    let mut sql = format!("`{}` {}", column.name, column.data_type);

    if !column.nullable {
        sql.push_str(" NOT NULL");
    }

    if let Some(generated) = &column.generated {
        // 生成列：MySQL 不允许 DEFAULT / ON UPDATE，直接渲染生成子句。
        // 统一用 GENERATED ALWAYS 全写（模板里 `AS (expr)` 简写在解析时已归一）。
        let mode = if generated.stored {
            "STORED"
        } else {
            "VIRTUAL"
        };
        sql.push_str(&format!(" GENERATED ALWAYS AS ({}) {mode}", generated.expr));
    } else {
        if let Some(default) = &column.default_value {
            sql.push_str(&format!(
                " DEFAULT {}",
                format_default_value_for_sql(default)
            ));
        }

        if let Some(on_update) = &column.on_update {
            sql.push_str(&format!(" ON UPDATE {on_update}"));
        }
    }

    if column.auto_increment {
        sql.push_str(" AUTO_INCREMENT");
    }

    if let Some(comment) = &column.comment {
        sql.push_str(&format!(" COMMENT '{comment}'"));
    }

    sql
}

/// 生成索引定义SQL
pub fn generate_index_sql(index: &TableIndex) -> String {
    if index.is_primary {
        format!(
            "PRIMARY KEY ({})",
            index
                .columns
                .iter()
                .map(|c| c.render())
                .collect::<Vec<_>>()
                .join(", ")
        )
    } else if index.is_fulltext {
        let parser_clause = index
            .parser
            .as_ref()
            .map(|p| format!(" WITH PARSER {p}"))
            .unwrap_or_default();
        format!(
            "FULLTEXT KEY `{}` ({}){}",
            index.name,
            index
                .columns
                .iter()
                .map(|c| c.render())
                .collect::<Vec<_>>()
                .join(", "),
            parser_clause
        )
    } else if index.is_spatial {
        format!(
            "SPATIAL KEY `{}` ({})",
            index.name,
            index
                .columns
                .iter()
                .map(|c| c.render())
                .collect::<Vec<_>>()
                .join(", ")
        )
    } else if index.is_unique {
        format!(
            "UNIQUE KEY `{}` ({})",
            index.name,
            index
                .columns
                .iter()
                .map(|c| c.render())
                .collect::<Vec<_>>()
                .join(", ")
        )
    } else {
        format!(
            "KEY `{}` ({})",
            index.name,
            index
                .columns
                .iter()
                .map(|c| c.render())
                .collect::<Vec<_>>()
                .join(", ")
        )
    }
}
