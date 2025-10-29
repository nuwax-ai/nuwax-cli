use anyhow::Result;
use bollard::Docker;
use bollard::models::{ContainerCreateBody, NetworkCreateRequest};
use bollard::query_parameters::{
    CreateImageOptionsBuilder, CreateContainerOptions, StartContainerOptions,
    ListContainersOptions, StopContainerOptions, RemoveContainerOptions,
};
use serde_yaml::Value as YamlValue;
use std::collections::HashMap;
use std::path::Path;
use tracing::{debug, error, info};

/// 使用 Bollard 库的现代化 Docker 管理器
/// TODO 待测试验证,尚未使用
pub struct ModernDockerManager {
    docker: Docker,
    compose_file: std::path::PathBuf,
    project_name: String,
}

impl ModernDockerManager {
    /// 创建新的现代化 Docker 管理器
    pub async fn new(compose_file: impl AsRef<Path>) -> Result<Self> {
        // 连接到 Docker daemon
        let docker = Docker::connect_with_local_defaults()
            .map_err(|e| anyhow::anyhow!("连接 Docker 失败: {}", e))?;

        let compose_file = compose_file.as_ref().to_path_buf();
        let project_name = compose_file
            .parent()
            .and_then(|p| p.file_name())
            .and_then(|n| n.to_str())
            .unwrap_or("default")
            .to_string();

        Ok(Self {
            docker,
            compose_file,
            project_name,
        })
    }

    /// 启动 Compose 项目中的所有服务
    pub async fn start_compose_services(&self) -> Result<()> {
        info!("🚀 使用 Bollard API 启动 Compose 服务...");

        // 1. 解析 docker-compose.yml
        let compose_config = self.parse_compose_file().await?;

        // 2. 创建网络（如果定义了的话）
        self.create_networks(&compose_config).await?;

        // 3. 拉取所需的镜像
        self.pull_images(&compose_config).await?;

        // 4. 创建并启动容器
        self.create_and_start_containers(&compose_config).await?;

        info!("✅ 所有 Compose 服务启动完成");
        Ok(())
    }

    /// 解析 docker-compose.yml 文件
    async fn parse_compose_file(&self) -> Result<YamlValue> {
        let content = tokio::fs::read_to_string(&self.compose_file)
            .await
            .map_err(|e| anyhow::anyhow!("读取 compose 文件失败: {}", e))?;

        let compose: YamlValue = serde_yaml::from_str(&content)
            .map_err(|e| anyhow::anyhow!("解析 compose 文件失败: {}", e))?;

        Ok(compose)
    }

    /// 创建网络
    async fn create_networks(&self, compose_config: &YamlValue) -> Result<()> {
        if let Some(networks) = compose_config.get("networks").and_then(|n| n.as_mapping()) {
            for (network_name, _config) in networks {
                let network_name = network_name.as_str().unwrap();
                let full_name = format!("{}_{}", self.project_name, network_name);

                info!("📡 创建网络: {}", full_name);

                let options = NetworkCreateRequest {
                    name: full_name.clone(),
                    driver: Some("bridge".to_string()),
                    ..Default::default()
                };

                match self.docker.create_network(options).await {
                    Ok(_) => info!("✅ 网络 {} 创建成功", full_name),
                    Err(e) => {
                        // 网络已存在不算错误
                        if e.to_string().contains("already exists") {
                            debug!("网络 {} 已存在", full_name);
                        } else {
                            return Err(anyhow::anyhow!("创建网络失败: {}", e));
                        }
                    }
                }
            }
        }
        Ok(())
    }

    /// 拉取镜像
    async fn pull_images(&self, compose_config: &YamlValue) -> Result<()> {
        if let Some(services) = compose_config.get("services").and_then(|s| s.as_mapping()) {
            for (service_name, service_config) in services {
                if let Some(image) = service_config.get("image").and_then(|i| i.as_str()) {
                    info!(
                        "📥 拉取镜像: {} (服务: {})",
                        image,
                        service_name.as_str().unwrap()
                    );

                    let create_options = CreateImageOptionsBuilder::default()
                        .from_image(image)
                        .build();

                    let mut stream = self.docker.create_image(Some(create_options), None, None);

                    use futures_util::stream::StreamExt;
                    while let Some(result) = stream.next().await {
                        match result {
                            Ok(info) => {
                                if let Some(status) = info.status {
                                    debug!("镜像下载: {}", status);
                                }
                            }
                            Err(e) => {
                                error!("拉取镜像失败: {}", e);
                                return Err(anyhow::anyhow!("拉取镜像失败: {}", e));
                            }
                        }
                    }
                }
            }
        }
        Ok(())
    }

