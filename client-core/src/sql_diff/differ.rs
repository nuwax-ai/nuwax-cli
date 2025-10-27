use super::generator::{generate_column_sql, generate_create_table_sql};
use super::types::{TableColumn, TableDefinition, TableIndex};
use crate::error::DuckError;
use std::collections::HashMap;
use tracing::info;

/// 生成MySQL差异SQL
pub fn generate_mysql_diff(
    from_tables: &HashMap<String, TableDefinition>,
    to_tables: &HashMap<String, TableDefinition>,
) -> Result<String, DuckError> {
    let mut diff_sql = Vec::new();

    // 添加注释头
    diff_sql.push("-- 数据库架构差异SQL".to_string());
    diff_sql.push(format!(
        "-- 生成时间: {}",
        chrono::Utc::now().format("%Y-%m-%d %H:%M:%S UTC")
    ));
    diff_sql.push("".to_string());

    // 1. 检查新增的表
    for (table_name, table_def) in to_tables {
        if !from_tables.contains_key(table_name) {
            info!("发现新增表: {}", table_name);
            diff_sql.push(format!("-- 新增表: {table_name}"));
            diff_sql.push(generate_create_table_sql(table_def));
            diff_sql.push("".to_string());
        }
    }

    // 2. 检查删除的表
    for table_name in from_tables.keys() {
        if !to_tables.contains_key(table_name) {
            info!("发现删除表: {}", table_name);
            diff_sql.push(format!("-- 删除表: {table_name}"));
            diff_sql.push(format!("DROP TABLE IF EXISTS `{table_name}`;"));
            diff_sql.push("".to_string());
        }
    }

    // 3. 检查修改的表
    for (table_name, new_table_def) in to_tables {
        if let Some(old_table_def) = from_tables.get(table_name) {
            let table_diffs = generate_table_diff(old_table_def, new_table_def);
            if !table_diffs.is_empty() {
                info!("发现表结构变化: {}", table_name);
                diff_sql.push(format!("-- 修改表: {table_name}"));
                diff_sql.extend(table_diffs);
                diff_sql.push("".to_string());
            }
        }
    }

    let result = diff_sql.join("\n");

    // 如果只有注释头，说明没有实际差异
    let meaningful_lines: Vec<&str> = result
        .lines()
        .filter(|line| !line.trim().is_empty() && !line.trim().starts_with("--"))
        .collect();

    if meaningful_lines.is_empty() {
        info!("没有发现实际的表结构差异");
        return Ok(String::new());
    }

    Ok(result)
}

/// 生成表差异SQL
pub fn generate_table_diff(
    old_table: &TableDefinition,
    new_table: &TableDefinition,
) -> Vec<String> {
    let mut diffs = Vec::new();

    // 比较列差异
    let column_diffs = generate_column_diffs(old_table, new_table);
    diffs.extend(column_diffs);

    // 比较索引差异
    let index_diffs = generate_index_diffs(old_table, new_table);
    diffs.extend(index_diffs);

    diffs
}

/// 生成列差异SQL
fn generate_column_diffs(old_table: &TableDefinition, new_table: &TableDefinition) -> Vec<String> {
    let mut diffs = Vec::new();
    let table_name = &new_table.name;

    // 创建列名到列定义的映射
    let old_columns: HashMap<String, &TableColumn> = old_table
        .columns
        .iter()
        .map(|c| (c.name.clone(), c))
        .collect();
    let new_columns: HashMap<String, &TableColumn> = new_table
        .columns
        .iter()
        .map(|c| (c.name.clone(), c))
        .collect();

    // 检查新增的列
    for (col_name, col_def) in &new_columns {
        if !old_columns.contains_key(col_name) {
            diffs.push(format!(
                "ALTER TABLE `{}` ADD COLUMN {};",
                table_name,
                generate_column_sql(col_def)
            ));
        }
    }

    // 检查删除的列
    for col_name in old_columns.keys() {
        if !new_columns.contains_key(col_name) {
            diffs.push(format!(
                "ALTER TABLE `{table_name}` DROP COLUMN `{col_name}`;"
            ));
        }
    }

    // 检查修改的列
    for (col_name, new_col) in &new_columns {
        if let Some(old_col) = old_columns.get(col_name) {
            if old_col != new_col {
                diffs.push(format!(
                    "ALTER TABLE `{}` MODIFY COLUMN {};",
                    table_name,
                    generate_column_sql(new_col)
                ));
            }
        }
    }

    diffs
}

