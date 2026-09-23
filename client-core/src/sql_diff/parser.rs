use super::types::{GeneratedColumnDef, TableColumn, TableDefinition, TableIndex};
use crate::error::DuckError;
use regex::Regex;
use sqlparser::ast::{
    ColumnDef, CreateTableOptions, DataType, FullTextOrSpatialKind, GeneratedExpressionMode,
    IndexOption, SqlOption, Statement, TableConstraint,
};
use sqlparser::dialect::MySqlDialect;
use sqlparser::parser::Parser;
use std::collections::HashMap;
use tracing::{debug, info, warn};

/// 移除标识符中的反引号
#[inline]
fn strip_backticks(s: &str) -> String {
    s.trim_matches('`').to_string()
}

/// 将 sqlparser 的标识符转换为字符串（移除反引号）
#[inline]
fn ident_to_string<T: ToString>(ident: &T) -> String {
    strip_backticks(&ident.to_string())
}

/// 解析SQL文件中的表结构
pub fn parse_sql_tables(sql_content: &str) -> Result<HashMap<String, TableDefinition>, DuckError> {
    parse_sql_tables_with_mode(sql_content, false)
}

/// 升级路径使用严格解析，避免跳过无法识别的表后生成错误迁移。
pub fn parse_sql_tables_strict(
    sql_content: &str,
) -> Result<HashMap<String, TableDefinition>, DuckError> {
    parse_sql_tables_with_mode(sql_content, true)
}

fn parse_sql_tables_with_mode(
    sql_content: &str,
    strict: bool,
) -> Result<HashMap<String, TableDefinition>, DuckError> {
    let mut tables = HashMap::new();

    // 使用正则表达式找到 USE 语句的位置，然后从该位置开始解析后续的 CREATE TABLE 语句
    let create_table_statements = extract_create_table_statements_with_regex(sql_content)?;

    let dialect = MySqlDialect {};

    for create_table_sql in create_table_statements {
        debug!("Parsing CREATE TABLE statement: {}", create_table_sql);

        match Parser::parse_sql(&dialect, &create_table_sql) {
            Ok(statements) => {
                for statement in statements {
                    if let Statement::CreateTable(create_table) = statement {
                        // 移除表名中的反引号，确保表名统一
                        let table_name = ident_to_string(&create_table.name);
                        debug!("Parsing table: {}", table_name);

                        let mut table_columns = Vec::new();
                        let mut table_indexes = Vec::new();
                        let mut primary_key_columns = Vec::new();

                        // 解析列定义
                        for column in &create_table.columns {
                            let column_def = parse_column_definition(column)?;

                            // 检查是否是列级别的主键
                            if is_column_primary_key(column) {
                                primary_key_columns.push(ident_to_string(&column.name));
                            }

                            table_columns.push(column_def);
                        }

                        // 如果有列级别的主键，添加到索引列表
                        if !primary_key_columns.is_empty() {
                            table_indexes.push(TableIndex {
                                name: "PRIMARY".to_string(),
                                columns: primary_key_columns,
                                is_primary: true,
                                is_unique: true,
                                is_fulltext: false,
                                is_spatial: false,
                                index_type: Some("PRIMARY".to_string()),
                                parser: None,
                            });
                        }

                        // 解析约束（包括索引）
                        for constraint in &create_table.constraints {
                            if let Some(index) = parse_table_constraint(constraint)? {
                                table_indexes.push(index);
                            }
                        }

                        // 提取表选项（ENGINE/CHARSET/COLLATE）；
                        // AUTO_INCREMENT 等 dump 计数器不属于 schema，不提取
                        let (engine, charset, collation) =
                            extract_table_options(&create_table.table_options);

                        let table_def = TableDefinition {
                            name: table_name.clone(),
                            columns: table_columns,
                            indexes: table_indexes,
                            engine,
                            charset,
                            collation,
                        };

                        tables.insert(table_name, table_def);
                    }
                }
            }
            Err(e) => {
                if strict {
                    return Err(DuckError::custom(format!(
                        "Failed to parse CREATE TABLE statement during upgrade: {e}; SQL: {create_table_sql}"
                    )));
                }
                warn!(
                    "Failed to parse SQL statement: {} - error: {}",
                    create_table_sql, e
                );
            }
        }
    }

    // 🔧 新增：解析独立的 CREATE INDEX 语句
    parse_standalone_indexes(sql_content, &mut tables)?;

    info!("Successfully parsed {} tables", tables.len());
    Ok(tables)
}

