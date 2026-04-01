use super::generator::{generate_column_sql, generate_create_table_sql};
use super::types::{TableColumn, TableDefinition, TableIndex};
use crate::error::DuckError;
use std::collections::HashMap;
use tracing::info;

/// 生成MySQL差异SQL（返回SQL和统计信息）
pub fn generate_mysql_diff(
    from_tables: &HashMap<String, TableDefinition>,
    to_tables: &HashMap<String, TableDefinition>,
) -> Result<(String, super::types::DiffStats), DuckError> {
    let mut diff_sql = Vec::new();
    let mut stats = super::types::DiffStats::default();

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
            info!("Found new table: {}", table_name);
            diff_sql.push(format!("-- 新增表: {table_name}"));
            diff_sql.push(generate_create_table_sql(table_def));
            diff_sql.push("".to_string());
            stats.tables_added += 1;
        }
    }

    // 2. 检查删除的表（仅警告，不生成删除SQL）
    for table_name in from_tables.keys() {
        if !to_tables.contains_key(table_name) {
            tracing::warn!(
                "⚠️  检测到表 `{}` 在新版本中被删除，为确保数据安全，不生成 DROP TABLE 语句",
                table_name
            );
            diff_sql.push(format!(
                "-- ⚠️  警告: 表 `{table_name}` 在新版本中不存在，但为了数据安全未生成删除语句"
            ));
            diff_sql.push(format!(
                "-- 如需删除，请手动执行: DROP TABLE IF EXISTS `{table_name}`;"
            ));
            diff_sql.push("".to_string());
            stats.tables_dropped += 1;
        }
    }

    // 3. 检查修改的表
    for (table_name, new_table_def) in to_tables {
        if let Some(old_table_def) = from_tables.get(table_name) {
            let (table_diffs, table_stats) = generate_table_diff(old_table_def, new_table_def);
            if !table_diffs.is_empty() {
                info!("Table structure changes detected: {}", table_name);
                diff_sql.push(format!("-- 修改表: {table_name}"));
                diff_sql.extend(table_diffs);
                diff_sql.push("".to_string());

                // 累加统计信息
                if table_stats.columns_added > 0
                    || table_stats.columns_dropped > 0
                    || table_stats.columns_modified > 0
                    || table_stats.indexes_added > 0
                    || table_stats.indexes_dropped > 0
                    || table_stats.indexes_modified > 0
                {
                    stats.tables_modified += 1;
                }
                stats.columns_added += table_stats.columns_added;
                stats.columns_dropped += table_stats.columns_dropped;
                stats.columns_modified += table_stats.columns_modified;
                stats.indexes_added += table_stats.indexes_added;
                stats.indexes_dropped += table_stats.indexes_dropped;
                stats.indexes_modified += table_stats.indexes_modified;
            }
        }
    }

    let result = diff_sql.join("\n");

    if !stats.has_changes() {
        info!("No actual table structure differences found");
        return Ok((String::new(), stats));
    }

    if !stats.has_executable_operations() && stats.has_dangerous_operations() {
        info!("Schema differences detected but only includes delete operation warnings, no executable SQL");
    }

    Ok((result, stats))
}

/// 生成表差异SQL（返回SQL和统计信息）
pub fn generate_table_diff(
    old_table: &TableDefinition,
    new_table: &TableDefinition,
) -> (Vec<String>, super::types::DiffStats) {
    let mut diffs = Vec::new();
    let mut stats = super::types::DiffStats::default();

    // 比较列差异
    let (column_diffs, column_stats) = generate_column_diffs(old_table, new_table);
    diffs.extend(column_diffs);
    stats.columns_added = column_stats.columns_added;
    stats.columns_dropped = column_stats.columns_dropped;
    stats.columns_modified = column_stats.columns_modified;

    // 比较索引差异
    let (index_diffs, index_stats) = generate_index_diffs(old_table, new_table);
    diffs.extend(index_diffs);
    stats.indexes_added = index_stats.indexes_added;
    stats.indexes_dropped = index_stats.indexes_dropped;
    stats.indexes_modified = index_stats.indexes_modified;

    (diffs, stats)
}

/// 生成列差异SQL（返回SQL和统计信息）
fn generate_column_diffs(
    old_table: &TableDefinition,
    new_table: &TableDefinition,
) -> (Vec<String>, super::types::DiffStats) {
    let mut diffs = Vec::new();
    let mut stats = super::types::DiffStats::default();
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
            stats.columns_added += 1;
        }
    }

    // 检查删除的列（仅警告，不生成删除SQL）
    for col_name in old_columns.keys() {
        if !new_columns.contains_key(col_name) {
            tracing::warn!(
                "⚠️  检测到表 `{}` 的列 `{}` 在新版本中被删除，为确保数据安全，不生成 DROP COLUMN 语句",
                table_name,
                col_name
            );
            diffs.push(format!(
                "-- ⚠️  警告: 列 `{col_name}` 在新版本中不存在，但为了数据安全未生成删除语句"
            ));
            diffs.push(format!(
                "-- 如需删除，请手动执行: ALTER TABLE `{table_name}` DROP COLUMN `{col_name}`;"
            ));
            stats.columns_dropped += 1;
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
                stats.columns_modified += 1;
            }
        }
    }

    (diffs, stats)
}

