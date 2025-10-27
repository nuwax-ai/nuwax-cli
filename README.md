# Duck CLI - 智能升级系统

## 🎯 项目概述

Duck CLI 是一个强大的容器化服务管理工具，提供完整的 Docker 服务生命周期管理，包括下载、部署、升级、备份和回滚功能。

### 🚀 核心特性

- **🏗️ 智能升级架构**：支持全量升级和增量升级，自动选择最优策略
- **💻 跨平台架构支持**：原生支持 x86_64 和 aarch64 架构
- **📦 增量升级**：减少 60-80% 的带宽使用，显著提升升级速度
- **🔄 智能策略选择**：根据版本差异自动确定升级方式
- **💾 完整备份回滚**：升级前自动备份，支持一键回滚
- **🖥️ 双界面模式**：CLI 命令行和图形界面（GUI）两种操作方式

## 📁 项目结构

```
duck_client/
├── nuwax-cli/           # 核心CLI工具
├── client-core/        # 共享核心库
├── cli-ui/            # Tauri GUI应用
├── docs/              # 技术文档
├── spec/              # 设计规范
└── CLI_USAGE.md       # 详细使用说明
```

## 🚀 快速开始

### 环境要求

- **Rust**: 1.75+
- **Node.js**: 22+ (GUI开发)
- **Docker**: 28.10+ 和 Docker Compose
- **操作系统**: Windows 10+, macOS 10.15+, Linux


### GUI 应用启动

```bash
# 切换到GUI目录
cd cli-ui

# 安装依赖
npm install

# 启动开发模式
npm run tauri dev

# 构建生产版本
npm run tauri build
```



**架构支持**：
- ✅ **x86_64**: Intel/AMD 处理器
- ✅ **aarch64**: Apple Silicon M1/M2, ARM 处理器
- ⚡ **自动检测**: 智能识别当前系统架构


## 🛠️ 开发指南

### 构建要求

```bash
# 确保Rust工具链正确安装
rustup update

# 验证编译环境
cargo check

# 运行测试套件
cargo test

# 性能基准测试
cargo bench
```

### 开发流程

1. **代码更改**：在对应模块目录下修改代码
2. **单元测试**：`cargo test -p <module-name>`
3. **集成测试**：`cargo test --test integration_tests`
4. **性能测试**：`cargo bench`
5. **文档更新**：更新相关 `.md` 文件

### 项目模块

- **`nuwax-cli`**: 命令行接口实现
- **`client-core`**: 核心业务逻辑、API客户端、数据库管理
- **`cli-ui`**: Tauri + React GUI 应用
