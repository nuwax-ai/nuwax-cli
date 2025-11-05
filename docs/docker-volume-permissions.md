# Docker 卷挂载权限问题详解

## 问题现象

MySQL 容器首次启动时报错：
```
[ERROR] [MY-010187] Could not open file '/var/lib/mysql/...' for error logging: Permission denied
```

## 根本原因

### Docker 的 UID/GID 映射机制

Docker 卷挂载时，**通过 UID/GID 数值进行权限映射，而不是用户名**：

```
宿主机文件系统                    Docker 容器内
├─ data/mysql/                   ├─ /var/lib/mysql/
│  ├─ owner: user (UID 1000)     │  ├─ 容器看到: UID 1000
│  └─ perms: 755                 │  └─ MySQL 进程: mysql (UID 999)
                                     └─ 无法写入！❌
```

### 问题流程

1. **Rust 程序创建目录**
   ```bash
   # 在宿主机上
   mkdir -p data/mysql
   # owner: 当前用户 (UID 1000)
   ```

2. **MySQL 容器启动**
   ```bash
   # 容器内 MySQL 进程
   User: mysql (UID 999)
   ```

3. **权限检查失败**
   ```bash
   # 容器看到的权限
   /var/lib/mysql/ -> owner: UID 1000
   # 当前进程
   mysql (UID 999) -> 无权限写入 UID 1000 的目录
   ```

## 解决方案

### 方案 1：使用 Init 容器（推荐）✅

在 `docker-compose.yml` 中添加一次性权限修复容器：

```yaml
services:
  mysql-permission-fix:
    image: busybox:1.36-uclibc
    volumes:
      - ./data/mysql:/var/lib/mysql
      - ./logs/mysql:/var/log/mysql
    command: |
      sh -c "
      echo '🔧 修复 MySQL 目录所有权...'
      chown -R 999:999 /var/lib/mysql /var/log/mysql
      chmod -R 755 /var/lib/mysql /var/log/mysql
      echo '✅ 权限修复完成'
      "
    restart: "no"

  mysql:
    depends_on:
      mysql-permission-fix:
        condition: service_completed_successfully
    # ... 其他配置
```

**优点**：
- 可靠：以 root 身份运行，保证 chown 成功
- 自动化：集成在 docker-compose 中
- 跨平台：在 Linux/macOS/Windows/WSL2 都能工作

### 方案 2：在 Rust 代码中处理（不推荐）❌

```rust
// 问题：需要 sudo 权限
std::process::Command::new("sudo")
    .args(["chown", "-R", "999:999", "data/mysql"])
    .status()?;
```

**缺点**：
- 需要 sudo 权限
- 跨平台兼容性差
- 用户体验不好

### 方案 3：使用 Docker 的 user 参数（有限制）

```yaml
mysql:
  user: "${UID}:${GID}"  # 使用当前用户
```

**缺点**：
- MySQL 官方镜像可能不支持非 999 用户
- 需要修改镜像内部配置

## 权限模式 vs 所有权

### chmod (权限模式) - 尽力而为

```bash
chmod 755 data/mysql/
```

**特点**：
- 在 WSL2 挂载的 Windows 分区可能失败
- 在网络文件系统（NFS/CIFS）可能失败
- **失败通常不影响容器运行**

**原因**：只要所有权正确，默认权限通常够用

### chown (所有权) - 必须成功

```bash
chown 999:999 data/mysql/
```

**特点**：
- **必须成功，否则容器无法访问**
- 需要 root 权限
- Docker 通过 UID 映射，所有权错误就无法访问

## 为什么 Rust 代码只做 chmod？

在 `DirectoryPermissionManager` 中，我们只做 `chmod`（权限模式），原因：

1. **不需要 sudo**：chmod 当前用户的文件不需要特殊权限
2. **尽力而为**：失败了也不影响容器运行
3. **所有权交给 Docker**：通过 init 容器以 root 身份处理 chown

```rust
// Rust 代码：只做 chmod（尽力而为）
self.set_directory_permission(&mysql_dir, 0o755)?;

// Docker init 容器：做 chown（必须成功）
chown -R 999:999 /var/lib/mysql
```

## 常见问题

### Q1: 为什么不在 Rust 代码中直接 chown？