/// 生成索引差异SQL（返回SQL和统计信息）
fn generate_index_diffs(
    old_table: &TableDefinition,
    new_table: &TableDefinition,
) -> (Vec<String>, super::types::DiffStats) {
    let mut diffs = Vec::new();
    let mut stats = super::types::DiffStats::default();
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

    // 辅助函数：检查两个索引是否在语义上相同（列和类型相同，忽略名称）
    let indexes_semantically_equal = |idx1: &TableIndex, idx2: &TableIndex| -> bool {
        idx1.is_primary == idx2.is_primary
            && idx1.is_unique == idx2.is_unique
            && idx1.columns == idx2.columns
    };

    // 辅助函数：在旧索引中查找语义上相同的索引
    let find_semantically_equal_old_index = |new_idx: &TableIndex| -> Option<&TableIndex> {
        old_table
            .indexes
            .iter()
            .find(|old_idx| indexes_semantically_equal(old_idx, new_idx))
    };

    // 检查新增的索引
    for (idx_name, idx_def) in &new_indexes {
        // 先检查是否有同名索引
        if old_indexes.contains_key(idx_name) {
            continue; // 同名索引存在，稍后在"修改"部分处理
        }

        // 检查是否有语义上相同的索引（列相同但名字不同）
        if let Some(old_idx) = find_semantically_equal_old_index(idx_def) {
            // 找到了语义上相同的索引，这是索引重命名
            // 我们选择保留旧索引名，不做任何操作
            tracing::debug!(
                "索引重命名检测: 表 {} 的索引 '{}' 在数据库中名为 '{}', 列相同，跳过操作",
                table_name,
                idx_name,
                old_idx.name
            );
            continue;
        }

        // 真正的新索引
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
            stats.indexes_added += 1;
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
            stats.indexes_added += 1;
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
            stats.indexes_added += 1;
        }
    }

    // 辅助函数：在新索引中查找语义上相同的索引
    let find_semantically_equal_new_index = |old_idx: &TableIndex| -> Option<&TableIndex> {
        new_table
            .indexes
            .iter()
            .find(|new_idx| indexes_semantically_equal(old_idx, new_idx))
    };

    // 检查删除的索引（仅警告，不生成删除SQL）
    for (idx_name, idx_def) in &old_indexes {
        // 先检查是否有同名索引
        if new_indexes.contains_key(idx_name) {
            continue; // 同名索引存在，稍后在"修改"部分处理
        }

        // 检查是否有语义上相同的索引（列相同但名字不同）
        if find_semantically_equal_new_index(idx_def).is_some() {
            // 找到了语义上相同的索引，这是索引重命名
            // 我们选择保留旧索引名，不做任何操作
            tracing::debug!(
                "索引重命名检测: 表 {} 的索引 '{}' 在目标中有不同名称但列相同，跳过删除",
                table_name,
                idx_name
            );
            continue;
        }

        // 真正需要删除的索引（仅警告）
        if idx_def.is_primary {
            tracing::warn!(
                "⚠️  检测到表 `{}` 的主键在新版本中被删除，为确保数据安全，不生成 DROP PRIMARY KEY 语句",
                table_name
            );
            diffs.push(format!(
                "-- ⚠️  警告: 主键在新版本中不存在，但为了数据安全未生成删除语句"
            ));
            diffs.push(format!(
                "-- 如需删除，请手动执行: ALTER TABLE `{table_name}` DROP PRIMARY KEY;"
            ));
            stats.indexes_dropped += 1;
        } else {
            tracing::warn!(
                "⚠️  检测到表 `{}` 的索引 `{}` 在新版本中被删除，为确保数据安全，不生成 DROP KEY 语句",
                table_name,
                idx_name
            );
            diffs.push(format!(
                "-- ⚠️  警告: 索引 `{idx_name}` 在新版本中不存在，但为了数据安全未生成删除语句"
            ));
            diffs.push(format!(
                "-- 如需删除，请手动执行: ALTER TABLE `{table_name}` DROP KEY `{idx_name}`;"
            ));
            stats.indexes_dropped += 1;
        }
    }

    // 检查修改的索引（先警告删除，再添加新的）
    for (idx_name, new_idx) in &new_indexes {
        if let Some(old_idx) = old_indexes.get(idx_name) {
            if old_idx != new_idx {
                // 警告需要删除旧索引（不生成删除SQL）
                if old_idx.is_primary {
                    tracing::warn!(
                        "⚠️  检测到表 `{}` 的主键定义发生变化，需要先删除旧主键，为确保数据安全，不自动生成删除语句",
                        table_name
                    );
                    diffs.push(format!("-- ⚠️  警告: 主键定义已变化，需要先手动删除旧主键"));
                    diffs.push(format!(
                        "-- 请手动执行: ALTER TABLE `{table_name}` DROP PRIMARY KEY;"
                    ));
                } else {
                    tracing::warn!(
                        "⚠️  检测到表 `{}` 的索引 `{}` 定义发生变化，需要先删除旧索引，为确保数据安全，不自动生成删除语句",
                        table_name,
                        idx_name
                    );
                    diffs.push(format!(
                        "-- ⚠️  警告: 索引 `{idx_name}` 定义已变化，需要先手动删除旧索引"
                    ));
                    diffs.push(format!(
                        "-- 请手动执行: ALTER TABLE `{table_name}` DROP KEY `{idx_name}`;"
                    ));
                }
                stats.indexes_modified += 1;

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

    (diffs, stats)
}
