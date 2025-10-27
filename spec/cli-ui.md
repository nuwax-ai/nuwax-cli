# Duck CLI GUI 界面技术设计文档

## 项目概述

本文档描述如何在现有的 duck_client workspace 中创建一个 Tauri GUI 应用，通过图形界面调用 duck-cli 命令行工具，实现 Docker 服务的可视化管理。

### 项目结构
本项目采用 Cargo workspace 结构，新增的 GUI 模块与现有的 CLI 模块并存：

```
duck_client/                    # Workspace 根目录
├── Cargo.toml                 # Workspace 配置
├── duck-cli/                  # 现有 CLI 模块
├── client-core/               # 共享核心库
├── client-ui/                 # 现有 UI 模块 (Deno + React)
└── cli-ui/                    # 新增 Tauri GUI 模块 ← 本文档重点
    ├── package.json           # npm 依赖管理
    ├── src/                   # React + TypeScript 前端
    │   ├── App.tsx
    │   ├── main.tsx
    │   └── components/
    └── src-tauri/             # Rust 后端
        ├── Cargo.toml
        ├── src/
        │   ├── main.rs
        │   ├── lib.rs
        │   └── commands/
        ├── tauri.conf.json
        └── capabilities/
```

### 技术栈选择
基于 `cargo create-tauri-app` 创建，具体配置：

- **项目名称**: `cli-ui`
- **应用标识**: `com.soddy.cli-ui`
- **前端**: React + TypeScript
- **包管理器**: npm
- **后端**: Rust (Tauri 2.0)
- **构建工具**: Tauri CLI

## 核心需求

### 功能需求
1. **工作目录管理**：顶部工作目录设置，首次使用引导设置，所有命令在此目录下执行
2. **CLI 工具集成**：自动下载并集成最新的 duck-cli 工具
3. **分割式界面**：上半部分为操作面板，下半部分为终端窗口
4. **核心功能按钮**：提供主要 Docker 服务管理功能的 GUI 操作
5. **实时日志**：在终端窗口中显示 duck-cli 命令的输出
6. **应用更新**：支持 Tauri updater 插件进行应用自动更新

### 非功能需求
- 跨平台支持（Windows、macOS、Linux）
- 响应式界面设计
- 实时命令执行状态反馈
- 错误处理和用户友好的错误提示

## 架构设计

### 整体架构

```
┌─────────────────────────────────────────┐
│           Tauri Frontend (React)        │
├─────────────────────────────────────────┤
│  ┌──────── Working Directory Bar ─────────┐ │
│  │ 📁 /path/to/working/dir [更改目录]   │ │
│  └───────────────────────────────────────┘ │
│  ┌───────────── GUI Panel ─────────────┐ │
│  │ • 初始化     • 服务启停            │ │
│  │ • 下载更新   • 版本检测            │ │  
│  │ • 一键部署   • 备份回滚            │ │
│  │ • 应用更新                         │ │
│  └───────────────────────────────────────┘ │
│  ┌──────────── Terminal Panel ────────────┐ │
│  │ • 命令输出日志                     │ │
│  │ • 交互式命令输入                   │ │
│  │ • 实时状态显示                     │ │
│  └───────────────────────────────────────┘ │
├─────────────────────────────────────────┤
│           Tauri Backend (Rust)          │
│  • Command 执行                        │
│  • 文件系统操作                       │
│  • CLI 工具管理                       │
│  • 工作目录管理                       │
│  • 进程管理                           │
└─────────────────────────────────────────┘
```

## 技术实现方案

### 1. CLI 工具集成策略

我们提供两种CLI工具集成方案，根据不同使用场景选择：

#### 1.1 Tauri Sidecar 集成方案（推荐）

基于项目的GitHub自动化构建流程（https://github.com/soddygo/duck_client/releases），我们采用Tauri Sidecar方式将duck-cli工具直接打包到应用中，确保版本一致性和离线可用性。

**适用场景**：
- 生产环境部署
- 离线使用需求
- 版本严格控制
- 一站式安装包

#### 1.2 Tauri Shell 插件方案（可选）

