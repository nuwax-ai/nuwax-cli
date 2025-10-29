use super::types::{TableColumn, TableDefinition, TableIndex};
use crate::error::DuckError;
use regex::Regex;
use sqlparser::ast::{ColumnDef, DataType, Statement, TableConstraint};
use sqlparser::dialect::MySqlDialect;
use sqlparser::parser::Parser;
use std::collections::HashMap;
use tracing::{debug, info, warn};

/// 解析SQL文件中的表结构
pub fn parse_sql_tables(sql_content: &str) -> Result<HashMap<String, TableDefinition>, DuckError> {
    let mut tables = HashMap::new();

    // 使用正则表达式找到 USE 语句的位置，然后从该位置开始解析后续的 CREATE TABLE 语句
    let create_table_statements = extract_create_table_statements_with_regex(sql_content)?;

    let dialect = MySqlDialect {};

    for create_table_sql in create_table_statements {
        debug!("解析 CREATE TABLE 语句: {}", create_table_sql);

        match Parser::parse_sql(&dialect, &create_table_sql) {
            Ok(statements) => {
                for statement in statements {
                    if let Statement::CreateTable(create_table) = statement {
                        let table_name = create_table.name.to_string();
                        debug!("解析表: {}", table_name);

                        let mut table_columns = Vec::new();
                        let mut table_indexes = Vec::new();
                        let mut primary_key_columns = Vec::new();

                        // 解析列定义
                        for column in &create_table.columns {
                            let column_def = parse_column_definition(column)?;

                            // 检查是否是列级别的主键
                            if is_column_primary_key(column) {
                                primary_key_columns.push(column.name.to_string());
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
                                index_type: Some("PRIMARY".to_string()),
                            });
                        }

                        // 解析约束（包括索引）
                        for constraint in &create_table.constraints {
                            if let Some(index) = parse_table_constraint(constraint)? {
                                table_indexes.push(index);
                            }
                        }

                        let table_def = TableDefinition {
                            name: table_name.clone(),
                            columns: table_columns,
                            indexes: table_indexes,
                            engine: None,  // 可以从原始SQL字符串中提取
                            charset: None, // 可以从原始SQL字符串中提取
                        };

                        tables.insert(table_name, table_def);
                    }
                }
            }
            Err(e) => {
                warn!("解析 SQL 语句失败: {} - 错误: {}", create_table_sql, e);
            }
        }
    }

    info!("成功解析 {} 个表", tables.len());
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
            debug!("找到 USE 语句在第 {} 行: {}", line_idx + 1, line);
            start_parsing_from_line = line_idx + 1; // 从下一行开始
            break;
        }
    }

    // 如果没有找到 USE 语句，从头开始解析
    if start_parsing_from_line == 0 {
        debug!("未找到 USE 语句，从头开始解析整个文件");
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

    debug!("提取到 {} 个 CREATE TABLE 语句", statements.len());
    Ok(statements)
}

/// 解析列定义
fn parse_column_definition(column: &ColumnDef) -> Result<TableColumn, DuckError> {
    let column_name = column.name.to_string();
    let data_type = format_data_type(&column.data_type);

    let mut nullable = true;
    let mut default_value = None;
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
            sqlparser::ast::ColumnOption::Comment(c) => {
                comment = Some(c.clone());
            }
            sqlparser::ast::ColumnOption::Unique { is_primary, .. } => {
                if *is_primary {
                    nullable = false; // 主键不能为空
                }
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
        auto_increment,
        comment,
    })
}

/// 解析表约束
fn parse_table_constraint(constraint: &TableConstraint) -> Result<Option<TableIndex>, DuckError> {
    match constraint {
        TableConstraint::PrimaryKey { columns, .. } => {
            let column_names: Vec<String> = columns.iter().map(|col| col.to_string()).collect();

            Ok(Some(TableIndex {
                name: "PRIMARY".to_string(),
                columns: column_names,
                is_primary: true,
                is_unique: true,
                index_type: Some("PRIMARY".to_string()),
            }))
        }
        TableConstraint::Unique { columns, name, .. } => {
            let column_names: Vec<String> = columns.iter().map(|col| col.to_string()).collect();

            let index_name = name
                .as_ref()
                .map(|n| n.to_string())
                .unwrap_or_else(|| format!("unique_{}", column_names.join("_")));

            Ok(Some(TableIndex {
                name: index_name,
                columns: column_names,
                is_primary: false,
                is_unique: true,
                index_type: Some("UNIQUE".to_string()),
            }))
        }
        TableConstraint::Index { name, columns, .. } => {
            let column_names: Vec<String> = columns.iter().map(|col| col.to_string()).collect();

            let index_name = name
                .as_ref()
                .map(|n| n.to_string())
                .unwrap_or_else(|| format!("idx_{}", column_names.join("_")));

            Ok(Some(TableIndex {
                name: index_name,
                columns: column_names,
                is_primary: false,
                is_unique: false,
                index_type: Some("INDEX".to_string()),
            }))
        }
        _ => Ok(None),
    }
}

