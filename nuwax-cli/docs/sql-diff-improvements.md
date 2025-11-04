# SQL 差异生成改进说明

## 问题描述

在执行 `nuwax-cli auto-upgrade-deploy run` 命令时，数据库升级步骤报错：
- 错误信息：`Table 'agent_config' already exists`
- 原因：`generate_live_schema_diff` 函数没有正确获取 MySQL 中已存在的表结构
- 日志显示：从在线数据库获取了 0 个表，导致所有表都被认为是"新增表"

## 根本原因

`init_mysql.sql` 文件包含多个数据库的建表语句，但 `fetch_live_schema_diff` 只查询 `agent_platform` 数据库的表。如果 SQL 文件包含其他数据库的表定义，会导致对比不准确。

## 解决方案

### 方案概述

复用 `client-core/src/sql_diff/parser.rs` 中已有的 `extract_create_table_statements_with_regex` 函数，该函数已经实现了：
1. ✅ 查找 USE 语句
2. ✅ 从 USE 语句之后开始解析
3. ✅ 提取所有 CREATE TABLE 语句
4. ✅ 处理括号平衡和字符串转义

### 代码改进

**之前的实现：**
```rust
// 需要手动提取 USE 语句之后的内容
let full_sql_content = fs::read_to_string(&new_sql_path)?;
let new_sql_content = extract_database_ddl(&full_sql_content, "agent_platform")?;
```

**改进后的实现：**
```rust
// 直接读取完整 SQL，parse_sql_tables 会自动处理 USE 语句
let new_sql_content = fs::read_to_string(&new_sql_path)?;
// parse_sql_tables 内部会自动调用 extract_create_table_statements_with_regex
```

### 关键改进点

1. **删除冗余代码**：移除了 `extract_database_ddl` 函数（约 70 行代码）
2. **复用现有逻辑**：直接使用 `parse_sql_tables`，它内部已经处理了 USE 语句
3. **更好的维护性**：只需要维护一处 SQL 解析逻辑
4. **更健壮**：`extract_create_table_statements_with_regex` 已经过充分测试

## parser.rs 中的 USE 语句处理

`extract_create_table_statements_with_regex` 函数的处理逻辑：

```rust
// 创建正则表达式来匹配 USE 语句
let use_regex = Regex::new(r"(?i)^\s*USE\s+[^;]+;\s*$")?;

// 查找 USE 语句
for (line_idx, line) in lines.iter().enumerate() {
    if use_regex.is_match(line) {
        start_parsing_from_line = line_idx + 1; // 从下一行开始
        break;
    }
}
```

支持的格式：
- `USE database_name;` - 标准格式
- `USE `database_name`;` - 带反引号
- `use database_name;` - 小写（大小写不敏感）
- `USE  database_name  ;` - 多个空格

## 测试覆盖

测试文件：`nuwax-cli/tests/test_extract_database_ddl.rs`

测试用例：
1. ✅ 各种 USE 语句格式
2. ✅ 正则表达式多行模式匹配
3. ✅ USE 语句后的内容提取
4. ✅ 与 `parse_sql_tables` 的集成测试

## 使用建议

重新测试升级命令：
```bash
cd /Volumes/soddy/20251031-docker
NUWAX_CLI_ENV=test nuwax-cli auto-upgrade-deploy run --port 8091
```

预期结果：
- ✅ 正确识别已存在的表
- ✅ 只生成实际需要的差异 SQL
- ✅ 不会再报 `Table already exists` 错误

## 相关文件

- `nuwax-cli/src/commands/auto_upgrade_deploy.rs` - 主要改进文件
- `client-core/src/sql_diff/parser.rs` - 复用的 SQL 解析逻辑
- `nuwax-cli/tests/test_extract_database_ddl.rs` - 测试文件
