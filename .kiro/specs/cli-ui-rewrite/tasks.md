# Implementation Plan - CLI-UI 重写任务列表

## Phase 1: 项目基础设施

- [x] 1. 验证现有 Tauri + React 项目配置
  - 检查 TypeScript、Tailwind CSS 配置
  - 检查 workspace 结构
  - 验证 tsconfig.json 和 vite.config.ts
  - _Requirements: 1.1, 1.2_

- [x] 1.1 安装新增依赖
  - 安装 React Router v6
  - 验证 Heroicons 已安装
  - 配置 TypeScript 类型定义
  - _Requirements: 1.1_

- [x] 1.2 调整项目目录结构
  - 创建 src/pages 目录（新增）
  - 创建 src/context 目录（新增）
  - 保留现有 src/components、src/hooks、src/types、src/utils 目录
  - _Requirements: 1.1_

## Phase 2: Tauri 后端集成

- [x] 2. 重构现有 Tauri 命令接口
  - 保留现有的 `execute_duck_cli_command` 统一入口
  - 保留现有的配置管理命令
  - 验证事件系统（cli-output、cli-error、cli-complete）
  - _Requirements: 1.1, 1.2, 2.1_

- [x] 2.1 实现容器状态查询命令
  - 实现 `get_container_status()` 命令
  - 调用 CliApp.docker_manager.list_containers()
  - 返回容器名称、状态、镜像、端口信息
  - _Requirements: 3.1, 3.2, 4.1_

- [x] 2.2 实现容器日志流式传输命令
  - 实现 `stream_container_logs()` 命令
  - 使用 Tauri Event 发送容器日志（container-log 事件）
  - 实现 `stop_container_logs()` 命令停止日志流
  - _Requirements: 4.3_

- [x] 2.3 实现单个容器操作命令
  - 实现 `start_container()` 命令
  - 实现 `stop_container()` 命令
  - 实现 `restart_container()` 命令
  - 调用 Docker API 执行容器操作
  - _Requirements: 4.4_

- [x] 2.4 实现备份管理命令
  - 实现 `list_backups()` 命令（已有，验证即可）
  - 验证备份创建和恢复通过 `execute_duck_cli_command` 执行
  - _Requirements: 5.1, 5.2, 5.3_

## Phase 3: React 前端开发

- [x] 3. 扩展 TypeScript 类型定义
  - 添加 ContainerInfo 类型（名称、状态、镜像、端口）
  - 添加 PageRoute 类型（路由定义）
  - 保留现有的 LogEntry、BackupRecord 等类型
  - _Requirements: 所有需求_

- [x] 3.1 创建全局状态管理（Context）
  - 创建 AppContext.tsx（工作目录、日志、容器状态）
  - 创建 useApp Hook（访问全局状态）
  - 实现日志循环缓冲区逻辑
  - _Requirements: 1.1, 3.1_

- [x] 3.2 创建自定义 Hooks
  - 创建 useContainers.ts（容器状态管理和刷新）
  - 创建 useContainerLogs.ts（容器日志流式监听）
  - 保留现有的 useTauriCommand、useTauriEvent
  - _Requirements: 3.3, 4.1, 4.3_

- [x] 3.3 创建布局组件
  - 创建 Header.tsx（顶部栏，显示工作目录）
  - 创建 Sidebar.tsx（侧边栏导航菜单）
  - 创建 MainLayout.tsx（整体布局容器）
  - _Requirements: 所有需求_

- [x] 3.4 创建可复用 UI 组件
  - 创建 LogTerminal.tsx（日志终端，可复用）
  - 创建 ContainerCard.tsx（容器信息卡片）
  - 创建 StatusBadge.tsx（状态徽章）
  - 保留现有的 ErrorBoundary、ParameterInputModal
  - _Requirements: 2.3, 3.2, 4.1_

## Phase 4: 页面开发

- [x] 4. 创建概览页面（OverviewPage.tsx）
  - 显示服务整体状态（运行中/已停止）
  - 显示快速操作按钮（一键部署、启动、停止、重启）
  - 显示容器列表概览（使用 ContainerCard）
  - 实现容器状态自动刷新（每 5 秒）
  - _Requirements: 3.1, 3.2, 3.3_

