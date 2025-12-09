# 01 - cli-ui 重构需求与技术方案

## 背景
- cli-ui（Tauri + React）包装 `nuwax-cli` 命令、工作目录与日志，但当前存在初始化重复、配置多源、命令抽象分裂、事件生命周期不清晰等问题（详见 `spec/cli-ui-refactor.md`）。
- 目标：提高可维护性与一致性，降低 CLI 参数漂移风险，并改善可观测性与性能。

## 范围与目标
- 统一应用初始化与状态管理，去除 `window.__duck_*` 旗标与隐式全局状态。
- 统一命令执行入口，支持 sidecar→system 降级、超时与重试，并标准化参数拼装。
- 配置与工作目录只保留后端单源（AppData），前端仅通过命令读写。
- 事件/日志管线可复用、可清理，支持多命令并行标识与虚拟化渲染。
- 进程/锁检查策略化，提供可重试和可视化的状态提示。
- 提升可测试性（前端 services/hooks + 后端命令）与埋点能力。

## 非目标
- 不改动 `nuwax-cli` 业务逻辑，仅增加元数据输出接口（若需要）。
- 不处理 UI 视觉重设计，聚焦架构/可维护性。

## 功能需求
1) **初始化管线（单入口）**
   - Hook `useAppInit()` 串行：读取配置 → 验证目录 → 进程/锁检查 → 注册 CLI 事件。
   - 去除 `WorkingDirectoryBar` 自行初始化逻辑；全局状态由 store 提供。
2) **统一命令网关**
   - `services/cliGateway.execute(commandId, params, options)`；内部调用单一 Tauri invoke（如 `execute_duck_cli_smart`）。
   - 支持超时、可选重试、sidecar→system 降级提示；返回结构含 `commandId/args/exit_code/stdout/stderr/duration`.
3) **命令元数据中心化**
   - `commandConfigs` 作为单一真相源；按钮、参数校验、帮助提示共用同一配置。
   - 若 `nuwax-cli` 支持，接入 `--json-schema/--help-json` 同步，后端缓存，下发前端。
4) **配置单源**
   - 仅后端在 AppData 存储：`working_directory.json` + 未来扩展字段（上次使用时间、首选项）。
   - 前端移除直接文件写入；暴露 `get/set_working_directory` 等命令。
5) **事件与日志**
   - `useCliEvents()` 封装注册/清理 `cli-output/error/complete`，事件载荷包含 `commandId`、序号、时间。
   - 日志存储在全局环形缓冲（store），终端使用虚拟列表（如 react-window）；导出、统计抽象为 service。
6) **进程/锁检查策略**
   - 后端提供结构化结果：`{processes, processes_killed, db_locked, advice}`；前端渲染提示与禁用态。
   - 提供“重新检测”入口，失败可指数退避。
7) **可测试性与埋点**
   - 前端：参数拼装、状态迁移、事件订阅的单元测试；日志缓冲与虚拟列表渲染测试。
   - 后端：sidecar/system 回退链路的集成测试；事件 payload 结构测试。
   - 埋点：命令开始/结束/耗时、退出码、目录切换结果。

## 技术方案
### 前端（React）
- 新建 `src/store`（Zustand/RTK，轻量优先）：工作目录、执行状态、日志 ring buffer、CLI 可用性、监听注册状态。
- 新建 `src/services/cliGateway.ts`：拼装参数、调用 `invoke('execute_duck_cli_smart')`，支持超时/重试/降级提示。
- 新建 hooks：
  - `useAppInit`：驱动初始化流程并写入 store。
  - `useCliEvents`：注册/清理事件，带 `commandId`、序号。
- UI 调整：
  - `WorkingDirectoryBar` 仅展示与触发选择/验证，不再自带初始化。
  - `OperationPanel` 按钮全部通过 `cliGateway` + `commandConfigs` 拼装参数；移除散落的拼接逻辑。
  - `TerminalWindow` 使用虚拟列表，日志数据由 store 提供。
  - 提供“重新检测进程/锁”按钮与状态展示。
- 配置读写：全部通过 `ConfigManager` → Tauri 后端命令，去除前端直接写文件。

### 后端（Tauri Rust）
- `execute_duck_cli_smart` 增强：可选超时、结构化事件 payload（`{commandId, stream, line, ts}`），返回耗时。
- 事件派发：在 sidecar/system 路径均发送 `cli-output/error/complete`，附带 `commandId` 与序号。
- 配置存储：只在 AppData 维护；暴露 get/set；为后续首选项预留字段。
- 进程/锁检查：返回结构化 `CheckStatusResult`；增加 advice 字段。
- 可选：提供 CLI 元数据拉取命令（调用 `nuwax-cli --json-schema` 并缓存）。

### 数据/接口契约（示例）
- `execute_duck_cli_smart(args, workingDir?) -> {success, exit_code, stdout, stderr, duration_ms, command_id?}`
- 事件 payload：`{commandId, stream: 'stdout'|'stderr', chunk, seq, ts}`；完成事件 `{commandId, exit_code, duration_ms}`。
- `CheckStatusResult`：`{processes_found, processes_killed, db_locked, advice, message}`。

## 里程碑
- M1：store + `useAppInit` + `useCliEvents`，移除全局旗标；配置只读写后端。
- M2：`cliGateway` 接入，按钮全面改用统一拼装；（可选）CLI schema 同步。
- M3：日志虚拟化、导出/统计抽象；进程/锁检查策略与 UI。
- M4：测试矩阵与埋点落地，补开发/调试文档。

## 风险与缓解
- CLI 元数据缺口：先手工配置，待 `--json-schema` 支持后替换。
- 性能风险（大量日志）：虚拟列表 + 环形缓冲；限制事件 chunk 大小。
- 兼容性：sidecar 在部分环境失败，保持 system 降级与显式提示。

## 验收标准
- 多次热更新/新窗口无重复监听或幽灵日志；目录切换/命令执行流程可重复验证。
- 新增/变更 CLI 参数时，仅改元数据（或 CLI schema）即可驱动 UI，无双处修改。
- 1 万条日志渲染无明显卡顿；失败路径有清晰提示和诊断信息。

