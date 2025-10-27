# GitHub Actions 使用说明

这个项目包含两个 GitHub Actions 工作流程：

## 📋 CI 工作流程 (ci.yml)

### 触发条件
- 推送到 `main` 或 `dev` 分支
- 对 `main` 或 `dev` 分支创建 Pull Request

### 包含的检查
1. **代码检查** - `cargo check`
2. **测试运行** - `cargo test`
3. **代码格式** - `cargo fmt --check`
4. **代码规范** - `cargo clippy`
5. **构建验证** - 在三个平台上构建验证

### 使用方法
- 每次推送代码或创建 PR 时自动运行
- 确保代码质量和跨平台兼容性

## 🚀 Release 工作流程 (release.yml)

### 触发条件
1. **自动触发**：推送以 `v` 开头的标签（如 `v1.0.0`）
2. **手动触发**：在 GitHub Actions 页面手动运行

### 构建的二进制文件

#### 🍎 macOS
- `duck-cli-macos-universal.tar.gz` - **推荐** 通用二进制文件
  - 同时支持 Intel (x86_64) 和 Apple Silicon (ARM64)
- `duck-cli-macos-amd64.tar.gz` - Intel 专用版本
- `duck-cli-macos-arm64.tar.gz` - Apple Silicon 专用版本

#### 🐧 Linux
- `duck-cli-linux-amd64.tar.gz` - x86_64 架构
- `duck-cli-linux-arm64.tar.gz` - ARM64 架构

#### 🪟 Windows
- `duck-cli-windows-amd64.zip` - x86_64 架构
- `duck-cli-windows-arm64.zip` - ARM64 架构

### 发布新版本

#### 方法一：Git 标签（推荐）
```bash
# 创建并推送标签
git tag v1.0.0
git push origin v1.0.0
```

#### 方法二：手动触发
1. 进入 GitHub 仓库
2. 点击 "Actions" 标签
3. 选择 "Release" 工作流程
4. 点击 "Run workflow"
5. 输入标签名称（如 `v1.0.0` 或 `nightly`）

### 版本号规则
建议使用语义化版本号：
- `v1.0.0` - 主要版本
- `v1.1.0` - 功能更新
- `v1.0.1` - 修复版本
- `nightly` - 开发版本

## 🔧 本地测试

在推送前可以本地运行这些检查：

```bash
# 代码检查
cargo check

# 运行测试
cargo test

# 格式化代码
cargo fmt

# 检查代码规范
cargo clippy

# 构建发布版本
cargo build --release
```

## 📦 自动化流程

1. **开发** → 推送到 `dev` 分支 → CI 检查
2. **合并** → 创建 PR 到 `main` → CI 检查
3. **发布** → 推送标签 → 自动构建和发布

## 🚨 注意事项

1. **标签命名**：必须以 `v` 开头（如 `v1.0.0`）
2. **权限**：需要仓库的 `GITHUB_TOKEN` 权限（自动提供）
3. **构建时间**：完整构建大约需要 15-20 分钟
4. **缓存**：使用 Cargo 缓存加速构建过程

## 🔍 状态检查

在仓库主页可以看到：
- [![CI](https://github.com/soddygo/duck_client/workflows/CI/badge.svg)](https://github.com/soddygo/duck_client/actions)
- 最新的构建状态和测试结果

## 🛠️ 故障排除

### 构建失败
1. 检查 Rust 代码是否编译通过
2. 确保所有测试都能通过
3. 检查代码格式和 clippy 警告

### 发布失败
1. 确保标签格式正确（以 `v` 开头）
2. 检查是否有权限问题
3. 查看具体的错误日志

### 跨编译问题
- Linux ARM64 使用 `cross` 工具
- Windows 和 macOS 在原生环境构建
- 如有问题，检查依赖是否支持目标架构 