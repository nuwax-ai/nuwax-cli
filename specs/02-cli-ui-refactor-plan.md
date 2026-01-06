# 02 - cli-ui 重构开发计划

## 前提理解（当前工程要点）
- 桌面端：React + Tauri（Vite），核心在 `cli-ui/src/App.tsx` 与组件 `WorkingDirectoryBar`、`OperationPanel`、`TerminalWindow` 等；命令配置在 `src/config/commandConfigs.ts`，命令与文件/进程操作封装在 `src/utils/tauri.ts`。
- 后端：Tauri Rust 命令集中在 `cli-ui/src-tauri/src/commands/cli.rs`、`config.rs`，通过 `execute_duck_cli_sidecar/system/smart` 执行 `nuwax-cli`，并发事件 `cli-output/error/complete`。配置存储目前分裂：前端 `ConfigManager` 写相对路径，后端在 AppData 写 `working_directory.json`。
- 已知问题：初始化/监听使用 `window.__duck_*` 旗标且无清理；工作目录加载在 App 与 WorkingDirectoryBar 双写；命令抽象重复；日志为组件 state，未虚拟化；进程/锁检查在前端用 `setTimeout` 触发。

## 目标对照（来自 01-spec）
1. 单一状态与初始化管线（无全局旗标）。
2. 统一命令网关，支持降级/超时/重试，标准化参数拼装。
3. 配置与工作目录单源（后端 AppData）。
4. 事件/日志管线可清理，日志虚拟化。
5. 进程/锁检查策略化，可视化提示与重试。
6. 完成测试与埋点。

## 分阶段计划

### M1：状态与初始化收敛（约 2-3 天）
- 引入全局 store（Zustand/RTK 选型，轻量优先），管理：工作目录、执行状态、日志 ring buffer、CLI 可用性、事件监听注册态。
- 实现 `useAppInit`：顺序 = 读取配置（后端）→ 验证目录 → 进程/锁检查 → 注册 CLI 事件；移除 `window.__duck_*`。
- 重构 `WorkingDirectoryBar`：仅负责显示/选择/触发变更，初始化逻辑交由 `useAppInit`。
- 移除前端直接文件写入：`ConfigManager` 仅调用后端命令。

### M2：命令网关与元数据（约 2-3 天）
- 新建 `services/cliGateway.ts`：单一入口 `execute(commandId, params, opts)`，内部调用 `execute_duck_cli_smart`，带超时/降级提示/可选重试；统一返回结构。
- 统一参数拼装函数，所有按钮调用网关；清理 `OperationPanel` 内手写 args 逻辑。
- `commandConfigs` 作为单一真相源；若 CLI 支持 `--json-schema/--help-json`，在后端缓存并下发前端（可留 TODO/feature flag）。

### M3：事件/日志与进程检查（约 2 天）
- 实现 `useCliEvents`：注册/清理 `cli-output/error/complete`，事件 payload 增补 `commandId/seq/ts`；支持组件卸载时清理。
- 日志：store 中环形缓冲；`TerminalWindow` 改为虚拟列表（react-window）；导出/统计抽象为 service。
- 进程/锁检查：后端返回结构化 `CheckStatusResult`；前端提供“重新检测”按钮，失败指数退避/提示。

### M4：测试与埋点（约 2 天）
- 前端：参数拼装、store 状态迁移、事件订阅、日志虚拟列表的单元测试。
- 后端：sidecar/system 回退链路、事件 payload 结构、配置读写的集成测试。
- 埋点：命令开始/结束/耗时/退出码、目录切换结果；记录在可观测接口（可先落到 console/文件，后续接入遥测）。

## 任务拆分与优先级
- P0：移除全局旗标、引入 store、`useAppInit`、`useCliEvents`、配置单源化、命令网关替换所有按钮。
- P1：日志虚拟化、进程/锁检查策略化、结构化事件 payload。
- P2：CLI 元数据自动同步（若 CLI 提供 JSON 输出）、埋点完善、测试覆盖。

## 风险与缓解
- CLI schema 不可用：保持手工 `commandConfigs`，预留接口在可用时切换。
- sidecar 兼容性：保留 system 降级与显式提示；在网关层做超时/重试。
- 性能：虚拟列表 + 环形缓冲，限制事件 chunk；必要时对日志渲染节流。

## 验收检查清单
- 热更新/新窗口无重复监听或幽灵日志；工作目录加载仅单源，App/组件无双写。
- 新增 CLI 参数仅改元数据（或 schema）即可驱动 UI；按钮参数与实际 CLI 保持一致。
- 1 万条日志渲染无明显卡顿；命令失败有清晰提示与诊断信息。

