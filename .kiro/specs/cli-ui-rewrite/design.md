# Design Document - CLI-UI 现代化界面设计

## Overview

基于用户核心需求，设计简洁、直观、高效的 Docker 服务管理界面。核心目标是让用户能够快速部署启动 Docker 服务，实时查看部署和容器日志，监控服务健康状态，以及管理备份和回滚。

### 设计理念

- **快捷优先**：最核心的"一键部署"功能放在最显眼位置
- **实时反馈**：所有操作都提供实时日志输出，方便排查问题
- **简洁直观**：减少页面层级，所有功能在一个界面内完成
- **渐进式操作**：从简单到复杂，支持快速上手和高级操作

### 技术栈

**前端**：
- React 18 + TypeScript
- Tailwind CSS（原子化 CSS）
- Heroicons（图标库）
- React Router（页面路由）

**后端**：
- Tauri 2.x
- Rust（nuwax-cli 作为库依赖）
- Tokio（异步运行时）

**设计决策**：
- 使用 React Router：支持侧边栏导航和多页面视图
- 不使用 Zustand：使用 React Context + Hooks 管理全局状态
- 使用 Heroicons：已在现有代码中使用，保持一致性
- 响应式设计：支持不同屏幕尺寸（侧边栏可折叠）

## Architecture

### 主界面布局

采用**侧边栏 + 主内容区**的经典布局：

```
┌─────────────────────────────────────────────────────────────┐
│  顶部栏 (Header)                                             │
│  🦆 Duck CLI  |  工作目录: /path/to/project  [更改]          │
├──────────┬──────────────────────────────────────────────────┤
│          │                                                   │
│ 侧边栏   │  主内容区 (MainContent)                           │
│ (Sidebar)│                                                   │
│          │  ┌─ 概览 (Overview) ─────────────────────────┐   │
│ 📊 概览  │  │                                            │   │
│ 🚀 部署  │  │  服务状态: 🟢 运行中 (3/3)                 │   │
│ 📦 容器  │  │                                            │   │
│ 💾 备份  │  │  快速操作:                                 │   │
│ 📋 日志  │  │  [一键部署] [启动服务] [停止服务]          │   │
│          │  │                                            │   │
│          │  │  容器列表:                                 │   │
│          │  │  🟢 mysql    [日志] [重启]                 │   │
│          │  │  🟢 redis    [日志] [重启]                 │   │
│          │  │  🟢 nginx    [日志] [重启]                 │   │
│          │  └────────────────────────────────────────────┘   │
│          │                                                   │
│          │  ┌─ 部署 (Deploy) ────────────────────────────┐   │
│          │  │                                            │   │
│          │  │  当前版本: v1.2.3                          │   │
│          │  │  最新版本: v1.2.4                          │   │
│          │  │                                            │   │
│          │  │  [🚀 一键部署]  [⚙️ 高级选项]              │   │
│          │  │                                            │   │
│          │  │  部署日志:                                 │   │
│          │  │  ┌──────────────────────────────────────┐ │   │
│          │  │  │ [12:34:56] 🚀 开始部署...            │ │   │
│          │  │  │ [12:34:57] 📥 下载服务包...          │ │   │
│          │  │  │ [12:34:58] ✅ 部署成功！             │ │   │
│          │  │  └──────────────────────────────────────┘ │   │
│          │  └────────────────────────────────────────────┘   │
│          │                                                   │
│          │  ┌─ 容器 (Containers) ────────────────────────┐   │
│          │  │                                            │   │
│          │  │  ┌─ mysql ─────────────────────────────┐  │   │
│          │  │  │ 状态: 🟢 运行中                      │  │   │
│          │  │  │ 镜像: mysql:8.0                      │  │   │
│          │  │  │ 端口: 3306:3306                      │  │   │
│          │  │  │ [查看日志] [重启] [停止]             │  │   │
│          │  │  └──────────────────────────────────────┘  │   │
│          │  │                                            │   │
│          │  │  容器日志:                                 │   │
│          │  │  ┌──────────────────────────────────────┐ │   │
│          │  │  │ 2024-01-01 12:34:56 [Note] Ready... │ │   │
│          │  │  │ 2024-01-01 12:34:57 [Note] Started  │ │   │
│          │  │  └──────────────────────────────────────┘ │   │
│          │  └────────────────────────────────────────────┘   │
│          │                                                   │
│          │  ┌─ 备份 (Backup) ────────────────────────────┐   │
│          │  │                                            │   │
│          │  │  [创建备份]                                │   │
│          │  │                                            │   │
│          │  │  备份列表:                                 │   │
│          │  │  ┌──────────────────────────────────────┐ │   │
│          │  │  │ #1  2024-01-01 12:00  v1.2.3  100MB │ │   │
│          │  │  │     [恢复] [删除]                    │ │   │
│          │  │  ├──────────────────────────────────────┤ │   │
│          │  │  │ #2  2024-01-02 12:00  v1.2.4  105MB │ │   │
│          │  │  │     [恢复] [删除]                    │ │   │
│          │  │  └──────────────────────────────────────┘ │   │
│          │  └────────────────────────────────────────────┘   │
│          │                                                   │
│          │  ┌─ 日志 (Logs) ──────────────────────────────┐   │
│          │  │                                            │   │
│          │  │  [清除] [导出] [搜索...]                   │   │
│          │  │                                            │   │
│          │  │  ┌──────────────────────────────────────┐ │   │
│          │  │  │ [12:34:56] [INFO] 🚀 开始执行...     │ │   │
│          │  │  │ [12:34:57] [INFO] 📥 下载中...       │ │   │
│          │  │  │ [12:34:58] [SUCCESS] ✅ 完成！       │ │   │
│          │  │  │ [12:34:59] [ERROR] ❌ 错误信息       │ │   │
│          │  │  └──────────────────────────────────────┘ │   │
│          │  └────────────────────────────────────────────┘   │
│          │                                                   │
└──────────┴──────────────────────────────────────────────────┘
```

