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
    diff_sql.push("-- Database schema diff SQL".to_string());
    diff_sql.push(format!(
        "-- Generated at: {}",
        chrono::Utc::now().format("%Y-%m-%d %H:%M:%S UTC")
    ));
    diff_sql.push("".to_string());

    // 1. 检查新增的表
    for (table_name, table_def) in to_tables {
        if !from_tables.contains_key(table_name) {
            info!("Found new table: {}", table_name);
            diff_sql.push(format!("-- Added table: {table_name}"));
            diff_sql.push(generate_create_table_sql(table_def));
            diff_sql.push("".to_string());
            stats.tables_added += 1;
        }
    }

    // 2. 检查删除的表（仅警告，不生成删除SQL）
    for table_name in from_tables.keys() {
        if !to_tables.contains_key(table_name) {
            tracing::warn!(
                "⚠️  Table `{}` was removed in the new version. For data safety, DROP TABLE SQL will not be generated",
                table_name
            );
            diff_sql.push(format!(
                "-- ⚠️  Warning: table `{table_name}` does not exist in the new version; no drop statement was generated for data safety"
            ));
            diff_sql.push(format!(
                "-- To delete it, run manually: DROP TABLE IF EXISTS `{table_name}`;"
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
                diff_sql.push(format!("-- Modified table: {table_name}"));
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
        info!(
            "Schema differences detected but only includes delete operation warnings, no executable SQL"
        );
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
                "⚠️  Column `{}` in table `{}` was removed in the new version. For data safety, DROP COLUMN SQL will not be generated",
                col_name,
                table_name,
            );
            diffs.push(format!(
                "-- ⚠️  Warning: column `{col_name}` does not exist in the new version; no drop statement was generated for data safety"
            ));
            diffs.push(format!(
                "-- To delete it, run manually: ALTER TABLE `{table_name}` DROP COLUMN `{col_name}`;"
            ));
            stats.columns_dropped += 1;
        }
    }

    // 检查修改的列
    for (col_name, new_col) in &new_columns {
        if let Some(old_col) = old_columns.get(col_name)
            && old_col != new_col
        {
            diffs.push(format!(
                "ALTER TABLE `{}` MODIFY COLUMN {};",
                table_name,
                generate_column_sql(new_col)
            ));
            stats.columns_modified += 1;
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
            && idx1.is_fulltext == idx2.is_fulltext
            && idx1.is_spatial == idx2.is_spatial
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
                "Index rename detected: table {} index '{}' exists as '{}' in DB with same columns; skipping operation",
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
        } else if idx_def.is_fulltext {
            let parser_clause = idx_def
                .parser
                .as_ref()
                .map(|p| format!(" WITH PARSER {p}"))
                .unwrap_or_default();
            diffs.push(format!(
                "ALTER TABLE `{}` ADD FULLTEXT KEY `{}` ({}){};",
                table_name,
                idx_name,
                idx_def
                    .columns
                    .iter()
                    .map(|c| format!("`{c}`"))
                    .collect::<Vec<_>>()
                    .join(", "),
                parser_clause
            ));
            stats.indexes_added += 1;
        } else if idx_def.is_spatial {
            diffs.push(format!(
                "ALTER TABLE `{}` ADD SPATIAL KEY `{}` ({});",
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
                "Index rename detected: table {} index '{}' has a different name in target but same columns; skipping drop",
                table_name,
                idx_name
            );
            continue;
        }

        // 真正需要删除的索引（仅警告）
        if idx_def.is_primary {
            tracing::warn!(
                "⚠️  Primary key in table `{}` was removed in the new version. For data safety, DROP PRIMARY KEY SQL will not be generated",
                table_name
            );
            diffs.push("-- ⚠️  Warning: primary key does not exist in the new version; no drop statement was generated for data safety".to_string());
            diffs.push(format!(
                "-- To delete it, run manually: ALTER TABLE `{table_name}` DROP PRIMARY KEY;"
            ));
            stats.indexes_dropped += 1;
        } else {
            tracing::warn!(
                "⚠️  Index `{}` in table `{}` was removed in the new version. For data safety, DROP KEY SQL will not be generated",
                idx_name,
                table_name,
            );
            diffs.push(format!(
                "-- ⚠️  Warning: index `{idx_name}` does not exist in the new version; no drop statement was generated for data safety"
            ));
            diffs.push(format!(
                "-- To delete it, run manually: ALTER TABLE `{table_name}` DROP KEY `{idx_name}`;"
            ));
            stats.indexes_dropped += 1;
        }
    }

    // 检查修改的索引（先警告删除，再添加新的）
    for (idx_name, new_idx) in &new_indexes {
        if let Some(old_idx) = old_indexes.get(idx_name)
            && old_idx != new_idx
        {
            // 警告需要删除旧索引（不生成删除SQL）
            if old_idx.is_primary {
                tracing::warn!(
                    "⚠️  Primary key definition changed in table `{}`. Old primary key must be dropped first; no automatic drop SQL is generated for data safety",
                    table_name
                );
                diffs.push("-- ⚠️  Warning: primary key definition changed; manually drop old primary key first".to_string());
                diffs.push(format!(
                    "-- Please run manually: ALTER TABLE `{table_name}` DROP PRIMARY KEY;"
                ));
            } else {
                tracing::warn!(
                    "⚠️  Index definition for `{}` in table `{}` changed. Old index must be dropped first; no automatic drop SQL is generated for data safety",
                    idx_name,
                    table_name,
                );
                diffs.push(format!(
                        "-- ⚠️  Warning: index `{idx_name}` definition changed; manually drop old index first"
                    ));
                diffs.push(format!(
                    "-- Please run manually: ALTER TABLE `{table_name}` DROP KEY `{idx_name}`;"
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

    (diffs, stats)
}