    /// 创建并启动容器
    async fn create_and_start_containers(&self, compose_config: &YamlValue) -> Result<()> {
        if let Some(services) = compose_config.get("services").and_then(|s| s.as_mapping()) {
            for (service_name, service_config) in services {
                let service_name = service_name.as_str().unwrap();
                let container_name = format!("{}_{}_1", self.project_name, service_name);

                info!("🐳 创建容器: {}", container_name);

                // 构建容器配置
                let mut config = ContainerCreateBody::default();

                // 设置镜像
                if let Some(image) = service_config.get("image").and_then(|i| i.as_str()) {
                    config.image = Some(image.to_string());
                }

                // 设置环境变量
                if let Some(env) = service_config.get("environment") {
                    config.env = Some(self.parse_environment(env)?);
                }

                // 设置端口映射
                if let Some(ports) = service_config.get("ports") {
                    config.exposed_ports = Some(self.parse_exposed_ports(ports)?);
                }

                // 设置命令
                if let Some(command) = service_config.get("command") {
                    config.cmd = Some(self.parse_command(command)?);
                }

                // 创建容器
                let options = CreateContainerOptions {
                    name: Some(container_name.clone()),
                    platform: String::new(),
                };

                let create_result = self
                    .docker
                    .create_container(Some(options), config)
                    .await
                    .map_err(|e| anyhow::anyhow!("创建容器失败: {}", e))?;

                info!(
                    "✅ 容器 {} 创建成功, ID: {}",
                    container_name, create_result.id
                );

                // 启动容器
                info!("▶️ 启动容器: {}", container_name);
                self.docker
                    .start_container(&create_result.id, None::<StartContainerOptions>)
                    .await
                    .map_err(|e| anyhow::anyhow!("启动容器失败: {}", e))?;

                info!("✅ 容器 {} 启动成功", container_name);
            }
        }
        Ok(())
    }

    /// 解析环境变量
    fn parse_environment(&self, env: &YamlValue) -> Result<Vec<String>> {
        let mut result = Vec::new();

        match env {
            YamlValue::Sequence(seq) => {
                for item in seq {
                    if let Some(env_var) = item.as_str() {
                        result.push(env_var.to_string());
                    }
                }
            }
            YamlValue::Mapping(map) => {
                for (key, value) in map {
                    if let (Some(k), Some(v)) = (key.as_str(), value.as_str()) {
                        result.push(format!("{}={}", k, v));
                    }
                }
            }
            _ => {}
        }

        Ok(result)
    }

    /// 解析暴露端口
    fn parse_exposed_ports(&self, ports: &YamlValue) -> Result<HashMap<String, HashMap<(), ()>>> {
        let mut result = HashMap::new();

        if let YamlValue::Sequence(seq) = ports {
            for port in seq {
                if let Some(port_str) = port.as_str() {
                    // 解析 "8080:80" 或 "80" 格式
                    let container_port = if port_str.contains(':') {
                        port_str.split(':').nth(1).unwrap_or(port_str)
                    } else {
                        port_str
                    };

                    let port_key = if container_port.contains('/') {
                        container_port.to_string()
                    } else {
                        format!("{}/tcp", container_port)
                    };

                    result.insert(port_key, HashMap::new());
                }
            }
        }

        Ok(result)
    }

    /// 解析命令
    fn parse_command(&self, command: &YamlValue) -> Result<Vec<String>> {
        match command {
            YamlValue::String(cmd) => {
                // 简单的命令分割，实际使用中可能需要更复杂的解析
                Ok(cmd.split_whitespace().map(String::from).collect())
            }
            YamlValue::Sequence(seq) => Ok(seq
                .iter()
                .filter_map(|v| v.as_str())
                .map(String::from)
                .collect()),
            _ => Ok(vec![]),
        }
    }

    /// 停止所有服务
    pub async fn stop_compose_services(&self) -> Result<()> {
        info!("🛑 停止所有 Compose 服务...");

        // 获取项目相关的所有容器
        let containers = self
            .docker
            .list_containers(None::<ListContainersOptions>)
            .await
            .map_err(|e| anyhow::anyhow!("获取容器列表失败: {}", e))?;

        for container in containers {
            if let Some(names) = container.names {
                for name in names {
                    if name.contains(&self.project_name) {
                        info!("🛑 停止容器: {}", name);
                        if let Some(id) = &container.id {
                            let _ = self.docker.stop_container(id, None::<StopContainerOptions>).await;
                            let _ = self.docker.remove_container(id, None::<RemoveContainerOptions>).await;
                        }
                    }
                }
            }
        }

        info!("✅ 所有 Compose 服务已停止");
        Ok(())
    }

    /// 获取服务状态
    pub async fn get_compose_services_status(
        &self,
    ) -> Result<Vec<crate::container::types::ServiceInfo>> {
        let containers = self
            .docker
            .list_containers(None::<ListContainersOptions>)
            .await
            .map_err(|e| anyhow::anyhow!("获取容器列表失败: {}", e))?;

        let mut services = Vec::new();

        for container in containers {
            if let Some(names) = container.names {
                for name in names {
                    if name.contains(&self.project_name) {
                        let status = if let Some(state) = &container.state {
                            if state.to_string().to_lowercase() == "running" {
                                crate::container::types::ServiceStatus::Running
                            } else {
                                crate::container::types::ServiceStatus::Stopped
                            }
                        } else {
                            crate::container::types::ServiceStatus::Stopped
                        };

                        let image = container.image.clone().unwrap_or_default();
                        let ports = container
                            .ports
                            .clone()
                            .unwrap_or_default()
                            .into_iter()
                            .map(|p| {
                                format!("{}:{}", p.private_port, p.public_port.unwrap_or_default())
                            })
                            .collect();

                        services.push(crate::container::types::ServiceInfo {
                            name: name.trim_start_matches('/').to_string(),
                            status,
                            image: image,
                            ports: ports,
                        });
                    }
                }
            }
        }

        Ok(services)
    }
}