**布局优势**：
1. **侧边栏导航**：清晰的功能分类，用户可快速切换不同视图
2. **顶部栏**：显示应用标识和当前工作目录，始终可见
3. **主内容区**：根据侧边栏选择显示不同的功能页面
4. **上下文相关**：每个页面只显示相关的操作和信息

**页面设计**：

1. **概览页面**：
   - 服务整体状态（运行中/已停止）
   - 快速操作按钮（一键部署、启动、停止）
   - 容器列表概览（状态 + 快速操作）

2. **部署页面**：
   - 版本信息（当前版本、最新版本）
   - 部署操作按钮（一键部署、高级选项）
   - 实时部署日志（占据主要空间）

3. **容器页面**：
   - 容器详细信息卡片（状态、镜像、端口）
   - 容器操作按钮（查看日志、重启、停止、启动）
   - 选中容器的实时日志显示

4. **备份页面**：
   - 创建备份按钮
   - 备份列表（时间、版本、大小）
   - 备份操作（恢复、删除）

5. **日志页面**：
   - 全局日志视图
   - 日志操作（清除、导出、搜索）
   - 日志过滤（按类型、时间）

### 核心组件

1. **Header**：顶部栏，显示应用标识和工作目录
2. **Sidebar**：侧边栏导航菜单
3. **MainContent**：主内容区，根据路由显示不同页面
4. **OverviewPage**：概览页面
5. **DeployPage**：部署页面
6. **ContainersPage**：容器管理页面
7. **BackupPage**：备份管理页面
8. **LogsPage**：日志查看页面
9. **ContainerCard**：容器信息卡片
10. **LogTerminal**：日志终端组件（可复用）
11. **WelcomeSetupModal**：首次使用引导
12. **ParameterInputModal**：高级参数输入

## Components and Interfaces

### Tauri 命令接口设计

基于现有 `nuwax-cli` 的 `CliApp` 结构，设计以下 Tauri 命令接口：