**A**: 需要 sudo 权限，用户体验差，跨平台兼容性差。

### Q2: WSL2 上 chmod 失败怎么办？

**A**: 不用担心，只要 chown 成功（由 init 容器处理），chmod 失败通常不影响。

### Q3: 如何验证权限是否正确？

```bash
# 在宿主机上检查
ls -la data/mysql/
# 应该看到 owner 是 999:999（或 systemd-coredump）

# 在容器内检查
docker exec mysql ls -la /var/lib/mysql/
# 应该看到 owner 是 mysql:mysql
```

### Q4: 为什么容器内看到的用户名不同？

```bash
# 宿主机
ls -la data/mysql/
# drwxr-xr-x 999 systemd-coredump  # UID 999 映射到这个用户名

# 容器内
docker exec mysql ls -la /var/lib/mysql/
# drwxr-xr-x mysql mysql  # UID 999 映射到 mysql 用户名
```

**原因**：用户名只是 UID 的别名，不同系统对同一个 UID 可能有不同的用户名映射。

## MySQL 配置文件的特殊处理

### MySQL 的安全检查

MySQL 会**拒绝使用权限过高的配置文件**：

```bash
# MySQL 拒绝这些权限（会忽略配置文件）：
-rwxrwxrwx (777)  # world-writable ❌
-rwxrwxr-x (775)  # group-writable ❌
-rw-rw-r-- (664)  # group-writable ❌

# MySQL 接受这些权限：
-rw-r--r-- (644)  # 只有 owner 可写 ✅
-rw-r----- (640)  # 更严格 ✅
-r--r--r-- (444)  # 只读 ✅
```

### 错误示例

```yaml
# ❌ 错误：init 容器修改了配置文件权限
mysql-permission-fix:
  command: |
    chown -R 999:999 /var/lib/mysql /var/log/mysql
    chmod -R 755 /var/lib/mysql /var/log/mysql  # 这会影响配置文件！
  volumes:
    - ./config/mysql.cnf:/tmp/mysql.cnf  # 配置文件被改为 755
```

**结果**：MySQL 启动时会输出警告并忽略配置文件：
```
[Warning] World-writable config file '/etc/mysql/conf.d/mysql.cnf' is ignored
```

### 正确做法

```yaml
# ✅ 正确：只修改数据目录，不修改配置文件
mysql-permission-fix:
  command: |
    chown -R 999:999 /var/lib/mysql /var/log/mysql
    chmod -R 755 /var/lib/mysql /var/log/mysql
    # 不挂载配置文件，权限由 Rust 程序处理
  volumes:
    - ./data/mysql:/var/lib/mysql
    - ./logs/mysql:/var/log/mysql
    # 不挂载 mysql.cnf
```

```rust
// Rust 代码：确保配置文件是 644 权限
self.ensure_mysql_config_safe_permissions(&mysql_cnf)?;
```

### 验证配置文件权限

```bash
# 检查权限
ls -la config/mysql.cnf
# 应该看到：-rw-r--r-- (644)

# 检查 MySQL 是否使用了配置文件
docker exec mysql mysql -uroot -p -e "SHOW VARIABLES LIKE 'character_set%';"
# 如果看到 utf8mb4，说明配置生效了
```

## 总结

| 操作 | 在哪里做 | 是否必须 | 失败影响 | 特殊说明 |
|------|---------|---------|---------|---------|
| mkdir | Rust 代码 | 是 | 无法启动 | - |
| chmod (数据目录) | Rust 代码 | 否 | 通常无影响 | 755 即可 |
| chmod (配置文件) | Rust 代码 | 是 | MySQL 忽略配置 | 必须 644 |
| chown (数据目录) | Docker init 容器 | 是 | 容器无法写入 | 999:999 |
| chown (配置文件) | 不需要 | 否 | - | 保持当前用户 |

**最佳实践**：
1. Rust 代码负责：
   - 创建目录
   - 设置数据目录权限（chmod 755）
   - **设置配置文件权限（chmod 644）** ← 关键！
2. Docker init 容器负责：
   - 修改数据目录所有权（chown 999:999）
   - **不修改配置文件**
3. 配置文件保持当前用户所有权，但权限必须是 644