基于[Tauri Shell插件](https://tauri.app/plugin/shell/)，通过系统Shell执行已安装的duck-cli工具，提供更灵活的命令执行方式。

**适用场景**：
- 开发和测试环境
- 用户已安装duck-cli
- 需要执行系统级辅助命令
- 灵活的版本管理

**Sidecar配置** (tauri.conf.json)：
```json
{
  "bundle": {
    "externalBin": [
      {
        "name": "duck-cli",
        "src": "binaries/duck-cli",
        "targets": "all"
      }
    ]
  },
  "plugins": {
    "shell": {
      "sidecar": true,
      "scope": [
        {
          "name": "duck-cli",
          "sidecar": true,
          "args": true
        }
      ]
    }
  }
}
```

**权限配置** (capabilities/default.json)：
```json
{
  "permissions": [
    "shell:allow-execute",
    "shell:allow-kill",
    "dialog:default",
    "dialog:allow-ask",
    "dialog:allow-confirm", 
    "dialog:allow-message",
    "dialog:allow-open",
    "dialog:allow-save",
    "fs:default",
    "fs:allow-read-text-file",
    "fs:allow-write-text-file",
    "fs:allow-read-dir",
    "fs:allow-create-dir",
    "fs:allow-exists",
    "fs:allow-metadata",
    "fs:scope-appdata",
    "fs:scope-appdata-recursive",
    "fs:scope-applog",
    "fs:scope-applog-recursive",
    {
      "identifier": "shell:allow-open",
      "allow": [
        {
          "name": "duck-cli",
          "sidecar": true,
          "args": [
            "init",
            "upgrade",
            "docker-service",
            "backup",
            "check-update",
            "auto-upgrade-deploy",
            "cache",
            "ducker",
            "--help",
            "--version",
            "--check",
            "--full",
            "start",
            "stop",
            "restart",
            "status",
            "run",
            "clear",
            "clean-downloads"
          ]
        }
      ]
    },
    {
      "identifier": "fs:allow-read-text-file",
      "allow": [
        {
          "path": "$APPDATA/duck-client/**"
        }
      ]
    },
    {
      "identifier": "fs:allow-write-text-file", 
      "allow": [
        {
          "path": "$APPDATA/duck-client/**"
        }
      ]
    }
  ]
}
```

#### 1.2 构建时CLI工具集成

**平台对应关系**：
| Tauri平台 | GitHub Release文件 | 说明 |
|-----------|-------------------|------|
| `x86_64-pc-windows-msvc` | `duck-cli-windows-amd64.zip` | Windows x64 |
| `aarch64-pc-windows-msvc` | `duck-cli-windows-arm64.zip` | Windows ARM64 |
| `x86_64-apple-darwin` | `duck-cli-macos-universal.tar.gz` | macOS Intel |
| `aarch64-apple-darwin` | `duck-cli-macos-universal.tar.gz` | macOS Apple Silicon |
| `x86_64-unknown-linux-gnu` | `duck-cli-linux-amd64.tar.gz` | Linux x64 |
| `aarch64-unknown-linux-gnu` | `duck-cli-linux-arm64.tar.gz` | Linux ARM64 |

**构建脚本** (build.rs)：
```rust
use std::env;
use std::path::PathBuf;

fn main() {
    let target = env::var("TARGET").unwrap();
    let out_dir = env::var("OUT_DIR").unwrap();
    
    // 确定要下载的CLI工具版本
    let cli_file = match target.as_str() {
        "x86_64-pc-windows-msvc" => "duck-cli-windows-amd64.zip",
        "aarch64-pc-windows-msvc" => "duck-cli-windows-arm64.zip", 
        "x86_64-apple-darwin" | "aarch64-apple-darwin" => "duck-cli-macos-universal.tar.gz",
        "x86_64-unknown-linux-gnu" => "duck-cli-linux-amd64.tar.gz",
        "aarch64-unknown-linux-gnu" => "duck-cli-linux-arm64.tar.gz",
        _ => panic!("Unsupported target: {}", target),
    };
    
    // 下载并解压CLI工具到binaries目录
    download_and_extract_cli(cli_file, &out_dir);
}

fn download_and_extract_cli(filename: &str, out_dir: &str) {
    // 从GitHub Releases下载最新版本的CLI工具
    // https://github.com/soddygo/duck_client/releases/latest/download/{filename}
    // 解压到 binaries/ 目录
}
```

#### 1.3 运行时CLI工具调用

**Rust后端调用**：
```rust
use tauri::command;
use tauri_plugin_shell::{ShellExt, process::CommandEvent};

#[command]
pub async fn execute_duck_cli_command(
    app: tauri::AppHandle,
    args: Vec<String>,
    working_dir: String,
) -> Result<String, String> {
    let sidecar_command = app
        .shell()
        .sidecar("duck-cli")
        .map_err(|e| format!("创建sidecar命令失败: {}", e))?
        .args(args)
        .current_dir(working_dir);
    
    let (mut rx, mut child) = sidecar_command
        .spawn()
        .map_err(|e| format!("执行命令失败: {}", e))?;
    
    let mut output = String::new();
    
    while let Some(event) = rx.recv().await {
        match event {
            CommandEvent::Stdout(data) => {
                let stdout = String::from_utf8_lossy(&data);
                output.push_str(&stdout);
                // 实时发送输出到前端
                app.emit("cli-output", &stdout).ok();
            }
            CommandEvent::Stderr(data) => {
                let stderr = String::from_utf8_lossy(&data);
                output.push_str(&stderr);
                // 实时发送错误到前端
                app.emit("cli-error", &stderr).ok();
            }
            CommandEvent::Terminated(payload) => {
                app.emit("cli-complete", payload.code).ok();
                break;
            }
            _ => {}
        }
    }
    
    Ok(output)
}
```

**前端调用**：
```typescript
import { Command } from '@tauri-apps/plugin-shell';

export async function executeDuckCliCommand(
  args: string[],
  workingDir: string
): Promise<void> {
  try {
    const command = Command.sidecar('duck-cli', args, {
      cwd: workingDir
    });
    
    // 监听输出事件
    command.on('close', (data) => {
      console.log('Command finished with code:', data.code);
    });
    
    command.on('error', (error) => {
      console.error('Command error:', error);
    });
    
    // 执行命令
    const child = await command.spawn();
    
    // 可以通过child.kill()来终止命令
    return child;
  } catch (error) {
    console.error('Failed to execute command:', error);
    throw error;
  }
}
```

#### 1.3 Shell插件实现方案

**Shell配置** (tauri.conf.json)：
```json
{
  "plugins": {
    "shell": {
      "open": true,
      "scope": [
        {
          "name": "duck-cli",
          "cmd": "duck-cli",
          "args": true,
          "sidecar": false
        }
      ]
    }
  }
}
```

**权限配置** (capabilities/default.json)：
```json
{
  "permissions": [
    "shell:allow-execute",
    "shell:allow-kill",
    "shell:allow-spawn",
    {
      "identifier": "shell:allow-execute",
      "allow": [
        {
          "name": "duck-cli",
          "cmd": "duck-cli",
          "args": [
            "init",
            "upgrade", 
            "docker-service",
            "backup",
            "check-update",
            "auto-upgrade-deploy",
            "cache",
            "ducker",
            "--help",
            "--version",
            "--check",
            "--full",
            "start",
            "stop", 
            "restart",
            "status",
            "run",
            "clear",
            "clean-downloads"
          ],
          "sidecar": false
        }
      ]
    }
  ]
}
```

**跨平台命令执行器**：
```rust
use tauri::{command, AppHandle};
use tauri_plugin_shell::{ShellExt, process::CommandEvent};
use std::path::Path;

pub struct CommandExecutor {
    app: AppHandle,
}

impl CommandExecutor {
    pub fn new(app: AppHandle) -> Self {
        Self { app }
    }

    // 跨平台执行duck-cli命令
    pub async fn execute_duck_cli(
        &self,
        args: Vec<String>,
        working_dir: String,
    ) -> Result<String, String> {
        // 检测系统平台并构建适当的命令
        let (command, shell_args) = self.build_platform_command(&args)?;
        
        let shell = self.app.shell();
        let mut cmd = shell.command(&command);
        
        if !shell_args.is_empty() {
            cmd = cmd.args(shell_args);
        }
        
        // 设置工作目录
        if Path::new(&working_dir).exists() {
            cmd = cmd.current_dir(&working_dir);
        }
        
        // 执行命令并获取输出
        let (mut rx, mut child) = cmd.spawn()
            .map_err(|e| format!("命令执行失败: {}", e))?;
        
        let mut output = String::new();
        
        while let Some(event) = rx.recv().await {
            match event {
                CommandEvent::Stdout(data) => {
                    let stdout = String::from_utf8_lossy(&data);
                    output.push_str(&stdout);
                    // 发送实时输出到前端
                    self.app.emit("cli-output", &stdout).ok();
                }
                CommandEvent::Stderr(data) => {
                    let stderr = String::from_utf8_lossy(&data);
                    output.push_str(&stderr);
                    self.app.emit("cli-error", &stderr).ok();
                }
                CommandEvent::Terminated(payload) => {
                    self.app.emit("cli-complete", payload.code).ok();
                    break;
                }
                _ => {}
            }
        }
        
        Ok(output)
    }
    
    // 构建平台特定的命令
    fn build_platform_command(&self, args: &[String]) -> Result<(String, Vec<String>), String> {
        #[cfg(target_os = "windows")]
        {
            // Windows: 优先使用PowerShell，fallback到cmd
            if self.check_command_exists("powershell") {
                let script = format!("duck-cli {}", args.join(" "));
                Ok(("powershell".to_string(), vec!["-Command".to_string(), script]))
            } else {
                let script = format!("duck-cli {}", args.join(" "));
                Ok(("cmd".to_string(), vec!["/C".to_string(), script]))
            }
        }
        
        #[cfg(any(target_os = "macos", target_os = "linux"))]
        {
            // Unix系统: 使用sh
            let mut cmd_args = vec!["duck-cli".to_string()];
            cmd_args.extend(args.iter().cloned());
            Ok(("sh".to_string(), vec!["-c".to_string(), cmd_args.join(" ")]))
        }
        
        #[cfg(not(any(target_os = "windows", target_os = "macos", target_os = "linux")))]
        {
            Err("不支持的操作系统".to_string())
        }
    }
    
    // 检查命令是否存在
    fn check_command_exists(&self, command: &str) -> bool {
        let shell = self.app.shell();
        
        #[cfg(target_os = "windows")]
        {
            shell.command("where")
                .args([command])
                .status()
                .map(|status| status.success())
                .unwrap_or(false)
        }
        
        #[cfg(any(target_os = "macos", target_os = "linux"))]
        {
            shell.command("which")
                .args([command])
                .status()
                .map(|status| status.success())
                .unwrap_or(false)
        }
    }
}

// Tauri命令封装
#[command]
pub async fn execute_duck_cli_shell(
    app: AppHandle,
    args: Vec<String>,
    working_dir: String,
) -> Result<String, String> {
    let executor = CommandExecutor::new(app);
    executor.execute_duck_cli(args, working_dir).await
}

// 检查duck-cli是否已安装
#[command]
pub async fn check_duck_cli_installed(app: AppHandle) -> Result<bool, String> {
    let executor = CommandExecutor::new(app);
    Ok(executor.check_command_exists("duck-cli"))
}

// 获取已安装的duck-cli版本
#[command]
pub async fn get_installed_duck_cli_version(app: AppHandle) -> Result<String, String> {
    let executor = CommandExecutor::new(app);
    let output = executor.execute_duck_cli(vec!["--version".to_string()], ".".to_string()).await?;
    
    // 解析版本号
    let version = output.lines()
        .find(|line| line.contains("duck-cli"))
        .and_then(|line| line.split_whitespace().last())
        .unwrap_or("unknown")
        .to_string();
    
    Ok(version)
}
```

**前端Shell调用**：
```typescript
import { Command } from '@tauri-apps/plugin-shell';
import { invoke } from '@tauri-apps/api/core';

export class ShellExecutor {
  // 使用Shell插件直接执行duck-cli
  async executeDuckCliDirect(args: string[], workingDir: string): Promise<void> {
    try {
      // 跨平台命令构建
      const isWindows = navigator.platform.includes('Win');
      
      let command: Command;
      
      if (isWindows) {
        // Windows使用PowerShell
        const script = `duck-cli ${args.join(' ')}`;
        command = Command.create('powershell', ['-Command', script], {
          cwd: workingDir
        });
      } else {
        // Unix系统使用sh
        const script = `duck-cli ${args.join(' ')}`;
        command = Command.create('sh', ['-c', script], {
          cwd: workingDir
        });
      }
      
      // 监听输出
      command.on('close', (data) => {
        console.log('Command finished with code:', data.code);
        this.emit('command-complete', data.code);
      });
      
      command.on('error', (error) => {
        console.error('Command error:', error);
        this.emit('command-error', error);
      });
      
      // 监听实时输出
      command.stdout.on('data', (data) => {
        console.log('stdout:', data);
        this.emit('command-output', data);
      });
      
      command.stderr.on('data', (data) => {
        console.error('stderr:', data);
        this.emit('command-error-output', data);
      });
      
      // 执行命令
      await command.spawn();
      
    } catch (error) {
      console.error('Failed to execute command:', error);
      throw error;
    }
  }
  
  // 使用后端封装的跨平台执行器
  async executeDuckCliWrapped(args: string[], workingDir: string): Promise<string> {
    try {
      return await invoke<string>('execute_duck_cli_shell', {
        args,
        workingDir
      });
    } catch (error) {
      console.error('Failed to execute command:', error);
      throw error;
    }
  }
  
  // 检查duck-cli是否已安装
  async checkDuckCliInstalled(): Promise<boolean> {
    try {
      return await invoke<boolean>('check_duck_cli_installed');
    } catch (error) {
      console.error('Failed to check duck-cli installation:', error);
      return false;
    }
  }
  
  // 获取已安装的duck-cli版本
  async getInstalledVersion(): Promise<string> {
    try {
      return await invoke<string>('get_installed_duck_cli_version');
    } catch (error) {
      console.error('Failed to get duck-cli version:', error);
      throw error;
    }
  }
  
  private emit(event: string, data: any) {
    // 发送事件到应用的其他部分
    window.dispatchEvent(new CustomEvent(event, { detail: data }));
  }
}
```

#### 1.4 混合执行策略

为了提供最佳的用户体验，我们可以实现混合执行策略：

```typescript
export class HybridCliExecutor {
  private sidecarAvailable: boolean = true;
  private shellExecutor: ShellExecutor;
  
  constructor() {
    this.shellExecutor = new ShellExecutor();
  }
  
  async executeDuckCli(args: string[], workingDir: string): Promise<void> {
    try {
      // 优先使用Sidecar（内置版本）
      if (this.sidecarAvailable) {
        return await this.executeSidecar(args, workingDir);
      }
      
      // Fallback到Shell执行（系统安装版本）
      const installed = await this.shellExecutor.checkDuckCliInstalled();
      if (installed) {
        return await this.shellExecutor.executeDuckCliWrapped(args, workingDir);
      }
      
      // 都不可用时提示用户
      throw new Error('duck-cli工具不可用，请安装duck-cli或使用完整版本的应用');
      
    } catch (error) {
      // 如果Sidecar失败，尝试Shell方式
      if (this.sidecarAvailable) {
        console.warn('Sidecar执行失败，尝试Shell方式:', error);
        this.sidecarAvailable = false;
        return await this.executeDuckCli(args, workingDir);
      }
      throw error;
    }
  }
  
  private async executeSidecar(args: string[], workingDir: string): Promise<void> {
    // 使用之前定义的Sidecar执行逻辑
    const command = Command.sidecar('duck-cli', args, {
      cwd: workingDir
    });
    
    return await command.execute();
  }
}
```

#### 1.5 方案对比与选择建议

| 特性 | Sidecar方案 | Shell方案 | 混合方案 |
|------|-------------|-----------|----------|
| **部署复杂度** | 低（一体化） | 中（需预装CLI） | 中 |
| **离线可用性** | ✅ 完全离线 | ❌ 依赖系统 | ✅ 部分离线 |
| **版本一致性** | ✅ 严格一致 | ❌ 可能不一致 | ⚠️ 混合管理 |
| **安全性** | ✅ 预签名验证 | ⚠️ 依赖系统 | ✅ 双重保障 |
| **灵活性** | ❌ 版本固定 | ✅ 版本灵活 | ✅ 最佳平衡 |
| **包大小** | 大（含CLI） | 小（仅GUI） | 大（含CLI） |
| **更新机制** | GUI+CLI同步 | 独立更新 | 智能切换 |
| **跨平台支持** | ✅ 构建时处理 | ✅ 运行时适配 | ✅ 双重支持 |
| **命令行差异** | ❌ 不涉及 | ✅ 自动处理 | ✅ 智能选择 |

**Shell插件的跨平台优势**：
根据[Tauri Shell插件文档](https://tauri.app/plugin/shell/)，Shell插件能够有效屏蔽系统命令行差异：

1. **统一的API接口**：无论在Windows、macOS还是Linux，都使用相同的JavaScript/Rust API
2. **自动平台检测**：后端可以自动检测当前操作系统并选择合适的Shell（PowerShell/cmd/sh/bash）
3. **路径处理统一**：Tauri自动处理不同系统的路径分隔符差异
4. **权限管理统一**：通过Tauri权限系统统一管理不同平台的命令执行权限
5. **错误处理统一**：统一的错误码和异常处理机制

**实际使用建议**：

**生产环境（推荐Sidecar）**：
```typescript
// 生产环境配置 - 使用Sidecar确保稳定性
const productionExecutor = new SidecarExecutor();
await productionExecutor.execute(['init'], workingDir);
```

**开发测试（推荐Shell）**：
```typescript
// 开发环境配置 - 使用Shell提供灵活性
const devExecutor = new ShellExecutor();
if (await devExecutor.checkDuckCliInstalled()) {
  await devExecutor.executeDuckCliWrapped(['--version'], workingDir);
}
```

**企业部署（推荐混合）**：
```typescript
// 企业环境配置 - 混合策略保证兼容性
const enterpriseExecutor = new HybridCliExecutor();
await enterpriseExecutor.executeDuckCli(['docker-service', 'start'], workingDir);
```

**推荐使用场景**：
- **生产环境**：Sidecar方案（稳定可靠，离线可用）
- **开发测试**：Shell方案（灵活便捷，实时更新）  
- **企业部署**：混合方案（兼容性最佳，降级保障）
- **轻量版本**：仅Shell方案（小包体积，依赖系统安装）

### 2. 用户界面设计

#### 2.1 工作目录管理设计

##### 2.1.1 功能需求分析

**核心功能**：
- 工作目录是所有 duck-cli 命令执行的根目录
- 第一次打开应用必须设置工作目录
- 工作目录设置后，所有操作都在此目录下进行
- 支持随时更改工作目录

**交互流程**：
1. **首次启动** → 显示欢迎引导弹窗 → 必须选择工作目录 → 验证目录有效性 → 保存配置
2. **再次启动** → 自动加载已保存的工作目录 → 验证目录是否仍然有效
3. **目录无效** → 显示警告状态 → 禁用所有功能按钮 → 提示重新设置

##### 2.1.2 UI布局设计

**工作目录显示栏**（位于界面顶部）：
```
┌─────────────────────────────────────────────────────────────┐
│ 📁 工作目录: /path/to/working/directory    [更改目录]       │
│    状态: ✅有效 / ❌无效 / ⚠️警告                          │
└─────────────────────────────────────────────────────────────┘
```

**视觉状态指示**：
- **绿色背景** + ✅图标：目录有效，功能正常
- **红色背景** + ❌图标：目录无效，功能禁用
- **黄色背景** + ⚠️图标：目录警告，部分功能受限

**首次使用引导弹窗**：
```
┌─────────────────────────────────────────┐
│              🦆 Duck CLI GUI            │
│                                         │
│   欢迎使用！请选择工作目录：            │
│   ┌─────────────────┐ [浏览...]        │
│   │ 选择的路径...   │                  │
│   └─────────────────┘                  │
│                                         │
│   💡 建议：                            │
│   • 选择空目录或新建目录                │
│   • 确保目录有读写权限                  │
│   • 避免选择系统目录                    │
│                                         │
│   [稍后设置]  [确认并开始]              │
└─────────────────────────────────────────┘
```

##### 2.1.3 状态管理逻辑

**应用启动时**：
1. 检查本地存储的工作目录配置
2. 如果没有配置 → 标记为首次使用，显示引导弹窗
3. 如果有配置 → 验证目录是否存在且可访问
4. 根据验证结果设置UI状态（有效/无效）

**目录选择流程**：
1. 用户点击"选择目录"或"更改目录"按钮
2. 调用系统文件选择对话框
3. 用户选择目录后，进行验证检查：
   - 目录是否存在
   - 是否有读写权限
   - 是否为合适的工作目录
4. 验证通过 → 保存配置，更新UI状态
5. 验证失败 → 显示错误提示，要求重新选择

**功能联动机制**：
- 工作目录未设置或无效时：
  - 所有功能按钮置为禁用状态
  - 终端命令输入框禁用
  - 显示状态提示信息
- 工作目录有效时：
  - 启用所有功能按钮
  - 启用终端交互
  - 更新终端提示符显示当前目录

#### 2.2 整体布局结构设计

**垂直分割式布局**：
```
┌─────────────────────────────────────────────────────────────┐
│ 工作目录栏: 📁 /path/to/work/dir    [更改目录]              │
├─────────────────────────────────────────────────────────────┤
│                    操作面板区域                              │
│  ┌─────┐ ┌─────┐ ┌─────┐ ┌─────┐ ┌─────┐                    │
│  │初始化│ │下载  │ │部署 │ │启动 │ │停止 │                    │
│  └─────┘ └─────┘ └─────┘ └─────┘ └─────┘                    │
│  ┌─────┐ ┌─────┐ ┌─────┐ ┌─────┐ ┌─────┐                    │
│  │重启 │ │检查  │ │升级 │ │备份 │ │回滚 │                    │
│  └─────┘ └─────┘ └─────┘ └─────┘ └─────┘                    │
│                          ┌─────┐                            │
│                          │应用  │                            │
│                          │更新 │                            │
│                          └─────┘                            │
├═════════════════════════════════════════════════════════════┤ ← 可拖拽分割线
│ 🖥️ Duck CLI Terminal                                 ●      │
├─────────────────────────────────────────────────────────────┤
│ [23:45:12] $ duck-cli init                                  │
│ [23:45:12] ✅ 初始化完成                                     │
│ [23:45:15] $ duck-cli docker-service start                 │
│ [23:45:15] 🚀 正在启动 Docker 服务...                      │
│ [23:45:20] ✅ Docker 服务启动成功                           │
│                                                             │
│ duck-cli@myproject$ _                                       │
└─────────────────────────────────────────────────────────────┘
```

**布局特点**：
1. **工作目录栏**：固定在顶部，高度约50px
2. **操作面板**：占用上半部分，可滚动，响应式网格布局
3. **分割线**：支持拖拽调整上下比例
4. **终端窗口**：占用下半部分，包含头部状态栏和交互区域

#### 2.3 操作面板设计

##### 2.3.1 核心功能按钮设计

**功能分类与布局**：

```
基础操作区 (第一行):
┌─────┐ ┌─────┐ ┌─────┐ ┌─────┐ ┌─────┐
│🚀   │ │⬇️   │ │🚀   │ │▶️   │ │⏹️   │
│初始化│ │下载  │ │部署 │ │启动 │ │停止 │
│     │ │服务 │ │     │ │服务 │ │服务 │
└─────┘ └─────┘ └─────┘ └─────┘ └─────┘

管理操作区 (第二行):
┌─────┐ ┌─────┐ ┌─────┐ ┌─────┐ ┌─────┐
│🔄   │ │🔍   │ │⬆️   │ │💾   │ │↩️   │
│重启 │ │检查  │ │升级 │ │备份 │ │回滚 │
│服务 │ │更新 │ │服务 │ │     │ │服务 │
└─────┘ └─────┘ └─────┘ └─────┘ └─────┘

应用管理区 (第三行):
              ┌─────┐
              │🔄   │
              │应用  │
              │更新 │
              └─────┘
```

**功能映射表**：
| 按钮 | 图标 | 命令 | 说明 |
|------|------|------|------|
| 初始化 | 🚀 | `duck-cli init` | 初始化工作目录配置 |
| 下载服务 | ⬇️ | `duck-cli upgrade --full` | 下载最新Docker服务包 |
| 一键部署 | 🚀 | `duck-cli auto-upgrade-deploy run` | 自动部署服务 |
| 启动服务 | ▶️ | `duck-cli docker-service start` | 启动Docker服务 |
| 停止服务 | ⏹️ | `duck-cli docker-service stop` | 停止Docker服务 |
| 重启服务 | 🔄 | `duck-cli docker-service restart` | 重启Docker服务 |
| 检查更新 | 🔍 | `duck-cli upgrade --check` | 检查Docker服务新版本 |
| 升级服务 | ⬆️ | `duck-cli upgrade --full` | 下载Docker服务 |
| 创建备份 | 💾 | `duck-cli backup` | 创建服务备份 |
| 回滚服务 | ↩️ | `duck-cli list-backups` | 列出并选择回滚点 |
| 应用更新 | 🔄 | Tauri Updater | 检查并更新GUI应用 |
| 导出日志 | 📋 | Dialog + FS | 将命令执行日志导出到文件 |
| 导入配置 | 📁 | Dialog + FS | 从文件导入应用配置 |

**扩展功能按钮设计**：

除了核心功能外，我们还可以添加一些便民功能按钮，充分利用 Dialog 和 File System 插件：

```
扩展功能区 (第四行，可选):
┌─────┐ ┌─────┐ ┌─────┐ ┌─────┐ ┌─────┐
│📋   │ │📁   │ │🔧   │ │📊   │ │❓   │
│导出  │ │导入  │ │设置 │ │状态 │ │帮助 │
│日志 │ │配置 │ │     │ │报告 │ │     │
└─────┘ └─────┘ └─────┘ └─────┘ └─────┘
```

**扩展功能映射表**：
| 按钮 | 图标 | Dialog类型 | FS操作 | 说明 |
|------|------|-----------|-------|------|
| 导出日志 | 📋 | `save()` | `readTextFile()` | 收集并导出所有命令执行日志 |
| 导入配置 | 📁 | `open()` | `writeTextFile()` | 从备份文件恢复应用配置 |
| 高级设置 | 🔧 | `confirm()` | `readTextFile()` | 编辑高级配置选项 |
| 生成报告 | 📊 | `save()` | `readDir()` | 生成系统状态报告文件 |
| 使用帮助 | ❓ | `message()` | - | 显示功能说明和快捷键 |

**交互流程示例**：

```typescript
// 导出日志按钮点击处理
async function handleExportLogs() {
  try {
    // 1. 使用Dialog插件选择保存位置
    const savePath = await save({
      title: '导出日志文件',
      defaultPath: `duck-cli-logs-${new Date().toISOString().split('T')[0]}.txt`,
      filters: [
        { name: '文本文件', extensions: ['txt'] },
        { name: '日志文件', extensions: ['log'] },
        { name: '所有文件', extensions: ['*'] }
      ]
    });

    if (!savePath) return; // 用户取消

    // 2. 显示进度信息
    await message('正在收集日志文件...', { 
      title: '导出日志', 
      kind: 'info' 
    });

    // 3. 调用后端收集和导出日志
    const result = await invoke<{fileCount: number, totalSize: number}>('export_logs_detailed');

    // 4. 显示完成信息
    await message(
      `日志导出完成！\n\n文件位置: ${savePath}\n包含文件: ${result.fileCount} 个\n总大小: ${(result.totalSize / 1024).toFixed(2)} KB`,
      { title: '导出成功', kind: 'info' }
    );

  } catch (error) {
    await message(`导出日志失败: ${error}`, { 
      title: '导出错误', 
      kind: 'error' 
    });
  }
}

// 导入配置按钮点击处理
async function handleImportConfig() {
  try {
    // 1. 使用Dialog插件选择配置文件
    const selectedFile = await open({
      title: '选择配置文件',
      filters: [
        { name: 'JSON 配置文件', extensions: ['json'] },
        { name: 'TOML 配置文件', extensions: ['toml'] },
        { name: '所有文件', extensions: ['*'] }
      ]
    });

    if (!selectedFile) return; // 用户取消

    // 2. 询问是否备份现有配置
    const shouldBackup = await confirm(
      '导入新配置前，是否要备份当前配置？\n\n建议选择"是"以便在需要时恢复当前设置。',
      { title: '备份确认', kind: 'warning' }
    );

    // 3. 显示处理进度
    await message('正在验证和导入配置文件...', { 
      title: '导入配置', 
      kind: 'info' 
    });

    // 4. 调用后端处理导入
    const result = await invoke<{success: boolean, backupPath?: string}>('import_config_advanced', {
      filePath: selectedFile,
      createBackup: shouldBackup
    });

    if (result.success) {
      let successMessage = '配置导入成功！';
      if (result.backupPath) {
        successMessage += `\n\n原配置已备份到:\n${result.backupPath}`;
      }
      successMessage += '\n\n应用将重新启动以应用新配置。';

      await message(successMessage, { 
        title: '导入成功', 
        kind: 'info' 
      });

      // 5. 重启应用应用新配置
      await invoke('restart_application');
    }

  } catch (error) {
    await message(`导入配置失败: ${error}`, { 
      title: '导入错误', 
      kind: 'error' 
    });
  }
}
```

##### 2.3.2 按钮状态设计

**视觉状态**：
1. **正常状态**：白色背景，灰色边框，悬浮时蓝色边框+阴影
2. **禁用状态**：灰色背景，禁用光标，半透明
3. **执行中状态**：蓝色背景，显示"执行中..."文字，旋转图标
4. **错误状态**：红色边框，错误提示

**交互逻辑**：
- 工作目录无效时：所有按钮（除应用更新）置为禁用
- 命令执行中时：当前按钮显示执行状态，其他按钮保持可用
- 点击后立即反馈：按钮状态变更，终端显示命令
- 执行完成后：恢复正常状态，显示结果

##### 2.3.3 错误处理与用户反馈

**状态提示区域**：
```
┌─────────────────────────────────────────────────────────────┐
│ ⚠️ 请先设置有效的工作目录才能使用功能按钮                      │
└─────────────────────────────────────────────────────────────┘
```

**反馈机制**：
- 目录无效时：显示黄色警告条
- 命令执行失败：终端显示错误信息，按钮闪红色
- 网络错误：显示重试选项
- 权限错误：提示用户检查权限

#### 2.4 终端窗口设计

##### 2.4.1 终端界面布局

**终端窗口结构**：
```
┌─────────────────────────────────────────────────────────────┐
│ 🖥️ Duck CLI Terminal     当前目录: /work/dir         ●      │ ← 头部状态栏
├─────────────────────────────────────────────────────────────┤
│ [23:45:12] $ duck-cli init                                  │ ← 日志输出区域
│ [23:45:12] ✅ 初始化完成                                     │   (可滚动)
│ [23:45:15] $ duck-cli docker-service start                 │
│ [23:45:15] 🚀 正在启动 Docker 服务...                      │
│ [23:45:20] ✅ Docker 服务启动成功                           │
│ [23:45:25] ❌ 错误: 权限不足                                │
│                                                             │
│ ⏳ 命令执行中...                                            │ ← 执行状态指示
├─────────────────────────────────────────────────────────────┤
│ duck-cli@myproject$ _                                       │ ← 命令输入行
└─────────────────────────────────────────────────────────────┘
```

**头部状态栏设计**：
- **左侧**：🖥️ + "Duck CLI Terminal" 标题
- **右侧**：当前工作目录路径 + 连接状态指示灯
  - 🟢 绿色：工作目录有效，终端可用
  - 🔴 红色：工作目录无效，终端禁用
  - 🟡 黄色：正在验证或切换目录

##### 2.4.2 日志显示设计

**日志类型与样式**：
| 类型 | 颜色 | 前缀 | 示例 |
|------|------|------|------|
| 用户输入 | 蓝色 | `$` | `[23:45:12] $ duck-cli init` |
| 正常输出 | 绿色 | 无 | `[23:45:12] ✅ 初始化完成` |
| 错误输出 | 红色 | 无 | `[23:45:12] ❌ 错误: 权限不足` |
| 警告信息 | 黄色 | 无 | `[23:45:12] ⚠️ 警告: 配置文件不存在` |
| 系统消息 | 灰色 | 无 | `[23:45:12] ℹ️ 系统消息` |

**时间戳格式**：`[HH:MM:SS]` 格式，小字号，灰色显示

**特殊状态显示**：
- 命令执行中：`⏳ 命令执行中...` (蓝色，闪烁动画)
- 长时间操作：显示进度指示或旋转动画
- 空状态：显示欢迎信息和使用提示

##### 2.4.3 命令输入设计

**提示符设计**：
```
duck-cli@{project_name}$ _
```
- `duck-cli`：固定前缀，表示CLI工具
- `@{project_name}`：当前工作目录的文件夹名称
- `$`：命令提示符
- `_`：光标闪烁

**输入交互**：
- **Enter键**：执行命令
- **↑/↓箭头**：历史命令导航
- **Tab键**：命令自动补全（如果支持）
- **Ctrl+C**：中断当前命令
- **Ctrl+L**：清屏

**命令处理逻辑**：
1. 用户输入命令并按Enter
2. 验证工作目录是否有效
3. 在日志区显示输入的命令
4. 执行命令（自动添加`duck-cli`前缀，如果用户没有输入）
5. 实时显示命令输出
6. 命令完成后恢复输入状态

##### 2.4.4 禁用状态设计

**工作目录无效时**：
- 输入框置灰，显示提示文字："请先设置工作目录..."
- 状态指示灯显示红色
- 日志区显示警告信息："⚠️ 请先设置工作目录才能使用终端"
- 禁用所有键盘输入

**命令执行中时**：
- 输入框暂时禁用
- 显示执行状态指示
- 右侧显示旋转的加载图标

### 3. 后端架构设计

#### 3.1 工作目录管理模块

##### 3.1.1 目录管理需求

**核心功能**：
- 工作目录的选择、验证、存储和恢复
- 目录权限检查和安全验证
- 跨平台目录路径处理
- 配置持久化存储

**Tauri Commands 设计**：
| 命令 | 参数 | 返回值 | 说明 |
|------|------|--------|------|
| `select_directory` | 无 | `Option<String>` | 调用系统目录选择对话框 |
| `validate_working_directory` | `path: String` | `bool` | 验证目录是否有效 |
| `set_working_directory` | `path: String` | `Result<(), String>` | 设置并保存工作目录 |
| `get_working_directory` | 无 | `Option<String>` | 获取当前工作目录 |
| `check_directory_permissions` | `path: String` | `DirectoryPermissions` | 检查目录权限状态 |
| `show_error_dialog` | `title: String, message: String` | `()` | 显示错误对话框 |
| `show_confirm_dialog` | `title: String, message: String` | `bool` | 显示确认对话框 |
| `export_logs` | 无 | `Option<String>` | 导出日志文件 |
| `import_config` | 无 | `Option<String>` | 导入配置文件 |

**工作目录管理完整实现**：

```rust
use tauri::{command, AppHandle, State};
use tauri_plugin_dialog::{DialogExt, MessageDialogKind};
use tauri_plugin_fs::{FsExt, OpenOptions};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct DirectoryPermissions {
    pub exists: bool,
    pub readable: bool,
    pub writable: bool,
    pub executable: bool,
    pub available_space: Option<u64>,
    pub is_empty: bool,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct WorkingDirectoryConfig {
    pub path: String,
    pub last_updated: String,
    pub user_selected: bool,
}

// 选择工作目录 - 使用 Dialog 插件
#[command]
pub async fn select_directory(app: AppHandle) -> Result<Option<String>, String> {
    let dialog = app.dialog().file();
    
    let selected_path = dialog
        .set_title("选择工作目录")
        .set_directory_mode(true)
        .blocking_pick_folder();
    
    match selected_path {
        Some(path) => {
            let path_str = path.to_string_lossy().to_string();
            println!("用户选择了目录: {}", path_str);
            Ok(Some(path_str))
        }
        None => {
            println!("用户取消了目录选择");
            Ok(None)
        }
    }
}

// 验证工作目录 - 使用 File System 插件
#[command]
pub async fn validate_working_directory(
    app: AppHandle,
    path: String,
) -> Result<DirectoryPermissions, String> {
    let fs = app.fs();
    let path_buf = PathBuf::from(&path);
    
    // 检查目录是否存在
    let exists = fs.exists(&path_buf).await
        .map_err(|e| format!("检查目录存在性失败: {}", e))?;
    
    if !exists {
        return Ok(DirectoryPermissions {
            exists: false,
            readable: false,
            writable: false,
            executable: false,
            available_space: None,
            is_empty: false,
        });
    }
    
    // 获取目录元数据
    let metadata = fs.metadata(&path_buf).await
        .map_err(|e| format!("获取目录元数据失败: {}", e))?;
    
    // 检查是否为目录
    if !metadata.is_dir() {
        return Err("选择的路径不是目录".to_string());
    }
    
    // 检查读权限
    let readable = fs.read_dir(&path_buf).await.is_ok();
    
    // 检查写权限 - 尝试创建临时文件
    let test_file_path = path_buf.join(".duck_cli_test_write");
    let writable = fs.write_text_file(&test_file_path, "test").await.is_ok();
    if writable {
        let _ = fs.remove(&test_file_path).await; // 清理测试文件
    }
    
    // 检查目录是否为空
    let dir_entries = fs.read_dir(&path_buf).await.unwrap_or_default();
    let is_empty = dir_entries.is_empty();
    
    // 获取可用空间 (使用std库)
    let available_space = get_available_space(&path);
    
    Ok(DirectoryPermissions {
        exists,
        readable,
        writable,
        executable: readable, // 在文件系统中，读权限通常意味着可执行
        available_space,
        is_empty,
    })
}

// 设置工作目录 - 使用 File System 插件保存配置
#[command]
pub async fn set_working_directory(
    app: AppHandle,
    path: String,
) -> Result<(), String> {
    // 首先验证目录
    let permissions = validate_working_directory(app.clone(), path.clone()).await?;
    
    if !permissions.exists {
        return Err("目录不存在".to_string());
    }
    
    if !permissions.readable || !permissions.writable {
        return Err("目录权限不足，需要读写权限".to_string());
    }
    
    // 保存配置到应用数据目录
    let config = WorkingDirectoryConfig {
        path: path.clone(),
        last_updated: chrono::Utc::now().to_rfc3339(),
        user_selected: true,
    };
    
    let fs = app.fs();
    let config_dir = app.path().app_data_dir()
        .map_err(|e| format!("获取应用数据目录失败: {}", e))?;
    
    // 确保配置目录存在
    if !fs.exists(&config_dir).await.unwrap_or(false) {
        fs.create_dir_all(&config_dir).await
            .map_err(|e| format!("创建配置目录失败: {}", e))?;
    }
    
    let config_file = config_dir.join("working_directory.json");
    let config_json = serde_json::to_string_pretty(&config)
        .map_err(|e| format!("序列化配置失败: {}", e))?;
    
    fs.write_text_file(&config_file, &config_json).await
        .map_err(|e| format!("保存配置文件失败: {}", e))?;
    
    println!("工作目录已设置为: {}", path);
    Ok(())
}

// 获取工作目录 - 从配置文件读取
#[command]
pub async fn get_working_directory(app: AppHandle) -> Result<Option<String>, String> {
    let fs = app.fs();
    let config_dir = app.path().app_data_dir()
        .map_err(|e| format!("获取应用数据目录失败: {}", e))?;
    
    let config_file = config_dir.join("working_directory.json");
    
    if !fs.exists(&config_file).await.unwrap_or(false) {
        return Ok(None);
    }
    
    let config_content = fs.read_text_file(&config_file).await
        .map_err(|e| format!("读取配置文件失败: {}", e))?;
    
    let config: WorkingDirectoryConfig = serde_json::from_str(&config_content)
        .map_err(|e| format!("解析配置文件失败: {}", e))?;
    
    // 验证保存的目录是否仍然有效
    let permissions = validate_working_directory(app.clone(), config.path.clone()).await?;
    
    if permissions.exists && permissions.readable && permissions.writable {
        Ok(Some(config.path))
    } else {
        // 如果目录无效，清除配置
        let _ = fs.remove(&config_file).await;
        Ok(None)
    }
}

// 显示错误对话框 - 使用 Dialog 插件
#[command]
pub async fn show_error_dialog(
    app: AppHandle,
    title: String,
    message: String,
) -> Result<(), String> {
    let dialog = app.dialog()
        .message(&message)
        .title(&title)
        .kind(MessageDialogKind::Error);
    
    dialog.blocking_show();
    Ok(())
}

// 显示确认对话框 - 使用 Dialog 插件
#[command]
pub async fn show_confirm_dialog(
    app: AppHandle,
    title: String,
    message: String,
) -> Result<bool, String> {
    let result = app.dialog()
        .message(&message)
        .title(&title)
        .kind(MessageDialogKind::Warning)
        .blocking_show();
    
    Ok(result)
}

// 导出日志文件 - 使用 Dialog 插件选择保存位置
#[command]
pub async fn export_logs(app: AppHandle) -> Result<Option<String>, String> {
    let dialog = app.dialog().file();
    
    let save_path = dialog
        .set_title("导出日志文件")
        .set_file_name("duck-cli-logs.txt")
        .add_filter("文本文件", &["txt"])
        .add_filter("所有文件", &["*"])
        .blocking_save_file();
    
    match save_path {
        Some(path) => {
            let fs = app.fs();
            
            // 读取应用日志
            let log_dir = app.path().app_log_dir()
                .map_err(|e| format!("获取日志目录失败: {}", e))?;
            
            let log_content = collect_logs(&fs, &log_dir).await?;
            
            // 写入到用户选择的位置
            fs.write_text_file(&path, &log_content).await
                .map_err(|e| format!("写入日志文件失败: {}", e))?;
            
            let path_str = path.to_string_lossy().to_string();
            Ok(Some(path_str))
        }
        None => Ok(None),
    }
}

// 导入配置文件 - 使用 Dialog 插件选择文件
#[command]
pub async fn import_config(app: AppHandle) -> Result<Option<String>, String> {
    let dialog = app.dialog().file();
    
    let selected_file = dialog
        .set_title("导入配置文件")
        .add_filter("JSON 文件", &["json"])
        .add_filter("所有文件", &["*"])
        .blocking_pick_file();
    
    match selected_file {
        Some(path) => {
            let fs = app.fs();
            
            // 读取配置文件
            let config_content = fs.read_text_file(&path).await
                .map_err(|e| format!("读取配置文件失败: {}", e))?;
            
            // 验证配置格式
            let _: WorkingDirectoryConfig = serde_json::from_str(&config_content)
                .map_err(|e| format!("配置文件格式无效: {}", e))?;
            
            // 复制到应用配置目录
            let config_dir = app.path().app_data_dir()
                .map_err(|e| format!("获取应用数据目录失败: {}", e))?;
            
            let target_file = config_dir.join("working_directory.json");
            fs.write_text_file(&target_file, &config_content).await
                .map_err(|e| format!("保存配置文件失败: {}", e))?;
            
            let path_str = path.to_string_lossy().to_string();
            Ok(Some(path_str))
        }
        None => Ok(None),
    }
}

// 辅助函数：获取可用磁盘空间
fn get_available_space(path: &str) -> Option<u64> {
    use std::fs;
    
    // 这里使用标准库，在实际项目中可能需要使用第三方库如 `sysinfo`
    if let Ok(_metadata) = fs::metadata(path) {
        // 简化实现，实际中需要使用系统API获取磁盘空间
        Some(1_000_000_000) // 假设有1GB可用空间
    } else {
        None
    }
}

// 辅助函数：收集日志内容
async fn collect_logs(
    fs: &tauri_plugin_fs::Fs,
    log_dir: &std::path::Path,
) -> Result<String, String> {
    let mut log_content = String::new();
    
    if fs.exists(log_dir).await.unwrap_or(false) {
        let log_files = fs.read_dir(log_dir).await.unwrap_or_default();
        
        for entry in log_files {
            if let Some(ext) = entry.path().extension() {
                if ext == "log" || ext == "txt" {
                    match fs.read_text_file(&entry.path()).await {
                        Ok(content) => {
                            log_content.push_str(&format!(
                                "\n=== {} ===\n{}\n",
                                entry.file_name().unwrap_or_default().to_string_lossy(),
                                content
                            ));
                        }
                        Err(e) => {
                            log_content.push_str(&format!(
                                "\n=== 读取 {} 失败: {} ===\n",
                                entry.file_name().unwrap_or_default().to_string_lossy(),
                                e
                            ));
                        }
                    }
                }
            }
        }
    }
    
    if log_content.is_empty() {
        log_content = "没有找到日志文件".to_string();
    }
    
    Ok(log_content)
}
```

**目录验证逻辑**：
1. **存在性检查**：目录是否存在
2. **权限检查**：是否有读写权限
3. **安全检查**：避免系统关键目录
4. **空间检查**：可用磁盘空间是否充足
5. **路径检查**：路径格式是否有效

**配置存储策略**：
- 使用 Tauri 的应用数据目录存储配置
- JSON 格式存储工作目录路径和相关设置
- 支持配置备份和恢复
- 跨平台兼容的路径处理

##### 3.1.2 错误处理设计

**错误类型定义**：
```rust
pub enum WorkingDirectoryError {
    DirectoryNotFound,
    PermissionDenied,
    InvalidPath,
    SystemDirectory,
    InsufficientSpace,
    ConfigurationError,
}
```

**用户友好的错误消息**：
- 目录不存在：提示创建目录或选择其他路径
- 权限不足：提示检查目录权限或选择其他位置
- 路径无效：提示路径格式错误
- 系统目录：警告避免使用系统关键目录

### 4. 后端命令执行设计

#### 4.1 命令执行架构

**执行流程设计**：
```
用户操作 → 前端验证 → 后端命令 → CLI工具 → 实时输出 → 前端显示
    ↓           ↓           ↓          ↓         ↓           ↓
  按钮点击   工作目录检查  参数构建   进程启动   流式读取    终端显示
```

**核心组件**：
1. **命令调度器**：管理命令队列和执行状态
2. **进程管理器**：负责CLI进程的启动、监控和清理
3. **输出处理器**：实时处理命令输出并转发到前端
4. **错误处理器**：统一处理各种执行错误

**Tauri Commands 设计**：
| 命令 | 参数 | 说明 |
|------|------|------|
| `execute_cli_command` | `command: String, args: Vec<String>, working_dir: String` | 执行duck-cli命令 |
| `stop_cli_command` | `process_id: u32` | 中断正在执行的命令 |
| `get_command_history` | 无 | 获取命令历史记录 |
| `clear_command_history` | 无 | 清空命令历史 |

#### 4.2 实时输出处理

**输出流管理**：
- **标准输出(stdout)**：正常命令输出，绿色显示
- **标准错误(stderr)**：错误信息，红色显示
- **组合输出**：合并两个流，保持时序正确

**事件通信机制**：
```
后端进程 → 输出监听器 → 数据处理 → Tauri事件 → 前端更新
   ↓           ↓           ↓          ↓         ↓
 CLI输出   异步读取    格式化处理   emit事件   终端显示
```

**输出事件类型**：
- `cli-output`：普通输出内容
- `cli-error`：错误输出内容
- `cli-progress`：进度信息
- `cli-complete`：命令执行完成
- `cli-interrupted`：命令被中断

#### 4.3 Sidecar版本管理设计

**版本同步策略**：
- GUI应用版本与duck-cli版本保持同步
- 通过构建脚本在编译时集成最新的duck-cli
- 每次发布GUI应用时自动包含最新的CLI工具
- 避免运行时版本不一致问题

**构建时集成流程**：
```
触发构建 → 获取目标平台 → 下载对应CLI → 校验完整性 → 打包到应用 → 设置权限
    ↓           ↓            ↓           ↓           ↓          ↓
  CI/CD     平台检测     GitHub API   SHA256校验   Sidecar   可执行权限
```

**版本信息管理**：
```rust
// 在构建时生成版本信息
pub const DUCK_CLI_VERSION: &str = env!("DUCK_CLI_VERSION");
pub const BUILD_TIME: &str = env!("BUILD_TIME");

#[command]
pub fn get_cli_version() -> String {
    format!("duck-cli {}", DUCK_CLI_VERSION)
}
```

**错误处理策略**：
- **Sidecar执行失败**：检查文件权限和可执行性
- **命令不存在**：验证sidecar配置是否正确
- **权限不足**：提示用户检查工作目录权限
- **平台不兼容**：构建时确保平台匹配

### 5. 应用更新集成设计

#### 5.1 更新机制设计

**更新策略**：
- **自动检查**：应用启动时自动检查更新
- **手动检查**：用户点击按钮主动检查
- **后台更新**：不干扰用户正常使用
- **强制更新**：关键安全更新时强制升级

**更新流程**：
```
启动检查 → 版本对比 → 下载更新 → 验证签名 → 安装更新 → 重启应用
    ↓          ↓          ↓          ↓          ↓          ↓
  定时/手动  GitHub API  下载进度   数字签名   替换文件   自动重启
```

**用户体验设计**：
1. **非侵入式检查**：后台静默检查，不影响正常使用
2. **进度反馈**：下载和安装过程显示进度条
3. **用户选择**：允许用户选择立即更新或稍后更新
4. **回滚机制**：更新失败时自动回滚到旧版本

#### 5.2 更新按钮设计

**按钮状态**：
```
🔄 检查应用更新     (正常状态，蓝色)
🔍 检查中...       (检查中，蓝色+旋转动画)
🆙 有可用更新       (发现更新，橙色+闪烁)
⬇️ 下载中... 45%   (下载中，绿色+进度)
🔄 安装中...       (安装中，绿色+旋转)
✅ 更新完成         (完成，绿色)
❌ 更新失败         (失败，红色)
```

**交互逻辑**：
- 点击检查更新：调用 Tauri updater API
- 发现更新：显示更新对话框，询问用户是否更新
- 用户确认：开始下载和安装流程
- 下载完成：提示用户重启应用

#### 5.3 Tauri Updater 插件集成

**基础配置** (tauri.conf.json)：
```json
{
  "plugins": {
    "updater": {
      "active": true,
      "endpoints": [
        "https://github.com/soddygo/duck_client/releases/latest/download/latest.json"
      ],
      "dialog": true,
      "pubkey": "YOUR_PUBLIC_KEY_HERE"
    }
  }
}
```

**权限配置** (capabilities/default.json)：
```json
{
  "permissions": [
    "updater:default",
    "updater:allow-check",
    "updater:allow-download", 
    "updater:allow-install",
    "updater:allow-download-and-install"
  ]
}
```

**前端更新检查实现**：
```typescript
import { check, Update } from '@tauri-apps/plugin-updater';
import { relaunch } from '@tauri-apps/plugin-process';

interface UpdateState {
  isChecking: boolean;
  updateAvailable: boolean;
  currentVersion: string;
  newVersion?: string;
  downloadProgress?: number;
  error?: string;
}

export class UpdateManager {
  private state: UpdateState = {
    isChecking: false,
    updateAvailable: false,
    currentVersion: '1.0.0',
  };

  // 检查更新
  async checkForUpdates(): Promise<void> {
    this.state.isChecking = true;
    this.state.error = undefined;

    try {
      const update = await check({
        timeout: 30000,
        headers: {
          'User-Agent': 'Duck-Client-GUI'
        }
      });

      if (update?.available) {
        this.state.updateAvailable = true;
        this.state.newVersion = update.version;
        console.log(`发现新版本: ${update.version}`);
        return update;
      } else {
        console.log('已是最新版本');
        this.state.updateAvailable = false;
      }
    } catch (error) {
      this.state.error = `检查更新失败: ${error}`;
      console.error('检查更新失败:', error);
      throw error;
    } finally {
      this.state.isChecking = false;
    }
  }

  // 下载并安装更新
  async downloadAndInstall(update: Update): Promise<void> {
    try {
      console.log('开始下载更新...');
      
      // 监听下载进度
      await update.downloadAndInstall((event) => {
        switch (event.event) {
          case 'Started':
            console.log('开始下载更新包');
            this.state.downloadProgress = 0;
            break;
          case 'Progress':
            const progress = Math.round((event.data.chunkLength / event.data.contentLength) * 100);
            this.state.downloadProgress = progress;
            console.log(`下载进度: ${progress}%`);
            break;
          case 'Finished':
            console.log('更新下载完成，准备安装...');
            this.state.downloadProgress = 100;
            break;
        }
      });

      console.log('更新安装完成，准备重启应用...');
      
      // 在Windows上会自动退出，其他平台需要手动重启
      await relaunch();
      
    } catch (error) {
      this.state.error = `更新失败: ${error}`;
      console.error('更新失败:', error);
      throw error;
    }
  }

  // 获取当前状态
  getState(): UpdateState {
    return { ...this.state };
  }
}
```

**后端更新检查实现**：
```rust
use tauri::{command, AppHandle, State};
use tauri_plugin_updater::{Update, UpdaterExt};
use serde::{Deserialize, Serialize};

#[derive(Debug, Serialize, Deserialize)]
pub struct UpdateInfo {
    pub available: bool,
    pub current_version: String,
    pub new_version: Option<String>,
    pub body: Option<String>,
    pub published_date: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct DownloadProgress {
    pub chunk_length: usize,
    pub content_length: Option<u64>,
    pub progress_percent: f64,
}

// 检查更新
#[command]
pub async fn check_for_updates(app: AppHandle) -> Result<UpdateInfo, String> {
    let update = app
        .updater_builder()
        .timeout(std::time::Duration::from_secs(30))
        .header("User-Agent", "Duck-Client-GUI")
        .build()
        .map_err(|e| format!("构建更新器失败: {}", e))?
        .check()
        .await
        .map_err(|e| format!("检查更新失败: {}", e))?;

    let current_version = app.package_info().version.to_string();

    if let Some(update) = update {
        Ok(UpdateInfo {
            available: true,
            current_version,
            new_version: Some(update.version.clone()),
            body: update.body.clone(),
            published_date: update.date.map(|d| d.to_string()),
        })
    } else {
        Ok(UpdateInfo {
            available: false,
            current_version,
            new_version: None,
            body: None,
            published_date: None,
        })
    }
}

// 下载并安装更新
#[command]
pub async fn download_and_install_update(app: AppHandle) -> Result<(), String> {
    let update = app
        .updater_builder()
        .timeout(std::time::Duration::from_secs(120))
        .on_before_exit(|| {
            println!("应用即将退出以安装更新...");
        })
        .build()
        .map_err(|e| format!("构建更新器失败: {}", e))?
        .check()
        .await
        .map_err(|e| format!("检查更新失败: {}", e))?;

    if let Some(update) = update {
        update
            .download_and_install(
                |chunk_length, content_length| {
                    let progress = if let Some(total) = content_length {
                        (chunk_length as f64 / total as f64) * 100.0
                    } else {
                        0.0
                    };
                    
                    // 发送进度到前端
                    let _ = app.emit("update-progress", DownloadProgress {
                        chunk_length,
                        content_length,
                        progress_percent: progress,
                    });
                },
                || {
                    // 下载完成
                    let _ = app.emit("update-downloaded", ());
                },
            )
            .await
            .map_err(|e| format!("下载安装更新失败: {}", e))?;
            
        Ok(())
    } else {
        Err("没有可用的更新".to_string())
    }
}

// 获取当前版本信息
#[command]
pub fn get_app_version(app: AppHandle) -> String {
    app.package_info().version.to_string()
}
```

**React组件示例**：
```tsx
import React, { useState, useEffect } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';

interface UpdateInfo {
  available: boolean;
  currentVersion: string;
  newVersion?: string;
  body?: string;
}

export const UpdateButton: React.FC = () => {
  const [updateInfo, setUpdateInfo] = useState<UpdateInfo | null>(null);
  const [checking, setChecking] = useState(false);
  const [downloading, setDownloading] = useState(false);
  const [progress, setProgress] = useState(0);

  useEffect(() => {
    // 监听下载进度
    const unlistenProgress = listen('update-progress', (event: any) => {
      setProgress(Math.round(event.payload.progress_percent));
    });

    const unlistenDownloaded = listen('update-downloaded', () => {
      setDownloading(false);
      alert('更新下载完成，应用即将重启...');
    });

    return () => {
      unlistenProgress.then(fn => fn());
      unlistenDownloaded.then(fn => fn());
    };
  }, []);

  const checkForUpdates = async () => {
    setChecking(true);
    try {
      const result = await invoke<UpdateInfo>('check_for_updates');
      setUpdateInfo(result);
      
      if (result.available) {
        const shouldUpdate = confirm(
          `发现新版本 ${result.newVersion}！\n当前版本: ${result.currentVersion}\n\n是否立即更新？`
        );
        if (shouldUpdate) {
          await downloadAndInstall();
        }
      } else {
        alert('已是最新版本！');
      }
    } catch (error) {
      alert(`检查更新失败: ${error}`);
    } finally {
      setChecking(false);
    }
  };

  const downloadAndInstall = async () => {
    setDownloading(true);
    setProgress(0);
    try {
      await invoke('download_and_install_update');
    } catch (error) {
      alert(`更新失败: ${error}`);
      setDownloading(false);
    }
  };

  const getButtonText = () => {
    if (checking) return '检查中...';
    if (downloading) return `下载中... ${progress}%`;
    if (updateInfo?.available) return '🆙 有可用更新';
    return '🔄 检查应用更新';
  };

  const getButtonColor = () => {
    if (updateInfo?.available) return 'bg-orange-500 animate-pulse';
    if (downloading) return 'bg-green-500';
    return 'bg-blue-500';
  };

  return (
    <button
      onClick={checkForUpdates}
      disabled={checking || downloading}
      className={`
        px-4 py-2 text-white rounded-md font-medium
        ${getButtonColor()}
        hover:opacity-80 disabled:opacity-50
        transition-all duration-200
      `}
    >
      {getButtonText()}
    </button>
  );
};
```

**安全配置与最佳实践**：

1. **数字签名配置**：
```bash
# 生成密钥对
tauri signer generate -w ~/.tauri/myapp.key

# 获取公钥
tauri signer sign -k ~/.tauri/myapp.key --password "$TAURI_SIGNING_PRIVATE_KEY_PASSWORD" /path/to/app.tar.gz
```

2. **GitHub Actions自动签名**：
```yaml
- name: Sign and create update
  env:
    TAURI_SIGNING_PRIVATE_KEY: ${{ secrets.TAURI_SIGNING_PRIVATE_KEY }}
    TAURI_SIGNING_PRIVATE_KEY_PASSWORD: ${{ secrets.TAURI_SIGNING_PRIVATE_KEY_PASSWORD }}
  run: |
    # 构建时自动签名
    deno task tauri build
```

3. **更新端点配置**：
- **主端点**：`https://github.com/soddygo/duck_client/releases/latest/download/latest.json`
- **备用端点**：CDN镜像地址（可选）
- **企业端点**：内部更新服务器（可选）

4. **版本兼容性检查**：
```rust
.version_comparator(|current, update| {
    // 自定义版本比较逻辑
    semver::Version::parse(&update.version)
        .and_then(|update_ver| {
            semver::Version::parse(&current)
                .map(|current_ver| update_ver > current_ver)
        })
        .unwrap_or(false)
})
```

**错误处理和回滚策略**：
- **网络错误**：提供重试机制，支持离线检查
- **下载失败**：保留旧版本，确保应用可用
- **安装失败**：自动回滚到安装前状态
- **签名验证失败**：拒绝安装，显示安全警告

## 实施计划

### Phase 1: 基础框架搭建 (Week 1-2)
**目标**：建立基本的应用框架和工作目录管理

#### ✅ 已完成：项目创建
使用官方命令创建了 Tauri 2.0 项目：
```bash
cargo create-tauri-app
# ✔ Project name · cli-ui
# ✔ Identifier · com.soddy.cli-ui  
# ✔ Choose which language to use for your frontend · TypeScript / JavaScript
# ✔ Choose your package manager · npm
# ✔ Choose your UI template · React
# ✔ Choose your UI flavor · TypeScript
```

#### 🔄 进行中：基础配置
1. **项目初始化**
   - ✅ 创建 Tauri 2.0 + React + TypeScript 项目结构
   - ✅ 集成到现有 workspace 配置
   - 🔄 配置 Tailwind CSS 和基础样式
   - 🔄 设置开发环境和构建流程

2. **工作目录管理**
   - ✅ 技术设计完成
   - 🔄 实现工作目录选择、验证和存储 (Dialog + FS 插件)
   - 🔄 开发首次使用引导弹窗
   - 🔄 建立目录状态管理和UI联动

3. **基础布局**
   - ✅ 设计方案完成
   - 🔄 创建分割式界面布局（工作目录栏+操作面板+终端）
   - 🔄 实现可拖拽的上下分割线
   - 🔄 建立响应式设计基础

#### 🔄 下一步：完善开发环境
- 配置 Tailwind CSS 样式系统
- 设置 Vite 开发服务器配置
- 集成 Dialog 和 FS 插件基础配置
- 建立组件和类型定义结构

### Phase 2: 核心功能实现 (Week 3-4)
**目标**：实现所有主要功能按钮和CLI集成（Sidecar + Shell双方案）
1. 🔄 **CLI集成方案实现**
   - 配置Tauri sidecar和Shell插件及权限设置
   - 实现构建时CLI工具下载和打包（Sidecar）
   - 实现跨平台Shell命令执行器（Shell）
   - 开发混合执行策略和自动降级机制
   - 集成实时命令输出流处理

2. 🔄 **功能按钮实现**
   - 实现所有11个核心功能按钮
   - 建立按钮状态管理（正常/禁用/执行中/错误）
   - 集成双方案命令调用和参数构建
   - 实现CLI工具检测和版本管理

3. 🔄 **终端窗口开发**
   - 实现实时命令输出显示
   - 开发交互式命令输入功能
   - 建立命令历史记录和导航
   - 集成Tauri事件系统进行实时通信
   - 支持Shell/Sidecar模式切换显示

### Phase 3: 高级功能和优化 (Week 5-6)
**目标**：完善用户体验和应用稳定性
1. ⏳ **应用更新集成**
   - 集成 Tauri updater 插件
   - 实现自动和手动更新检查
   - 建立更新进度显示和用户确认流程

2. ⏳ **错误处理和反馈**
   - 完善各种错误情况的处理
   - 实现用户友好的错误提示和解决建议
   - 建立日志记录和调试机制

3. ⏳ **界面优化**
   - 优化交互动画和视觉效果
   - 改进响应性能和内存使用
   - 完善无障碍访问支持

### Phase 4: 测试和发布 (Week 7-8)
**目标**：确保应用质量并发布第一个稳定版本
1. ⏳ **跨平台测试**
   - Windows 10/11 测试
   - macOS 测试
   - Linux 主要发行版测试

2. ⏳ **功能测试**
   - 所有功能按钮的完整测试
   - 工作目录管理的边界测试
   - 错误恢复和异常处理测试

3. ⏳ **构建和发布**
   - 配置 GitHub Actions 自动构建
   - 设置多平台打包和签名
   - 发布到 GitHub Releases

## 技术依赖

### 前端依赖 (package.json)
基于官方 `cargo create-tauri-app` 模板，使用 npm 包管理器：

```json
{
  "name": "cli-ui",
  "private": true,
  "version": "0.1.0",
  "type": "module",
  "scripts": {
    "dev": "vite",
    "build": "tsc && vite build",
    "preview": "vite preview",
    "tauri": "tauri"
  },
  "dependencies": {
    "react": "^18.0.0",
    "react-dom": "^18.0.0",
    "@tauri-apps/api": "^2.0.0",
    "@tauri-apps/plugin-updater": "^2.0.0",
    "@tauri-apps/plugin-shell": "^2.0.0",
    "@tauri-apps/plugin-process": "^2.0.0",
    "@tauri-apps/plugin-dialog": "^2.0.0",
    "@tauri-apps/plugin-fs": "^2.0.0"
  },
  "devDependencies": {
    "@types/react": "^18.0.0",
    "@types/react-dom": "^18.0.0",
    "@vitejs/plugin-react": "^4.0.0",
    "typescript": "^5.0.0",
    "vite": "^5.0.0",
    "tailwindcss": "^3.0.0",
    "autoprefixer": "^10.0.0",
    "postcss": "^8.0.0"
  }
}
```

### 后端依赖 (cli-ui/src-tauri/Cargo.toml)
基于 Tauri 2.0 官方模板，集成所需插件：

```toml
[package]
name = "cli-ui"
version = "0.1.0"
description = "Duck CLI GUI Application"
authors = ["soddy"]
edition = "2021"

[build-dependencies]
tauri-build = { version = "2.0", features = [] }

[dependencies]
# Tauri 核心和插件
tauri = { version = "2.0", features = ["protocol-asset"] }
tauri-plugin-updater = "2.0"
tauri-plugin-shell = "2.0"
tauri-plugin-process = "2.0"
tauri-plugin-dialog = "2.0"
tauri-plugin-fs = "2.0"

# 序列化和异步支持
serde = { version = "1.0", features = ["derive"] }
serde_json = "1.0"
tokio = { version = "1.0", features = ["full"] }

# 时间和版本处理
chrono = { version = "0.4", features = ["serde"] }
semver = "1.0"
uuid = { version = "1.0", features = ["v4"] }

# 网络和文件处理 (用于CLI工具下载)
reqwest = { version = "0.11", features = ["json"] }
zip = "0.6"
tar = "0.4"
flate2 = "1.0"

# 本地依赖 (workspace 共享库)
client-core = { path = "../../client-core" }

[target.'cfg(not(any(target_os = "android", target_os = "ios")))'.dependencies]
tauri-plugin-updater = "2.0"

[features]
default = ["custom-protocol"]
custom-protocol = ["tauri/custom-protocol"]
```

### Workspace 配置
需要在根目录的 `Cargo.toml` 中添加新的 `cli-ui` 模块：

```toml
# duck_client/Cargo.toml (workspace根目录)
[workspace]
members = [
    "duck-cli",
    "client-core", 
    "cli-ui/src-tauri"  # 新增 Tauri 后端模块
]

# 共享依赖版本管理
[workspace.dependencies]
serde = { version = "1.0", features = ["derive"] }
tokio = { version = "1.0", features = ["full"] }
chrono = { version = "0.4", features = ["serde"] }
```

### 开发和构建命令

**开发环境启动**：
```bash
# 在 cli-ui 目录下
cd cli-ui

# 安装前端依赖
npm install

# 启动开发服务器 (前端 + 后端热重载)
npm run tauri dev

# 或者分别启动
npm run dev          # 仅前端开发服务器
npm run tauri dev    # Tauri + 前端完整开发环境
```

**生产环境构建**：
```bash
# 构建生产版本
npm run tauri build

# 构建产物位置：
# - Windows: cli-ui/src-tauri/target/release/bundle/msi/
# - macOS: cli-ui/src-tauri/target/release/bundle/dmg/
# - Linux: cli-ui/src-tauri/target/release/bundle/appimage/
```

### Tauri 配置 (cli-ui/src-tauri/tauri.conf.json)

```json
{
  "$schema": "../node_modules/@tauri-apps/cli/schema.json",
  "productName": "Duck CLI GUI",
  "version": "0.1.0",
  "identifier": "com.soddy.cli-ui",
  "build": {
    "beforeDevCommand": "npm run dev",
    "beforeBuildCommand": "npm run build",
    "devUrl": "http://localhost:1420",
    "frontendDist": "../dist"
  },
  "bundle": {
    "active": true,
    "targets": "all",
    "icon": [
      "icons/32x32.png",
      "icons/128x128.png",
      "icons/128x128@2x.png",
      "icons/icon.icns",
      "icons/icon.ico"
    ],
    "externalBin": [
      {
        "name": "duck-cli",
        "src": "binaries/duck-cli",
        "targets": "all"
      }
    ]
  },
  "plugins": {
    "updater": {
      "active": true,
      "endpoints": [
        "https://github.com/soddygo/duck_client/releases/latest/download/latest.json"
      ],
      "dialog": true
    },
    "shell": {
      "sidecar": true,
      "scope": [
        {
          "name": "duck-cli",
          "sidecar": true,
          "args": true
        }
      ]
    },
    "dialog": {
      "all": true
    },
    "fs": {
      "scope": ["$APPDATA/duck-client/**", "$APPLOG/duck-client/**"]
    }
  },
  "app": {
    "windows": [
      {
        "title": "Duck CLI GUI",
        "width": 1200,
        "height": 800,
        "resizable": true,
        "minimizable": true,
        "maximizable": true,
        "closable": true,
        "center": true
      }
    ]
  }
}
```

**前端Dialog和文件系统使用示例**：

```typescript
import { open, message, ask, confirm, save } from '@tauri-apps/plugin-dialog';
import { 
  readTextFile, 
  writeTextFile, 
  exists, 
  readDir, 
  createDir,
  metadata 
} from '@tauri-apps/plugin-fs';
import { invoke } from '@tauri-apps/api/core';

interface DirectoryPermissions {
  exists: boolean;
  readable: boolean;
  writable: boolean;
  executable: boolean;
  available_space?: number;
  is_empty: boolean;
}

interface WorkingDirectoryConfig {
  path: string;
  last_updated: string;
  user_selected: boolean;
}

export class DirectoryManager {
  // 选择工作目录
  async selectWorkingDirectory(): Promise<string | null> {
    try {
      const selectedPath = await open({
        directory: true,
        title: '选择工作目录',
        defaultPath: '~/',
      });
      
      if (selectedPath) {
        console.log('用户选择的目录:', selectedPath);
        return selectedPath;
      }
      
      return null;
    } catch (error) {
      console.error('选择目录失败:', error);
      await message(`选择目录失败: ${error}`, { 
        title: '错误', 
        kind: 'error' 
      });
      return null;
    }
  }

  // 验证工作目录
  async validateDirectory(path: string): Promise<DirectoryPermissions> {
    try {
      return await invoke<DirectoryPermissions>('validate_working_directory', { path });
    } catch (error) {
      console.error('验证目录失败:', error);
      throw error;
    }
  }

  // 设置工作目录并保存配置
  async setWorkingDirectory(path: string): Promise<boolean> {
    try {
      // 首先验证目录
      const permissions = await this.validateDirectory(path);
      
      if (!permissions.exists) {
        await message('所选目录不存在', { 
          title: '错误', 
          kind: 'error' 
        });
        return false;
      }

      if (!permissions.readable || !permissions.writable) {
        await message('所选目录权限不足，需要读写权限', { 
          title: '权限错误', 
          kind: 'error' 
        });
        return false;
      }

      // 如果目录不为空，询问用户确认
      if (!permissions.is_empty) {
        const confirmed = await confirm(
          `所选目录不为空，继续使用此目录可能会影响现有文件。\n\n目录: ${path}\n\n确定要使用此目录作为工作目录吗？`,
          { 
            title: '确认工作目录', 
            kind: 'warning' 
          }
        );
        
        if (!confirmed) {
          return false;
        }
      }

      // 调用后端保存配置
      await invoke('set_working_directory', { path });
      
      await message(`工作目录已设置为:\n${path}`, { 
        title: '设置成功', 
        kind: 'info' 
      });
      
      return true;
    } catch (error) {
      console.error('设置工作目录失败:', error);
      await message(`设置工作目录失败: ${error}`, { 
        title: '错误', 
        kind: 'error' 
      });
      return false;
    }
  }

  // 获取已保存的工作目录
  async getSavedWorkingDirectory(): Promise<string | null> {
    try {
      return await invoke<string | null>('get_working_directory');
    } catch (error) {
      console.error('获取工作目录失败:', error);
      return null;
    }
  }

  // 导出应用日志
  async exportLogs(): Promise<void> {
    try {
      const savePath = await save({
        title: '导出日志文件',
        defaultPath: 'duck-cli-logs.txt',
        filters: [
          { name: '文本文件', extensions: ['txt'] },
          { name: '所有文件', extensions: ['*'] }
        ]
      });

      if (savePath) {
        const result = await invoke<string | null>('export_logs');
        if (result) {
          await message(`日志已导出到:\n${result}`, { 
            title: '导出成功', 
            kind: 'info' 
          });
        }
      }
    } catch (error) {
      console.error('导出日志失败:', error);
      await message(`导出日志失败: ${error}`, { 
        title: '错误', 
        kind: 'error' 
      });
    }
  }

  // 导入配置文件
  async importConfig(): Promise<void> {
    try {
      const selectedFile = await open({
        title: '导入配置文件',
        filters: [
          { name: 'JSON 文件', extensions: ['json'] },
          { name: '所有文件', extensions: ['*'] }
        ]
      });

      if (selectedFile) {
        const result = await invoke<string | null>('import_config');
        if (result) {
          await message(`配置已从以下文件导入:\n${result}`, { 
            title: '导入成功', 
            kind: 'info' 
          });
          
          // 重新加载应用配置
          window.location.reload();
        }
      }
    } catch (error) {
      console.error('导入配置失败:', error);
      await message(`导入配置失败: ${error}`, { 
        title: '错误', 
        kind: 'error' 
      });
    }
  }

  // 显示错误对话框
  async showError(title: string, message: string): Promise<void> {
    await message(message, { title, kind: 'error' });
  }

  // 显示确认对话框
  async showConfirm(title: string, message: string): Promise<boolean> {
    return await confirm(message, { title, kind: 'warning' });
  }

  // 显示信息对话框
  async showInfo(title: string, message: string): Promise<void> {
    await message(message, { title, kind: 'info' });
  }
}

// React Hook 示例
import React, { useState, useEffect } from 'react';

export const useDirectoryManager = () => {
  const [workingDirectory, setWorkingDirectory] = useState<string | null>(null);
  const [directoryValid, setDirectoryValid] = useState<boolean>(false);
  const [loading, setLoading] = useState<boolean>(true);
  const dirManager = new DirectoryManager();

  useEffect(() => {
    loadSavedDirectory();
  }, []);

  const loadSavedDirectory = async () => {
    setLoading(true);
    try {
      const savedDir = await dirManager.getSavedWorkingDirectory();
      if (savedDir) {
        setWorkingDirectory(savedDir);
        const permissions = await dirManager.validateDirectory(savedDir);
        setDirectoryValid(permissions.exists && permissions.readable && permissions.writable);
      }
    } catch (error) {
      console.error('加载工作目录失败:', error);
    } finally {
      setLoading(false);
    }
  };

  const selectAndSetDirectory = async () => {
    const selectedPath = await dirManager.selectWorkingDirectory();
    if (selectedPath) {
      const success = await dirManager.setWorkingDirectory(selectedPath);
      if (success) {
        setWorkingDirectory(selectedPath);
        setDirectoryValid(true);
      }
    }
  };

  return {
    workingDirectory,
    directoryValid,
    loading,
    selectAndSetDirectory,
    dirManager,
    reloadDirectory: loadSavedDirectory
  };
};

// React 组件示例
export const WorkingDirectoryBar: React.FC = () => {
  const { 
    workingDirectory, 
    directoryValid, 
    loading, 
    selectAndSetDirectory,
    dirManager 
  } = useDirectoryManager();

  const getStatusIcon = () => {
    if (loading) return '⏳';
    if (directoryValid) return '✅';
    return '❌';
  };

  const getStatusText = () => {
    if (loading) return '检查中...';
    if (directoryValid) return '有效';
    return '无效';
  };

  const getStatusColor = () => {
    if (loading) return 'bg-yellow-100 border-yellow-300';
    if (directoryValid) return 'bg-green-100 border-green-300';
    return 'bg-red-100 border-red-300';
  };

  return (
    <div className={`flex items-center justify-between p-3 border-b-2 ${getStatusColor()}`}>
      <div className="flex items-center space-x-3">
        <span className="text-lg">📁</span>
        <div>
          <span className="font-medium">工作目录: </span>
          <span className="font-mono text-sm">
            {workingDirectory || '未设置'}
          </span>
        </div>
        <div className="flex items-center space-x-1">
          <span>{getStatusIcon()}</span>
          <span className="text-sm font-medium">{getStatusText()}</span>
        </div>
      </div>
      
      <div className="flex space-x-2">
        <button
          onClick={selectAndSetDirectory}
          className="px-3 py-1 bg-blue-500 text-white rounded hover:bg-blue-600 transition-colors"
        >
          {workingDirectory ? '更改目录' : '选择目录'}
        </button>
        
        {directoryValid && (
          <>
            <button
              onClick={() => dirManager.exportLogs()}
              className="px-3 py-1 bg-gray-500 text-white rounded hover:bg-gray-600 transition-colors"
            >
              导出日志
            </button>
            <button
              onClick={() => dirManager.importConfig()}
              className="px-3 py-1 bg-purple-500 text-white rounded hover:bg-purple-600 transition-colors"
            >
              导入配置
            </button>
          </>
        )}
      </div>
    </div>
  );
};
```

**主程序初始化**：
```rust
#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_shell::init())
        .plugin(tauri_plugin_process::init())
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_fs::init())
        .setup(|app| {
            #[cfg(desktop)]
            {
                // 初始化updater插件
                app.handle().plugin(tauri_plugin_updater::Builder::new().build())?;
                
                // 应用启动时检查更新（可选）
                let app_handle = app.handle().clone();
                tokio::spawn(async move {
                    tokio::time::sleep(tokio::time::Duration::from_secs(3)).await;
                    if let Err(e) = check_for_updates(app_handle).await {
                        eprintln!("启动时检查更新失败: {}", e);
                    }
                });
            }
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            execute_duck_cli_command,
            execute_duck_cli_shell,
            check_duck_cli_installed,
            get_installed_duck_cli_version,
            select_directory,
            validate_working_directory,
            set_working_directory,
            get_working_directory,
            show_error_dialog,
            show_confirm_dialog,
            export_logs,
            import_config,
            #[cfg(desktop)]
            check_for_updates,
            #[cfg(desktop)]
            download_and_install_update,
            get_app_version
        ])
        .run(tauri::generate_context!())
        .expect("运行Tauri应用时出错");
}
```

### 6. Dialog 和 File System 插件深度集成

基于 [Tauri Dialog 插件](https://tauri.app/plugin/dialog/) 和 [Tauri File System 插件](https://tauri.app/plugin/file-system/)，我们为应用提供了丰富的用户交互和文件操作功能。

#### 6.1 Dialog 插件应用场景

**核心功能映射**：
| Dialog类型 | 使用场景 | API方法 | 示例用途 |
|-----------|----------|---------|----------|
| **文件/目录选择** | 工作目录选择 | `open({ directory: true })` | 首次设置和更改工作目录 |
| **文件保存** | 日志导出 | `save()` | 导出命令执行日志到文件 |
| **确认对话框** | 危险操作确认 | `confirm()` | 删除备份、重启服务确认 |
| **询问对话框** | 用户选择 | `ask()` | 是否覆盖现有配置文件 |
| **消息对话框** | 状态通知 | `message()` | 操作成功/失败反馈 |

**实际应用示例**：

1. **工作目录设置流程**：
```typescript
// 1. 目录选择
const path = await open({ directory: true, title: '选择工作目录' });

// 2. 非空目录确认  
if (!isEmpty) {
  const confirmed = await confirm(
    '目录不为空，继续使用可能影响现有文件。确定吗？',
    { title: '确认工作目录', kind: 'warning' }
  );
}

// 3. 设置成功通知
await message('工作目录设置成功！', { kind: 'info' });
```

2. **危险操作确认**：
```typescript
// 服务重启确认
const shouldRestart = await confirm(
  '重启服务将中断所有连接，确定要继续吗？',
  { title: '重启服务', kind: 'warning' }
);

// 备份删除确认  
const shouldDelete = await ask(
  '删除备份文件无法恢复，确定要删除吗？',
  { title: '删除备份', kind: 'warning' }
);
```

#### 6.2 File System 插件应用场景

**核心功能映射**：
| 操作类型 | 使用场景 | API方法 | 安全范围 |
|---------|----------|---------|----------|
| **目录检查** | 工作目录验证 | `exists()`, `metadata()` | 用户选择的路径 |
| **权限检查** | 读写权限验证 | `readDir()`, `writeTextFile()` | 工作目录及子目录 |
| **配置管理** | 应用设置存储 | `readTextFile()`, `writeTextFile()` | `$APPDATA/duck-client/` |
| **日志管理** | 日志收集导出 | `readDir()`, `readTextFile()` | `$APPLOG/duck-client/` |
| **备份操作** | 配置备份恢复 | `createDir()`, `writeTextFile()` | 工作目录 |

**权限安全设计**：
```json
{
  "fs:scope-appdata": true,           // 应用数据目录
  "fs:scope-appdata-recursive": true, // 应用数据子目录  
  "fs:scope-applog": true,            // 应用日志目录
  "fs:scope-applog-recursive": true,  // 应用日志子目录
  "fs:allow-read-text-file": [        // 限制读取范围
    { "path": "$APPDATA/duck-client/**" }
  ],
  "fs:allow-write-text-file": [       // 限制写入范围
    { "path": "$APPDATA/duck-client/**" }
  ]
}
```

#### 6.3 用户体验优化设计

**智能错误处理**：
```typescript
export class SmartErrorHandler {
  async handleDirectoryError(error: string, path: string) {
    if (error.includes('权限')) {
      await message(
        `目录权限不足：${path}\n\n解决方案：\n1. 选择其他目录\n2. 修改目录权限\n3. 以管理员身份运行`,
        { title: '权限错误', kind: 'error' }
      );
    } else if (error.includes('不存在')) {
      const shouldCreate = await confirm(
        `目录不存在：${path}\n\n是否创建此目录？`,
        { title: '目录不存在', kind: 'warning' }
      );
      // 处理创建逻辑...
    }
  }
}
```

**进度反馈机制**：
```typescript
export class OperationProgress {
  async exportLogsWithProgress() {
    // 1. 显示选择对话框
    await message('正在打开文件保存对话框...', { kind: 'info' });
    
    const savePath = await save({
      title: '导出日志文件',
      defaultPath: `duck-cli-logs-${new Date().toISOString().split('T')[0]}.txt`
    });
    
    if (savePath) {
      // 2. 显示处理进度
      await message('正在收集日志文件...', { kind: 'info' });
      
      // 3. 执行导出
      const result = await invoke('export_logs');
      
      // 4. 完成通知
      await message(
        `日志已成功导出到：\n${savePath}\n\n包含 ${result.fileCount} 个日志文件`,
        { title: '导出完成', kind: 'info' }
      );
    }
  }
}
```

#### 6.4 文件操作最佳实践

**配置文件管理**：
```rust
// 后端：安全的配置文件操作
#[command]
async fn save_app_config(
    app: AppHandle,
    config: AppConfig,
) -> Result<(), String> {
    let fs = app.fs();
    
    // 确保配置目录存在
    let config_dir = app.path().app_data_dir()?;
    if !fs.exists(&config_dir).await? {
        fs.create_dir_all(&config_dir).await?;
    }
    
    // 备份现有配置
    let config_file = config_dir.join("config.json");
    if fs.exists(&config_file).await? {
        let backup_file = config_dir.join("config.backup.json");
        let current_config = fs.read_text_file(&config_file).await?;
        fs.write_text_file(&backup_file, &current_config).await?;
    }
    
    // 写入新配置
    let config_json = serde_json::to_string_pretty(&config)?;
    fs.write_text_file(&config_file, &config_json).await?;
    
    Ok(())
}
```

**临时文件清理**：
```typescript
export class TempFileManager {
  private tempFiles: string[] = [];
  
  async createTempFile(name: string, content: string): Promise<string> {
    const tempPath = await invoke<string>('create_temp_file', { name, content });
    this.tempFiles.push(tempPath);
    return tempPath;
  }
  
  async cleanup() {
    for (const tempFile of this.tempFiles) {
      try {
        await invoke('remove_file', { path: tempFile });
      } catch (error) {
        console.warn('清理临时文件失败:', tempFile, error);
      }
    }
    this.tempFiles = [];
  }
  
  // 应用关闭时自动清理
  async beforeAppClose() {
    await this.cleanup();
  }
}
```

## 安全考虑

1. **命令执行安全**：通过Sidecar和权限系统限制可执行的命令范围
2. **文件系统访问**：使用 Tauri 的安全文件系统 API，限制访问范围到应用数据目录
3. **对话框安全**：所有用户交互通过 Tauri Dialog 插件，避免恶意弹窗
4. **路径验证**：严格验证用户选择的路径，防止路径遍历攻击
5. **网络请求**：仅允许从可信的GitHub端点下载
6. **更新安全**：使用数字签名验证更新包完整性
7. **权限最小化**：只授予必要的系统权限，采用白名单方式

## 部署策略

### 1. CI/CD 构建流程

**GitHub Actions 工作流程**：
```yaml
name: Build and Release Tauri App

on:
  push:
    tags: [ 'v*' ]
  workflow_dispatch:

jobs:
  build-tauri:
    strategy:
      matrix:
        platform: 
          - macos-latest
          - ubuntu-latest  
          - windows-latest
    
    runs-on: ${{ matrix.platform }}
    
    steps:
      - uses: actions/checkout@v3
      
      - name: Setup Rust
        uses: dtolnay/rust-toolchain@stable
        
      - name: Setup Node.js
        uses: actions/setup-node@v3
        with:
          node-version: 18
          
      - name: Setup Deno
        uses: denoland/setup-deno@v1
        
      - name: Download duck-cli for current platform
        run: |
          # 在构建脚本中自动下载对应平台的duck-cli
          # 确保与当前发布版本同步
          
      - name: Build Tauri App
        env:
          TAURI_SIGNING_PRIVATE_KEY: ${{ secrets.TAURI_SIGNING_PRIVATE_KEY }}
          TAURI_SIGNING_PRIVATE_KEY_PASSWORD: ${{ secrets.TAURI_SIGNING_PRIVATE_KEY_PASSWORD }}
        run: |
          cd client-ui
          deno task tauri build
          
      - name: Generate updater metadata
        run: |
          # 生成latest.json文件供Tauri updater使用
          python scripts/generate_updater_metadata.py
          
      - name: Upload Release Assets
        uses: softprops/action-gh-release@v1
        with:
          files: |
            client-ui/src-tauri/target/release/bundle/**/*
            latest.json
        env:
          GITHUB_TOKEN: ${{ secrets.GITHUB_TOKEN }}
```

### 2. 版本同步机制

**版本管理策略**：
- GUI应用版本号与duck-cli保持同步（如都使用v1.0.10）
- 构建时自动获取最新的duck-cli版本
- 通过环境变量传递版本信息到应用中
- 确保sidecar打包的CLI工具与发布版本一致

**构建时版本检查**：
```rust
// build.rs
fn main() {
    // 获取当前的git标签作为版本号
    let version = env::var("CARGO_PKG_VERSION").unwrap();
    
    // 从GitHub API获取最新的duck-cli版本
    let cli_version = fetch_latest_cli_version().unwrap();
    
    // 确保版本匹配或兼容
    if !versions_compatible(&version, &cli_version) {
        panic!("GUI版本{}与CLI版本{}不兼容", version, cli_version);
    }
    
    // 下载对应版本的CLI工具
    download_cli_for_platform(&cli_version);
}
```

### 3. 跨平台构建配置

**平台特定设置**：
| 平台 | 构建器 | 特殊配置 |
|------|--------|----------|
| Windows | windows-latest | 代码签名、UAC权限 |
| macOS | macos-latest | 公证、通用二进制 |
| Linux | ubuntu-latest | AppImage、deb包 |

**构建产物结构**：
```
releases/
├── duck-client-v1.0.10-windows-x64.msi
├── duck-client-v1.0.10-windows-arm64.msi  
├── duck-client-v1.0.10-macos-universal.dmg
├── duck-client-v1.0.10-linux-amd64.AppImage
├── duck-client-v1.0.10-linux-arm64.AppImage
└── latest.json  # Tauri updater配置文件
```

### 4. 发布和分发

**发布流程**：
1. **标签创建**：创建新的git标签触发构建
2. **自动构建**：GitHub Actions并行构建所有平台
3. **质量检查**：自动化测试和签名验证
4. **发布到Releases**：自动创建GitHub Release
5. **更新通知**：通过Tauri updater通知用户

**分发渠道**：
- **主要渠道**：GitHub Releases
- **备用镜像**：国内CDN镜像（可选）
- **企业分发**：内部包管理系统（可选）

### 5. Updater元数据生成

**latest.json文件格式**：
```json
{
  "version": "1.0.10",
  "pub_date": "2024-01-15T10:30:00.000Z",
  "platforms": {
    "windows-x86_64": {
      "signature": "signature_here",
      "url": "https://github.com/soddygo/duck_client/releases/download/v1.0.10/duck-client-v1.0.10-windows-x64.msi.tar.gz"
    },
    "windows-aarch64": {
      "signature": "signature_here", 
      "url": "https://github.com/soddygo/duck_client/releases/download/v1.0.10/duck-client-v1.0.10-windows-arm64.msi.tar.gz"
    },
    "darwin-universal": {
      "signature": "signature_here",
      "url": "https://github.com/soddygo/duck_client/releases/download/v1.0.10/duck-client-v1.0.10-macos-universal.app.tar.gz"
    },
    "linux-x86_64": {
      "signature": "signature_here",
      "url": "https://github.com/soddygo/duck_client/releases/download/v1.0.10/duck-client-v1.0.10-linux-amd64.AppImage.tar.gz"
    },
    "linux-aarch64": {
      "signature": "signature_here",
      "url": "https://github.com/soddygo/duck_client/releases/download/v1.0.10/duck-client-v1.0.10-linux-arm64.AppImage.tar.gz"
    }
  },
  "notes": "# 更新日志\n\n## 新增功能\n- 新增应用自动更新功能\n- 优化用户界面响应速度\n\n## 修复问题\n- 修复工作目录设置问题\n- 修复命令执行失败的情况"
}
```

**自动生成脚本** (scripts/generate_updater_metadata.py)：
```python
import json
import os
import hashlib
from datetime import datetime, timezone

def generate_updater_metadata():
    # 从环境变量获取版本信息
    version = os.environ.get('GITHUB_REF_NAME', 'v1.0.0').lstrip('v')
    
    # 构建平台信息
    platforms = {}
    base_url = f"https://github.com/soddygo/duck_client/releases/download/v{version}"
    
    platform_files = {
        "windows-x86_64": f"duck-client-v{version}-windows-x64.msi.tar.gz",
        "windows-aarch64": f"duck-client-v{version}-windows-arm64.msi.tar.gz", 
        "darwin-universal": f"duck-client-v{version}-macos-universal.app.tar.gz",
        "linux-x86_64": f"duck-client-v{version}-linux-amd64.AppImage.tar.gz",
        "linux-aarch64": f"duck-client-v{version}-linux-arm64.AppImage.tar.gz"
    }
    
    for platform, filename in platform_files.items():
        # 读取签名文件
        sig_file = f"client-ui/src-tauri/target/release/bundle/{filename}.sig"
        if os.path.exists(sig_file):
            with open(sig_file, 'r') as f:
                signature = f.read().strip()
        else:
            signature = "SIGNATURE_PLACEHOLDER"
            
        platforms[platform] = {
            "signature": signature,
            "url": f"{base_url}/{filename}"
        }
    
    # 生成元数据
    metadata = {
        "version": version,
        "pub_date": datetime.now(timezone.utc).isoformat(),
        "platforms": platforms,
        "notes": generate_release_notes()
    }
    
    # 写入latest.json
    with open('latest.json', 'w', encoding='utf-8') as f:
        json.dump(metadata, f, indent=2, ensure_ascii=False)
    
    print(f"Generated updater metadata for version {version}")

def generate_release_notes():
    # 可以从CHANGELOG.md或git log生成
    return """# 更新日志

## 新增功能
- 新增应用自动更新功能
- 集成Tauri Sidecar CLI工具执行
- 优化用户界面响应速度

## 修复问题  
- 修复工作目录设置和验证问题
- 修复命令执行失败的错误处理
- 改进跨平台兼容性

## 技术改进
- 升级到Tauri 2.0稳定版
- 优化构建流程和打包速度
- 加强安全性和权限控制"""

if __name__ == "__main__":
    generate_updater_metadata()
```

---

## 总结

本设计文档提供了一个完整的技术方案，在现有的 duck_client workspace 中新增 `cli-ui` 模块，将 duck-cli 命令行工具通过 Tauri Sidecar 方式集成到用户友好的 GUI 应用中。

### 项目技术栈

基于 `cargo create-tauri-app` 官方脚手架创建：

```bash
✔ Project name · cli-ui
✔ Identifier · com.soddy.cli-ui  
✔ Frontend language · TypeScript / JavaScript - (npm)
✔ Package manager · npm
✔ UI template · React
✔ UI flavor · TypeScript
```

**技术架构**：
- **前端**：React + TypeScript + Vite + npm  
- **后端**：Rust + Tauri 2.0
- **项目结构**：Cargo workspace 集成
- **官方插件**：Shell、Updater、Process、Dialog、FS
- **CLI 集成**：Sidecar打包 + Shell执行双方案
- **更新机制**：Tauri updater 插件
- **包管理**：npm (前端) + Cargo (后端) + workspace 共享依赖

**与现有项目集成**：
- 复用 `client-core` 共享库
- 独立的 GUI 模块，不影响现有 CLI 和 UI 模块
- 统一的 workspace 依赖管理

通过分割式界面设计和成熟的构建流程，用户可以通过图形界面轻松管理 Docker 服务，同时保留命令行的强大功能和灵活性。

### 核心设计亮点

1. **工作目录优先设计**
   - 所有操作都基于用户选择的工作目录
   - 首次使用引导确保用户正确设置
   - 智能的目录验证和状态管理
   - 无效目录时的功能禁用保护

2. **直观的用户界面**
   - 清晰的三层布局：目录栏 + 操作面板 + 终端
   - 11个核心功能按钮覆盖所有主要操作
   - 实时状态反馈和错误提示
   - 类终端的命令交互体验

3. **灵活可靠的CLI集成**
   - **Sidecar方案**：构建时打包CLI工具，确保版本一致性和离线可用
   - **Shell方案**：运行时执行系统CLI，提供灵活性和跨平台兼容
   - **混合策略**：智能降级机制，优先Sidecar，fallback到Shell
   - 通过Tauri安全机制执行CLI命令，统一权限管理

4. **成熟的构建和发布流程**
   - 与主项目版本同步的自动化构建
   - 跨平台构建支持（Windows/macOS/Linux）
   - GitHub Actions自动化CI/CD
   - 通过Tauri updater实现应用自动更新

5. **良好的用户体验**
   - 离线可用，无需网络连接下载CLI工具
   - 响应式设计适配不同屏幕
   - 跨平台一致的使用体验
   - 渐进式功能展示和错误恢复

**Dialog 和 File System 插件集成总结**

通过集成这两个官方插件，我们的应用获得了以下核心能力：

**Dialog 插件增强功能**：
- 🗂️ **原生文件/目录选择**：系统级的文件选择对话框，支持多种过滤器
- ⚠️ **智能用户确认**：危险操作前的确认机制，提升操作安全性
- 📢 **友好消息提示**：分类的消息对话框（错误/警告/信息），改善用户体验
- 💾 **文件保存引导**：引导用户选择合适的保存位置和文件格式

**File System 插件增强功能**：
- 📁 **安全文件操作**：限制在应用数据目录内的安全文件读写
- 🔍 **智能目录验证**：全面的目录存在性、权限和状态检查
- ⚙️ **配置文件管理**：自动化的配置保存、加载和备份机制
- 📋 **日志收集导出**：统一的日志文件收集和导出功能

**集成效果**：
1. **用户体验提升**：从命令行工具升级为现代化的图形界面应用
2. **操作安全性**：所有文件操作都经过验证和用户确认
3. **配置管理**：完整的配置备份、恢复和迁移能力
4. **调试支持**：便于问题排查的日志导出功能
5. **跨平台一致性**：在不同操作系统上提供统一的交互体验

**安全设计亮点**：
- **权限最小化**：仅访问必要的应用数据和日志目录
- **路径验证**：防止路径遍历和恶意文件操作
- **操作确认**：重要操作前的多层确认机制
- **自动清理**：临时文件的自动管理和清理

### 技术优势

**多方案CLI集成的技术优势**：

**Sidecar方案优势**：
- **安全性更高**：CLI工具在构建时集成，避免运行时下载的安全风险
- **可靠性更强**：离线可用，不依赖网络环境和GitHub可用性
- **版本一致性**：GUI应用与CLI工具版本严格匹配，避免兼容性问题
- **分发简化**：单一安装包包含所有必要组件，用户体验更好

**Shell方案优势**：
- **跨平台统一**：通过[Tauri Shell插件](https://tauri.app/plugin/shell/)自动处理Windows/macOS/Linux差异
- **灵活性强**：支持用户自行管理CLI工具版本，适应不同需求
- **包体积小**：不包含CLI工具，显著减小应用安装包大小
- **开发友好**：开发阶段可直接使用系统安装的CLI工具，无需重新构建

**混合方案优势**：
- **最佳兼容性**：结合两种方案优势，适应各种部署环境
- **智能降级**：Sidecar失败时自动切换到Shell方式，提高成功率
- **权限可控**：通过Tauri权限系统精确控制CLI工具的执行权限
- **用户选择**：允许用户在不同执行模式间切换，满足个性化需求

这个设计充分利用了现有的开源项目生态（GitHub自动构建的跨平台duck-cli工具）和Tauri的现代化插件系统（Sidecar + Shell），通过灵活的多方案集成策略，创建了一个既安全又易用的Docker服务管理GUI应用。

通过提供Sidecar、Shell和混合三种CLI集成方案，我们实现了：
- **生产环境的稳定性**：Sidecar确保版本一致和离线可用
- **开发环境的灵活性**：Shell支持实时CLI工具更新和调试
- **跨平台的兼容性**：统一的API接口屏蔽系统差异
- **用户体验的优化**：智能降级和多模式切换

既保持了命令行工具的强大功能，又通过图形界面和智能的CLI集成策略大大降低了使用门槛，让更多用户能够轻松管理 Docker 服务。

### 项目现状

#### ✅ 已完成
1. **项目创建**：使用 `cargo create-tauri-app` 创建了 `cli-ui` 项目
2. **技术选型**：确定了 React + TypeScript + npm + Tauri 2.0 技术栈
3. **完整设计**：完成了功能设计、UI设计、架构设计和实施计划
4. **插件集成方案**：Dialog、FS、Shell、Updater、Process 插件的完整集成策略

#### 🔄 进行中
1. **开发环境配置**：Tailwind CSS、Vite、插件权限配置
2. **Workspace 集成**：将 `cli-ui/src-tauri` 添加到根目录 Cargo.toml

#### 📋 待实施
1. **Phase 1**：基础框架和工作目录管理 (2周)
2. **Phase 2**：核心功能和CLI集成 (2周) 
3. **Phase 3**：高级功能和优化 (2周)
4. **Phase 4**：测试和发布 (2周)

### 开发建议

**立即开始**：
```bash
cd cli-ui
npm install
npm run tauri dev
```

**优先实现**：
1. 工作目录管理 (Dialog + FS 插件)
2. 基础UI布局 (React + Tailwind)
3. Sidecar CLI 集成 (Shell 插件)
4. 基础功能按钮

该设计文档提供了从概念到实施的完整指导，现在已经有了坚实的项目基础，可以直接开始开发工作。 