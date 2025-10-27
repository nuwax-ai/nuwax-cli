use anyhow::Result;
use client_core::constants::{docker, timeout};
use ducker::docker::container::DockerContainer;
use ducker::docker::util::new_local_docker_connection;
use serde_yaml::Value;
use std::fs;
use std::path::Path;
use std::time::Duration;
use tokio::time::sleep;
use tracing::{error, info, warn};

/// 简单的Docker服务过滤条件
///
/// 注意：推荐使用 health_check.rs 中的 HealthChecker 来获得更准确的状态判断
#[derive(Debug, Clone)]
pub enum ServiceFilter {
    /// 按容器名称关键字过滤
    NameContains(Vec<String>),
    /// 检查所有容器
    All,
}

impl ServiceFilter {
    /// 检查容器是否匹配过滤条件
    pub fn matches(&self, container: &DockerContainer) -> bool {
        match self {
            ServiceFilter::NameContains(keywords) => {
                if keywords.is_empty() {
                    return true;
                }
                keywords.iter().any(|keyword| {
                    container
                        .names
                        .to_lowercase()
                        .contains(&keyword.to_lowercase())
                })
            }
            ServiceFilter::All => true,
        }
    }
}

/// 简单检查指定的Docker服务是否在运行
///
/// ⚠️ 注意：这是一个简化版本，只检查容器的 running 状态。
/// 对于更准确的状态判断（包括一次性任务的正确处理），
/// 请使用 health_check.rs 中的 HealthChecker。
pub async fn check_services_running(filter: &ServiceFilter) -> Result<bool> {
    match new_local_docker_connection(docker::DOCKER_SOCKET_PATH, None).await {
        Ok(docker) => {
            match DockerContainer::list(&docker).await {
                Ok(containers) => {
                    let filtered_containers: Vec<_> =
                        containers.iter().filter(|c| filter.matches(c)).collect();

                    // 简单计算：只看运行中的容器
                    let running_count = filtered_containers
                        .iter()
                        .filter(|container| container.running)
                        .count();

                    let total_filtered = filtered_containers.len();

                    match filter {
                        ServiceFilter::All => {
                            info!(
                                "发现 {} 个运行中的容器（总共 {} 个）",
                                running_count, total_filtered
                            );
                        }
                        ServiceFilter::NameContains(keywords) => {
                            info!(
                                "匹配关键字 {:?} 的容器: {} 个运行中（总共 {} 个）",
                                keywords, running_count, total_filtered
                            );
                        }
                    }

                    Ok(running_count > 0)
                }
                Err(e) => {
                    error!("获取容器列表失败: {}", e);
                    Err(anyhow::anyhow!(format!("获取容器列表失败: {e}")))
                }
            }
        }
        Err(e) => {
            error!("无法连接到Docker: {}", e);
            Err(anyhow::anyhow!(format!("无法连接到Docker: {e}")))
        }
    }
}

/// 等待指定的Docker服务完全停止
///
/// 注意：推荐使用 health_check.rs 中的 wait_for_services_ready 方法
pub async fn wait_for_services_stopped(filter: &ServiceFilter, timeout_secs: u64) -> Result<bool> {
    let start_time = tokio::time::Instant::now();
    let timeout = Duration::from_secs(timeout_secs);

    info!(
        "开始等待服务停止，过滤条件: {:?}，超时: {} 秒",
        filter, timeout_secs
    );

    while start_time.elapsed() < timeout {
        match check_services_running(filter).await {
            Ok(false) => {
                info!("指定的Docker服务已完全停止");
                return Ok(true);
            }
            Ok(true) => {
                info!("等待Docker服务停止...");
                sleep(Duration::from_secs(timeout::SERVICE_CHECK_INTERVAL)).await;
            }
            Err(e) => {
                warn!("检查服务状态时出错: {}", e);
                sleep(Duration::from_secs(timeout::SERVICE_CHECK_INTERVAL)).await;
            }
        }
    }

    warn!("等待服务停止超时 ({} 秒)", timeout_secs);
    Ok(false)
}

/// 等待指定的Docker服务完全启动
///
/// 注意：推荐使用 health_check.rs 中的 wait_for_services_ready 方法
pub async fn wait_for_services_started(filter: &ServiceFilter, timeout_secs: u64) -> Result<bool> {
    let start_time = tokio::time::Instant::now();
    let timeout = Duration::from_secs(timeout_secs);

    info!(
        "开始等待服务启动，过滤条件: {:?}，超时: {} 秒",
        filter, timeout_secs
    );

    while start_time.elapsed() < timeout {
        match check_services_running(filter).await {
            Ok(true) => {
                info!("指定的Docker服务已启动");
                return Ok(true);
            }
            Ok(false) => {
                info!("等待Docker服务启动...");
                sleep(Duration::from_secs(timeout::SERVICE_CHECK_INTERVAL)).await;
            }
            Err(e) => {
                warn!("检查服务状态时出错: {}", e);
                sleep(Duration::from_secs(timeout::SERVICE_CHECK_INTERVAL)).await;
            }
        }
    }

    warn!("等待服务启动超时 ({} 秒)", timeout_secs);
    Ok(false)
}

/// 从docker-compose.yml文件中解析服务名称
pub async fn parse_service_names_from_compose(compose_file_path: &Path) -> Result<Vec<String>> {
    if !compose_file_path.exists() {
        warn!(
            "docker-compose.yml 文件不存在: {}",
            compose_file_path.display()
        );
        return Ok(vec![]);
    }

    match fs::read_to_string(compose_file_path) {
        Ok(content) => match serde_yaml::from_str::<Value>(&content) {
            Ok(yaml) => {
                let mut service_names = Vec::new();

                if let Some(services) = yaml.get("services") {
                    if let Some(services_map) = services.as_mapping() {
                        for (key, _value) in services_map {
                            if let Some(service_name) = key.as_str() {
                                service_names.push(service_name.to_string());
                            }
                        }
                    }
                }

                info!(
                    "从 {} 解析到 {} 个服务:",
                    compose_file_path.display(),
                    service_names.len()
                );
                for name in &service_names {
                    info!("  - {}", name);
                }

                Ok(service_names)
            }
            Err(e) => {
                error!("解析docker-compose.yml失败: {}", e);
                Err(anyhow::anyhow!(format!("解析docker-compose.yml失败: {e}")))
            }
        },
        Err(e) => {
            error!("读取docker-compose.yml文件失败: {}", e);
            Err(anyhow::anyhow!(format!(
                "读取docker-compose.yml文件失败: {e}"
            )))
        }
    }
}

/// 基于docker-compose.yml创建简单的服务过滤器
pub async fn create_compose_filter(compose_file_path: &Path) -> Result<ServiceFilter> {
    let service_names = parse_service_names_from_compose(compose_file_path).await?;

    if service_names.is_empty() {
        warn!("未找到服务配置，将检查所有容器");
        Ok(ServiceFilter::All)
    } else {
        Ok(ServiceFilter::NameContains(service_names))
    }
}

/// 便捷函数：等待compose服务停止
pub async fn wait_for_compose_services_stopped(
    compose_file_path: &Path,
    timeout_secs: u64,
) -> Result<bool> {
    let filter = create_compose_filter(compose_file_path).await?;
    wait_for_services_stopped(&filter, timeout_secs).await
}

/// 便捷函数：等待compose服务启动
pub async fn wait_for_compose_services_started(
    compose_file_path: &Path,
    timeout_secs: u64,
) -> Result<bool> {
    let filter = create_compose_filter(compose_file_path).await?;
    wait_for_services_started(&filter, timeout_secs).await
}