- [x] 4.1 创建部署页面（DeployPage.tsx）
  - 显示当前版本和最新版本信息
  - 显示"一键部署"按钮和"高级选项"按钮
  - 集成 LogTerminal 组件显示实时部署日志
  - 监听 cli-output、cli-error、cli-complete 事件
  - _Requirements: 2.1, 2.2, 2.3_

- [x] 4.2 创建容器页面（ContainersPage.tsx）
  - 显示所有容器的详细信息卡片
  - 每个容器卡片显示：状态、镜像、端口、操作按钮
  - 实现容器操作：查看日志、启动、停止、重启
  - 点击"查看日志"后，在页面下方显示该容器的实时日志
  - 使用 LogTerminal 组件显示容器日志
  - _Requirements: 4.1, 4.2, 4.3, 4.4_

- [x] 4.3 创建备份页面（BackupPage.tsx）
  - 显示"创建备份"按钮
  - 显示备份列表（时间、版本、大小、类型）
  - 每个备份显示"恢复"和"删除"按钮
  - 集成现有的 BackupSelectionModal（用于恢复确认）
  - _Requirements: 5.1, 5.2, 5.3, 5.4_

- [x] 4.4 创建日志页面（LogsPage.tsx）
  - 显示全局日志（所有命令的输出）
  - 显示日志操作按钮（清除、导出、搜索）
  - 集成 LogTerminal 组件
  - 实现日志搜索和过滤功能
  - _Requirements: 2.3, 4.3, 4.5_

## Phase 5: 路由和集成

- [-] 5. 配置 React Router
  - 安装 react-router-dom v6
  - 定义路由路径（/overview, /deploy, /containers, /backup, /logs）
  - 在 App.tsx 中配置 BrowserRouter 和 Routes
  - 实现路由懒加载（使用 React.lazy）
  - _Requirements: 所有需求_

- [x] 5.1 集成侧边栏导航
  - 在 Sidebar 组件中使用 NavLink
  - 实现路由高亮显示（active 状态）
  - 实现侧边栏折叠功能（响应式设计）
  - _Requirements: 所有需求_

- [x] 5.2 重构 App.tsx
  - 移除现有的单页面布局
  - 使用 MainLayout 包裹路由
  - 使用 AppProvider 包裹整个应用
  - 保留 WelcomeSetupModal 和 ErrorBoundary
  - _Requirements: 1.1, 1.2_

- [x] 5.3 实现错误恢复机制
  - 验证现有的 ErrorBoundary
  - 实现全局错误处理（在 AppContext 中）
  - 实现错误提示（使用 Tauri dialog API）
  - _Requirements: 6.1, 6.2, 6.3_

- [ ] 5.4 实现性能优化
  - 使用 React.memo 优化组件渲染
  - 实现日志虚拟滚动（在 LogTerminal 中）
  - 实现防抖和节流（容器状态刷新）
  - _Requirements: 所有需求_

## Phase 6: 测试

- [ ] 6. 编写 Rust 后端单元测试
  - 测试容器状态查询命令
  - 测试容器操作命令（启动/停止/重启）
  - 测试容器日志流式传输
  - 测试错误处理和边界条件
  - _Requirements: 所有需求_

- [ ] 6.1 编写 React 前端组件测试
  - 测试页面组件渲染（OverviewPage、DeployPage 等）
  - 测试用户交互（按钮点击、路由切换）
  - 测试 Context 和 Hooks（useApp、useContainers）
  - 测试错误边界（ErrorBoundary）
  - _Requirements: 所有需求_

- [ ] 6.2 编写集成测试
  - 测试完整的部署流程（从点击按钮到日志显示）
  - 测试容器操作流程（查看日志、重启容器）
  - 测试备份恢复流程
  - 测试错误恢复机制
  - _Requirements: 所有需求_

---

## 任务执行说明

1. **任务顺序**：按 Phase 顺序执行
2. **任务标记**：`[ ]` 未开始，`[x]` 已完成，`*` 可选任务
3. **需求追溯**：每个任务标注对应需求编号