/// 使用正则表达式找到 USE 语句位置，然后提取后续的 CREATE TABLE 语句
fn extract_create_table_statements_with_regex(sql_content: &str) -> Result<Vec<String>, DuckError> {
    // 创建正则表达式来匹配 USE 语句
    let use_regex = Regex::new(r"(?i)^\s*USE\s+[^;]+;\s*$")
        .map_err(|e| DuckError::custom(format!("正则表达式编译失败: {e}")))?;

    let lines: Vec<&str> = sql_content.lines().collect();
    let mut start_parsing_from_line = 0;

    // 查找 USE 语句
    for (line_idx, line) in lines.iter().enumerate() {
        if use_regex.is_match(line) {
            debug!("Found USE statement at line {}: {}", line_idx + 1, line);
            start_parsing_from_line = line_idx + 1; // 从下一行开始
            break;
        }
    }

    // 如果没有找到 USE 语句，从头开始解析
    if start_parsing_from_line == 0 {
        debug!("No USE statement found, parsing entire file from the beginning");
    }

    // 从指定位置开始提取内容
    let content_to_parse = if start_parsing_from_line < lines.len() {
        lines[start_parsing_from_line..].join("\n")
    } else {
        sql_content.to_string()
    };

    extract_create_table_statements_from_content(&content_to_parse)
}

/// 从指定内容中提取 CREATE TABLE 语句
fn extract_create_table_statements_from_content(content: &str) -> Result<Vec<String>, DuckError> {
    let mut statements = Vec::new();

    // 创建正则表达式来匹配 CREATE TABLE 语句的开始
    let create_table_regex = Regex::new(r"(?i)^\s*CREATE\s+TABLE")
        .map_err(|e| DuckError::custom(format!("正则表达式编译失败: {e}")))?;

    let lines: Vec<&str> = content.lines().collect();
    let mut current_statement = String::new();
    let mut in_create_table = false;
    let mut paren_count = 0;
    let mut in_string = false;
    let mut escape_next = false;

    for line in lines {
        let trimmed = line.trim();

        // 跳过空行和注释
        if trimmed.is_empty() || trimmed.starts_with("--") || trimmed.starts_with("/*") {
            continue;
        }

        // 检查是否是 CREATE TABLE 语句的开始
        if !in_create_table && create_table_regex.is_match(line) {
            in_create_table = true;
            current_statement.clear();
            paren_count = 0;
            in_string = false;
            escape_next = false;
        }

        if in_create_table {
            current_statement.push_str(line);
            current_statement.push('\n');

            // 逐字符分析以正确处理括号平衡
            for ch in line.chars() {
                if escape_next {
                    escape_next = false;
                    continue;
                }

                match ch {
                    '\\' if in_string => {
                        escape_next = true;
                    }
                    '\'' | '"' | '`' => {
                        in_string = !in_string;
                    }
                    '(' if !in_string => {
                        paren_count += 1;
                    }
                    ')' if !in_string => {
                        paren_count -= 1;
                    }
                    ';' if !in_string && paren_count <= 0 => {
                        // 找到语句结束
                        statements.push(current_statement.trim().to_string());
                        current_statement.clear();
                        in_create_table = false;
                        paren_count = 0;
                        break;
                    }
                    _ => {}
                }
            }
        }
    }

    // 处理可能没有分号结尾的语句
    if in_create_table && !current_statement.trim().is_empty() {
        statements.push(current_statement.trim().to_string());
    }

    debug!("Extracted {} CREATE TABLE statements", statements.len());
    Ok(statements)
}