```rust
// 核心设计：通过 Tauri 事件系统实现实时日志流式传输
// 所有命令执行都通过 execute_duck_cli_command 统一处理

// 执行 duck-cli 命令（统一入口）
#[tauri::command]
async fn execute_duck_cli_command(
    window: tauri::Window,
    args: Vec<String>,
    working_directory: String,
) -> Result<(), String> {
    // 1. 切换到工作目录
    // 2. 初始化 CliApp（如果需要）
    // 3. 执行命令并通过事件发送实时输出
    //    - 'cli-output': 标准输出
    //    - 'cli-error': 错误输出
    //    - 'cli-complete': 命令完成（包含退出码）
}

// 获取容器状态（用于容器列表）
#[tauri::command]
async fn get_container_status(
    working_directory: String,
) -> Result<Vec<ContainerInfo>, String> {
    // 调用 CliApp.docker_manager.list_containers()
    // 返回容器名称、状态、镜像、端口等信息
}

// 获取容器日志（实时流式）
#[tauri::command]
async fn stream_container_logs(
    window: tauri::Window,
    working_directory: String,
    container_name: String,
    follow: bool,
) -> Result<(), String> {
    // 调用 docker logs -f <container_name>
    // 通过事件发送日志：
    //    - 'container-log': 容器日志输出
    //    - 'container-log-complete': 日志流结束
}

// 停止容器日志流
#[tauri::command]
async fn stop_container_logs(
    container_name: String,
) -> Result<(), String> {
    // 停止正在进行的日志流
}

// 容器操作
#[tauri::command]
async fn start_container(
    working_directory: String,
    container_name: String,
) -> Result<(), String> {
    // 调用 docker start <container_name>
}

#[tauri::command]
async fn stop_container(
    working_directory: String,
    container_name: String,
) -> Result<(), String> {
    // 调用 docker stop <container_name>
}

#[tauri::command]
async fn restart_container(
    working_directory: String,
    container_name: String,
) -> Result<(), String> {
    // 调用 docker restart <container_name>
}

// 获取备份列表（用于回滚选择）
#[tauri::command]
async fn list_backups(
    working_directory: String,
) -> Result<Vec<BackupRecord>, String> {
    // 调用 CliApp.backup_manager.list_backups()
}

// 验证工作目录
#[tauri::command]
async fn validate_working_directory(
    path: String,
) -> Result<DirectoryValidation, String> {
    // 检查目录是否存在、是否包含 config.toml 等
}

// 配置管理
#[tauri::command]
async fn get_working_directory() -> Result<Option<String>, String>

#[tauri::command]
async fn set_working_directory(path: String) -> Result<(), String>
```

**设计决策**：
- **统一命令入口**：全局操作通过 `execute_duck_cli_command` 执行
- **容器专用接口**：容器操作使用专用命令，提供更好的用户体验
- **事件驱动架构**：使用 Tauri 事件系统实现实时日志流式传输
- **日志源切换**：支持全局日志和容器日志的无缝切换
- **工作目录隔离**：每次命令执行都指定工作目录，支持多项目管理

### TypeScript 类型定义

```typescript
// 日志条目
export interface LogEntry {
  id: string;
  timestamp: string;
  type: 'info' | 'success' | 'error' | 'warning' | 'command';
  message: string;
  command?: string;
  args?: string[];
}

// 备份记录
export interface BackupRecord {
  id: number;
  backup_type: 'Manual' | 'PreUpgrade';
  created_at: string;
  service_version: string;
  file_path: string;
  file_size?: number;
  file_exists: boolean;
}

// 工作目录验证结果
export interface DirectoryValidation {
  valid: boolean;
  message: string;
  has_config: boolean;
  has_docker_compose: boolean;
}

// 命令配置（用于参数输入）
export interface CommandConfig {
  id: string;
  name: string;
  description: string;
  parameters: ParameterDefinition[];
}

export interface ParameterDefinition {
  name: string;
  label: string;
  type: 'string' | 'number' | 'boolean' | 'select';
  required: boolean;
  default?: any;
  options?: string[];
  description?: string;
}
```

## Data Models

### 状态管理（React Context + Hooks）

使用 React Context 管理全局状态，避免 props 层层传递。

```typescript
// AppContext.tsx - 全局状态管理
interface AppContextType {
  // 工作目录
  workingDirectory: string | null;
  isDirectoryValid: boolean;
  setWorkingDirectory: (dir: string | null, valid: boolean) => void;
  
  // 全局日志
  logs: LogEntry[];
  addLog: (log: LogEntry) => void;
  clearLogs: () => void;
  
  // 容器状态
  containers: ContainerInfo[];
  refreshContainers: () => Promise<void>;
  
  // 执行状态
  isExecuting: boolean;
  setIsExecuting: (executing: boolean) => void;
}

// 使用 Context
const AppProvider: React.FC = ({ children }) => {
  const [workingDirectory, setWorkingDirectory] = useState<string | null>(null);
  const [logs, setLogs] = useState<LogEntry[]>([]);
  const [containers, setContainers] = useState<ContainerInfo[]>([]);
  
  // ... 其他状态和方法
  
  return (
    <AppContext.Provider value={contextValue}>
      {children}
    </AppContext.Provider>
  );
};

// 自定义 Hook
const useApp = () => {
  const context = useContext(AppContext);
  if (!context) {
    throw new Error('useApp must be used within AppProvider');
  }
  return context;
};

// 日志管理配置
interface LogConfig {
  maxEntries: number;      // 最大日志条目数（默认 1000）
  trimBatchSize: number;   // 清理批次大小（默认 200）
}
```

