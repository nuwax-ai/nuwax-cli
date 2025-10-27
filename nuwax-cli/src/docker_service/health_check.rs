use crate::docker_service::{DockerServiceError, DockerServiceResult};
use bollard::Docker;
use bollard::container::{InspectContainerOptions, ListContainersOptions};
use bollard::models::{Health, HealthStatusEnum};
use client_core::constants::timeout;
use client_core::container::DockerManager;
use serde::{Deserialize, Serialize};
use std::time::Duration;
use std::{collections::HashSet, sync::Arc};
use tracing::{debug, error, info, warn};

/// Docker容器重启策略
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum RestartPolicy {
    /// 不自动重启 (restart: no)
    No,
    /// 总是重启 (restart: always)
    Always,
    /// 除非手动停止否则重启 (restart: unless-stopped)
    UnlessStopped,
    /// 失败时重启 (restart: on-failure)
    OnFailure,
    /// 失败时重启，最大重试次数 (restart: on-failure:3)
    OnFailureWithRetries(u32),
}

impl RestartPolicy {
    /// 从字符串解析重启策略
    pub fn from_str(s: &str) -> Option<Self> {
        match s.to_lowercase().as_str() {
            "no" | "false" => Some(Self::No),
            "always" => Some(Self::Always),
            "unless-stopped" => Some(Self::UnlessStopped),
            "on-failure" => Some(Self::OnFailure),
            s if s.starts_with("on-failure:") => {
                if let Ok(retries) = s[11..].parse::<u32>() {
                    Some(Self::OnFailureWithRetries(retries))
                } else {
                    Some(Self::OnFailure)
                }
            }
            _ => None,
        }
    }

    /// 转换为字符串
    pub fn to_string(&self) -> String {
        match self {
            Self::No => "no".to_string(),
            Self::Always => "always".to_string(),
            Self::UnlessStopped => "unless-stopped".to_string(),
            Self::OnFailure => "on-failure".to_string(),
            Self::OnFailureWithRetries(retries) => format!("on-failure:{retries}"),
        }
    }

    /// 判断是否为一次性任务
    pub fn is_oneshot(&self) -> bool {
        matches!(self, Self::No)
    }

    /// 判断是否应该持续运行
    pub fn should_keep_running(&self) -> bool {
        matches!(
            self,
            Self::Always | Self::UnlessStopped | Self::OnFailure | Self::OnFailureWithRetries(_)
        )
    }

    /// 获取显示名称
    pub fn display_name(&self) -> &str {
        match self {
            Self::No => "no (一次性任务)",
            Self::Always => "always (持续服务)",
            Self::UnlessStopped => "unless-stopped (持续服务)",
            Self::OnFailure => "on-failure (持续服务)",
            Self::OnFailureWithRetries(_) => "on-failure:n (持续服务)",
        }
    }
}

/// Docker Compose 容器标签信息
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ComposeLabels {
    /// 项目名称
    pub project: Option<String>,
    /// 服务名称
    pub service: Option<String>,
    /// 容器编号
    pub container_number: Option<String>,
    /// 是否为一次性任务
    pub oneoff: Option<bool>,
    /// 配置文件路径
    pub config_files: Option<String>,
    /// 工作目录
    pub working_dir: Option<String>,
}

/// 容器状态
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum ContainerStatus {
    /// 运行中
    Running,
    /// 已停止
    Stopped,
    /// 正在启动
    Starting,
    /// 已完成 (一次性任务成功退出)
    Completed,
    /// 未知状态
    Unknown,
}

impl ContainerStatus {
    /// 从ducker的容器状态和退出码解析状态
    pub fn from_ducker_status(running: bool, status: &str, is_oneshot: bool) -> Self {
        if running {
            ContainerStatus::Running
        } else if status.to_lowercase().contains("exited") {
            if is_oneshot {
                // 一次性任务：检查退出码
                if status.contains("(0)") {
                    ContainerStatus::Completed // 成功完成
                } else {
                    ContainerStatus::Stopped // 失败退出
                }
            } else {
                ContainerStatus::Stopped // 持续服务退出都视为异常
            }
        } else if status.to_lowercase().contains("restarting")
            || status.to_lowercase().contains("created")
        {
            ContainerStatus::Starting
        } else {
            ContainerStatus::Unknown
        }
    }