/// 解析列定义
fn parse_column_definition(column: &ColumnDef) -> Result<TableColumn, DuckError> {
    let column_name = ident_to_string(&column.name);
    let data_type = format_data_type(&column.data_type);

    let mut nullable = true;
    let mut default_value = None;
    let mut on_update = None;
    let mut generated = None;
    let mut comment = None;
    let mut auto_increment = false;

    // 检查列选项
    for option in &column.options {
        match &option.option {
            sqlparser::ast::ColumnOption::NotNull => {
                nullable = false;
            }
            sqlparser::ast::ColumnOption::Default(expr) => {
                default_value = Some(format_default_value(expr));
            }
            sqlparser::ast::ColumnOption::OnUpdate(expr) => {
                // MySQL `ON UPDATE CURRENT_TIMESTAMP` / `current_timestamp(3)`；
                // expr 的 Display 渲染即为合法 SQL 片段
                on_update = Some(expr.to_string());
            }
            sqlparser::ast::ColumnOption::Generated {
                generation_expr: Some(expr),
                generation_expr_mode,
                ..
            } => {
                // MySQL `GENERATED ALWAYS AS (expr) STORED/VIRTUAL` 与 `AS (expr)` 简写；
                // 统一存归一化形态（是否 STORED），渲染时统一用 GENERATED ALWAYS 全写。
                // generation_expr 为 None 的是其他方言的 sequence 选项，MySQL 不会出现，
                // 由下方通配分支忽略。
                generated = Some(GeneratedColumnDef {
                    expr: expr.to_string(),
                    stored: matches!(generation_expr_mode, Some(GeneratedExpressionMode::Stored)),
                });
            }
            sqlparser::ast::ColumnOption::Comment(c) => {
                comment = Some(c.clone());
            }
            sqlparser::ast::ColumnOption::Unique(_) => {
                // sqlparser 0.62: 列级 UNIQUE 不再包含 is_primary 字段
                // 列级 PRIMARY KEY 现在是独立的 ColumnOption::PrimaryKey 变体
            }
            sqlparser::ast::ColumnOption::PrimaryKey(_) => {
                nullable = false; // 主键不能为空
            }
            sqlparser::ast::ColumnOption::DialectSpecific(tokens) => {
                // 检查是否是AUTO_INCREMENT
                let token_str = tokens
                    .iter()
                    .map(|t| t.to_string())
                    .collect::<Vec<_>>()
                    .join(" ")
                    .to_uppercase();
                if token_str.contains("AUTO_INCREMENT") {
                    auto_increment = true;
                }
            }
            _ => {}
        }
    }

    Ok(TableColumn {
        name: column_name,
        data_type,
        nullable,
        default_value,
        on_update,
        generated,
        auto_increment,
        comment,
    })
}

/// 解析表约束
fn parse_table_constraint(constraint: &TableConstraint) -> Result<Option<TableIndex>, DuckError> {
    match constraint {
        TableConstraint::PrimaryKey(pk) => {
            let column_names = extract_index_columns(&pk.columns);

            Ok(Some(TableIndex {
                name: "PRIMARY".to_string(),
                columns: column_names,
                is_primary: true,
                is_unique: true,
                is_fulltext: false,
                is_spatial: false,
                index_type: Some("PRIMARY".to_string()),
                parser: None,
            }))
        }
        TableConstraint::Unique(uq) => {
            let column_names = extract_index_columns(&uq.columns);
            // MySQL 方言 `UNIQUE KEY uk_x (...)` 的名字在 index_name 字段；
            // ANSI `CONSTRAINT uk_x UNIQUE (...)` 的约束名在 name 字段。两者都取，
            // 否则 MySQL 风格的唯一索引会丢失名字、被合成为 unique_<列名>。
            let index_name = uq
                .index_name
                .as_ref()
                .or(uq.name.as_ref())
                .map(ident_to_string)
                .unwrap_or_else(|| format!("unique_{}", column_names.join("_")));

            Ok(Some(TableIndex {
                name: index_name,
                columns: column_names,
                is_primary: false,
                is_unique: true,
                is_fulltext: false,
                is_spatial: false,
                index_type: Some("UNIQUE".to_string()),
                parser: None,
            }))
        }
        TableConstraint::Index(idx) => {
            let column_names = extract_index_columns(&idx.columns);
            let index_name = idx
                .name
                .as_ref()
                .map(ident_to_string)
                .unwrap_or_else(|| format!("idx_{}", column_names.join("_")));

            Ok(Some(TableIndex {
                name: index_name,
                columns: column_names,
                is_primary: false,
                is_unique: false,
                is_fulltext: false,
                is_spatial: false,
                index_type: Some("INDEX".to_string()),
                parser: None,
            }))
        }
        // CREATE TABLE 内部的 FULLTEXT / SPATIAL 约束形式
        TableConstraint::FulltextOrSpatial(ft) => {
            let column_names = extract_index_columns(&ft.columns);
            let index_name = ft
                .opt_index_name
                .as_ref()
                .map(ident_to_string)
                .unwrap_or_else(|| {
                    format!(
                        "{}_{}",
                        if ft.fulltext { "fulltext" } else { "spatial" },
                        column_names.join("_")
                    )
                });
            let (is_fulltext, index_type) = if ft.fulltext {
                (true, "FULLTEXT".to_string())
            } else {
                (false, "SPATIAL".to_string())
            };

            // 提取 `WITH PARSER <name>` 子句（fork 版 sqlparser 0.63.2+ 支持，
            // 例如 FULLTEXT 索引常用 ngram parser 支持中日韩分词）。
            // `SHOW CREATE TABLE` 的内联 FULLTEXT KEY 形式也会走到这里。
            let parser_name = ft.index_options.iter().find_map(|opt| {
                if let IndexOption::WithParser(name) = opt {
                    Some(ident_to_string(name))
                } else {
                    None
                }
            });

            Ok(Some(TableIndex {
                name: index_name,
                columns: column_names,
                is_primary: false,
                is_unique: false,
                is_fulltext,
                is_spatial: !ft.fulltext,
                index_type: Some(index_type),
                parser: parser_name,
            }))
        }
        _ => Ok(None),
    }
}