/// 格式化默认值（特别处理函数类型的默认值）
fn format_default_value(expr: &sqlparser::ast::Expr) -> String {
    debug!("🔍 format_default_value 调用，表达式: {:?}", expr);

    match expr {
        // 处理函数调用，如 CURRENT_TIMESTAMP
        sqlparser::ast::Expr::Function(function) => {
            let function_name = function.name.to_string();
            debug!("🎯 检测到函数调用: {}", function_name);
            // 对于 MySQL 的日期时间函数，不需要加引号，直接返回函数名
            match function_name.to_uppercase().as_str() {
                "CURRENT_TIMESTAMP" | "NOW" | "CURRENT_DATE" | "CURRENT_TIME"
                | "LOCALTIMESTAMP" | "LOCALTIME" => {
                    debug!("✅ 识别为MySQL日期时间函数，返回: {}", function_name);
                    function_name
                }
                _ => {
                    debug!("⚠️  其他函数，使用默认格式: {}", function_name);
                    // 其他函数保持原有格式
                    format!("{expr}")
                }
            }
        }

        // 处理各种值类型
        sqlparser::ast::Expr::Value(value_with_span) => {
            debug!("🔢 检测到值类型: {:?}", value_with_span);
            match &value_with_span.value {
                sqlparser::ast::Value::SingleQuotedString(s) => {
                    debug!("💬 字符串值: {} -> '{}'", s, s);
                    format!("'{}'", s)
                }
                sqlparser::ast::Value::Number(_, _) => {
                    debug!("🔢 数字值");
                    // 数字类型不需要引号，直接返回表达式格式化结果
                    format!("{expr}")
                }
                sqlparser::ast::Value::Null => {
                    debug!("⭕ NULL值");
                    "NULL".to_string()
                }
                sqlparser::ast::Value::Boolean(b) => {
                    debug!("🔘 布尔值: {}", b);
                    b.to_string()
                }
                // 处理其他值类型
                _ => {
                    debug!("❓ 其他值类型");
                    format!("{expr}")
                }
            }
        }

        // 其他情况使用默认格式化
        _ => {
            debug!("❓ 其他表达式类型");
            format!("{expr}")
        }
    }
}

/// 格式化数据类型
fn format_data_type(data_type: &DataType) -> String {
    match data_type {
        DataType::Char(size) => {
            if let Some(size) = size {
                format!("CHAR({size})")
            } else {
                "CHAR".to_string()
            }
        }
        DataType::Varchar(size) => {
            if let Some(size) = size {
                format!("VARCHAR({size})")
            } else {
                "VARCHAR".to_string()
            }
        }
        DataType::Text => "TEXT".to_string(),
        DataType::Int(_) => "INT".to_string(),
        DataType::BigInt(_) => "BIGINT".to_string(),
        DataType::TinyInt(_) => "TINYINT".to_string(),
        DataType::SmallInt(_) => "SMALLINT".to_string(),
        DataType::MediumInt(_) => "MEDIUMINT".to_string(),
        DataType::Float(_) => "FLOAT".to_string(),
        DataType::Double(_) => "DOUBLE".to_string(),
        DataType::Decimal(exact_number_info) => match exact_number_info {
            sqlparser::ast::ExactNumberInfo::PrecisionAndScale(precision, scale) => {
                format!("DECIMAL({precision},{scale})")
            }
            sqlparser::ast::ExactNumberInfo::Precision(precision) => {
                format!("DECIMAL({precision})")
            }
            sqlparser::ast::ExactNumberInfo::None => "DECIMAL".to_string(),
        },
        DataType::Boolean => "BOOLEAN".to_string(),
        DataType::Date => "DATE".to_string(),
        DataType::Time(_, _) => "TIME".to_string(),
        DataType::Timestamp(_, _) => "TIMESTAMP".to_string(),
        DataType::Datetime(_) => "DATETIME".to_string(),
        DataType::JSON => "JSON".to_string(),
        DataType::Enum(variants, _max_length) => {
            // 正确处理 ENUM 变体
            let enum_values: Vec<String> = variants
                .iter()
                .filter_map(|variant| match variant {
                    sqlparser::ast::EnumMember::Name(name) => Some(format!("'{}'", name)),
                    sqlparser::ast::EnumMember::NamedValue(name, _expr) => {
                        Some(format!("'{}'", name))
                    }
                })
                .collect();

            if enum_values.is_empty() {
                "ENUM()".to_string()
            } else {
                format!("ENUM({})", enum_values.join(","))
            }
        }
        _ => format!("{data_type:?}"), // 对于其他类型，使用 Debug 格式
    }
}

/// 检查列是否是列级别的主键
fn is_column_primary_key(column: &ColumnDef) -> bool {
    for option in &column.options {
        if let sqlparser::ast::ColumnOption::Unique { is_primary, .. } = &option.option {
            if *is_primary {
                return true;
            }
        }
    }
    false
}

/// 检查列是否是主键列
fn is_primary_key_column(column: &ColumnDef, constraints: &[TableConstraint]) -> bool {
    // 首先检查列级别的主键定义
    for option in &column.options {
        if let sqlparser::ast::ColumnOption::Unique { is_primary, .. } = &option.option {
            if *is_primary {
                return true;
            }
        }
    }

    // 然后检查表级别的主键约束
    let column_name = column.name.to_string();
    for constraint in constraints {
        if let TableConstraint::PrimaryKey { columns, .. } = constraint {
            for pk_column in columns {
                if pk_column.to_string() == column_name {
                    return true;
                }
            }
        }
    }

    false
}