    /// 获取状态的显示名称
    pub fn display_name(&self) -> &'static str {
        match self {
            ContainerStatus::Running => "运行中",
            ContainerStatus::Stopped => "已停止",
            ContainerStatus::Starting => "启动中",
            ContainerStatus::Completed => "已完成",
            ContainerStatus::Unknown => "未知",
        }
    }
    /// 判断是否运行中
    pub fn is_running(&self) -> bool {
        matches!(self, ContainerStatus::Running)
    }

    /// 判断状态是否健康（运行中或已完成都算健康）
    pub fn is_healthy(&self) -> bool {
        matches!(self, ContainerStatus::Running | ContainerStatus::Completed)
    }

    /// 判断状态是否为过渡状态（需要继续等待）
    pub fn is_transitioning(&self) -> bool {
        matches!(self, ContainerStatus::Starting)
    }

    /// 判断状态是否为失败状态
    pub fn is_failed(&self) -> bool {
        matches!(self, ContainerStatus::Stopped | ContainerStatus::Unknown)
    }
}

/// 容器信息
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ContainerInfo {
    /// 容器名称
    pub name: String,
    /// 容器状态
    pub status: ContainerStatus,
    /// 镜像名称
    pub image: String,
    /// 端口映射
    pub ports: Vec<String>,
    /// 启动时间
    pub uptime: Option<String>,
    /// 健康检查状态
    pub health: Option<HealthStatusEnum>,
    /// 是否为一次性任务
    pub is_oneshot: bool,
    /// 重启策略
    pub restart: Option<RestartPolicy>,
}

impl ContainerInfo {
    /// 判断是否为一次性任务
    /// 仅基于restart策略进行判断，不使用名称匹配
    pub fn is_oneshot(&self) -> bool {
        match &self.restart {
            Some(policy) => policy.is_oneshot(),
            None => {
                // 如果没有restart信息，默认不是一次性任务
                // 这样更安全，避免误判持续服务为一次性任务
                false
            }
        }
    }

    /// 判断是否为持续服务（需要一直运行）
    /// 仅基于restart策略进行判断，不使用名称匹配
    pub fn is_persistent_service(&self) -> bool {
        match &self.restart {
            Some(policy) => policy.should_keep_running(),
            None => {
                // 如果没有restart信息，默认认为是持续服务
                // 这样更安全，避免误判持续服务导致备份时出现问题
                true
            }
        }
    }

    /// 获取restart策略的显示字符串
    pub fn get_restart_display(&self) -> String {
        match &self.restart {
            Some(policy) => policy.to_string(),
            None => "未知".to_string(),
        }
    }
}

/// 服务整体状态
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum ServiceStatus {
    /// 所有服务都在运行
    AllRunning,
    /// 部分服务在运行
    PartiallyRunning,
    /// 所有服务都已停止
    AllStopped,
    /// 服务正在启动中
    Starting,
    /// 服务状态未知
    Unknown,
    /// 没有发现容器
    NoContainer,
}

impl ServiceStatus {
    /// 获取状态的显示名称
    pub fn display_name(&self) -> &'static str {
        match self {
            ServiceStatus::AllRunning => "全部运行",
            ServiceStatus::PartiallyRunning => "部分运行",
            ServiceStatus::AllStopped => "全部停止",
            ServiceStatus::Starting => "启动中",
            ServiceStatus::Unknown => "未知",
            ServiceStatus::NoContainer => "没有发现容器",
        }
    }

    /// 判断状态是否健康
    pub fn is_healthy(&self) -> bool {
        matches!(self, ServiceStatus::AllRunning)
    }
}

/// 健康检查报告
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HealthReport {
    /// 容器详细信息
    pub containers: Vec<ContainerInfo>,
    /// 运行中的容器数量
    running_count: usize,
    /// 已完成的容器数量 (一次性任务)
    one_shot_count: usize,
    /// 总容器数量
    total_count: usize,
    /// 检查时间
    pub check_time: chrono::DateTime<chrono::Utc>,
    /// 错误信息
    pub errors: Vec<String>,
}

impl HealthReport {
    /// 添加容器信息
    pub fn add_container(&mut self, container: ContainerInfo) {
        self.containers.push(container);
    }

    /// 添加错误信息
    pub fn add_error(&mut self, error: String) {
        self.errors.push(error);
    }

