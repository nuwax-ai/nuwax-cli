# Docker 服务管理模块开发任务

## 概述

基于 `docker.zip` 压缩包的复杂部署逻辑，开发一个完整的 Docker 服务管理模块，支持离线镜像加载、架构检测、服务生命周期管理等功能。

## 核心功能需求

### 1. 架构检测与镜像管理
- [x] 检测宿主机架构（amd64/arm64）
- [x] 从 `images/` 目录加载对应架构的离线镜像
- [x] 设置镜像标签（`latest-${ARCH}` → `latest`）
- [x] 支持业务镜像和基础组件镜像的标签转换

### 2. 环境检查与预处理
- [x] Docker 环境检查（版本、运行状态、权限）
- [x] Docker Compose 检查（V1/V2 兼容性）
- [x] 端口冲突检测与自动调整
- [x] 系统资源检查（内存、磁盘空间）
- [x] 必要目录创建（data/, logs/, upload/ 等）

### 3. 服务生命周期管理
- [x] 启动所有服务
- [x] 停止所有服务
- [x] 重启所有服务
- [x] 重启单个容器
- [x] 服务健康状态检查
- [x] 服务状态监控与报告

### 4. 配置管理
- [x] 环境变量处理（.env 文件）
- [x] Docker Compose 配置动态调整
- [x] 兼容模式配置文件选择
- [x] 端口映射配置

## 开发任务列表

### Phase 1: 基础模块结构搭建 ✅
- [x] **Task 1.1**: 创建 `duck-cli/src/docker_service/` 目录
- [x] **Task 1.2**: 创建 `mod.rs` 并定义公共接口
- [x] **Task 1.3**: 创建核心结构体 `DockerServiceManager`
- [x] **Task 1.4**: 定义错误类型和结果类型

### Phase 2: 架构检测与镜像管理 ✅
- [x] **Task 2.1**: 实现系统架构检测 (`architecture.rs`)
- [x] **Task 2.2**: 实现镜像加载逻辑 (`image_loader.rs`)
- [x] **Task 2.3**: 实现镜像标签管理
- [x] **Task 2.4**: 支持业务镜像和基础镜像的分类处理

### Phase 3: 环境检查模块 ✅
- [x] **Task 3.1**: Docker 环境检查 (`environment.rs`)
- [x] **Task 3.2**: Docker Compose 版本检测与兼容性处理
- [x] **Task 3.3**: 端口冲突检测与解决
- [x] **Task 3.4**: 系统资源检查（内存、磁盘）

### Phase 4: 服务管理核心 ✅
- [x] **Task 4.1**: 服务启动逻辑 (`service_manager.rs`)
- [x] **Task 4.2**: 服务停止逻辑
- [x] **Task 4.3**: 服务重启逻辑
- [x] **Task 4.4**: 单个容器重启功能
- [x] **Task 4.5**: 健康检查与状态监控

### Phase 5: 配置管理 ✅
- [x] **Task 5.1**: 环境变量处理 (`config.rs`)
- [x] **Task 5.2**: Docker Compose 配置文件动态选择
- [x] **Task 5.3**: 端口映射配置管理
- [x] **Task 5.4**: 兼容模式配置处理

### Phase 6: 核心功能实现 ✅
- [x] **Task 6.1**: 完整的镜像加载器实现
- [x] **Task 6.2**: 健康检查与服务监控
- [x] **Task 6.3**: 主管理器 (`DockerServiceManager`) 完整实现
- [x] **Task 6.4**: 编译通过验证

### Phase 6: 集成与测试 🔄
- [x] **Task 6.1**: 将新模块集成到现有 CLI 命令中
- [x] **Task 6.2**: 更新 `start`、`stop`、`restart` 命令
- [x] **Task 6.3**: 添加新的容器管理命令
- [ ] **Task 6.4**: 编写单元测试
- [ ] **Task 6.5**: 编写集成测试

### Phase 7: 高级功能 🚀
- [ ] **Task 7.1**: 服务日志收集与查看
- [ ] **Task 7.2**: 服务性能监控
- [ ] **Task 7.3**: 自动故障恢复
- [ ] **Task 7.4**: 滚动更新支持

## 文件结构设计

```
duck-cli/src/docker_service/
├── mod.rs                 # 模块入口，定义公共 API
├── manager.rs             # DockerServiceManager 主结构体
├── architecture.rs        # 系统架构检测
├── image_loader.rs        # Docker 镜像加载与标签管理
├── environment.rs         # 环境检查（Docker、Compose、资源）
├── service_manager.rs     # 服务生命周期管理
├── config.rs             # 配置管理（.env、compose 文件）
├── health_check.rs       # 健康检查与状态监控
├── port_manager.rs       # 端口冲突检测与管理
└── error.rs             # Docker 服务专用错误类型
```

## 核心接口设计

```rust
pub struct DockerServiceManager {
    config: AppConfig,
    docker_manager: DockerManager,
    work_dir: PathBuf,
}

impl DockerServiceManager {
    // 生命周期管理
    pub async fn start_services(&self) -> Result<()>;
    pub async fn stop_services(&self) -> Result<()>;
    pub async fn restart_services(&self) -> Result<()>;
    pub async fn restart_container(&self, name: &str) -> Result<()>;
    
    // 镜像管理
    pub async fn load_images(&self) -> Result<()>;
    pub async fn setup_image_tags(&self) -> Result<()>;
    
    // 环境管理
    pub async fn check_environment(&self) -> Result<()>;
    pub async fn setup_directories(&self) -> Result<()>;
    
    // 状态监控
    pub async fn get_service_status(&self) -> Result<ServiceStatus>;
    pub async fn health_check(&self) -> Result<HealthReport>;
}
```

## 关键技术点

### 1. 架构检测
```rust
pub enum Architecture {
    Amd64,
    Arm64,
}

pub fn detect_architecture() -> Architecture;
```

### 2. 镜像加载策略
- 扫描 `images/` 目录下的 `*-amd64.tar` 和 `*-arm64.tar` 文件
- 根据系统架构选择对应镜像
- 加载后设置统一标签，去除架构后缀

### 3. 服务健康监控
- 等待服务启动完成（最长 180 秒）
- 定期检查容器状态
- 提供实时进度反馈

### 4. 错误处理与恢复
- 详细的错误日志记录
- 自动端口冲突解决
- 优雅的服务停止机制

## 开发优先级

1. **P0 (关键)**: Phase 1-4 基础功能实现
2. **P1 (重要)**: Phase 5-6 配置管理和集成
3. **P2 (增强)**: Phase 7 高级功能

## 测试策略

### 单元测试
- 架构检测逻辑
- 镜像标签转换
- 配置文件解析

### 集成测试
- 完整的服务启动流程
- 错误场景处理
- 跨平台兼容性

### 性能测试
- 大型镜像加载时间
- 服务启动时间
- 内存使用情况

---

*此任务列表将根据开发进度动态更新，确保开发过程的透明性和可追踪性。* 