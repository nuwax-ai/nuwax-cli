use std::collections::{HashMap, HashSet};

use super::types::{DockerManager, ServiceInfo, ServiceStatus};
use crate::constants::timeout;
use anyhow::Result;
use ducker::docker::{container::DockerContainer, util::new_local_docker_connection};
use tokio::time::{Duration, sleep};
use tracing::{debug, error, info, warn};

impl DockerManager {
    /// 启动所有服务
    pub async fn start_services(&self) -> Result<()> {
        info!("🚀 开始启动Docker服务...");

        info!("📋 步骤1: 检查环境先决条件...");
        self.check_prerequisites().await?;

        info!("📁 步骤2: 检查并创建宿主机挂载目录...");
        self.ensure_host_volumes_exist().await?;

        info!("🎯 步骤3: 执行docker-compose up命令...");
        let output = self.run_compose_command(&["up", "-d"]).await?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            let stdout = String::from_utf8_lossy(&output.stdout);
            let exit_code = output.status.code().unwrap_or(-1);

            let error_msg = format!(
                "启动服务失败 (退出码: {exit_code}):\n标准错误: {stderr}\n标准输出: {stdout}"
            );

            error!("❌ 启动服务失败详情: {}", error_msg);
            return Err(anyhow::anyhow!(error_msg));
        }

        info!("✅ docker-compose up命令执行成功");

        // 等待服务启动并验证状态
        info!("⏳ 步骤3: 等待服务启动并验证状态...");
        self.verify_services_started(None).await?;