    /// 完成报告并计算整体状态
    pub fn finalize(&self) -> ServiceStatus {
        let healthy_count = self.get_healthy_count();
        let total_count = self.get_total_count();
        let one_shot_count = self.get_one_shot_count();
        let running_count = self.get_running_count();

        let overall_status = if total_count == 0 {
            ServiceStatus::NoContainer
        } else if (healthy_count + one_shot_count) == total_count {
            ServiceStatus::AllRunning
        } else if running_count == 0 {
            ServiceStatus::AllStopped
        } else {
            // 检查是否有正在启动的容器
            let has_starting = self.containers.iter().any(|c| c.status.is_transitioning());
            if has_starting {
                ServiceStatus::Starting
            } else {
                ServiceStatus::PartiallyRunning
            }
        };
        overall_status
    }

    /// 获取运行中的容器列表
    pub fn get_running_containers(&self) -> Vec<&ContainerInfo> {
        self.containers
            .iter()
            .filter(|c| matches!(c.status, ContainerStatus::Running))
            .collect()
    }

    /// 获取已完成的容器列表
    pub fn get_completed_containers(&self) -> Vec<&ContainerInfo> {
        self.containers
            .iter()
            .filter(|c| matches!(c.status, ContainerStatus::Completed))
            .collect()
    }

    /// 获取失败的容器列表
    pub fn get_failed_containers(&self) -> Vec<&ContainerInfo> {
        self.containers
            .iter()
            .filter(|c| c.status.is_failed())
            .collect()
    }

    /// 获取运行中的容器数量 ,不保证一次性的初始化容器
    pub fn get_running_count(&self) -> usize {
        self.containers
            .iter()
            .filter(|c| c.status.is_running())
            .count()
    }

    /// 获取总容器数
    pub fn get_total_count(&self) -> usize {
        self.containers.len()
    }

    /// 获取正在启动的容器列表
    pub fn get_starting_containers(&self) -> Vec<&ContainerInfo> {
        self.containers
            .iter()
            .filter(|c| c.status.is_transitioning())
            .collect()
    }

    /// 获取一次性容器数量
    pub fn get_one_shot_count(&self) -> usize {
        self.containers.iter().filter(|c| c.is_oneshot()).count()
    }

    /// 获取健康容器总数
    pub fn get_healthy_count(&self) -> usize {
        self.containers
            .iter()
            .filter_map(|c| c.health)
            .filter(|&c| c == HealthStatusEnum::HEALTHY)
            .count()
    }

    /// 获取失败容器名称列表
    pub fn get_failed_container_names(&self) -> Vec<String> {
        self.get_failed_containers()
            .iter()
            .map(|c| c.name.clone())
            .collect()
    }

    /// 获取状态摘要字符串
    pub fn get_status_summary(&self) -> String {
        let failed_containers = self.get_failed_container_names();
        let starting_containers: Vec<String> = self
            .get_starting_containers()
            .iter()
            .map(|c| c.name.clone())
            .collect();

        let mut summary = format!(
            "📊 [健康: {}/{}] ✅ 运行: {} | ✔️ 一次性(初始化用): {} | ❌ 失败: {} | ⏳ 启动中: {}",
            self.get_healthy_count(),
            self.get_total_count(),
            self.get_running_count(),
            self.get_one_shot_count(),
            failed_containers.len(),
            starting_containers.len()
        );

        if !failed_containers.is_empty() {
            summary.push_str(&format!(" | 失败容器: {}", failed_containers.join(", ")));
        }

        if !starting_containers.is_empty() {
            summary.push_str(&format!(" | 启动中: {}", starting_containers.join(", ")));
        }

        summary
    }

    /// 检查是否所有服务都健康
    pub fn is_all_healthy(&self) -> bool {
        let healthy_count = self.get_healthy_count();
        let one_shot_count = self.get_one_shot_count();
        let total_count = self.get_total_count();
        healthy_count > 0 && healthy_count == total_count - one_shot_count
    }

    /// 获取所有健康容器（运行中 + 已完成）
    pub fn healthy_containers(&self) -> Vec<&ContainerInfo> {
        self.containers
            .iter()
            .filter(|c| c.status.is_healthy())
            .collect()
    }

    /// 获取总容器数
    pub fn total_containers(&self) -> usize {
        self.containers.len()
    }

