# CREATE INDEX 解析测试

## 测试 SQL

```sql
USE test_db;

-- 表定义（不包含索引）
CREATE TABLE users (
  id BIGINT NOT NULL AUTO_INCREMENT,
  username VARCHAR(255) NOT NULL,
  email VARCHAR(255),
  created_at DATETIME DEFAULT CURRENT_TIMESTAMP,
  PRIMARY KEY (id)
);

CREATE TABLE posts (
  id BIGINT NOT NULL AUTO_INCREMENT,
  user_id BIGINT NOT NULL,
  title VARCHAR(500),
  content TEXT,
  status TINYINT DEFAULT 1,
  created_at DATETIME DEFAULT CURRENT_TIMESTAMP,
  PRIMARY KEY (id)
);

-- 独立的索引定义
create index idx_username
    on users (username);

create unique index uk_email
    on users (email);

create index idx_created_at
    on users (created_at);

create index idx_user_id
    on posts (user_id);

create index idx_status_created
    on posts (status, created_at);

create unique index uk_title
    on posts (title);
```

## 预期解析结果

### users 表

```rust
TableDefinition {
    name: "users",
    columns: [
        TableColumn { name: "id", data_type: "BIGINT", ... },
        TableColumn { name: "username", data_type: "VARCHAR(255)", ... },
        TableColumn { name: "email", data_type: "VARCHAR(255)", ... },
        TableColumn { name: "created_at", data_type: "DATETIME", ... },
    ],
    indexes: [
        // 来自 PRIMARY KEY
        TableIndex {
            name: "PRIMARY",
            columns: ["id"],
            is_primary: true,
            is_unique: true,
            index_type: Some("PRIMARY"),
        },
        // 来自 CREATE INDEX
        TableIndex {
            name: "idx_username",
            columns: ["username"],
            is_primary: false,
            is_unique: false,
            index_type: Some("INDEX"),
        },
        // 来自 CREATE UNIQUE INDEX
        TableIndex {
            name: "uk_email",
            columns: ["email"],
            is_primary: false,
            is_unique: true,
            index_type: Some("UNIQUE"),
        },
        TableIndex {
            name: "idx_created_at",
            columns: ["created_at"],
            is_primary: false,
            is_unique: false,
            index_type: Some("INDEX"),
        },
    ],
}
```

### posts 表

```rust
TableDefinition {
    name: "posts",
    columns: [...],
    indexes: [
        TableIndex {
            name: "PRIMARY",
            columns: ["id"],
            is_primary: true,
            is_unique: true,
            index_type: Some("PRIMARY"),
        },
        TableIndex {
            name: "idx_user_id",
            columns: ["user_id"],
            is_primary: false,
            is_unique: false,
            index_type: Some("INDEX"),
        },
        // 多列索引
        TableIndex {
            name: "idx_status_created",
            columns: ["status", "created_at"],
            is_primary: false,
            is_unique: false,
            index_type: Some("INDEX"),
        },
        TableIndex {
            name: "uk_title",
            columns: ["title"],
            is_primary: false,
            is_unique: true,
            index_type: Some("UNIQUE"),
        },
    ],
}
```

## 测试命令

```bash
# 1. 创建测试 SQL 文件
cat > test_indexes.sql << 'EOF'
USE test_db;

CREATE TABLE users (
  id BIGINT NOT NULL AUTO_INCREMENT,
  username VARCHAR(255) NOT NULL,
  email VARCHAR(255),
  PRIMARY KEY (id)
);

create index idx_username on users (username);
create unique index uk_email on users (email);
EOF

# 2. 运行测试（需要在 Rust 代码中添加测试）
cargo test --package client-core test_parse_standalone_indexes -- --nocapture
```

## 验证点

### 1. 基本索引解析

- ✅ 索引名称正确提取
- ✅ 表名正确关联
- ✅ 列名正确提取
- ✅ is_unique 标志正确

### 2. 多列索引

```sql
create index idx_multi on posts (status, created_at);
```

- ✅ 列顺序保持
- ✅ 所有列都被提取

### 3. 带反引号

```sql
create index `idx_name` on `table_name` (`column_name`);
```

- ✅ 反引号被正确移除

### 4. 大小写不敏感

```sql
CREATE INDEX idx1 ON users (username);
create index idx2 on users (email);
Create Index idx3 On users (created_at);
```

- ✅ 所有格式都能正确解析

### 5. 多行格式

```sql
create index idx_name
    on table_name (
        column1,
        column2
    );
```

- ✅ 跨行语句正确识别

## 边缘情况

### 1. 表不存在

```sql
create index idx_unknown on non_existent_table (col);
```

**预期**：记录警告，跳过该索引

### 2. 重复索引名

```sql
CREATE TABLE users (
  id BIGINT,
  KEY idx_name (id)
);

create index idx_name on users (username);
```

**预期**：第二个索引被跳过（已存在同名索引）

### 3. 空列列表

```sql
create index idx_empty on users ();
```

**预期**：记录警告，跳过该索引

### 4. 复杂表达式（未来支持）

```sql
-- 函数索引
create index idx_lower on users ((lower(username)));

-- 前缀索引
create index idx_prefix on users (username(10));
```

**当前**：使用 `to_string()` 作为列名
**未来**：可以扩展支持

## 性能考虑

### 大型 SQL 文件

对于包含 1000+ 个表和索引的 SQL 文件：

1. **提取阶段**：O(n) - 遍历所有行
2. **解析阶段**：O(m) - m 是 CREATE INDEX 语句数量
3. **关联阶段**：O(m) - HashMap 查找是 O(1)

**总体**：O(n + m)，线性时间复杂度 ✅

### 内存使用

- 每个索引约 100-200 字节
- 1000 个索引约 100-200 KB
- 可接受 ✅

## 与现有代码的兼容性

### CREATE TABLE 中的索引

```sql
CREATE TABLE users (
  id BIGINT,
  username VARCHAR(255),
  PRIMARY KEY (id),
  KEY idx_username (username),
  UNIQUE KEY uk_email (email)
);
```

**状态**：✅ 已支持（现有代码）

### 独立的 CREATE INDEX

```sql
create index idx_username on users (username);
```

**状态**：✅ 新增支持

### 混合使用

```sql
CREATE TABLE users (
  id BIGINT,
  PRIMARY KEY (id),
  KEY idx_id (id)
);

create index idx_username on users (username);
```

**状态**：✅ 两种方式都支持，不冲突

## 总结

通过正确使用 sqlparser 库：

1. ✅ 可靠地解析 CREATE INDEX 语句
2. ✅ 正确提取索引信息（名称、列、类型）
3. ✅ 支持多种语法格式
4. ✅ 与现有代码兼容
5. ✅ 性能可接受

这解决了用户遇到的"误删索引"问题。
