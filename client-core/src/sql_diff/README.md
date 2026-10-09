# SQL差异生成器模块

这个模块提供了智能的MySQL数据库架构差异生成功能，能够分析两个版本的SQL文件并生成可执行的差异SQL。
支持**多库**：`generate_live_schema_diff_multi` 按模板清单逐库对比在线架构（Live Diff），用于自动升级部署。

## 模块结构

```
sql_diff/
├── mod.rs              # 模块入口，重新导出公共接口
├── types.rs            # 数据结构定义（表、列、索引、SchemaTemplate、多库结果）
├── parser.rs           # SQL解析器：CREATE TABLE 解析 + parse_schema_template（多库模板）
├── generator.rs        # SQL生成器：CREATE TABLE 渲染 + 多库 Live Diff 组装
├── differ.rs           # 差异比较器，比较两个版本的表结构差异
├── tests.rs            # 单元测试
├── backtick_normalization_tests.rs  # 反引号标准化测试
├── schema_template_tests.rs         # 多库模板解析与组装测试
└── README.md           # 本文件
```

## 核心功能

### 1. SQL解析
- 解析CREATE TABLE语句
- 提取表名、列定义、索引定义
- 支持ENGINE、CHARSET等表选项

### 2. 差异检测
- **表级别差异**：新增表、删除表
- **列级别差异**：新增列、删除列、修改列
- **索引级别差异**：新增索引、删除索引、修改索引

### 3. SQL生成
- 生成可执行的MySQL差异SQL
- 支持ALTER TABLE语句
- 包含详细的注释和时间戳

### 4. 多库 Live Diff（自动升级部署路径）

模板清单见 `constants.rs` 的 `SCHEMA_SQL_FILES`（一个文件一个库）：

- `parse_schema_template`：按文件内**唯一** `USE <db>;` 归属库；提取
  `CREATE DATABASE`/`GRANT`/`FLUSH` 原句透传（幂等、保留 collation 细节）；
  纯数据文件（无 USE/无 CREATE TABLE）会被 Fail Fast 拦截
- `generate_live_schema_diff_multi`：逐库 `fetch_live_schema_with_sql(schema)`
  （库不存在/空库 → 空表集 → 自然生成"建库 + 全量建表"，即存量机器补建路径），
  差异计算复用单库 differ；组装为单份 diff，每库一段：
  `建库/授权原句 → USE db → 该库 DDL`（仅有可执行变更的库才输出完整段；
  仅警告的库只输出注释，不产生任何可执行语句）
- 执行须用 **root 管理连接**（`MySqlConfig::for_container_admin`，无默认库），
  因为建库/授权/跨库 DDL 超出应用账号权限

## 使用示例

```rust
use crate::sql_diff::generate_schema_diff;

// 比较两个版本的SQL
let from_sql = "CREATE TABLE users (id INT PRIMARY KEY);";
let to_sql = "CREATE TABLE users (id INT PRIMARY KEY, name VARCHAR(255));";

let (diff_sql, description) = generate_schema_diff(
    Some(from_sql),
    to_sql,
    Some("1.0.0"),
    "1.1.0"
)?;

println!("差异SQL: {}", diff_sql);
// 输出: ALTER TABLE `users` ADD COLUMN `name` VARCHAR(255);
```

## 测试

模块包含完整的单元测试，覆盖以下场景：

- `test_simple_diff` - 测试添加列的差异生成
- `test_parse_table` - 测试SQL表解析功能
- `test_add_table` - 测试新增表的差异生成
- `test_drop_table` - 测试删除表的差异生成
- `test_no_changes` - 测试无变化时的处理
- `test_modify_column` - 测试列修改的差异生成
- `test_add_index` - 测试索引添加的差异生成
- `parse_schema_template_*` - 多库模板解析与 Fail Fast 语义
- `assemble_*` - 多库组装（preamble/USE 段渲染规则）

运行测试：
```bash
cargo nextest run -p client-core sql_diff
```

## 设计原则

1. **模块化**：每个文件职责单一，便于维护和扩展
2. **可测试性**：所有核心功能都有对应的单元测试
3. **可扩展性**：易于添加新的SQL语法支持
4. **错误处理**：完整的错误处理和日志记录