    /// 获取失败容器列表（别名，兼容性）
    pub fn failed_containers(&self) -> Vec<&ContainerInfo> {
        self.get_failed_containers()
    }
}

impl Default for HealthReport {
    fn default() -> Self {
        Self {
            containers: Vec::new(),
            running_count: 0,
            one_shot_count: 0,
            total_count: 0,
            check_time: chrono::Utc::now(),
            errors: Vec::new(),
        }
    }
}

/// 健康检查器
pub struct HealthChecker {
    docker_manager: Arc<DockerManager>,
}

impl HealthChecker {
    /// 创建新的健康检查器
    pub fn new(docker_manager: Arc<DockerManager>) -> Self {
        Self { docker_manager }
    }

    /// 获取服务的restart策略
    async fn get_restart_policy(&self, service_name: &str) -> Option<RestartPolicy> {
        if let Ok(service_config) = self.docker_manager.parse_service_config(service_name).await {
            if let Some(restart_str) = service_config.restart {
                return RestartPolicy::from_str(&restart_str);
            }
        }
        None
    }

    /// 执行健康检查
    /// 使用基于Docker Compose标签的精确匹配
    pub async fn health_check(&self) -> DockerServiceResult<HealthReport> {
        info!("🏥 开始健康检查...");

        // 获取 docker-compose 项目信息
        let compose_project_name = self.docker_manager.get_compose_project_name();
        let compose_file_path = self
            .docker_manager
            .get_compose_file()
            .to_string_lossy()
            .to_string();

        info!("📋 Docker Compose 项目信息:");
        info!("   - 项目名称: {}", compose_project_name);
        info!("   - 配置文件: {}", compose_file_path);

        // 创建健康检查报告
        let mut report = HealthReport::default();

        // 获取compose文件中定义的所有服务
        let compose_services = self
            .docker_manager
            .get_compose_service_names()
            .await
            .unwrap_or_else(|e| {
                error!("获取compose服务列表失败: {}", e);
                HashSet::new()
            });

        if compose_services.is_empty() {
            warn!("⚠️  compose文件中未找到任何服务定义");
            return Ok(report);
        }

        info!("🔍 compose文件中定义的服务: {:?}", compose_services);

        // 获取系统中所有容器
        let all_containers = self
            .docker_manager
            .get_all_containers_status()
            .await
            .unwrap_or_else(|e| {
                error!("获取容器状态失败: {}", e);
                Vec::new()
            });

        info!("📊 系统中发现 {} 个容器", all_containers.len());

        // 🔧 使用标签精确匹配容器
        let mut found_services = HashSet::new();
        let mut added_containers = HashSet::new();

        // 第一轮：处理正在运行的和已停止的容器
        for service in &all_containers {
            // 🆕 使用标签精确匹配
            if let Some(service_name) = self.get_container_service_name(&service.name).await {
                // 验证是否属于当前项目
                if self
                    .is_container_from_compose_project(
                        &service.name,
                        &compose_project_name,
                        &compose_file_path,
                    )
                    .await
                {
                    // 检查是否在compose文件中定义
                    if compose_services.contains(&service_name) {
                        info!(
                            "✅ 精确匹配compose服务: {} -> {}",
                            service.name, service_name
                        );

                        // 🔧 防重复：检查是否已经添加过这个compose服务
                        if added_containers.contains(&service_name) {
                            warn!(
                                "⚠️  跳过重复的compose服务: {} (容器: {})",
                                service_name, service.name
                            );
                            continue;
                        }

                        found_services.insert(service_name.clone());
                        added_containers.insert(service_name.clone());

                        // 检查是否为一次性服务
                        let is_oneshot = self.is_oneshot_service(&service_name).await;

                        // 获取restart策略
                        let restart_policy = self.get_restart_policy(&service_name).await;

                        // 使用增强的状态解析逻辑
                        let status = self.determine_container_status(service, is_oneshot);

                        // 获取容器的健康检查状态
                        let health = self.get_container_health_status(&service.name).await;

                        let container = ContainerInfo {
                            name: service_name.clone(), // 使用compose中定义的服务名
                            status,
                            image: service.image.clone(),
                            ports: service.ports.clone(),
                            uptime: None,
                            health,
                            is_oneshot,
                            restart: restart_policy,
                        };

                        debug!(
                            "📦 添加容器: {} (状态: {:?}, 一次性: {})",
                            container.name, container.status, is_oneshot
                        );
                        report.add_container(container);
                    } else {
                        // 不在compose文件中定义的容器（可能是历史遗留）
                        warn!(
                            "⏭️  跳过非项目容器: {} (服务: {}, 不在compose文件中定义)",
                            service.name, service_name
                        );
                    }
                } else {
                    // 不属于当前项目的容器
                    debug!("⏭️  跳过其他项目容器: {} (项目: 其他)", service.name);
                }
            } else {
                // 无法获取服务名称，可能不是compose容器
                debug!("⏭️  跳过非compose容器: {} (无标签信息)", service.name);
            }
        }

        info!(
            "📊 第一轮处理完成: 已添加 {} 个容器",
            added_containers.len()
        );

        // 为未找到的compose服务创建"已停止"状态的条目
        for service_name in &compose_services {
            if !found_services.contains(service_name) {
                // 🔧 防重复：再次检查是否已经添加过
                if added_containers.contains(service_name) {
                    warn!("⚠️  跳过重复的未运行服务: {}", service_name);
                    continue;
                }

                let is_oneshot = self.is_oneshot_service(service_name).await;

                // 获取restart策略
                let restart_policy = self.get_restart_policy(service_name).await;

                let status = if is_oneshot {
                    // 一次性服务未运行通常表示已完成
                    ContainerStatus::Completed
                } else {
                    // 持续服务未运行表示已停止
                    ContainerStatus::Stopped
                };

                let container = ContainerInfo {
                    name: service_name.clone(),
                    status,
                    image: "未启动".to_string(),
                    ports: Vec::new(),
                    uptime: None,
                    health: None,
                    is_oneshot,
                    restart: restart_policy,
                };

                info!(
                    "📦 添加未运行服务: {} (状态: {:?}, 一次性: {})",
                    container.name, container.status, is_oneshot
                );
                report.add_container(container);
                added_containers.insert(service_name.clone());
            }
        }

        info!(
            "📊 最终统计: compose服务={}, 已添加容器={}",
            compose_services.len(),
            added_containers.len()
        );

        // 生成健康检查摘要
        let summary = format!(
            "健康检查完成: {}/{} 容器健康",
            report.get_healthy_count(),
            report.get_total_count()
        );

        info!("🎯 {}", summary);

        Ok(report)
    }

