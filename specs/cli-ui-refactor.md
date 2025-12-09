# cli-ui 重构设计草案

## 背景与目标
- cli-ui 作为 Tauri + React 的桌面客户端，核心职责是包装 `nuwax-cli` 的命令执行、工作目录管理与日志展示。
- 现有实现中，状态初始化、命令层与 UI 耦合度较高，存在重复逻辑与配置漂移风险。目标是提升可维护性、可观测性，并降低与 CLI 同步成本。

## 主要问题（现状观察）
- **初始化与状态分散**：`App.tsx` 与 `WorkingDirectoryBar` 都在做工作目录加载/验证，并依赖 `window.__duck_*` 旗标避免重复监听，导致隐含的全局状态与潜在竞态。  
- **命令层重复且无统一抽象**：前端 `utils/tauri` 里既有 `ShellManager`（未被 UI 使用），又有大量 `DuckCliManager` 方法，每个按钮单独拼接参数，缺少单一入口和重试/超时策略。  
- **配置读写不一致**：前端 `ConfigManager` 通过插件 FS 直接读写相对路径 `duck-client/config.json`，而后端命令又在 `AppData` 写 `working_directory.json`，存在双写与源定义分裂。  
- **事件监听生命周期不清晰**：`App.tsx` 全局注册 `cli-output/error/complete` 事件且不清理，依赖全局 flag 避免重复，热更新或多实例下可能重复收集、难以测试。  
- **命令元数据漂移风险**：按钮定义、参数校验与 `commandConfigs` 手工维护，容易与 `nuwax-cli` 实际参数不一致；部分动作（如 init）在 UI 被注释掉但命令层仍存在。  
- **日志与性能**：日志保存在组件状态，虽有循环缓冲，但渲染未虚拟化，大量输出时 UI 仍可能卡顿；导出/统计逻辑与状态混杂在 App。  
- **进程/锁检查耦合 UI**：目录切换时在前端使用 `setTimeout` 调用 `ProcessManager.initializeProcessCheck`，失败回调仅写日志，状态与重试策略缺失。

## 重构设计要点
1) **单一应用状态与初始化管线**  
   - 引入状态容器（优先轻量如 Zustand/Redux Toolkit），集中管理：工作目录、执行状态、日志 ring buffer、CLI 可用性。  
   - 提供 `useAppInit()` hook：串行步骤 = 读取持久化配置 → 验证目录 → 进程/锁检查 → 注册事件监听（可 idempotent）。移除 `window.__duck_*` 旗标。

2) **统一命令执行网关**  
   - 在前端新增 `services/cliGateway.ts`，只暴露 `execute(commandId, params, options)`；内部统一走单个 Tauri invoke（如 `execute_duck_cli_smart`），并附带超时、重试/降级（sidecar -> system）与工作目录前置校验。  
   - Rust 侧保留 `execute_duck_cli_smart`，但补充：可选超时、结构化事件 payload（类型 + 行号），并在结果中返回命令回显与耗时。

3) **命令元数据中心化**  
   - 将 `commandConfigs` 提升为“单一真相源”，并让按钮/参数校验/帮助提示全部消费该配置；生成命令行参数由统一函数完成。  
   - 理想方案：在 `nuwax-cli` 增加 `--json-schema`/`--help-json` 输出，由 Tauri 后端缓存并下发给前端，减少手工同步。

4) **工作目录与配置单源化**  
   - 仅由后端在 `AppData` 路径维护配置（含工作目录、上次使用时间等），前端通过 `get/set` 命令读取/更新；前端不再直接写文件系统。  
   - 目录验证与锁检测下沉到 Rust，前端只展示状态；提供明确的错误码与可重试策略。

5) **事件与日志管线**  
   - 封装 `useCliEvents()`：注册/清理 `cli-output/error/complete`，支持窗口卸载清理与重新订阅；事件 payload 加入 `commandId`、`seq`，方便多命令并行或队列。  
   - 日志存储移到全局 store 的环形缓冲，并在 UI 端使用虚拟列表（如 react-window）。导出/统计从组件中抽成 service。

6) **进程/锁检查策略化**  
   - 在后端提供 `CheckStatusResult { processes, db_locked, advice }`，前端按状态渲染按钮禁用/提示；失败可设定指数退避重试。  
   - 将当前前端的 `setTimeout` 检查改为：初始化时一次 + 显式“重新检测”入口，避免隐式后台调用。

7) **可测试性与可观测性**  
   - 为命令执行与目录切换增加事件埋点/metrics（开始、结束、耗时、退出码）。  
   - 为前端 service 与 hooks 编写单元测试（参数拼装、状态迁移），后端命令使用 integration tests 验证 Sidecar/System 回退链路。

## 交付物与里程碑（建议）
- M1：状态容器 + `useAppInit` + `useCliEvents` 接入，移除全局 flag；配置读写只留后端。  
- M2：`cliGateway` + 命令元数据中心化，按钮使用统一参数拼装；引入 JSON schema 同步（若 CLI 支持）。  
- M3：日志虚拟化与导出抽象、进程/锁检查策略化、埋点。  
- M4：补齐测试矩阵与文档（开发/调试手册，命令元数据更新流程）。

## 验收要点
- 切换工作目录、执行命令、导出日志路径在多次热更新或新窗口场景无重复监听、副作用。  
- 新增/变更 CLI 子命令时，仅更新命令元数据或 CLI schema，即可驱动 UI；按钮行为与实际 CLI 参数保持一致。  
- 大量日志（1万条）渲染无明显卡顿；命令执行失败路径有明确提示与可诊断信息。