/// 格式化默认值（特别处理函数类型的默认值）
fn format_default_value(expr: &sqlparser::ast::Expr) -> String {
    debug!("format_default_value called, expression: {:?}", expr);

    match expr {
        // 处理函数调用，如 CURRENT_TIMESTAMP
        sqlparser::ast::Expr::Function(function) => {
            let function_name = function.name.to_string();
            debug!("Detected function call: {}", function_name);
            // 对于 MySQL 的日期时间函数，不需要加引号，直接返回函数名
            match function_name.to_uppercase().as_str() {
                "CURRENT_TIMESTAMP" | "NOW" | "CURRENT_DATE" | "CURRENT_TIME"
                | "LOCALTIMESTAMP" | "LOCALTIME" => {
                    debug!(
                        "Recognized as MySQL datetime function, returning: {}",
                        function_name
                    );
                    function_name
                }
                _ => {
                    debug!("Other function, using default format: {}", function_name);
                    // 其他函数保持原有格式
                    format!("{expr}")
                }
            }
        }

        // 处理各种值类型
        sqlparser::ast::Expr::Value(value_with_span) => {
            debug!("Detected value type: {:?}", value_with_span);
            match &value_with_span.value {
                sqlparser::ast::Value::SingleQuotedString(s) => {
                    debug!("String value: {} -> '{}'", s, s);
                    format!("'{}'", s)
                }
                sqlparser::ast::Value::Number(_, _) => {
                    debug!("Numeric value");
                    // 数字类型不需要引号，直接返回表达式格式化结果
                    format!("{expr}")
                }
                sqlparser::ast::Value::Null => {
                    debug!("NULL value");
                    "NULL".to_string()
                }
                sqlparser::ast::Value::Boolean(b) => {
                    debug!("Boolean value: {}", b);
                    b.to_string()
                }
                // 处理其他值类型
                _ => {
                    debug!("Other value type");
                    format!("{expr}")
                }
            }
        }

        // 其他情况使用默认格式化
        _ => {
            debug!("Other expression type");
            format!("{expr}")
        }
    }
}

/// 格式化数据类型
///
/// 直接使用 nuwax-sqlparser 的 Display 实现：对 MySQL 方言的全部类型变体
/// （含 Unsigned 整型族、带显示宽度的整数、ENUM/BIT/VARBINARY 等）渲染合法 SQL。
/// 此前手写 match 的兜底分支用 Debug 格式（`{:?}`），会把
/// `DataType::BigIntUnsigned(None)` 之类的枚举名直接印进 DDL 产生非法 SQL，
/// 且整数分支丢弃显示宽度（tinyint(1) 退化为 TINYINT）。
fn format_data_type(data_type: &DataType) -> String {
    data_type.to_string()
}

