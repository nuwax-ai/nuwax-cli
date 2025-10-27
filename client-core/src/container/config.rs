use super::types::{DockerManager, ServiceConfig};
use crate::DuckError;
use anyhow::Result;
use docker_compose_types as dct;
use quick_cache::sync::Cache;
use std::collections::HashSet;
use std::fs;
use std::path::Path;
use std::sync::Arc;
use std::time::Duration;
use tracing::{debug, error, info};

// 缓存条目的结构
#[derive(Debug, Clone)]
struct CacheEntry {
    config: dct::Compose,
    timestamp: u64,
}

// 全局缓存实例，用于缓存docker-compose配置
// 缓存键：(compose文件路径, env文件路径)
// 缓存值：带时间戳的配置数据
static COMPOSE_CACHE: once_cell::sync::Lazy<Cache<(String, String), CacheEntry>> =
    once_cell::sync::Lazy::new(|| {
        Cache::new(100) // 最多缓存100个不同的配置组合
    });

impl DockerManager {
    /// 创建新的 Docker 管理器
    pub fn new<P: AsRef<Path>>(compose_file: P, env_file: P) -> Result<Self> {
        Self::with_project(compose_file, env_file, None)
    }

    /// 创建新的 Docker 管理器（指定项目名称）
    pub fn with_project<P: AsRef<Path>>(compose_file: P, env_file: P, project_name: Option<String>) -> Result<Self> {
        let compose_file = compose_file.as_ref().to_path_buf();
        let env_file = env_file.as_ref().to_path_buf();

        // 如果compose文件不存在，使用默认配置；否则正常加载配置
        let compose_config = if compose_file.exists() {
            Some(load_compose_config_with_env(&compose_file, &env_file)?)
        } else {
            error!("compose文件不存在");
            None
        };
        if compose_config.is_none() {
            info!("未加载到compose配置,可能是第一次部署,docker目录不存在");
        }

        Ok(Self {
            compose_file,
            env_file,
            compose_config,
            project_name,
        })
    }

    /// 检查 Docker Compose 文件是否存在
    pub fn compose_file_exists(&self) -> bool {
        self.compose_file.exists()
    }

    /// 获取 Docker Compose 文件路径
    pub fn get_compose_file(&self) -> &Path {
        &self.compose_file
    }

    /// 获取 Docker Compose 环境文件路径
    pub fn get_env_file(&self) -> &Path {
        &self.env_file
    }

    /// 获取 Docker Compose 工作目录
    pub fn get_working_directory(&self) -> Option<&Path> {
        self.env_file.parent()
    }
    /// 使用实例中配置的路径加载 docker-compose.yml 文件并解析
    /// 结果会缓存30秒，避免重复解析
    pub fn load_compose_config(&self) -> Result<dct::Compose> {
        use std::time::{SystemTime, UNIX_EPOCH};

        let cache_key = (
            self.compose_file.display().to_string(),
            self.env_file.display().to_string(),
        );

        // 检查缓存
        if let Some(cached) = COMPOSE_CACHE.get(&cache_key) {
            // 检查是否过期（30秒TTL）
            let now = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_secs();

            if now - cached.timestamp < 30 {
                debug!("从缓存中获取docker-compose配置");
                return Ok(cached.config.clone());
            } else {
                debug!("缓存已过期，重新加载配置");
            }
        }

        // 缓存未命中或已过期，重新加载
        debug!("重新加载docker-compose配置");
        let compose_config = load_compose_config_with_env(&self.compose_file, &self.env_file)?;

        // 更新缓存
        let timestamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs();
        COMPOSE_CACHE.insert(
            cache_key,
            CacheEntry {
                config: compose_config.clone(),
                timestamp,
            },
        );

        Ok(compose_config)
    }

    /// 检查服务是否是一次性任务（解析compose文件和名称模式判断）
    pub async fn is_oneshot_service(&self, service_name: &str) -> Result<bool> {
        // 使用已加载的compose_config，无需重新解析
        let services = &self.load_compose_config()?.services;

        if let Some(service_opt) = services.0.get(service_name) {
            if let Some(service) = service_opt {
                if let Some(restart_policy) = &service.restart {
                    let policy = restart_policy.to_string();
                    // restart: "no" 表示不自动重启，通常是一次性任务
                    if policy == "no" || policy == "false" {
                        return Ok(true);
                    }
                    // restart: "always" 或 "unless-stopped" 表示应该一直运行
                    if policy == "always" || policy == "unless-stopped" || policy == "on-failure" {
                        return Ok(false);
                    }
                }
            }

            Ok(false)
        } else {
            Err(anyhow::anyhow!("服务: {service_name} 不存在"))
        }
    }

