# SQL Parser 改进：支持独立的 CREATE INDEX 语句

## 问题背景

### 原始问题

用户的 SQL 文件格式：

```sql
CREATE TABLE agent_config (
  id BIGINT,
  space_id BIGINT,
  -- ... 没有索引定义
);

-- 独立的索引创建语句
create index idx_space_id
    on agent_config (space_id);

create unique index uk_username
    on users (username);
```

### 问题现象

1. **Parser 只解析 CREATE TABLE**：忽略了独立的 `CREATE INDEX` 语句
2. **Diff 算法误判**：
   - MySQL 数据库中有完整的索引
   - 模板 SQL 文件中没有索引（因为没被解析）
   - 生成大量 `ALTER TABLE ... DROP KEY` 语句 ❌

### 错误日志示例

```
ALTER TABLE `custom_table_definition` DROP KEY `idx_table_name`;
ALTER TABLE `knowledge_qa_segment` DROP KEY `idx_doc_id`;
ALTER TABLE `knowledge_qa_segment` DROP KEY `idx_kb_id_space_id_doc_id_index`;
-- ... 还有很多
```

## 解决方案

### 方案对比

| 方案 | 优点 | 缺点 | 选择 |
|------|------|------|------|
| 正则表达式 | 简单快速 | 不可靠，无法处理复杂语法 | ❌ |
| sqlparser 库 | 可靠，正确解析 SQL 语法 | 稍微复杂 | ✅ |
| 忽略索引差异 | 最简单 | 无法检测真正的索引变化 | ❌ |

### 实现方案：使用 sqlparser

#### 1. 提取 CREATE INDEX 语句

```rust
fn extract_create_index_statements(sql_content: &str) -> Result<Vec<String>, DuckError> {
    let mut statements = Vec::new();
    let mut current_statement = String::new();
    let mut in_create_index = false;

    // 正则表达式只用于识别语句开始
    let create_index_regex = Regex::new(r"(?i)^\s*CREATE\s+(UNIQUE\s+)?INDEX")?;

    for line in sql_content.lines() {
        if !in_create_index && create_index_regex.is_match(line) {
            in_create_index = true;
            current_statement.clear();
        }

        if in_create_index {
            current_statement.push_str(line);
            current_statement.push(' ');

            if line.trim().ends_with(';') {
                statements.push(current_statement.trim().to_string());
                current_statement.clear();
                in_create_index = false;
            }
        }
    }

    Ok(statements)
}
```

#### 2. 使用 sqlparser 解析

```rust
fn parse_standalone_indexes(
    sql_content: &str,
    tables: &mut HashMap<String, TableDefinition>,
) -> Result<(), DuckError> {
    let dialect = MySqlDialect {};
    let index_statements = extract_create_index_statements(sql_content)?;

    for index_sql in index_statements {
        match Parser::parse_sql(&dialect, &index_sql) {
            Ok(statements) => {
                for statement in statements {
                    if let Statement::CreateIndex(create_index) = statement {
                        // 提取索引信息
                        let index_name = create_index.name
                            .as_ref()
                            .map(|n| n.to_string().trim_matches('`').to_string())
                            .unwrap_or_else(|| "unnamed_index".to_string());

                        let table_name = create_index.table_name
                            .to_string()
                            .trim_matches('`')
                            .to_string();

                        let columns: Vec<String> = create_index.columns
                            .iter()
                            .map(|col| col.to_string().trim_matches('`').to_string())
                            .collect();

                        let is_unique = create_index.unique;

                        // 添加到表定义
                        if let Some(table_def) = tables.get_mut(&table_name) {
                            table_def.indexes.push(TableIndex {
                                name: index_name,
                                columns,
                                is_primary: false,
                                is_unique,
                                index_type: if is_unique {
                                    Some("UNIQUE".to_string())
                                } else {
                                    Some("INDEX".to_string())
                                },
                            });
                        }
                    }
                }
            }
            Err(e) => {
                warn!("解析失败: {}", e);
            }
        }
    }

    Ok(())
}
```

#### 3. 集成到主解析流程

```rust
pub fn parse_sql_tables(sql_content: &str) -> Result<HashMap<String, TableDefinition>, DuckError> {
    let mut tables = HashMap::new();

    // 1. 解析 CREATE TABLE 语句
    let create_table_statements = extract_create_table_statements_with_regex(sql_content)?;
    // ... 解析逻辑

    // 2. 解析独立的 CREATE INDEX 语句
    parse_standalone_indexes(sql_content, &mut tables)?;

    Ok(tables)
}
```

## 测试验证

### 测试 SQL

```sql
USE test_db;

CREATE TABLE users (
  id BIGINT NOT NULL AUTO_INCREMENT,
  username VARCHAR(255) NOT NULL,
  email VARCHAR(255),
  PRIMARY KEY (id)
);

-- 独立的索引
create index idx_username
    on users (username);

create unique index uk_email
    on users (email);
```

### 预期结果

```rust
TableDefinition {
    name: "users",
    columns: [...],
    indexes: [
        TableIndex {
            name: "PRIMARY",
            columns: ["id"],
            is_primary: true,
            is_unique: true,
        },
        TableIndex {
            name: "idx_username",
            columns: ["username"],
            is_primary: false,
            is_unique: false,
        },
        TableIndex {
            name: "uk_email",
            columns: ["email"],
            is_primary: false,
            is_unique: true,
        },
    ],
}
```

## 支持的语法

### 基本索引

```sql
create index idx_name
    on table_name (column1);
```

### 多列索引

```sql
create index idx_multi
    on table_name (column1, column2, column3);
```

### 唯一索引

```sql
create unique index uk_name
    on table_name (column1);
```

### 带反引号

```sql
create index `idx_name`
    on `table_name` (`column1`, `column2`);
```

## 不支持的语法（需要扩展）

### 索引类型

```sql
-- BTREE, HASH 等
create index idx_name using btree
    on table_name (column1);
```

### 索引选项

```sql
-- KEY_BLOCK_SIZE, COMMENT 等
create index idx_name
    on table_name (column1)
    key_block_size = 1024
    comment 'my index';
```

### 函数索引

```sql
-- MySQL 8.0+
create index idx_func
    on table_name ((lower(column1)));
```

如果需要支持这些语法，可以扩展 `parse_standalone_indexes` 函数。

## 优势

1. **可靠性**：使用 sqlparser 库，正确解析 SQL 语法
2. **完整性**：支持 UNIQUE 索引、多列索引等
3. **可维护性**：代码清晰，易于扩展
4. **兼容性**：与现有的 CREATE TABLE 解析逻辑一致

## 注意事项

1. **性能**：对于大型 SQL 文件，解析可能需要一些时间
2. **错误处理**：解析失败的语句会被跳过并记录警告
3. **顺序**：必须先解析 CREATE TABLE，再解析 CREATE INDEX（因为索引需要关联到表）

## 总结

通过使用 sqlparser 库正确解析独立的 `CREATE INDEX` 语句，我们解决了：

- ✅ 索引定义被正确识别
- ✅ Diff 算法不再误判
- ✅ 不会生成错误的 DROP KEY 语句
- ✅ 支持多种索引语法

这是一个**可靠、可维护、可扩展**的解决方案。
