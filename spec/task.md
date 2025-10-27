# Duck Client 开发任务规划

基于 `Design.md` 的系统设计，本文档按优先级规划客户端的开发任务。重点关注：**客户端升级**、**Docker服务升级**、**数据备份**和**CLI命令**功能。

## 项目结构

```
duck_client/
├── Cargo.toml           # Workspace配置
├── config.toml          # 用户配置文件
├── duck-tauri  # tauri桌面app ui, 技术栈: deno + react + JavaScript 
├── client-core # 核心业务逻辑库
├── duck-cli   # 命令行,同时提供 lib,可以供 duck-tauri 使用.不需要UI的时候,只使用 cli command ,编译此模块
└── spec/
    ├── Design.md 设计文档
    └── task.md   开发任务文档
```

## 阶段一：CLI核心功能实现 (当前阶段)

### 1. Docker服务升级功能 (优先级: 高)

#### 1.1 API客户端升级下载功能 ✅
- [x] 修改 `client-core/src/api.rs`，实现从指定服务器下载docker.zip
  - [x] 支持从 `192.168.2.138` 下载全量更新包
  - [x] 接口: `GET /downloads/docker/services/full/latest`
  - [x] 实现下载进度显示（通过reqwest流式下载）
  - [x] 支持断点续传功能

#### 1.2 升级管理器完善 ✅
- [x] 完善 `client-core/src/upgrade.rs` 中的 UpgradeManager
  - [x] 实现完整的升级流程：下载 -> 备份 -> 停止服务 -> 替换文件 -> 启动服务
  - [x] 智能文件合并：保留原有 `docker/data` 目录，不覆盖用户数据
  - [x] 错误处理和回滚机制
  - [x] 升级过程的详细日志记录

#### 1.3 文件操作和备份逻辑 ✅
- [x] 增强 `client-core/src/backup.rs` 备份功能
  - [x] 支持升级前的完整系统备份
  - [x] 实现数据目录单独备份策略
  - [x] 备份文件的压缩和存储管理

#### 1.4 Docker服务管理 ✅
- [x] 完善 `client-core/src/docker.rs` Docker管理功能
  - [x] 实现服务状态检查
  - [x] 服务启动/停止的可靠性保障
  - [x] 服务健康检查和验证

### 2. CLI命令完善 (优先级: 中)

#### 2.1 升级命令实现 ✅
- [x] 在 `duck-cli/src/lib.rs` 中完善 `upgrade` 命令
  - [x] 实现完整的CLI交互流程
  - [x] 添加进度条显示
  - [x] 用户确认和安全提示
  - [x] 详细的操作日志输出

#### 2.2 其他CLI命令 ✅
- [x] 完善 `status` 命令：显示服务状态和版本信息
- [x] 完善 `backup` 命令：手动创建备份
- [x] 完善 `list-backups` 命令：列出所有备份
- [x] 完善 `rollback` 命令：从备份恢复

### 3. 配置和数据库 (优先级: 中)

#### 3.1 配置管理 ✅
- [x] 完善 `client-core/src/config.rs` 配置管理
  - [x] 支持服务器地址配置
  - [x] 备份目录配置
  - [x] Docker compose文件路径配置
  - [x] 添加服务下载URL配置

#### 3.2 数据库操作 ✅
- [x] 完善 `client-core/src/database.rs` 数据库操作
  - [x] 升级历史记录
  - [x] 备份记录管理
  - [x] 客户端身份管理

## 阶段二：错误处理和优化 (后续阶段)

### 1. 错误处理完善
- [ ] 统一错误类型定义
- [ ] 详细的错误信息和用户友好提示
- [ ] 操作失败时的自动回滚机制

### 2. 性能优化
- [ ] 下载过程的断点续传
- [ ] 并发操作优化
- [ ] 内存使用优化

### 3. 安全性增强
- [ ] 文件完整性校验
- [ ] 下载包签名验证
- [ ] 权限检查和安全提示



## 当前开发重点

**第一步：实现Docker服务升级的核心流程**
1. 从指定服务器下载docker.zip
2. 停止现有Docker服务
3. 创建备份
4. 智能替换文件（保留data目录）
5. 启动新服务
6. 验证服务运行状态

**技术要点：**
- 使用reqwest库实现文件下载
- 使用zip库处理压缩包
- 智能文件合并，避免覆盖用户数据
- 完整的错误处理和回滚机制

## ✅ 阶段一完成总结

**已完成的核心功能：**

1. **Docker服务升级核心流程** ✅
   - 从 `http://192.168.2.138/downloads/docker/services/full/latest` 下载docker.zip
   - 智能文件合并，保留 `docker/data` 目录的用户数据
   - 完整的升级流程：备份 → 停止服务 → 下载 → 解压 → 启动服务
   - 支持断点续传和进度显示

2. **CLI命令接口** ✅
   - `cargo run -p duck-cli -- upgrade` - 执行服务升级
   - `cargo run -p duck-cli -- status` - 查看服务状态
   - `cargo run -p duck-cli -- backup` - 手动备份
   - `cargo run -p duck-cli -- list-backups` - 列出备份
   - `cargo run -p duck-cli -- rollback <ID>` - 从备份恢复

3. **配置和数据管理** ✅
   - 配置文件支持服务器地址和接口路径配置
   - 数据库记录升级历史和备份信息
   - 客户端身份管理

4. **安全和可靠性** ✅
   - 升级前自动备份
   - 智能数据保护（保留用户数据）
   - 详细的错误处理和日志记录
   - 支持强制模式和跳过备份选项

**使用方法：**
详见 `CLI_USAGE.md` 文档

**下一步：**
等待服务端准备就绪后，即可进行实际的升级测试。