/// 生成索引差异SQL
fn generate_index_diffs(old_table: &TableDefinition, new_table: &TableDefinition) -> Vec<String> {
    let mut diffs = Vec::new();
    let table_name = &new_table.name;

    // 创建索引名到索引定义的映射
    let old_indexes: HashMap<String, &TableIndex> = old_table
        .indexes
        .iter()
        .map(|i| (i.name.clone(), i))
        .collect();
    let new_indexes: HashMap<String, &TableIndex> = new_table
        .indexes
        .iter()
        .map(|i| (i.name.clone(), i))
        .collect();

    // 检查新增的索引
    for (idx_name, idx_def) in &new_indexes {
        if !old_indexes.contains_key(idx_name) {
            if idx_def.is_primary {
                diffs.push(format!(
                    "ALTER TABLE `{}` ADD PRIMARY KEY ({});",
                    table_name,
                    idx_def
                        .columns
                        .iter()
                        .map(|c| format!("`{c}`"))
                        .collect::<Vec<_>>()
                        .join(", ")
                ));
            } else if idx_def.is_unique {
                diffs.push(format!(
                    "ALTER TABLE `{}` ADD UNIQUE KEY `{}` ({});",
                    table_name,
                    idx_name,
                    idx_def
                        .columns
                        .iter()
                        .map(|c| format!("`{c}`"))
                        .collect::<Vec<_>>()
                        .join(", ")
                ));
            } else {
                diffs.push(format!(
                    "ALTER TABLE `{}` ADD KEY `{}` ({});",
                    table_name,
                    idx_name,
                    idx_def
                        .columns
                        .iter()
                        .map(|c| format!("`{c}`"))
                        .collect::<Vec<_>>()
                        .join(", ")
                ));
            }
        }
    }

    // 检查删除的索引
    for (idx_name, idx_def) in &old_indexes {
        if !new_indexes.contains_key(idx_name) {
            if idx_def.is_primary {
                diffs.push(format!("ALTER TABLE `{table_name}` DROP PRIMARY KEY;"));
            } else {
                diffs.push(format!("ALTER TABLE `{table_name}` DROP KEY `{idx_name}`;"));
            }
        }
    }

    // 检查修改的索引（删除旧的，添加新的）
    for (idx_name, new_idx) in &new_indexes {
        if let Some(old_idx) = old_indexes.get(idx_name) {
            if old_idx != new_idx {
                // 先删除旧索引
                if old_idx.is_primary {
                    diffs.push(format!("ALTER TABLE `{table_name}` DROP PRIMARY KEY;"));
                } else {
                    diffs.push(format!("ALTER TABLE `{table_name}` DROP KEY `{idx_name}`;"));
                }

                // 再添加新索引
                if new_idx.is_primary {
                    diffs.push(format!(
                        "ALTER TABLE `{}` ADD PRIMARY KEY ({});",
                        table_name,
                        new_idx
                            .columns
                            .iter()
                            .map(|c| format!("`{c}`"))
                            .collect::<Vec<_>>()
                            .join(", ")
                    ));
                } else if new_idx.is_unique {
                    diffs.push(format!(
                        "ALTER TABLE `{}` ADD UNIQUE KEY `{}` ({});",
                        table_name,
                        idx_name,
                        new_idx
                            .columns
                            .iter()
                            .map(|c| format!("`{c}`"))
                            .collect::<Vec<_>>()
                            .join(", ")
                    ));
                } else {
                    diffs.push(format!(
                        "ALTER TABLE `{}` ADD KEY `{}` ({});",
                        table_name,
                        idx_name,
                        new_idx
                            .columns
                            .iter()
                            .map(|c| format!("`{c}`"))
                            .collect::<Vec<_>>()
                            .join(", ")
                    ));
                }
            }
        }
    }

    diffs
}
