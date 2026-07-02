use anyhow::Result;
use bollard::Docker;
use bollard::query_parameters::{InspectContainerOptions, ListContainersOptions};
use serde::{Deserialize, Serialize};
use tracing::warn;

use super::types::DockerManager;

/// Docker Compose 容器标签信息（从容器 labels 中提取）
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ComposeLabels {
    pub project: Option<String>,
    pub service: Option<String>,
    pub container_number: Option<String>,
    pub oneoff: Option<bool>,
    pub config_files: Option<String>,
    pub working_dir: Option<String>,
}

/// 与 CLI 解耦的容器健康状态
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ContainerHealthStatus {
    Healthy,
    Unhealthy,
    Starting,
    Unknown,
}

impl DockerManager {
    /// 从 Docker API 获取容器的 docker-compose labels（不要求容器一定在 compose 中）
    pub async fn get_container_compose_labels(&self, container_name: &str) -> Result<Option<ComposeLabels>> {
        match Docker::connect_with_socket_defaults() {
            Ok(docker) => {
                let containers = docker
                    .list_containers(Some(ListContainersOptions {
                        all: true,
                        ..Default::default()
                    }))
                    .await?;

                for container in containers {
                    if let Some(names) = &container.names {
                        let container_matches = names.iter().any(|name| {
                            let clean_name = name.strip_prefix('/').unwrap_or(name);
                            clean_name == container_name
                        });

                        if container_matches {
                            if let Some(labels) = &container.labels {
                                return Ok(Some(ComposeLabels {
                                    project: labels.get("com.docker.compose.project").cloned(),
                                    service: labels.get("com.docker.compose.service").cloned(),
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
                                }));
                            }

                            return Ok(None);
                        }
                    }
                }

                Ok(None)
            }
            Err(e) => {
                warn!(
                    "Cannot connect to Docker to get compose labels: {error}",
                    error = e.to_string()
                );
                Ok(None)
            }
        }
    }

    /// 通过 Docker inspect 获取容器健康状态（若容器未配置 healthcheck，则返回 None）
    pub async fn get_container_health_status(
        &self,
        container_name: &str,
    ) -> Result<Option<ContainerHealthStatus>> {
        match Docker::connect_with_socket_defaults() {
            Ok(docker) => {
                let container_info = docker
                    .inspect_container(container_name, None::<InspectContainerOptions>)
                    .await?;

                let status = container_info
                    .state
                    .and_then(|state| state.health.and_then(|health| health.status));

                Ok(status.map(|s| match s.to_string().as_str() {
                    "healthy" => ContainerHealthStatus::Healthy,
                    "unhealthy" => ContainerHealthStatus::Unhealthy,
                    "starting" => ContainerHealthStatus::Starting,
                    _ => ContainerHealthStatus::Unknown,
                }))
            }
            Err(e) => {
                warn!(
                    "Cannot connect to Docker for health check: {error}",
                    error = e.to_string()
                );
                Ok(None)
            }
        }
    }
}