        info!("🎉 所有服务启动完成!");
        Ok(())
    }

    /// 停止所有服务
    pub async fn stop_services(&self) -> Result<()> {
        self.check_prerequisites().await?;

        let output = self.run_compose_command(&["down"]).await?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            let stdout = String::from_utf8_lossy(&output.stdout);
            let exit_code = output.status.code().unwrap_or(-1);

            let error_msg = format!(
                "停止服务失败 (退出码: {exit_code}):\n标准错误: {stderr}\n标准输出: {stdout}"
            );

            error!("{}", error_msg);
            return Err(anyhow::anyhow!(error_msg));
        }

        Ok(())
    }

    /// 重启所有服务
    pub async fn restart_services(&self) -> Result<()> {
        self.stop_services().await?;
        self.start_services().await?;
        Ok(())
    }

    /// 重启单个服务
    pub async fn restart_service(&self, service_name: &str) -> Result<()> {
        self.check_prerequisites().await?;

        // 先停止指定服务
        let output = self.run_compose_command(&["stop", service_name]).await?;
        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            let stdout = String::from_utf8_lossy(&output.stdout);
            let exit_code = output.status.code().unwrap_or(-1);

            let error_msg = format!(
                "停止服务 {service_name} 失败 (退出码: {exit_code}):\n标准错误: {stderr}\n标准输出: {stdout}"
            );

            error!("{}", error_msg);
            return Err(anyhow::anyhow!(error_msg));
        }

        // 再启动指定服务
        let output = self.run_compose_command(&["start", service_name]).await?;
        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            let stdout = String::from_utf8_lossy(&output.stdout);
            let exit_code = output.status.code().unwrap_or(-1);

            let error_msg = format!(
                "启动服务 {service_name} 失败 (退出码: {exit_code}):\n标准错误: {stderr}\n标准输出: {stdout}"
            );

            error!("{}", error_msg);
            return Err(anyhow::anyhow!(error_msg));
        }

        Ok(())
    }

    /// 获取服务状态 - 使用 ducker 库实现，只返回docker-compose中定义的服务
    pub async fn get_services_status(&self) -> Result<Vec<ServiceInfo>> {

        info!("使用 ducker 库获取容器状态...");

        // 1. 获取docker-compose.yml中定义的服务名称
        let compose_services = self.get_compose_service_names().await?;
        info!("docker-compose.yml 中定义的服务: {:?}", compose_services);

        // 2. 获取所有容器信息
        let containers = self.get_all_containers_with_ducker().await?;
        info!("系统中发现 {} 个容器", containers.len());

        // 3. 为每个compose服务收集匹配的容器
        let mut service_containers: HashMap<String, Vec<ServiceInfo>> = HashMap::new();
        let mut compose_services_found = HashSet::new();

        for container in containers {
            // 检查该容器是否属于任何compose服务
            for service_name in &compose_services {
                if self.is_service_name_match(&container.names, service_name) {
                    let service_info =
                        self.convert_docker_container_to_service_info(container.clone());
                    // 使用compose服务名称作为key，而不是容器名称
                    let mut normalized_service_info = service_info;
                    normalized_service_info.name = service_name.clone();

                    service_containers
                        .entry(service_name.clone())
                        .or_default()
                        .push(normalized_service_info);
                    compose_services_found.insert(service_name.clone());
                    break; // 避免同一个容器匹配多个服务
                }
            }
        }

        // 4. 为每个服务选择最优先的状态（优先级：Running > Stopped > Unknown）
        let mut final_services = Vec::new();

        for service_name in &compose_services {
            if let Some(containers) = service_containers.get(service_name) {
                // 对于有多个容器的服务，选择最优先的状态
                let best_container = containers
                    .iter()
                    .max_by_key(|container| {
                        // 状态优先级：Running=2, Stopped=1, Unknown=0
                        match container.status {
                            ServiceStatus::Running => 2,
                            ServiceStatus::Stopped => 1,
                            ServiceStatus::Unknown => 0,
                            ServiceStatus::Created => 0,
                            ServiceStatus::Restarting => 0,
                        }
                    })
                    .unwrap(); // 安全：containers不为空

                final_services.push(best_container.clone());
            } else {
                // 未找到容器的服务，添加为"已停止"状态
                final_services.push(ServiceInfo {
                    name: service_name.clone(),
                    status: ServiceStatus::Stopped,
                    image: "未启动".to_string(),
                    ports: Vec::new(),
                });
            }
        }

        info!(
            "匹配到 {}/{} 个compose服务容器",
            compose_services_found.len(),
            compose_services.len()
        );

        Ok(final_services)
    }

    /// 获取所有容器状态（包括非compose容器）- 保留原有功能
    pub async fn get_all_containers_status(&self) -> Result<Vec<ServiceInfo>> {
        self.check_prerequisites().await?;

        info!("使用 ducker 库获取所有容器状态...");

        // 获取所有容器信息
        let containers = self.get_all_containers_with_ducker().await?;

        // 转换为 ServiceInfo 格式
        let services = containers
            .into_iter()
            .map(|container| self.convert_docker_container_to_service_info(container))
            .collect();

        Ok(services)
    }

    /// 使用 ducker 库获取所有容器信息
    async fn get_all_containers_with_ducker(&self) -> Result<Vec<DockerContainer>> {
        match new_local_docker_connection(crate::constants::docker::DOCKER_SOCKET_PATH, None).await
        {
            Ok(docker) => match DockerContainer::list(&docker).await {
                Ok(containers) => {
                    info!("ducker 成功获取到 {} 个容器", containers.len());
                    Ok(containers)
                }
                Err(e) => {
                    error!("ducker 获取容器列表失败: {}", e);
                    Err(anyhow::anyhow!("获取容器列表失败: {e}"))
                }
            },
            Err(e) => {
                error!("ducker 连接 Docker 失败: {}", e);
                Err(anyhow::anyhow!("连接 Docker 失败: {e}"))
            }
        }
    }

    /// 将 DockerContainer 转换为 ServiceInfo
    fn convert_docker_container_to_service_info(&self, container: DockerContainer) -> ServiceInfo {
        let status = if container.running {
            ServiceStatus::Running
        } else {
            // 根据状态字符串进一步判断
            match container.status.to_lowercase().as_str() {
                s if s.contains("exited") => ServiceStatus::Stopped,
                s if s.contains("created") => ServiceStatus::Created,
                s if s.contains("restarting") => ServiceStatus::Restarting,
                s if s.contains("paused") => ServiceStatus::Stopped,
                s if s.contains("dead") => ServiceStatus::Stopped,
                s if s.contains("running") => ServiceStatus::Running,
                _ => ServiceStatus::Unknown,
            }
        };

        // 解析端口映射
        let ports = if container.ports.is_empty() {
            Vec::new()
        } else {
            container
                .ports
                .split(", ")
                .filter(|s| !s.trim().is_empty())
                .map(|s| s.trim().to_string())
                .collect()
        };

        ServiceInfo {
            name: container.names.clone(),
            status,
            image: container.image.clone(),
            ports,
        }
    }

    /// 检查单个服务是否正在运行 - 使用 ducker 实现
    pub async fn is_service_running(&self, service_name: &str) -> Result<bool> {
        let services = self.get_services_status().await?;

        for service in services {
            if self.is_service_name_match(&service.name, service_name) {
                return Ok(service.status == ServiceStatus::Running);
            }
        }

        Ok(false)
    }

    /// 判断容器是否属于指定的compose服务
    /// 使用docker-compose的容器命名规则进行匹配
    fn is_service_name_match(&self, container_name: &str, service_name: &str) -> bool {
        // 生成可能的容器名称模式
        let patterns = self.generate_compose_container_patterns(service_name);

        let container_lower = container_name.to_lowercase();

        // 检查容器名称是否匹配任何模式
        for pattern in patterns {
            let pattern_lower = pattern.to_lowercase();

            // 精确匹配
            if container_lower == pattern_lower {
                return true;
            }

            // 前缀匹配（处理有额外后缀的情况）
            if container_lower.starts_with(&pattern_lower) {
                return true;
            }
        }

        // 更严格的匹配逻辑：只有当服务名称在容器名称中作为完整单词出现时才匹配
        let service_lower = service_name.to_lowercase();

        // 检查是否是docker-compose标准格式的完整匹配
        // 标准格式：{项目名}_{服务名}_{实例号} 或 {项目名}-{服务名}-{实例号}
        let separators = vec!["_", "-"];

        for separator in separators {
            // 格式1: 项目名_服务名_数字
            let pattern1 = format!("{separator}{service_lower}{separator}");
            if container_lower.contains(&pattern1) {
                return true;
            }

            // 格式2: 项目名_服务名 (结尾)
            let pattern2 = format!("{separator}{service_lower}");
            if container_lower.ends_with(&pattern2) {
                return true;
            }

            // 格式3: 服务名_数字 (开头)
            let pattern3 = format!("{service_lower}{separator}");
            if container_lower.starts_with(&pattern3) {
                return true;
            }
        }

        // 如果所有严格匹配都失败，只有在完全相同的情况下才匹配
        container_lower == service_lower
    }

    /// 获取特定服务的详细信息
    pub async fn get_service_detail(&self, service_name: &str) -> Result<Option<ServiceInfo>> {
        let services = self.get_services_status().await?;

        for service in services {
            if self.is_service_name_match(&service.name, service_name) {
                return Ok(Some(service));
            }
        }

        Ok(None)
    }

    /// 检查所有服务的健康状况
    pub async fn check_services_health(&self) -> Result<()> {
        let services = self.get_services_status().await?;

        if services.is_empty() {
            return Err(anyhow::anyhow!("没有找到任何服务"));
        }

        let mut unhealthy_services = Vec::new();
        for service in services {
            if service.status != ServiceStatus::Running {
                unhealthy_services.push(service.name);
            }
        }

        if !unhealthy_services.is_empty() {
            return Err(anyhow::anyhow!(
                "部分服务未在运行: {}",
                unhealthy_services.join(", ")
            ));
        }

        Ok(())
    }

    /// 验证服务启动状态（启动后等待并检查实际状态）
    ///
    /// # 参数
    /// * `custom_timeout` - 自定义超时时间（秒），如果为None则使用默认的SERVICE_START_TIMEOUT
    async fn verify_services_started(&self, custom_timeout: Option<u64>) -> Result<()> {
        // 使用统一的常量配置
        let max_wait_time =
            Duration::from_secs(custom_timeout.unwrap_or(timeout::SERVICE_START_TIMEOUT));
        let check_interval = Duration::from_secs(timeout::SERVICE_CHECK_INTERVAL);
        let max_attempts = max_wait_time.as_secs() / check_interval.as_secs();

        info!(
            "🔍 开始验证服务启动状态 (超时: {}秒, 检查间隔: {}秒)",
            max_wait_time.as_secs(),
            check_interval.as_secs()
        );

        for attempt in 1..=max_attempts {
            info!("⏳ 第 {}/{} 次检查服务状态...", attempt, max_attempts);

            // 获取当前服务状态
            match self.get_services_status().await {
                Ok(services) => {
                    if services.is_empty() {
                        info!("⚠️ 没有找到任何服务，可能compose文件没有定义服务");
                        return Ok(()); // 允许空服务情况
                    }

                    info!("📊 发现 {} 个服务，正在检查状态...", services.len());

                    // 检查是否有必须运行的服务
                    let mut failed_services = Vec::new();
                    let mut pending_services = Vec::new();
                    let mut running_services = Vec::new();

                    for service in &services {
                        match service.status {
                            ServiceStatus::Running => {
                                // 服务正在运行，很好
                                running_services.push(service.name.clone());
                                debug!("服务 {} 运行正常", service.name);
                            }
                            ServiceStatus::Stopped => {
                                // 检查这是否是一次性任务服务
                                if self
                                    .is_oneshot_service(&service.name)
                                    .await
                                    .unwrap_or(false)
                                {
                                    debug!(
                                        "服务 {} 是一次性任务，已正常退出",
                                        service.name
                                    );
                                } else {
                                    failed_services.push(service.name.clone());
                                }
                            }
                            ServiceStatus::Unknown => {
                                pending_services.push(service.name.clone());
                            }
                            ServiceStatus::Created => {
                                pending_services.push(service.name.clone());
                            }
                            ServiceStatus::Restarting => {
                                pending_services.push(service.name.clone());
                            }
                        }
                    }

                    // 显示当前状态
                    if !running_services.is_empty() {
                        info!("✅ 运行中的服务: {}", running_services.join(", "));
                    }
                    if !pending_services.is_empty() {
                        info!("⏳ 等待启动的服务: {}", pending_services.join(", "));
                    }
                    if !failed_services.is_empty() {
                        info!("❌ 启动失败的服务: {}", failed_services.join(", "));
                    }

                    // 如果没有失败的服务且没有待定的服务，说明启动成功
                    if failed_services.is_empty() && pending_services.is_empty() {
                        info!("🎉 所有服务启动验证成功！");
                        tracing::info!("所有服务启动验证成功");
                        return Ok(());
                    }

                    // 如果有失败的服务，记录但继续等待（可能需要更多时间）
                    if !failed_services.is_empty() {
                        warn!("⚠️ 服务启动失败: {}", failed_services.join(", "));
                        tracing::warn!("服务启动失败: {}", failed_services.join(", "));
                    }

                    if !pending_services.is_empty() {
                        info!("⏳ 继续等待服务启动: {}", pending_services.join(", "));
                        tracing::debug!("等待服务启动: {}", pending_services.join(", "));
                    }

                    // 如果是最后一次尝试，返回错误
                    if attempt == max_attempts {
                        let mut error_msg = String::new();
                        if !failed_services.is_empty() {
                            error_msg.push_str(&format!(
                                "启动失败的服务: {}",
                                failed_services.join(", ")
                            ));
                        }
                        if !pending_services.is_empty() {
                            if !error_msg.is_empty() {
                                error_msg.push_str("; ");
                            }
                            error_msg.push_str(&format!(
                                "启动超时的服务: {}",
                                pending_services.join(", ")
                            ));
                        }
                        error!("❌ 服务启动验证失败: {}", error_msg);
                        return Err(anyhow::anyhow!("服务启动验证失败: {error_msg}"));
                    }
                }
                Err(e) => {
                    warn!("⚠️ 获取服务状态失败: {}", e);
                    if attempt == max_attempts {
                        error!("❌ 无法获取服务状态: {}", e);
                        return Err(anyhow::anyhow!("无法获取服务状态: {e}"));
                    }
                }
            }

            // 等待下次检查
            if attempt < max_attempts {
                info!("⏳ 等待 {} 秒后进行下次检查...", check_interval.as_secs());
                sleep(check_interval).await;
            }
        }

        Ok(())
    }
}
