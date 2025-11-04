# MySQL 配置文件权限快速参考

## 问题症状

✅ **MySQL 初始化 SQL 执行成功**  
❌ **但配置文件（如字符集）不生效**

## 根本原因

MySQL 拒绝使用权限过高的配置文件（安全特性）

## MySQL 权限检查规则

```bash
# ❌ MySQL 会忽略（并输出警告）
-rwxrwxrwx (777)  # world-writable
-rwxrwxr-x (775)  # group-writable
-rw-rw-r-- (664)  # group-writable

# ✅ MySQL 会使用
-rw-r--r-- (644)  # 推荐
-rw-r----- (640)  # 更严格
-r--r--r-- (444)  # 只读
```

## 错误日志示例

```
[Warning] World-writable config file '/etc/mysql/conf.d/mysql.cnf' is ignored
```

## 解决方案

### 1. Docker Compose 配置

```yaml
mysql-permission-fix:
  volumes:
    - ./data/mysql:/var/lib/mysql      # ✅ 需要 chown
    - ./logs/mysql:/var/log/mysql      # ✅ 需要 chown
    # ❌ 不要挂载配置文件到 init 容器
  command: |
    chown -R 999:999 /var/lib/mysql /var/log/mysql
    chmod -R 755 /var/lib/mysql /var/log/mysql
    # ✅ 不修改配置文件权限

mysql:
  volumes:
    - ./config/mysql.cnf:/etc/mysql/conf.d/mysql.cnf:ro  # ✅ 只读挂载
```

### 2. Rust 代码

```rust
// ✅ 在启动容器前，确保配置文件是 644
self.ensure_mysql_config_safe_permissions(&mysql_cnf)?;
```

## 验证步骤

### 1. 检查文件权限

```bash
ls -la config/mysql.cnf
# 期望输出：-rw-r--r-- 1 user group ... mysql.cnf
```

### 2. 检查 MySQL 日志

```bash
docker logs mysql 2>&1 | grep -i "world-writable\|ignored"
# 应该没有输出
```

### 3. 验证配置生效

```bash
# 检查字符集配置
docker exec mysql mysql -uroot -p${MYSQL_ROOT_PASSWORD} -e \
  "SHOW VARIABLES LIKE 'character_set%';"

# 期望输出：
# character_set_server    | utf8mb4
# character_set_database  | utf8mb4
```

## 权限分工表

| 文件/目录 | 所有权 | 权限 | 谁负责 |
|----------|--------|------|--------|
| `data/mysql/` | 999:999 | 755 | Init 容器 (chown) + Rust (chmod) |
| `logs/mysql/` | 999:999 | 755 | Init 容器 (chown) + Rust (chmod) |
| `config/mysql.cnf` | 当前用户 | 644 | Rust (chmod only) |

## 常见错误

### ❌ 错误 1：Init 容器修改了配置文件

```yaml
mysql-permission-fix:
  volumes:
    - ./config/mysql.cnf:/tmp/mysql.cnf:rw  # ❌ 不要这样做
  command: chmod 755 /tmp/mysql.cnf         # ❌ MySQL 会忽略
```

### ❌ 错误 2：配置文件挂载为可写

```yaml
mysql:
  volumes:
    - ./config/mysql.cnf:/etc/mysql/conf.d/mysql.cnf:rw  # ❌ 应该用 :ro
```

### ❌ 错误 3：忘记在 Rust 代码中设置权限

```rust
// ❌ 只创建文件，没有设置权限
fs::write("config/mysql.cnf", content)?;

// ✅ 创建后立即设置权限
fs::write("config/mysql.cnf", content)?;
self.ensure_mysql_config_safe_permissions(&mysql_cnf)?;
```

## 快速修复命令

如果配置文件权限错误，手动修复：

```bash
# 修复权限
chmod 644 config/mysql.cnf

# 重启 MySQL 容器
docker-compose restart mysql

# 验证配置生效
docker exec mysql mysql -uroot -p -e "SHOW VARIABLES LIKE 'character_set%';"
```

## 记住这个规则

> **数据目录需要 chown（999:999），配置文件只需要 chmod（644）**
> 
> - 数据目录：容器内进程需要写入 → 必须 chown
> - 配置文件：容器内进程只需读取 → 只需 chmod，保持当前用户所有权
