# Nuwax CLI - Docker 服务智能管理工具

<div align="center">

![Rust](https://img.shields.io/badge/rust-1.75+-orange.svg)
![Docker](https://img.shields.io/badge/docker-20.10+-blue.svg)
![License](https://img.shields.io/badge/license-MIT%2FApache--2.0-green.svg)

一个专业的 **Docker 服务管理和升级工具**，提供完整的容器化服务生命周期管理。

</div>

## 🎯 项目概述

Nuwax CLI 是一个基于 Rust 开发的现代化 Docker 服务管理工具，专门设计用于简化容器化应用的部署、升级、备份和维护工作。通过智能化的升级策略和完善的安全机制，为企业级应用提供可靠的运维支持。

### ✨ 核心特性

- **🐋 智能Docker管理**：完整的Docker容器生命周期管理，支持启动、停止、重启、健康检查
- **🔄 多策略升级**：支持全量升级和增量升级，自动选择最优升级策略
- **💾 完整备份系统**：升级前自动备份，支持数据和应用配置的完整回滚
- **🏗️ 跨平台架构**：原生支持 x86_64 和 aarch64 架构，自动识别系统类型
- **📊 实时监控**：服务状态监控、健康检查、性能指标收集
- **🛡️ 安全可靠**：事务性升级操作，失败自动回滚，保障服务稳定性
- **⚡ 高性能**：基于 Rust 异步运行时，提供卓越的并发性能
- **🎨 现代化CLI**：直观的命令行界面，丰富的进度显示和状态提示

## 📁 项目架构

```
nuwax-cli/
├── 📦 nuwax-cli/          # CLI 主程序
│   ├── src/
│   │   ├── main.rs        # 程序入口点
│   │   ├── cli.rs         # 命令行定义
│   │   ├── app.rs         # 应用主逻辑
│   │   ├── commands/      # 命令处理器
│   │   └── docker_service/ # Docker 服务管理
│   └── Cargo.toml
├── 🔧 client-core/        # 核心业务库
│   ├── src/
│   │   ├── upgrade.rs     # 升级管理
│   │   ├── backup.rs      # 备份系统
│   │   ├── database.rs    # 数据库管理
│   │   ├── api.rs         # API 客户端
│   │   ├── container/     # Docker 操作
│   │   └── sql_diff/      # SQL 差异对比
│   └── Cargo.toml
├── 📚 docs/              # 技术文档
├── 📋 spec/              # 设计规范
├── 🗄️ data/              # 数据目录
└── 📄 README.md
```

## 🚀 快速开始

### 环境要求

- **Rust**: 1.75+ 
- **Docker**: 20.10+ 和 Docker Compose v2+
- **操作系统**: Windows 10+, macOS 10.15+, Linux (主流发行版)
- **内存**: 最少 512MB 可用内存

### 安装

#### 从源码构建

```bash
# 克隆仓库
git clone https://github.com/soddygo/nuwax-cli.git
cd nuwax-cli

# 构建项目
cargo build --release

# 安装到系统
cargo install --path .
```

#### 直接运行

```bash
# 开发模式运行
cargo run -- --help

# 生产模式运行
./target/release/nuwax-cli --help
```

### 基础使用

```bash
# 1. 初始化工作环境
nuwax-cli init

# 2. 检查服务状态
nuwax-cli status

# 3. 下载并部署服务
nuwax-cli upgrade

# 4. 启动 Docker 服务
nuwax-cli docker-service start

# 5. 创建备份
nuwax-cli backup

# 6. 查看可用更新
nuwax-cli check-update check
```

## 📖 详细功能

### Docker 服务管理

```bash
# 服务控制
nuwax-cli docker-service start        # 启动服务
nuwax-cli docker-service stop         # 停止服务  
nuwax-cli docker-service restart      # 重启服务
nuwax-cli docker-service status       # 查看状态

# 镜像管理
nuwax-cli docker-service load-images  # 加载镜像
nuwax-cli docker-service arch-info    # 架构信息

# 实用工具
nuwax-cli ducker                      # 启动 Docker TUI
```

### 升级和备份

```bash
# 升级管理
nuwax-cli upgrade                     # 下载升级包（兼容旧入口）
nuwax-cli upgrade --check             # 检查更新（兼容旧入口）
nuwax-cli upgrade --force             # 强制全量下载
nuwax-cli upgrade check               # 检查更新
nuwax-cli upgrade download            # 下载离线部署全量包
nuwax-cli auto-upgrade-deploy offline-deploy --archive ./docker-x86_64.zip --version 1.2.3

# 备份恢复
nuwax-cli backup                     # 创建备份
nuwax-cli list-backups              # 列出备份
nuwax-cli rollback                  # 回滚恢复
nuwax-cli rollback --force         # 强制回滚
```

### 自动化运维

```bash
# 自动备份
nuwax-cli auto-backup run           # 立即备份
nuwax-cli auto-backup status        # 备份状态

# 自动升级部署
nuwax-cli auto-upgrade-deploy run   # 自动升级部署
nuwax-cli auto-upgrade-deploy status # 查看配置
```

### 工具命令

```bash
# SQL 差异对比
nuwax-cli diff-sql old.sql new.sql --old-version 1.0 --new-version 2.0

# 缓存管理
nuwax-cli cache clear               # 清理缓存
nuwax-cli cache status             # 缓存状态
```

## 🛠️ 开发指南

### 开发环境设置

```bash
# 1. 安装 Rust 工具链
rustup update stable
rustup component add rustfmt clippy

# 2. 验证依赖
cargo check --workspace

# 3. 运行测试
cargo test --workspace

# 4. 代码格式化
cargo fmt --all

# 5. 静态分析
cargo clippy --workspace -- -D warnings
```

### 性能测试

```bash
# 运行性能基准测试
cargo bench

# 生成性能报告
cargo bench -- --output-format html
```

### 项目依赖管理

项目使用 Cargo workspace 管理多个子模块：

- **nuwax-cli**: CLI 接口层，依赖 client-core
- **client-core**: 核心业务逻辑，独立可测试

所有依赖版本在根 `Cargo.toml` 中统一管理，确保版本一致性。

## 🔧 配置说明

### 配置文件结构

项目使用 `config.toml` 配置文件，支持智能配置查找：

```toml
[versions]
docker_service = "1.0.0"
patch_version = "1.0.1"
full_version_with_patches = "1.0.1+1"

[docker]
compose_file = "docker/docker-compose.yml"
env_file = "docker/.env"

[backup]
storage_dir = "./backups"
max_backups = 10

[cache]
download_dir = "./cache"
max_cache_size = "1GB"

[updates]
auto_check = true
auto_backup = true
```

### 智能配置查找

配置文件查找顺序：
1. 命令行指定路径 (`--config`)
2. 当前目录 `./config.toml`
3. 向上级目录递归查找
4. 用户主目录 `~/.nuwax/config.toml`

## 🏗️ 系统架构

### 核心组件

- **CLI 接口层**: 命令解析、用户交互、进度显示
- **业务逻辑层**: 升级策略、备份管理、Docker 操作
- **数据访问层**: DuckDB 存储、配置管理、状态持久化
- **API 客户端**: 版本检查、文件下载、服务通信

### 设计模式

- **分层架构**: 清晰的职责分离和依赖管理
- **依赖注入**: 通过 `CliApp` 统一管理组件生命周期
- **策略模式**: 支持多种升级策略的灵活切换
- **Actor 模式**: 数据库操作的并发安全处理

## 🤝 贡献指南

我们欢迎社区贡献！请遵循以下步骤：

1. **Fork** 项目到您的 GitHub 账户
2. **创建** 功能分支 (`git checkout -b feature/amazing-feature`)
3. **提交** 您的更改 (`git commit -m 'Add some amazing feature'`)
4. **推送** 到分支 (`git push origin feature/amazing-feature`)
5. **创建** Pull Request

### 代码规范

- 使用 `cargo fmt` 格式化代码
- 使用 `cargo clippy` 进行静态检查
- 为新功能添加单元测试
- 更新相关文档

## 📄 许可证

本项目采用双许可证：

- [MIT License](LICENSE-MIT)
- [Apache License 2.0](LICENSE-APACHE)

您可以选择其中任一许可证使用本项目。

## 🔗 相关链接

- **项目主页**: https://docx.xspaceagi.com/
- **GitHub 仓库**: https://github.com/soddygo/nuwax-cli
- **问题反馈**: https://github.com/soddygo/nuwax-cli/issues
- **更新日志**: [CHANGELOG.md](CHANGELOG.md)

## 💬 支持

如果您在使用过程中遇到问题或有改进建议：

1. 查看 [文档](docs/) 获取详细信息
2. 搜索 [已知问题](https://github.com/soddygo/nuwax-cli/issues)
3. 创建新的 [Issue](https://github.com/soddygo/nuwax-cli/issues/new)
4. 参与 [讨论](https://github.com/soddygo/nuwax-cli/discussions)

---

<div align="center">

**[⬆ 回到顶部](#nuwax-cli---docker-服务智能管理工具)**

Made with ❤️ by the Nuwax Team

</div>
