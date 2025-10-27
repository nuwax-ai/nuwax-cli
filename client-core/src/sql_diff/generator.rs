use super::differ::generate_mysql_diff;
use super::parser::parse_sql_tables;
use super::types::{TableColumn, TableDefinition, TableIndex};
use crate::error::DuckError;
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
            info!("生成初始版本 {} 的完整数据库架构", to_version);
            let description = format!("初始版本 {to_version} 的完整数据库架构");
            Ok((to_sql.to_string(), description))
        }
        Some(from_content) => {
            info!(
                "开始生成版本 {} 到 {} 的SQL差异",
                from_version.unwrap_or("unknown"),
                to_version
            );

            // 如果内容完全相同，返回空差异
            if from_content.trim() == to_sql.trim() {
                info!("版本内容完全相同，无需生成差异");
                return Ok((
                    String::new(),
                    format!(
                        "版本 {} 到 {}: 无变化",
                        from_version.unwrap_or("unknown"),
                        to_version
                    ),
                ));
            }

            // 解析两个SQL文件的表结构
            let from_tables = parse_sql_tables(from_content)?;
            let to_tables = parse_sql_tables(to_sql)?;

            // 生成差异SQL
            let diff_sql = generate_mysql_diff(&from_tables, &to_tables)?;

            let description = if diff_sql.trim().is_empty() {
                format!(
                    "版本 {} 到 {}: 仅有注释或格式变化，无实际架构差异",
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
                if diff_sql.contains("CREATE TABLE") {
                    change_types.push("新增表");
                }
                if diff_sql.contains("DROP TABLE") {
                    change_types.push("删除表");
                }
                if diff_sql.contains("ALTER TABLE") && diff_sql.contains("ADD COLUMN") {
                    change_types.push("新增列");
                }
                if diff_sql.contains("ALTER TABLE") && diff_sql.contains("DROP COLUMN") {
                    change_types.push("删除列");
                }
                if diff_sql.contains("ALTER TABLE") && diff_sql.contains("MODIFY COLUMN") {
                    change_types.push("修改列");
                }
                if diff_sql.contains("ALTER TABLE") && diff_sql.contains("ADD KEY") {
                    change_types.push("新增索引");
                }
                if diff_sql.contains("ALTER TABLE") && diff_sql.contains("DROP KEY") {
                    change_types.push("删除索引");
                }

                let change_summary = if change_types.is_empty() {
                    "架构变更".to_string()
                } else {
                    change_types.join("、")
                };

                format!(
                    "版本 {} 到 {}: {} - 生成 {} 行可执行的差异SQL",
                    from_version.unwrap_or("unknown"),
                    to_version,
                    change_summary,
                    lines_count
                )
            };

            info!("差异生成完成: {}", description);
            Ok((diff_sql, description))
        }
    }
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

    sql.push(';');
    sql
}

/// 生成列定义SQL
pub fn generate_column_sql(column: &TableColumn) -> String {
    let mut sql = format!("`{}` {}", column.name, column.data_type);

    if !column.nullable {
        sql.push_str(" NOT NULL");
    }

    if let Some(default) = &column.default_value {
        sql.push_str(&format!(" DEFAULT '{default}'"));
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
                .map(|c| format!("`{c}`"))
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
                .map(|c| format!("`{c}`"))
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
                .map(|c| format!("`{c}`"))
                .collect::<Vec<_>>()
                .join(", ")
        )
    }
}