    /// 智能判断容器状态
    fn determine_container_status(
        &self,
        service: &client_core::container::ServiceInfo,
        is_oneshot: bool,
    ) -> ContainerStatus {
        match service.status {
            client_core::container::ServiceStatus::Running => ContainerStatus::Running,
            client_core::container::ServiceStatus::Stopped => {
                if is_oneshot {
                    // 一次性任务停止通常表示已完成
                    ContainerStatus::Completed
                } else {
                    ContainerStatus::Stopped
                }
            }
            client_core::container::ServiceStatus::Unknown => ContainerStatus::Unknown,
            client_core::container::ServiceStatus::Created => ContainerStatus::Unknown,
            client_core::container::ServiceStatus::Restarting => ContainerStatus::Starting,
        }
    }

    /// 检查服务是否为一次性任务 - 增强版
    async fn is_oneshot_service(&self, service_name: &str) -> bool {
        // 1. 尝试从docker-compose.yml文件解析restart策略
        if let Ok(service_config) = self.docker_manager.parse_service_config(service_name).await {
            if let Some(restart_policy) = service_config.restart {
                // restart: "no" 表示不自动重启，通常是一次性任务
                if restart_policy == "no" || restart_policy == "false" {
                    info!(
                        "服务 {} 的restart策略为: {} (一次性任务)",
                        service_name, restart_policy
                    );
                    return true;
                }
                // restart: "always" 或 "unless-stopped" 表示应该一直运行
                if restart_policy == "always"
                    || restart_policy == "unless-stopped"
                    || restart_policy == "on-failure"
                {
                    info!(
                        "服务 {} 的restart策略为: {} (持续服务)",
                        service_name, restart_policy
                    );
                    return false;
                }
            }
        }

        false
    }