**设计决策**：
- **Context 管理全局状态**：工作目录、日志、容器状态等全局共享
- **自定义 Hooks**：封装常用逻辑（如 `useApp`、`useContainers`）
- **循环缓冲区**：日志使用循环缓冲区，自动清理旧日志
- **按需刷新**：容器状态按需刷新，避免不必要的轮询

## Error Handling

### 错误处理策略

1. **Rust 后端**：
   - 所有 Tauri 命令返回 `Result<T, String>`
   - 使用 `anyhow` 进行错误传播
   - 错误信息包含上下文和建议

2. **React 前端**：
   - 使用 `ErrorBoundary` 捕获组件渲染错误
   - 使用 `try-catch` 捕获异步操作错误
   - 所有错误都记录到日志窗口

3. **用户友好的错误提示**：
   - 使用 Tauri 的 `dialog` API 显示错误对话框
   - 错误消息使用中文，包含具体原因和解决建议
   - 提供"重试"选项

### 错误分类和处理

| 错误类型 | 处理方式 | 用户提示 |
|---------|---------|---------|
| 工作目录无效 | 禁用所有操作按钮 | "请先设置有效的工作目录" |
| 配置文件缺失 | 提示运行 `init` | "配置文件不存在，请先初始化项目" |
| Docker 未运行 | 提示启动 Docker | "Docker 服务未运行，请先启动 Docker Desktop" |
| 命令执行失败 | 显示错误日志 | 在终端窗口显示详细错误信息 |
| 网络错误 | 提示检查网络 | "网络连接失败，请检查网络设置" |
| 权限错误 | 提示使用管理员权限 | "权限不足，请使用管理员权限运行" |

**设计决策**：
- **非阻塞错误处理**：错误不应阻止用户继续使用其他功能
- **错误恢复**：提供明确的恢复路径（如重试、重新初始化）
- **错误日志**：所有错误都记录到终端窗口，方便排查

## User Experience Design

### 首次使用流程

1. **欢迎界面**：首次启动显示欢迎模态框
2. **选择工作目录**：引导用户选择或创建工作目录
3. **自动初始化检查**：检测是否需要运行 `init`
4. **快速开始**：提示用户点击"一键部署"

### 操作反馈设计

1. **按钮状态**：
   - 默认：蓝色边框，白色背景
   - 悬停：背景变深
   - 执行中：显示旋转加载图标
   - 禁用：灰色，不可点击

2. **日志颜色编码**：
   - `info`：灰色文本
   - `success`：绿色文本
   - `error`：红色文本
   - `warning`：黄色文本
   - `command`：蓝色文本，加粗

3. **进度指示**：
   - 全局执行状态：右下角浮动提示
   - 日志实时更新：自动滚动到最新日志
   - 日志统计：显示总日志数和缓冲区使用情况

### 性能优化

1. **虚拟滚动**：日志窗口使用虚拟滚动，支持大量日志
2. **循环缓冲区**：自动清理旧日志，避免内存溢出
3. **防抖和节流**：避免频繁的状态更新
4. **懒加载**：模态框按需加载

## Testing Strategy

### 测试范围

1. **Rust 后端**：
   - 单元测试：测试 Tauri 命令的核心逻辑
   - 集成测试：测试 CliApp 的完整工作流程

2. **React 前端**：
   - 组件测试：测试组件渲染和用户交互
   - 集成测试：测试 Tauri 命令调用和事件监听

3. **端到端测试**：
   - 测试完整的用户操作流程
   - 测试错误处理和恢复

### 测试策略

- **核心功能优先**：重点测试"一键部署"、日志显示、备份回滚
- **边界条件**：测试空目录、无效配置、网络错误等
- **性能测试**：测试大量日志的渲染性能
