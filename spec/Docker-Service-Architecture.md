# Docker 服务升级架构设计

## 概述

本文档描述了 duck-cli 工具管理的 Docker 服务升级架构。该架构通过下载 `docker.zip` 压缩包的方式，实现离线部署和升级 Docker Compose 服务集群。

## 核心目录结构

### docker.zip 压缩包结构

```
docker.zip
└── docker/                    # 外层统一的 docker 目录
    ├── docker-compose.yml     # Docker Compose 主配置文件
    ├── .env                   # 环境变量配置文件
    ├── deploy.sh             # Linux 部署脚本（将被程序逻辑替代）
    ├── images/               # 离线 Docker 镜像目录
    │   ├── *-amd64.tar      # x86_64 架构镜像
    │   └── *-arm64.tar      # ARM64 架构镜像
    ├── config/               # 服务配置文件目录
    │   ├── nginx.conf
    │   ├── mysql.cnf
    │   ├── redis.conf
    │   ├── application-external.yml
    │   ├── milvus/
    │   │   ├── embedEtcd.yaml
    │   │   └── user.yaml
    │   └── ...
    ├── data/                 # 持久化数据目录
    │   ├── mysql/           # MySQL 数据持久化
    │   ├── redis/           # Redis 数据持久化
    │   ├── milvus/          # Milvus 向量数据库数据
    │   └── ...
    ├── logs/                 # 服务日志目录
    │   ├── agent/           # 后端服务日志
    │   ├── mysql/           # MySQL 日志
    │   ├── redis/           # Redis 日志
    │   └── ...
    ├── upload/               # 文件上传目录
    ├── backups/              # 备份目录
    ├── binaries/             # 二进制文件目录
    ├── offline/              # 离线文件目录
    └── script/               # 辅助脚本目录
        └── init-minio.sh
```

### 目录用途详解

#### 1. `images/` - 离线镜像目录
- **用途**: 存储预先导出的 Docker 镜像，解决网络环境差的用户无法拉取镜像的问题
- **架构支持**: 
  - `*-amd64.tar`: x86_64 架构镜像（主要支持）
  - `*-arm64.tar`: ARM64 架构镜像（部分支持）
- **镜像类型**:
  - 业务镜像: `agent-platform-front`, `agent-platform-backend`, `mcp-proxy`, `log-platform`, `video-analysis` 等
  - 基础组件镜像: `mysql:8.0`, `redis:7.0`, `milvusdb/milvus:v2.5.8`, `quickwit/quickwit:latest` 等

#### 2. `config/` - 配置文件目录
- **用途**: 存储各服务的配置文件，通过 Docker volume 挂载到容器内
- **主要配置**:
  - `nginx.conf`: 前端反向代理配置
  - `mysql.cnf`: MySQL 数据库配置
  - `redis.conf`: Redis 缓存配置
  - `application-external.yml`: 后端服务外部配置
  - `milvus/`: Milvus 向量数据库配置

#### 3. `data/` - 持久化数据目录
- **用途**: 容器数据持久化存储，确保服务重启后数据不丢失
- **子目录**:
  - `mysql/`: MySQL 数据库文件
  - `redis/`: Redis 持久化数据
  - `milvus/`: Milvus 向量数据和元数据
  - `minio/`: 对象存储数据
  - `quickwit/`: 日志搜索引擎数据

#### 4. `logs/` - 日志目录
- **用途**: 收集各服务运行日志，便于问题排查和监控
- **日志分类**: 按服务名分目录存储

#### 5. `upload/` - 文件上传目录
- **用途**: 后端服务接收用户上传文件的存储位置
- **挂载**: 挂载到 backend 容器的 `/app/upload` 路径

#### 6. `.env` - 环境变量配置
- **用途**: 定义 Docker Compose 使用的环境变量
- **主要变量**:
  ```env
  # 镜像版本配置
  FRONTEND_VERSION=latest
  BACKEND_VERSION=latest
  MCP_PROXY_VERSION=latest
  
  # 端口配置
  FRONTEND_HOST_PORT=80
  APP_PORT=8080
  APP_DEBUG_PORT=5005
  
  # 数据库配置
  MYSQL_ROOT_PASSWORD=123456
  MYSQL_DATABASE=agent_platform
  MYSQL_USER=agent
  MYSQL_PASSWORD=123456
  
  # 其他服务配置
  REDIS_URL=redis://redis:6379
  MILVUS_HOST=milvus
  MILVUS_PORT=19530
  ```

## 服务架构组成

### 核心服务列表

1. **frontend** - 前端 Web 界面
   - 基于 Nginx 的静态文件服务
   - 端口: `${FRONTEND_HOST_PORT}:80`

2. **backend** - 后端 API 服务
   - Java Spring Boot 应用
   - 端口: `8080:${APP_PORT}`, `5005:${APP_DEBUG_PORT}`

3. **mcp-proxy** - MCP 代理服务
   - 端口: `8020:8089`

4. **mysql** - MySQL 数据库
   - 版本: 8.0
   - 端口: `3306:3306`

