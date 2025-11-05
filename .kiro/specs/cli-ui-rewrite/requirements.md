# Requirements Document - CLI-UI 重写

## Introduction

本文档定义了将 nuwax-cli 命令行工具重写为基于 Tauri 的图形化界面应用的需求。

### 核心目标

1. **快速部署启动 Docker 服务**：一键执行 `auto-upgrade-deploy run`
2. **实时查看部署日志**：方便排查问题
3. **监控服务健康状态**：查看容器状态、重启容器
4. **查看容器日志**：选中容器查看实时日志
5. **备份和回滚数据**：执行 `auto-backup` 和 `rollback`

### 设计原则

- 简洁直观的界面，降低学习成本
- 实时反馈，让用户了解操作进度
- 快捷操作，减少点击次数
- 错误友好，提供清晰的错误提示和解决方案

## Glossary

- **nuwax-cli**: 命令行工具模块，提供 Docker 服务管理和智能升级功能
- **cli-ui**: 基于 Tauri 的图形化界面模块
- **CliApp**: nuwax-cli 的核心应用结构
- **UpgradeStrategy**: 升级策略（FullUpgrade、PatchUpgrade、NoUpgrade）
- **DockerManager**: Docker 容器管理器
- **BackupManager**: 备份管理器
- **HealthChecker**: 服务健康检查器

## Requirements

### Requirement 1: 架构集成

**User Story:** 作为开发者，我希望 cli-ui 能够以库的方式集成 nuwax-cli，以便复用现有的业务逻辑。

#### Acceptance Criteria

1. THE cli-ui Tauri 后端 SHALL 将 nuwax-cli 作为 Cargo 依赖引入
2. THE CliApp 实例 SHALL 在 Tauri 应用启动时初始化
3. THE CliApp 实例 SHALL 使用 Arc<Mutex<CliApp>> 包装并存储在 Tauri State 中
4. THE Tauri 命令 SHALL 通过 State 参数访问共享的 CliApp 实例
5. THE cli-ui 应用 SHALL 在 cli-ui 目录下通过 `npm run tauri dev` 启动

### Requirement 2: 自动升级部署流程

**User Story:** 作为用户，我希望通过图形界面执行自动升级部署流程，并实时查看进度和日志。

#### Acceptance Criteria

1. WHEN 用户点击"自动升级部署"按钮时，THE GUI SHALL 调用 Tauri 命令
2. THE GUI SHALL 显示升级流程的关键阶段和进度
3. THE GUI SHALL 实时流式显示日志输出
4. IF 升级失败，THEN THE GUI SHALL 显示错误信息并提供回滚选项
5. WHEN 升级成功时，THE GUI SHALL 显示服务访问信息

### Requirement 3: 服务状态监控

**User Story:** 作为用户，我希望实时查看 Docker 服务的运行状态。

#### Acceptance Criteria

1. THE GUI SHALL 调用 HealthChecker 获取服务状态
2. THE GUI SHALL 显示容器状态（Running、Stopped、Starting 等）
3. THE GUI SHALL 每 5 秒自动刷新服务状态
4. THE GUI SHALL 使用颜色编码区分容器状态
5. THE GUI SHALL 显示容器的端口映射信息

### Requirement 4: 容器管理

**User Story:** 作为用户，我希望查看容器日志并执行容器操作。

#### Acceptance Criteria

1. THE GUI SHALL 显示容器列表
2. THE GUI SHALL 提供"查看日志"按钮打开日志查看器
3. THE GUI SHALL 实时流式显示容器日志
4. THE GUI SHALL 提供"重启容器"按钮
5. THE GUI SHALL 支持日志搜索和过滤

### Requirement 5: 备份管理

**User Story:** 作为用户，我希望管理备份并能恢复数据。

#### Acceptance Criteria

1. THE GUI SHALL 显示备份列表（ID、时间、版本、大小）
2. THE GUI SHALL 提供"创建备份"按钮
3. THE GUI SHALL 提供"恢复备份"功能
4. THE GUI SHALL 支持"仅恢复数据"和"完整恢复"选项
5. THE GUI SHALL 显示备份/恢复进度

### Requirement 6: 错误处理

**User Story:** 作为用户，我希望在操作失败时获得清晰的错误信息。

#### Acceptance Criteria

1. THE GUI SHALL 显示用户友好的错误消息
2. THE GUI SHALL 提供错误详情和解决建议
3. THE GUI SHALL 使用 ErrorBoundary 捕获 React 错误
4. THE GUI SHALL 提供"重试"按钮
5. THE GUI SHALL 记录错误到日志文件