    /// 解析docker-compose.yml文件中的服务配置
    pub async fn parse_service_config(&self, service_name: &str) -> Result<ServiceConfig> {
        // 使用已加载的compose_config，无需重新解析
        let services = &self.load_compose_config()?.services;

        let service = services
            .0
            .get(service_name)
            .ok_or_else(|| DuckError::Docker(format!("找不到服务: {service_name}")))?;

        let restart = service.as_ref().and_then(|s| s.restart.clone());

        Ok(ServiceConfig { restart })
    }

    /// 获取 docker-compose.yml 中定义的所有服务名称
    pub async fn get_compose_service_names(&self) -> Result<HashSet<String>> {
        // 使用已加载的compose_config，无需重新解析
        let services = &self.load_compose_config()?.services;
        let mut service_names = HashSet::new();

        for (service_name, _) in services.0.iter() {
            service_names.insert(service_name.to_string());
        }

        Ok(service_names)
    }

    /// 获取 docker-compose 项目名称
    pub fn get_compose_project_name(&self) -> String {
        // 优先使用指定的项目名称
        if let Some(ref project_name) = self.project_name {
            info!("使用指定的项目名称: {}", project_name);
            // 设置环境变量，确保docker-compose使用相同的项目名称
            unsafe {
                std::env::set_var("COMPOSE_PROJECT_NAME", project_name);
            }
            return project_name.clone();
        }

        // 尝试从compose配置中读取name字段
        if let Ok(compose_config) = self.load_compose_config() {
            // 尝试访问name字段，基于docker-compose-types v0.19的结构
            // 注意：这里假设name字段是Option<String>类型
            if let Some(project_name) = compose_config.name {
                info!("从compose文件读取到项目名称: {}", project_name);
                // 设置环境变量，确保docker-compose使用相同的项目名称
                unsafe {
                    std::env::set_var("COMPOSE_PROJECT_NAME", &project_name);
                }
                return project_name;
            }
        }

        // 默认项目名称
        let default_name = "docker".to_string();
        // 设置环境变量，确保docker-compose使用相同的项目名称
        unsafe {
            std::env::set_var("COMPOSE_PROJECT_NAME", &default_name);
        }
        default_name
    }

    /// 生成 docker-compose 容器名称模式
    /// Docker Compose 生成的容器名称格式：{项目名}_{服务名}_{实例号}
    pub fn generate_compose_container_patterns(&self, service_name: &str) -> Vec<String> {
        let project_name = self.get_compose_project_name();

        vec![
            // 标准格式：项目名_服务名_实例号
            format!("{project_name}_{service_name}_1"),
            format!("{project_name}-{service_name}-1"),
            // 无实例号格式
            format!("{project_name}_{service_name}"),
            format!("{project_name}-{service_name}"),
            // 直接服务名匹配
            service_name.to_string(),
        ]
    }
}

/// 使用 `docker-compose-types` crate 解析配置文件，并处理 .env 文件中的环境变量
pub fn load_compose_config_with_env(compose_path: &Path, env_path: &Path) -> Result<dct::Compose> {
    // 1. 加载 .env 文件
    dotenvy::from_path_override(env_path).ok(); // .ok() 忽略错误，如果文件不存在或无法解析

    // 2. 读取 docker-compose.yml 文件内容
    let content = fs::read_to_string(compose_path)
        .map_err(|e| DuckError::Docker(format!("Failed to read compose file: {e}")))?;

    // 3. 替换环境变量
    // 创建一个闭包，用于从当前环境中查找变量
    // 创建一个闭包，用于从当前环境中查找变量。它必须返回 Result<Option<String>, E>。
    // 在这里，我们使用 `Ok` 包装 `Option`，并且错误类型是 `Infallible`，因为 `std::env::var(s).ok()` 不会失败。
    let context = |s: &str| Ok(std::env::var(s).ok());
    let expanded_content = shellexpand::env_with_context(&content, context).map_err(
        |e: shellexpand::LookupError<std::convert::Infallible>| {
            DuckError::Docker(format!("Failed to expand env vars: {e}"))
        },
    )?;

    // 4. 解析 YAML
    let compose_config: dct::Compose = serde_yaml::from_str(&expanded_content).map_err(|e| {
        DuckError::Docker(format!("Failed to parse compose file with serde_yaml: {e}"))
    })?;

    debug!("Successfully parsed docker-compose.yml!");
    let services = &compose_config.services;
    info!("Found {} services:", services.0.len());
    for (name, service_opt) in services.0.iter() {
        let image = service_opt
            .as_ref()
            .and_then(|s| s.image.as_deref())
            .unwrap_or("N/A");
        info!("  - Service: {}, Image: {}", name, image);
    }

    Ok(compose_config)
}