/// 从 CREATE TABLE 表选项提取 ENGINE / CHARSET / COLLATE
///
/// MySQL dump 形如 `... ) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4 COLLATE=utf8mb4_0900_ai_ci`
/// 解析为 `CreateTableOptions::Plain(Vec<SqlOption>)`：
/// - `ENGINE=InnoDB` → `SqlOption::NamedParenthesizedList { key: "ENGINE", name: Some(...) }`
/// - `DEFAULT CHARSET=x` / `CHARSET=x` / `CHARACTER SET=x` → `SqlOption::KeyValue`
/// - `COLLATE=x` / `DEFAULT COLLATE=x` → `SqlOption::KeyValue`
/// - `AUTO_INCREMENT=N` 是 dump 计数器而非 schema，忽略；ROW_FORMAT 等同样忽略。
fn extract_table_options(
    table_options: &CreateTableOptions,
) -> (Option<String>, Option<String>, Option<String>) {
    let CreateTableOptions::Plain(options) = table_options else {
        return (None, None, None);
    };

    let mut engine = None;
    let mut charset = None;
    let mut collation = None;

    for option in options {
        match option {
            SqlOption::NamedParenthesizedList(npl) => {
                if npl.name.is_some() && ident_to_string(&npl.key).eq_ignore_ascii_case("ENGINE") {
                    engine = npl.name.as_ref().map(ident_to_string);
                }
            }
            SqlOption::KeyValue { key, value } => {
                let key_upper = ident_to_string(key).to_uppercase();
                if key_upper.contains("CHARSET") || key_upper.contains("CHARACTER SET") {
                    charset = Some(option_value_to_string(value));
                } else if key_upper.contains("COLLATE") {
                    collation = Some(option_value_to_string(value));
                }
            }
            _ => {}
        }
    }

    (engine, charset, collation)
}

/// 渲染表选项的值为字符串（charset/collation 值是标识符，剥反引号）
fn option_value_to_string(value: &sqlparser::ast::Expr) -> String {
    match value {
        sqlparser::ast::Expr::Identifier(ident) => strip_backticks(&ident.to_string()),
        other => other.to_string(),
    }
}

/// 检查列是否是列级别的主键
fn is_column_primary_key(column: &ColumnDef) -> bool {
    for option in &column.options {
        if let sqlparser::ast::ColumnOption::PrimaryKey(_) = &option.option {
            return true;
        }
    }
    false
}

/// 从 IndexColumn 列表中提取列名
///
/// 处理三种情况：
/// 1. 简单列名：`column_name`
/// 2. 复合标识符：`table.column` (只取最后一部分)
/// 3. 复杂表达式：函数索引等 (使用 Display)
fn extract_index_columns(index_columns: &[sqlparser::ast::IndexColumn]) -> Vec<String> {
    index_columns
        .iter()
        .filter_map(|index_col| {
            match &index_col.column.expr {
                sqlparser::ast::Expr::Identifier(ident) => Some(strip_backticks(&ident.value)),
                sqlparser::ast::Expr::CompoundIdentifier(idents) => {
                    // 处理 table.column 格式，只取最后一个部分
                    idents.last().map(|id| strip_backticks(&id.value))
                }
                _ => {
                    // 对于函数索引等复杂表达式，使用 Display
                    Some(strip_backticks(&index_col.column.to_string()))
                }
            }
        })
        .collect()
}

