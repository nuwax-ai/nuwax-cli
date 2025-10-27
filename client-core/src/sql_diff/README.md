# SQL差异生成器模块

这个模块提供了智能的MySQL数据库架构差异生成功能，能够分析两个版本的SQL文件并生成可执行的差异SQL。

## 模块结构

```
sql_diff/
├── mod.rs              # 模块入口，重新导出公共接口
├── types.rs            # 数据结构定义（表、列、索引）
├── parser.rs           # SQL解析器，解析CREATE TABLE语句
├── generator.rs        # SQL生成器，生成CREATE TABLE和差异SQL
├── differ.rs           # 差异比较器，比较两个版本的表结构差异
├── tests.rs            # 单元测试
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

运行测试：
```bash
cargo test --bin server -- --nocapture
```

## 设计原则

1. **模块化**：每个文件职责单一，便于维护和扩展
2. **可测试性**：所有核心功能都有对应的单元测试
3. **可扩展性**：易于添加新的SQL语法支持
4. **错误处理**：完整的错误处理和日志记录 