    /// 获取容器的Docker Compose标签信息
    /// 使用bollard库直接从Docker API获取容器标签信息
    async fn get_container_labels(&self, container_name: &str) -> Option<ComposeLabels> {
        match Docker::connect_with_socket_defaults() {
            Ok(docker) => {
                // 获取容器列表，查找指定容器
                let options = Some(ListContainersOptions::<String> {
                    all: true,
                    ..Default::default()
                });

                match docker.list_containers(options).await {
                    Ok(containers) => {
                        for container in containers {
                            // 检查容器名称是否匹配
                            if let Some(names) = &container.names {
                                let container_matches = names.iter().any(|name| {
                                    // Docker容器名称通常以/开头，需要去掉
                                    let clean_name = name.strip_prefix('/').unwrap_or(name);
                                    clean_name == container_name
                                });

                                if container_matches {
                                    if let Some(labels) = &container.labels {
                                        return Some(ComposeLabels {
                                            project: labels
                                                .get("com.docker.compose.project")
                                                .cloned(),
                                            service: labels
                                                .get("com.docker.compose.service")
                                                .cloned(),
                                            container_number: labels
                                                .get("com.docker.compose.container-number")
                                                .cloned(),
                                            oneoff: labels
                                                .get("com.docker.compose.oneoff")
                                                .and_then(|v| v.parse::<bool>().ok())
                                                .or_else(|| {
                                                    labels
                                                        .get("com.docker.compose.oneoff")
                                                        .map(|v| v.to_lowercase() == "true")
                                                }),
                                            config_files: labels
                                                .get("com.docker.compose.project.config_files")
                                                .cloned(),
                                            working_dir: labels
                                                .get("com.docker.compose.project.working_dir")
                                                .cloned(),
                                        });
                                    }
                                    return None; // 找到容器但没有标签
                                }
                            }
                        }
                        None // 没有找到匹配的容器
                    }
                    Err(e) => {
                        warn!("bollard 获取容器列表失败: {}", e);
                        None
                    }
                }
            }
            Err(e) => {
                warn!("bollard 连接Docker失败: {}", e);
                None
            }
        }
    }

    /// 验证容器是否属于指定的docker-compose项目
    /// 基于标签精确匹配，避免名称匹配的不准确性
    async fn is_container_from_compose_project(
        &self,
        container_name: &str,
        project_name: &str,
        compose_file_path: &str,
    ) -> bool {
        if let Some(labels) = self.get_container_labels(container_name).await {
            // 1. 检查项目名称是否匹配
            if let Some(label_project) = &labels.project {
                if label_project != project_name {
                    info!(
                        "❌ 容器 {} 项目名称不匹配: {} vs {}",
                        container_name, label_project, project_name
                    );
                    return false;
                }
            } else {
                info!("❌ 容器 {} 缺少项目标签", container_name);
                return false;
            }

            // 2. 检查配置文件路径是否匹配（处理相对路径vs绝对路径问题）
            if let Some(label_config_files) = &labels.config_files {
                // 将我们的配置文件路径转换为绝对路径
                let compose_file_absolute =
                    match std::path::Path::new(compose_file_path).canonicalize() {
                        Ok(abs_path) => abs_path.to_string_lossy().to_string(),
                        Err(_) => {
                            // 如果无法获取绝对路径，尝试基于当前目录构建
                            let current_dir = std::env::current_dir().unwrap_or_default();
                            let full_path = current_dir.join(compose_file_path);
                            full_path.to_string_lossy().to_string()
                        }
                    };

                debug!(
                    "🔍 路径比较: 容器标签路径={}, 我们的绝对路径={}",
                    label_config_files, compose_file_absolute
                );

                #[cfg(windows)]
                fn normalize_win_path(path: &str) -> &str {
                    if path.starts_with(r"\\?\") {
                        &path[4..]
                    } else {
                        path
                    }
                }

                #[cfg(windows)]
                let matched = normalize_win_path(label_config_files)
                    .eq_ignore_ascii_case(normalize_win_path(&compose_file_absolute));
                #[cfg(not(windows))]
                let matched = label_config_files == &compose_file_absolute;

                if matched {
                    debug!("✅ 容器 {} 配置文件路径匹配", container_name);
                    return true;
                } else {
                    debug!(
                        "❌ 容器 {} 配置文件路径不匹配: {} vs {}",
                        container_name, label_config_files, compose_file_absolute
                    );
                    return false;
                }
            }

            // 3. 如果没有配置文件路径信息，但项目名称匹配，则认为匹配
            info!(
                "⚠️  容器 {} 缺少配置文件路径，但项目名称匹配",
                container_name
            );
            true
        } else {
            // 如果无法获取标签，说明不是compose容器
            info!("❌ 容器 {} 无法获取Compose标签信息", container_name);
            false
        }
    }