/// 解析独立的 CREATE INDEX 语句并添加到表定义中
///
/// 使用 sqlparser 库正确解析 SQL 语法
///
/// 格式示例：
/// ```sql
/// create index idx_space_id
///     on agent_config (space_id);
///
/// create unique index uk_name
///     on users (username);
/// ```
fn parse_standalone_indexes(
    sql_content: &str,
    tables: &mut HashMap<String, TableDefinition>,
) -> Result<(), DuckError> {
    let dialect = MySqlDialect {};
    let mut index_count = 0;

    // 提取所有 CREATE INDEX 语句
    let index_statements = extract_create_index_statements(sql_content)?;

    for index_sql in index_statements {
        debug!("Parsing CREATE INDEX statement: {}", index_sql);

        match Parser::parse_sql(&dialect, &index_sql) {
            Ok(statements) => {
                for statement in statements {
                    if let Statement::CreateIndex(create_index) = statement {
                        // 提取索引名称
                        let index_name = create_index
                            .name
                            .as_ref()
                            .map(ident_to_string)
                            .unwrap_or_else(|| "unnamed_index".to_string());

                        // 提取表名
                        let table_name = ident_to_string(&create_index.table_name);

                        // 提取列名列表
                        let columns = extract_index_columns(&create_index.columns);

                        if columns.is_empty() {
                            warn!("Index {} has no column definition, skipping", index_name);
                            continue;
                        }

                        // 检查是否是 UNIQUE 索引
                        let is_unique = create_index.unique;

                        // 提取 FULLTEXT / SPATIAL 标记(fork 版 sqlparser 支持)
                        let (is_fulltext, is_spatial) = match &create_index.fulltext_or_spatial {
                            Some(FullTextOrSpatialKind::Fulltext) => (true, false),
                            Some(FullTextOrSpatialKind::Spatial) => (false, true),
                            None => (false, false),
                        };

                        // 确定索引类型字符串
                        let index_type = if is_fulltext {
                            "FULLTEXT".to_string()
                        } else if is_spatial {
                            "SPATIAL".to_string()
                        } else if is_unique {
                            "UNIQUE".to_string()
                        } else {
                            "INDEX".to_string()
                        };

                        // 提取 `WITH PARSER <name>` 子句（fork 版 sqlparser 0.63.1+ 支持，
                        // 例如 FULLTEXT 索引常用 ngram parser 支持中日韩分词）
                        let parser_name = create_index.index_options.iter().find_map(|opt| {
                            if let IndexOption::WithParser(name) = opt {
                                Some(ident_to_string(name))
                            } else {
                                None
                            }
                        });

                        // 查找对应的表
                        if let Some(table_def) = tables.get_mut(&table_name) {
                            // 检查是否已经存在同名索引
                            if table_def.indexes.iter().any(|idx| idx.name == index_name) {
                                debug!(
                                    "Index {} already exists in table {}, skipping",
                                    index_name, table_name
                                );
                                continue;
                            }

                            // 添加索引到表定义
                            table_def.indexes.push(TableIndex {
                                name: index_name.clone(),
                                columns: columns.clone(),
                                is_primary: false,
                                is_unique,
                                is_fulltext,
                                is_spatial,
                                index_type: Some(index_type),
                                parser: parser_name,
                            });

                            index_count += 1;
                            debug!(
                                "添加独立索引: {} 到表 {} (列: {:?}, unique: {}, fulltext: {}, spatial: {})",
                                index_name, table_name, columns, is_unique, is_fulltext, is_spatial
                            );
                        } else {
                            warn!(
                                "Index {} references table {} which does not exist, skipping",
                                index_name, table_name
                            );
                        }
                    }
                }
            }
            Err(e) => {
                warn!(
                    "Failed to parse CREATE INDEX statement: {} - error: {}",
                    index_sql, e
                );
            }
        }
    }

    if index_count > 0 {
        info!(
            "Successfully parsed {} standalone CREATE INDEX statements",
            index_count
        );
    }

    Ok(())
}

/// 提取所有 CREATE INDEX 语句
///
/// 使用简单的状态机来识别完整的 CREATE INDEX 语句
fn extract_create_index_statements(sql_content: &str) -> Result<Vec<String>, DuckError> {
    let mut statements = Vec::new();
    let mut current_statement = String::new();
    let mut in_create_index = false;

    // 正则表达式只用于识别语句开始，不用于解析
    let create_index_regex =
        Regex::new(r"(?i)^\s*CREATE\s+(UNIQUE\s+|FULLTEXT\s+|SPATIAL\s+)?INDEX")
            .map_err(|e| DuckError::custom(format!("正则表达式编译失败: {}", e)))?;

    for line in sql_content.lines() {
        let trimmed = line.trim();

        // 跳过空行和注释
        if trimmed.is_empty() || trimmed.starts_with("--") {
            continue;
        }

        // 检查是否是 CREATE INDEX 语句的开始
        if !in_create_index && create_index_regex.is_match(line) {
            in_create_index = true;
            current_statement.clear();
        }

        if in_create_index {
            current_statement.push_str(line);
            current_statement.push(' ');

            // 检查是否遇到分号（语句结束）
            if trimmed.ends_with(';') {
                statements.push(current_statement.trim().to_string());
                current_statement.clear();
                in_create_index = false;
            }
        }
    }

    // 处理可能没有分号结尾的语句
    if in_create_index && !current_statement.trim().is_empty() {
        statements.push(current_statement.trim().to_string());
    }

    debug!("Extracted {} CREATE INDEX statements", statements.len());
    Ok(statements)
}
