use anyhow::Result;
use client_core::constants::{docker, timeout};
use ducker::docker::container::DockerContainer;
use ducker::docker::util::new_local_docker_connection;
use rust_i18n::t;
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

                let container_name_lower = container.names.to_lowercase();

                keywords.iter().any(|keyword| {
                    let keyword_lower = keyword.to_lowercase();

                    // 使用更精确的匹配规则，避免误匹配其他项目的容器
                    // Docker Compose 容器命名规则: {项目名}_{服务名}_{实例号} 或 {项目名}-{服务名}-{实例号}

                    // 检查是否是 docker-compose 标准格式的完整匹配
                    let separators = vec!["_", "-"];

                    for separator in separators {
                        // 格式1: 项目名_服务名_数字 (中间匹配)
                        let pattern1 = format!("{separator}{keyword_lower}{separator}");
                        if container_name_lower.contains(&pattern1) {
                            return true;
                        }

                        // 格式2: 项目名_服务名 (结尾匹配，没有实例号)
                        let pattern2 = format!("{separator}{keyword_lower}");
                        if container_name_lower.ends_with(&pattern2) {
                            return true;
                        }

                        // 格式3: 服务名_数字 (开头匹配，没有项目名)
                        let pattern3 = format!("{keyword_lower}{separator}");
                        if container_name_lower.starts_with(&pattern3) {
                            return true;
                        }
                    }

                    // 如果所有严格匹配都失败，只有在完全相同的情况下才匹配
                    container_name_lower == keyword_lower
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
                                "{}",
                                t!("docker_utils.containers_found", running = running_count, total = total_filtered)
                            );
                        }
                        ServiceFilter::NameContains(keywords) => {
                            info!(
                                "{}",
                                t!("docker_utils.containers_matched", keywords = format!("{:?}", keywords), running = running_count, total = total_filtered)
                            );
                        }
                    }

                    Ok(running_count > 0)
                }
                Err(e) => {
                    error!("{}", t!("docker_utils.get_containers_failed", error = e.to_string()));
                    Err(anyhow::anyhow!(t!("docker_utils.get_containers_failed", error = e.to_string()).to_string()))
                }
            }
        }
        Err(e) => {
            error!("{}", t!("docker_utils.docker_connect_failed", error = e.to_string()));
            Err(anyhow::anyhow!(t!("docker_utils.docker_connect_failed", error = e.to_string()).to_string()))
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
        "{}",
        t!("docker_utils.wait_stop_start", filter = format!("{:?}", filter), timeout = timeout_secs)
    );

    while start_time.elapsed() < timeout {
        match check_services_running(filter).await {
            Ok(false) => {
                info!("{}", t!("docker_utils.services_stopped"));
                return Ok(true);
            }
            Ok(true) => {
                info!("{}", t!("docker_utils.waiting_stop"));
                sleep(Duration::from_secs(timeout::SERVICE_CHECK_INTERVAL)).await;
            }
            Err(e) => {
                warn!("{}", t!("docker_utils.check_status_error", error = e.to_string()));
                sleep(Duration::from_secs(timeout::SERVICE_CHECK_INTERVAL)).await;
            }
        }
    }

    warn!("{}", t!("docker_utils.wait_stop_timeout", timeout = timeout_secs));
    Ok(false)
}

/// 等待指定的Docker服务完全启动
///
/// 注意：推荐使用 health_check.rs 中的 wait_for_services_ready 方法
pub async fn wait_for_services_started(filter: &ServiceFilter, timeout_secs: u64) -> Result<bool> {
    let start_time = tokio::time::Instant::now();
    let timeout = Duration::from_secs(timeout_secs);

    info!(
        "{}",
        t!("docker_utils.wait_start_start", filter = format!("{:?}", filter), timeout = timeout_secs)
    );

    while start_time.elapsed() < timeout {
        match check_services_running(filter).await {
            Ok(true) => {
                info!("{}", t!("docker_utils.services_started"));
                return Ok(true);
            }
            Ok(false) => {
                info!("{}", t!("docker_utils.waiting_start"));
                sleep(Duration::from_secs(timeout::SERVICE_CHECK_INTERVAL)).await;
            }
            Err(e) => {
                warn!("{}", t!("docker_utils.check_status_error", error = e.to_string()));
                sleep(Duration::from_secs(timeout::SERVICE_CHECK_INTERVAL)).await;
            }
        }
    }

    warn!("{}", t!("docker_utils.wait_start_timeout", timeout = timeout_secs));
    Ok(false)
}

/// 从docker-compose.yml文件中解析服务名称
pub async fn parse_service_names_from_compose(compose_file_path: &Path) -> Result<Vec<String>> {
    if !compose_file_path.exists() {
        warn!(
            "{}",
            t!("docker_utils.compose_not_exists", path = compose_file_path.display())
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
                    "{}",
                    t!("docker_utils.parse_services", path = compose_file_path.display(), count = service_names.len())
                );
                for name in &service_names {
                    info!("  - {}", name);
                }

                Ok(service_names)
            }
            Err(e) => {
                error!("{}", t!("docker_utils.parse_compose_failed", error = e.to_string()));
                Err(anyhow::anyhow!(t!("docker_utils.parse_compose_failed", error = e.to_string()).to_string()))
            }
        },
        Err(e) => {
            error!("{}", t!("docker_utils.read_compose_failed", error = e.to_string()));
            Err(anyhow::anyhow!(t!("docker_utils.read_compose_failed", error = e.to_string()).to_string()))
        }
    }
}

/// 基于docker-compose.yml创建简单的服务过滤器
pub async fn create_compose_filter(compose_file_path: &Path) -> Result<ServiceFilter> {
    let service_names = parse_service_names_from_compose(compose_file_path).await?;

    if service_names.is_empty() {
        warn!("{}", t!("docker_utils.no_services_found"));
        Ok(ServiceFilter::All)
    } else {
        Ok(ServiceFilter::NameContains(service_names))
    }
}

/// 便捷函数：等待compose服务停止
///
/// ⚠️ 注意：此函数使用简单的名称匹配，可能会误匹配其他项目的容器。
/// 推荐使用 HealthChecker 来获得更准确的状态判断。
///
/// 此函数保留用于向后兼容，但建议迁移到 HealthChecker。
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

/// 等待 MySQL 服务就绪
///
/// 使用服务名 "mysql" 过滤容器，等待 MySQL 容器启动完成。
/// 此函数用于解决 MySQL-Java 服务启动死锁问题：
/// - Java 容器健康检查依赖 MySQL 表结构升级
/// - SQL 升级需要 MySQL 先就绪
///
/// # 参数
/// - `_compose_path`: docker-compose 文件路径（保留以便未来扩展）
/// - `timeout_secs`: 超时时间（秒）
///
/// # 返回
/// - `Ok(true)`: MySQL 服务已启动
/// - `Ok(false)`: 超时，MySQL 服务未能启动
/// - `Err`: 检查过程中发生错误
pub async fn wait_for_mysql_ready(_compose_path: &Path, timeout_secs: u64) -> Result<bool> {
    let filter = ServiceFilter::NameContains(vec!["mysql".to_string()]);
    wait_for_services_started(&filter, timeout_secs).await
}