    /// 根据标签获取容器的服务名称
    async fn get_container_service_name(&self, container_name: &str) -> Option<String> {
        self.get_container_labels(container_name)
            .await
            .and_then(|labels| labels.service)
    }

    /// 获取Docker容器的健康检查状态
    async fn get_container_health_status(&self, container_name: &str) -> Option<HealthStatusEnum> {
        match Docker::connect_with_socket_defaults() {
            Ok(docker) => {
                match docker
                    .inspect_container(container_name, None::<InspectContainerOptions>)
                    .await
                {
                    Ok(container_info) => container_info
                        .state
                        .and_then(|state| state.health.map(|health| health.status).flatten()),
                    Err(e) => {
                        warn!("无法获取容器 {} 的健康状态: {}", container_name, e);
                        None
                    }
                }
            }
            Err(e) => {
                warn!("无法连接Docker获取容器健康状态: {}", e);
                None
            }
        }
    }

    /// 等待服务启动完成 - 智能等待策略
    pub async fn wait_for_services_ready(
        &self,
        check_interval: Duration,
    ) -> DockerServiceResult<HealthReport> {
        use std::time::Instant;

        // 最长检查180秒
        let timeout = Duration::from_secs(timeout::HEALTH_CHECK_TIMEOUT);

        let start_time = Instant::now();

        info!("⏳ 开始检查服务启动状态，超时时间: {}秒", timeout.as_secs());

        loop {
            let elapsed = start_time.elapsed();
            if elapsed >= timeout {
                error!("⏰ 健康检查超时! 用时: {}秒", elapsed.as_secs());
                return Err(DockerServiceError::Timeout {
                    operation: "等待服务启动".to_string(),
                    timeout_seconds: timeout.as_secs(),
                });
            }

            // 执行健康检查
            let report = self.health_check().await?;

            // 检查是否所有服务都已就绪
            if report.is_all_healthy() {
                info!("🎉 所有服务已成功启动! 用时: {}秒", elapsed.as_secs());
                return Ok(report);
            } else {
                info!("⏳ 服务启动中... 已等待: {}秒", elapsed.as_secs());
                //打印尚未启动成功容器
                let failed_containers = report.failed_containers();
                if !failed_containers.is_empty() {
                    let failed_names: Vec<&str> =
                        failed_containers.iter().map(|c| c.name.as_str()).collect();
                    info!("❌ 尚未启动成功容器: {failed_names:?}");
                }
            }

            tokio::time::sleep(check_interval).await;
        }
    }

    /// 获取服务状态摘要
    pub async fn get_status_summary(&self) -> DockerServiceResult<String> {
        let report = self.health_check().await?;

        let mut summary = format!(
            "服务状态: 健康 ({}/{})",
            report.healthy_containers().len(),
            report.total_containers()
        );

        if !report.errors.is_empty() {
            summary.push_str(&format!("\n错误: {}", report.errors.join(", ")));
        }

        let failed_containers = report.failed_containers();
        if !failed_containers.is_empty() {
            let failed_names: Vec<&str> =
                failed_containers.iter().map(|c| c.name.as_str()).collect();
            summary.push_str(&format!("\n失败容器: {failed_names:?}"));
        }

        Ok(summary)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_health_report() {
        let mut report = HealthReport::default();

        report.add_container(ContainerInfo {
            name: "service1".to_string(),
            status: ContainerStatus::Running,
            image: "test:latest".to_string(),
            ports: vec!["8080:8080".to_string()],
            uptime: None,
            health: None,
            is_oneshot: false,
            restart: Some(RestartPolicy::UnlessStopped),
        });

        report.add_container(ContainerInfo {
            name: "service2".to_string(),
            status: ContainerStatus::Starting,
            image: "test2:latest".to_string(),
            ports: vec![],
            uptime: None,
            health: None,
            is_oneshot: false,
            restart: Some(RestartPolicy::Always),
        });

        assert_eq!(report.finalize(), ServiceStatus::Starting);
        assert_eq!(report.running_count, 1);
        assert_eq!(report.total_count, 2);
    }
}