5. **redis** - Redis 缓存
   - 版本: 7.0
   - 端口: `6379:6379`

6. **milvus** - 向量数据库
   - 版本: v2.5.8
   - 端口: `19530:19530`, `9091:9091`

7. **minio** - 对象存储服务
   - 端口: `9000:9000` (API), `9001:9001` (Console)

8. **quickwit** - 日志搜索引擎
   - 端口: `7280:7280`, `7281:7281`

9. **log_platform** - 日志平台服务
   - 端口: `8097:8097`

10. **video-analysis-master** - 视频分析主服务
    - 端口: `8989:8989`

11. **video-analysis-worker** - 视频分析工作服务
    - GPU 支持，CPU 密集型任务

## 部署流程设计

### 1. 下载阶段
```
用户触发升级 → 下载 docker.zip → 验证文件完整性
```

### 2. 解压阶段
```
解压 docker.zip → 智能目录结构检测 → 解压到正确位置 → 验证必要文件
```

#### 智能解压逻辑
程序会自动检测 `docker.zip` 内部的目录结构：

1. **检测阶段**：扫描压缩包内容，查找 `docker-compose.yml` 文件位置
   - 如果 `docker-compose.yml` 位于 `docker/docker-compose.yml` → 检测到有外层 docker 目录
   - 如果 `docker-compose.yml` 位于根目录 → 检测到无外层目录

2. **解压策略**：
   - **有外层 docker 目录**：直接解压到当前工作目录 (`.`)，保持 `docker/` 目录结构
     ```
     解压前: 当前目录
     解压后: ./docker/docker-compose.yml
     ```
   
   - **无外层 docker 目录**：解压到 `docker/` 目录下
     ```
     解压前: 当前目录
     解压后: ./docker/docker-compose.yml
     ```

3. **最终结果**：无论哪种情况，最终都会得到 `./docker/docker-compose.yml` 的标准路径

### 3. 环境检查阶段
```
检查 Docker 环境 → 检查 Docker Compose → 检查系统资源
```

### 4. 镜像处理阶段
```
检测系统架构 → 加载对应架构的镜像 → 设置镜像标签
```

### 5. 服务启动阶段
```
停止旧服务 → 启动新服务 → 健康检查 → 完成部署
```

## 核心部署逻辑

### 镜像加载逻辑
```rust
// 伪代码示例
fn load_docker_images(arch: &str) -> Result<()> {
    let image_pattern = format!("images/*-{}.tar", arch);
    let image_files = glob(&image_pattern)?;
    
    for image_file in image_files {
        // docker load -i {image_file}
        load_docker_image(&image_file)?;
    }
    
    // 设置镜像标签
    set_image_tags(arch)?;
    Ok(())
}
```

### 服务启动逻辑
```rust
// 伪代码示例
fn start_docker_services() -> Result<()> {
    // 1. 停止旧服务
    run_command("docker-compose down --remove-orphans")?;
    
    // 2. 启动新服务
    run_command("docker-compose up -d")?;
    
    // 3. 等待服务健康检查
    wait_for_services_healthy()?;
    
    Ok(())
}
```

### 架构检测逻辑
```rust
fn detect_architecture() -> String {
    match std::env::consts::ARCH {
        "x86_64" => "amd64".to_string(),
        "aarch64" => "arm64".to_string(),
        _ => "amd64".to_string(), // 默认使用 amd64
    }
}
```

## 配置管理策略

### 环境变量优先级
1. 用户自定义配置 (config.toml)
2. 解压包中的 .env 文件
3. 程序默认值

### 端口冲突处理
- 检测端口占用情况
- 自动调整冲突端口
- 更新 docker-compose.yml 配置

### 跨平台兼容性
- **Linux**: 完全支持所有功能
- **macOS**: 支持，但可能需要调整某些配置
- **Windows**: 基础支持，部分高级功能可能受限

## 错误处理和回滚

### 部署失败处理
1. **镜像加载失败**: 尝试从网络拉取镜像
2. **服务启动失败**: 检查日志，提供详细错误信息
3. **端口冲突**: 自动调整端口映射
4. **资源不足**: 提示用户并建议优化方案

### 数据备份策略
- 部署前自动备份 `data/` 目录
- 保留配置文件备份
- 支持快速回滚到上一个版本

## 监控和日志

### 健康检查
- 各服务定义了 healthcheck 配置
- 程序监控服务启动状态
- 提供服务状态查询接口

### 日志收集
- 统一日志目录管理
- 支持日志轮转和清理
- 提供日志查看和导出功能

## 未来扩展点

1. **多环境支持**: 开发、测试、生产环境配置分离
2. **灰度部署**: 支持部分服务的渐进式升级
3. **配置热更新**: 无需重启即可更新部分配置
4. **集群部署**: 支持多节点部署和负载均衡
5. **监控集成**: 集成 Prometheus/Grafana 监控体系

---

*该文档描述了当前 Docker 服务升级架构的核心概念和实现逻辑，为 duck-cli 工具的开发提供技术指导。* 