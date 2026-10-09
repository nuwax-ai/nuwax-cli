use super::interpolation::{MissingVariables, interpolate_env, is_whole_reference};
use super::types::{DockerManager, ServiceConfig};
use crate::DuckError;
use crate::container::environment::detect_runtime_environment;
use anyhow::{Context, Result};
use docker_compose_types as dct;
use quick_cache::sync::Cache;
use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::Path;
use tracing::{debug, info, warn};

#[path = "compose_env.rs"]
pub(crate) mod compose_env;

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
    /// 创建新的 Docker 管理器（指定项目名称）
    pub fn with_project<P: AsRef<Path>>(
        compose_file: P,
        env_file: P,
        project_name: Option<String>,
    ) -> Result<Self> {
        let compose_file = compose_file.as_ref().to_path_buf();
        let env_file = env_file.as_ref().to_path_buf();

        let runtime_env = detect_runtime_environment();

        // 如果compose文件不存在，记录警告
        if !compose_file.exists() {
            warn!("docker-compose file does not exist");
            info!(
                "Compose configuration not loaded; this may be the first deployment and docker directory is missing"
            );
        }

        Ok(Self {
            compose_file,
            env_file,
            project_name,
            runtime_env,
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
                debug!("Loaded docker-compose config from cache");
                return Ok(cached.config.clone());
            } else {
                debug!("Cache expired, reloading config");
            }
        }

        // 缓存未命中或已过期，重新加载
        debug!("Reloading docker-compose config");
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

    /// 升级包替换同一路径的 Compose 文件后，显式清除旧解析结果。
    pub fn invalidate_compose_config_cache(&self) {
        let cache_key = (
            self.compose_file.display().to_string(),
            self.env_file.display().to_string(),
        );
        COMPOSE_CACHE.remove(&cache_key);
    }

    /// 检查服务是否是一次性任务（解析compose文件和名称模式判断）
    pub async fn is_oneshot_service(&self, service_name: &str) -> Result<bool> {
        // 使用已加载的compose_config，无需重新解析
        let services = &self.load_compose_config()?.services;

        if let Some(service_opt) = services.0.get(service_name) {
            if let Some(service) = service_opt
                && let Some(restart_policy) = &service.restart
            {
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

            Ok(false)
        } else {
            Err(anyhow::anyhow!("Service does not exist: {service_name}"))
        }
    }

    /// 解析docker-compose.yml文件中的服务配置
    pub async fn parse_service_config(&self, service_name: &str) -> Result<ServiceConfig> {
        // 使用已加载的compose_config，无需重新解析
        let services = &self.load_compose_config()?.services;

        let service = services
            .0
            .get(service_name)
            .ok_or_else(|| DuckError::Docker(format!("Service not found: {service_name}")))?;

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

    /// 返回指定服务及其全部 Compose 前置服务。
    pub fn get_service_dependency_closure(&self, service_name: &str) -> Result<HashSet<String>> {
        fn visit(
            name: &str,
            services: &dct::Services,
            visited: &mut HashSet<String>,
        ) -> Result<()> {
            if !visited.insert(name.to_string()) {
                return Ok(());
            }

            let service = services
                .0
                .get(name)
                .and_then(Option::as_ref)
                .ok_or_else(|| anyhow::anyhow!("Compose service not found: {name}"))?;
            let dependencies: Vec<&str> = match &service.depends_on {
                dct::DependsOnOptions::Simple(names) => names.iter().map(String::as_str).collect(),
                dct::DependsOnOptions::Conditional(conditions) => {
                    conditions.keys().map(String::as_str).collect()
                }
            };
            for dependency in dependencies {
                visit(dependency, services, visited)?;
            }
            Ok(())
        }

        let compose = self.load_compose_config()?;
        let mut visited = HashSet::new();
        visit(service_name, &compose.services, &mut visited)?;
        Ok(visited)
    }

    /// 把非数据库阶段的服务按 depends_on 分层，供 --no-deps 启动时保持依赖顺序。
    pub fn get_compose_startup_layers(
        &self,
        excluded: &HashSet<String>,
    ) -> Result<Vec<Vec<String>>> {
        let compose = self.load_compose_config()?;
        let services = &compose.services.0;
        let mut pending: HashSet<String> = services
            .keys()
            .filter(|name| !excluded.contains(*name))
            .cloned()
            .collect();
        let mut started = excluded.clone();
        let mut layers = Vec::new();

        while !pending.is_empty() {
            let mut layer = Vec::new();
            for name in &pending {
                let service = services
                    .get(name)
                    .and_then(Option::as_ref)
                    .ok_or_else(|| anyhow::anyhow!("Compose service not found: {name}"))?;
                let dependencies: Vec<&str> = match &service.depends_on {
                    dct::DependsOnOptions::Simple(names) => {
                        names.iter().map(String::as_str).collect()
                    }
                    dct::DependsOnOptions::Conditional(conditions) => {
                        conditions.keys().map(String::as_str).collect()
                    }
                };
                for dependency in &dependencies {
                    if !services.contains_key(*dependency) {
                        return Err(anyhow::anyhow!(
                            "Compose service {name} depends on missing service {dependency}"
                        ));
                    }
                }
                if dependencies
                    .iter()
                    .all(|dependency| started.contains(*dependency))
                {
                    layer.push(name.clone());
                }
            }
            if layer.is_empty() {
                return Err(anyhow::anyhow!(
                    "Compose services have a dependency cycle: {:?}",
                    pending
                ));
            }
            layer.sort();
            for name in &layer {
                pending.remove(name);
                started.insert(name.clone());
            }
            layers.push(layer);
        }
        Ok(layers)
    }

    /// 获取 docker-compose 项目名称
    pub fn get_compose_project_name(&self) -> String {
        // 优先使用指定的项目名称
        if let Some(ref project_name) = self.project_name {
            info!("Using provided project name: {}", project_name);
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
                info!("Read project name from compose file: {}", project_name);
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

/// Read an environment file without exporting it into this process. A missing file
/// keeps Compose's existing shell-only configuration support.
pub(crate) fn load_env_values(env_path: &Path) -> Result<HashMap<String, String>> {
    if !env_path.exists() {
        return Ok(HashMap::new());
    }
    let source = fs::read_to_string(env_path).with_context(|| {
        format!(
            "Failed to read Compose environment file: {}",
            env_path.display()
        )
    })?;
    compose_env::parse_env_values(&source, &|key| {
        if crate::constants::device_info::ENV_MANAGED_KEYS.contains(&key) {
            None
        } else {
            std::env::var(key).ok()
        }
    })
    .with_context(|| {
        format!(
            "Failed to parse Compose environment file: {}",
            env_path.display()
        )
    })
}

fn compose_env_value(
    key: &str,
    file_values: &HashMap<String, String>,
    host_value: impl FnOnce(&str) -> Option<String>,
) -> Option<String> {
    if crate::constants::device_info::ENV_MANAGED_KEYS.contains(&key) {
        file_values.get(key).cloned()
    } else {
        // Match native Compose's shell-before-env-file precedence.
        host_value(key).or_else(|| file_values.get(key).cloned())
    }
}

/// 使用 `docker-compose-types` crate 解析配置文件，并处理 .env 文件中的环境变量
pub fn load_compose_config_with_env(compose_path: &Path, env_path: &Path) -> Result<dct::Compose> {
    let env_values = load_env_values(env_path)?;

    // 2. 读取 docker-compose.yml 文件内容
    let content = fs::read_to_string(compose_path)
        .map_err(|e| DuckError::Docker(format!("Failed to read compose file: {e}")))?;

    // Parse syntax before inserting environment data. Otherwise a password's
    // " #", ": ", quotes or newlines can become YAML syntax and change its value.
    let mut document: serde_yaml::Value = serde_yaml::from_str(&content).map_err(|error| {
        let location = error
            .location()
            .map(|location| format!(" at line {}, column {}", location.line(), location.column()))
            .unwrap_or_default();
        DuckError::Docker(format!(
            "Invalid Compose YAML{}: {}",
            location,
            compose_path.display()
        ))
    })?;
    let context = |key: &str| compose_env_value(key, &env_values, |key| std::env::var(key).ok());
    interpolate_compose_value(&mut document, &mut Vec::new(), &context)?;
    // A deserialization error may contain the offending value (credentials).
    // Schema coercion errors below identify structural fields without values.
    // Feed safely serialized scalar data to the schema's YAML deserializer. Its
    // string fields also accept YAML scalar text (e.g. version: 3.8/expose: 3306),
    // while Value::into_deserializer would reject those existing configurations.
    let encoded = serde_yaml::to_string(&document)
        .context("Failed to encode interpolated Compose configuration")?;
    let compose_config: dct::Compose = serde_yaml::from_str(&encoded).map_err(|_| {
        DuckError::Docker(format!(
            "Invalid interpolated Compose schema: {}",
            compose_path.display()
        ))
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

fn interpolate_scalar(raw: &str, context: &impl Fn(&str) -> Option<String>) -> Result<String> {
    interpolate_env(raw, context, MissingVariables::Empty)
}

fn interpolate_compose_value(
    value: &mut serde_yaml::Value,
    path: &mut Vec<String>,
    context: &impl Fn(&str) -> Option<String>,
) -> Result<()> {
    use serde_yaml::Value;
    match value {
        Value::String(raw) => {
            let whole_reference = is_whole_reference(raw);
            let expanded = interpolate_scalar(raw, context)?;
            *value = if whole_reference {
                adapt_compose_scalar(&expanded, path)?
            } else {
                Value::String(expanded)
            };
        }
        Value::Sequence(sequence) => {
            for item in sequence {
                path.push("[]".to_string());
                interpolate_compose_value(item, path, context)?;
                path.pop();
            }
        }
        Value::Mapping(mapping) => {
            // Native Compose interpolates mapping values, never keys. Dynamic
            // environment/label names use the KEY=value sequence syntax.
            for (key, value) in mapping.iter_mut() {
                path.push(key.as_str().unwrap_or("<mapping-key>").to_string());
                interpolate_compose_value(value, path, context)?;
                path.pop();
            }
        }
        Value::Tagged(tagged) => interpolate_compose_value(&mut tagged.value, path, context)?,
        Value::Null | Value::Bool(_) | Value::Number(_) => {}
    }
    Ok(())
}

#[derive(Clone, Copy)]
enum ComposeScalar {
    Boolean,
    Signed,
    SignedOrString,
    Unsigned,
    Float,
    Port,
    PublishedPort,
    DeviceCount,
}

/// Primitive schema positions in docker-compose-types 0.24. Untagged enums
/// (notably Ports::Long) prevent a generic serde wrapper from reaching their
/// numeric members. Adapt only these structural fields, never arbitrary strings
/// in environment, labels, driver options, extensions, commands or build args.
fn compose_scalar_schema(path: &[String]) -> Option<ComposeScalar> {
    use ComposeScalar::*;
    let keys: Vec<&str> = path.iter().map(String::as_str).collect();
    match keys.as_slice() {
        ["volumes" | "secrets", _, "external"] => return Some(Boolean),
        [
            "networks",
            _,
            "attachable" | "enable_ipv6" | "internal" | "external",
        ] => return Some(Boolean),
        _ => {}
    }
    let service = match keys.as_slice() {
        ["services", _, tail @ ..] | ["service", tail @ ..] => tail,
        _ => return None,
    };
    match service {
        ["privileged" | "read_only" | "init" | "stdin_open" | "tty"] => Some(Boolean),
        ["scale"] => Some(Signed),
        ["mem_swappiness"] => Some(Port),
        ["ports", "[]", "target"] => Some(Port),
        ["ports", "[]", "published"] => Some(PublishedPort),
        ["deploy", "replicas"] => Some(Signed),
        ["healthcheck", "retries"] => Some(Signed),
        ["healthcheck", "disable"] => Some(Boolean),
        ["depends_on", _, "restart" | "required"] => Some(Boolean),
        ["deploy", "restart_policy", "max_attempts"] => Some(Signed),
        ["deploy", "update_config", "parallelism"] => Some(Signed),
        ["deploy", "update_config", "max_failure_ratio"] => Some(Float),
        [
            "deploy",
            "resources",
            "reservations" | "limits",
            "devices",
            "[]",
            "count",
        ] => Some(DeviceCount),
        ["build", "shm_size"] => Some(Unsigned),
        ["ulimits", _] | ["ulimits", _, "soft" | "hard"] => Some(SignedOrString),
        ["volumes", "[]", "read_only"] => Some(Boolean),
        ["volumes", "[]", "bind", "create_host_path"] => Some(Boolean),
        ["volumes", "[]", "volume", "nocopy"] => Some(Boolean),
        ["volumes", "[]", "tmpfs", "size"] => Some(Unsigned),
        _ => None,
    }
}

fn adapt_compose_scalar(expanded: &str, path: &[String]) -> Result<serde_yaml::Value> {
    use ComposeScalar::*;
    use serde_yaml::Value;
    let Some(schema) = compose_scalar_schema(path) else {
        return Ok(Value::String(expanded.to_string()));
    };
    if matches!(schema, DeviceCount) && expanded == "all" {
        return Ok(Value::String(expanded.to_string()));
    }
    let parsed: Option<Value> = serde_yaml::from_str(expanded).ok();
    let valid = match (schema, &parsed) {
        (Boolean, Some(Value::Bool(_))) => true,
        (Signed | SignedOrString, Some(Value::Number(number))) => number.as_i64().is_some(),
        (Float, Some(Value::Number(number))) => number.as_f64().is_some(),
        (Unsigned | DeviceCount, Some(Value::Number(number))) => number.as_u64().is_some(),
        (Port | PublishedPort, Some(Value::Number(number))) => number
            .as_u64()
            .is_some_and(|number| number <= u16::MAX as u64),
        // PublishedPort also accepts a range string, e.g. 3306-3310.
        (PublishedPort, _) => return Ok(Value::String(expanded.to_string())),
        (SignedOrString, _) => return Ok(Value::String(expanded.to_string())),
        _ => false,
    };
    anyhow::ensure!(
        valid,
        "Invalid numeric or boolean Compose field: {}",
        path.join(".")
    );
    parsed.context("Missing adapted Compose scalar")
}

#[cfg(test)]
mod staged_deployment_tests {
    use super::DockerManager;

    #[test]
    fn mysql_stage_includes_permission_fix_but_not_backend() -> anyhow::Result<()> {
        let fixtures = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("fixtures");
        let manager = DockerManager::with_project(
            fixtures.join("docker-compose.yml"),
            fixtures.join(".env"),
            Some("staged-deploy-check".to_string()),
        )?;
        let stage = manager.get_service_dependency_closure("mysql")?;
        assert!(stage.contains("mysql"));
        assert!(stage.contains("mysql-permission-fix"));
        assert!(!stage.contains("backend"));
        let layers = manager.get_compose_startup_layers(&stage)?;
        let layer_of = |service: &str| {
            layers
                .iter()
                .position(|layer| layer.iter().any(|name| name == service))
        };
        assert!(layer_of("mysql").is_none());
        assert!(layer_of("mysql-permission-fix").is_none());
        assert!(layer_of("redis") < layer_of("backend"));
        assert!(layer_of("milvus") < layer_of("backend"));
        assert!(layer_of("backend") < layer_of("frontend"));
        Ok(())
    }
}

#[cfg(test)]
mod isolated_compose_environment_tests {
    use super::*;

    #[test]
    fn typed_credentials_match_unset_and_empty_default_operators() -> Result<()> {
        let directory = tempfile::tempdir()?;
        let compose = directory.path().join("compose.yml");
        let env_path = directory.path().join("defaults.env");
        std::fs::write(&env_path, "NUWAX_TYPED_EMPTY=\n")?;
        for (expression, expected) in [
            ("${NUWAX_TYPED_UNSET-fallback}", "fallback"),
            ("${NUWAX_TYPED_EMPTY-fallback}", ""),
            ("${NUWAX_TYPED_UNSET:-fallback}", "fallback"),
            ("${NUWAX_TYPED_EMPTY:-fallback}", "fallback"),
        ] {
            std::fs::write(
                &compose,
                format!(
                    "services:\n  mysql:\n    image: mysql:8.0\n    environment:\n      - MYSQL_PASSWORD={expression}\n"
                ),
            )?;
            let config = load_compose_config_with_env(&compose, &env_path)?;
            let mysql = config.services.0["mysql"].as_ref().unwrap();
            let dct::Environment::List(values) = &mysql.environment else {
                anyhow::bail!("Expected list environment");
            };
            assert_eq!(values, &[format!("MYSQL_PASSWORD={expected}")]);
        }
        Ok(())
    }

    #[test]
    fn nested_whole_references_keep_numeric_schema_and_literal_string_data() -> Result<()> {
        let directory = tempfile::tempdir()?;
        let compose = directory.path().join("compose.yml");
        let env_path = directory.path().join("nested.env");
        std::fs::write(&env_path, "NUWAX_NESTED_EMPTY=\nNUWAX_NESTED_SET=yes\n")?;
        std::fs::write(
            &compose,
            concat!(
                "services:\n  mysql:\n    image: mysql:8.0\n",
                "    privileged: ${NUWAX_NESTED_SET:+${NUWAX_NESTED_UNSET:-true}}\n",
                "    deploy:\n      replicas: ${NUWAX_NESTED_EMPTY:-${NUWAX_NESTED_UNSET:-2}}\n",
                "    ports:\n      - target: ${NUWAX_NESTED_EMPTY:-${NUWAX_NESTED_UNSET:-3306}}\n",
                "        published: ${NUWAX_NESTED_UNSET-${NUWAX_NESTED_OTHER:-13306}}\n",
                "    environment:\n      TEXT: ${NUWAX_NESTED_UNSET:-$${literal}}\n",
                "      DEVICE_FIELDS_DISK_SERIAL: ${DEVICE_FIELDS_DISK_SERIAL}\n",
                "      FALLBACK: ${DEVICE_FIELDS_DISK_SERIAL-missing}\n",
            ),
        )?;
        let config = load_compose_config_with_env(&compose, &env_path)?;
        let mysql = config.services.0["mysql"].as_ref().unwrap();
        assert!(mysql.privileged);
        assert_eq!(mysql.deploy.as_ref().unwrap().replicas, Some(2));
        let dct::Ports::Long(ports) = &mysql.ports else {
            anyhow::bail!("Expected long ports");
        };
        assert_eq!(ports[0].target, 3306);
        assert_eq!(ports[0].published, Some(dct::PublishedPort::Single(13306)));
        let environment = serde_json::to_value(&mysql.environment)?;
        assert_eq!(environment["TEXT"], "${literal}");
        assert_eq!(environment["DEVICE_FIELDS_DISK_SERIAL"], "");
        assert_eq!(environment["FALLBACK"], "missing");
        Ok(())
    }

    #[test]
    fn credential_interpolation_keeps_yaml_punctuation_and_literal_dollars_as_data() -> Result<()> {
        let directory = tempfile::tempdir()?;
        let compose = directory.path().join("compose.yml");
        let env_path = directory.path().join("literal.env");
        let password = r#"synthetic # literal: "quoted" ${not_a_reference} C:\path\folder"#;
        std::fs::write(
            &env_path,
            format!("NUWAX_YAML_SAFE_PASSWORD='{password}'\n"),
        )?;
        std::fs::write(
            &compose,
            concat!(
                "services:\n  mysql:\n    image: mysql:8.0\n",
                "    environment:\n      - MYSQL_PASSWORD=${NUWAX_YAML_SAFE_PASSWORD}\n",
                "  backend:\n    image: alpine\n",
                "    environment:\n      PASSWORD: ${NUWAX_YAML_SAFE_PASSWORD}\n",
                "    labels:\n      password-text: ${NUWAX_YAML_SAFE_PASSWORD}\n"
            ),
        )?;
        let config = load_compose_config_with_env(&compose, &env_path)?;
        let mysql = config.services.0["mysql"].as_ref().unwrap();
        let dct::Environment::List(values) = &mysql.environment else {
            anyhow::bail!("Expected list environment");
        };
        assert_eq!(values, &[format!("MYSQL_PASSWORD={password}")]);
        let backend = config.services.0["backend"].as_ref().unwrap();
        let environment = serde_json::to_value(&backend.environment)?;
        assert_eq!(environment["PASSWORD"].as_str(), Some(password));
        let labels = serde_json::to_value(&backend.labels)?;
        assert_eq!(labels["password-text"].as_str(), Some(password));
        Ok(())
    }

    #[test]
    fn whole_references_use_schema_types_without_coercing_string_data() -> Result<()> {
        let directory = tempfile::tempdir()?;
        let compose = directory.path().join("compose.yml");
        let env_path = directory.path().join("schema.env");
        std::fs::write(
            &env_path,
            concat!(
                "NUWAX_SCHEMA_FLAG=true\nNUWAX_SCHEMA_REPLICAS=2\n",
                "NUWAX_SCHEMA_TARGET=3306\nNUWAX_SCHEMA_PUBLISHED=13306\n",
                "NUWAX_SCHEMA_RANGE=23306-23308\n"
            ),
        )?;
        std::fs::write(
            &compose,
            concat!(
                "services:\n  mysql:\n    image: ${NUWAX_SCHEMA_REPLICAS}\n",
                "    privileged: ${NUWAX_SCHEMA_FLAG}\n    read_only: ${NUWAX_SCHEMA_FLAG}\n",
                "    deploy:\n      replicas: ${NUWAX_SCHEMA_REPLICAS}\n",
                "    ports:\n      - target: ${NUWAX_SCHEMA_TARGET}\n        published: ${NUWAX_SCHEMA_PUBLISHED}\n",
                "      - target: ${NUWAX_SCHEMA_TARGET}\n        published: ${NUWAX_SCHEMA_RANGE}\n",
                "    environment:\n      FLAG: ${NUWAX_SCHEMA_FLAG}\n      COUNT: ${NUWAX_SCHEMA_REPLICAS}\n",
                "    labels:\n      privileged: ${NUWAX_SCHEMA_FLAG}\n      replicas: ${NUWAX_SCHEMA_REPLICAS}\n",
                "    command: ['${NUWAX_SCHEMA_FLAG}', '${NUWAX_SCHEMA_REPLICAS}']\n"
            ),
        )?;
        let config = load_compose_config_with_env(&compose, &env_path)?;
        let mysql = config.services.0["mysql"].as_ref().unwrap();
        assert_eq!(mysql.image.as_deref(), Some("2"));
        assert!(mysql.privileged && mysql.read_only);
        assert_eq!(mysql.deploy.as_ref().unwrap().replicas, Some(2));
        let dct::Ports::Long(ports) = &mysql.ports else {
            anyhow::bail!("Expected long ports");
        };
        assert_eq!(ports[0].target, 3306);
        assert_eq!(ports[0].published, Some(dct::PublishedPort::Single(13306)));
        assert_eq!(
            ports[1].published,
            Some(dct::PublishedPort::Range("23306-23308".to_string()))
        );
        let environment = serde_json::to_value(&mysql.environment)?;
        let labels = serde_json::to_value(&mysql.labels)?;
        assert_eq!(environment["FLAG"].as_str(), Some("true"));
        assert_eq!(environment["COUNT"].as_str(), Some("2"));
        assert_eq!(labels["privileged"].as_str(), Some("true"));
        assert_eq!(labels["replicas"].as_str(), Some("2"));
        assert_eq!(
            serde_json::to_value(&mysql.command)?,
            serde_json::json!(["true", "2"])
        );
        Ok(())
    }

    #[test]
    fn existing_yaml_scalar_string_fields_still_deserialize() -> Result<()> {
        let directory = tempfile::tempdir()?;
        let compose = directory.path().join("compose.yml");
        std::fs::write(
            &compose,
            "version: 3.8\nservices:\n  backend:\n    image: 123\n    expose: [3306]\n",
        )?;
        let config = load_compose_config_with_env(&compose, &directory.path().join("missing.env"))?;
        assert_eq!(config.version.as_deref(), Some("3.8"));
        let backend = config.services.0["backend"].as_ref().unwrap();
        assert_eq!(backend.image.as_deref(), Some("123"));
        assert_eq!(backend.expose, ["3306"]);
        Ok(())
    }

    #[test]
    fn invalid_schema_reference_does_not_disclose_its_value() -> Result<()> {
        let directory = tempfile::tempdir()?;
        let compose = directory.path().join("compose.yml");
        let env_path = directory.path().join("invalid.env");
        let private_value = "synthetic-private-value";
        std::fs::write(&env_path, format!("NUWAX_BAD_PORT={private_value}\n"))?;
        std::fs::write(
            &compose,
            "services:\n  mysql:\n    image: mysql:8.0\n    ports:\n      - target: ${NUWAX_BAD_PORT}\n",
        )?;
        let error = load_compose_config_with_env(&compose, &env_path).unwrap_err();
        let message = format!("{error:#}");
        assert!(message.contains("ports"));
        assert!(!message.contains(private_value));
        Ok(())
    }

    #[test]
    fn mapping_keys_remain_literal_while_values_and_sequence_keys_interpolate() -> Result<()> {
        let directory = tempfile::tempdir()?;
        let compose = directory.path().join("compose.yml");
        let env_path = directory.path().join("keys.env");
        std::fs::write(
            &env_path,
            "NUWAX_LABEL_ONE=duplicate\nNUWAX_LABEL_TWO=duplicate\n",
        )?;
        std::fs::write(
            &compose,
            concat!(
                "services:\n  backend:\n    image: alpine\n    labels:\n",
                "      '${NUWAX_LABEL_ONE}': '${NUWAX_LABEL_TWO}'\n",
                "      '${NUWAX_LABEL_TWO}': second\n",
                "    environment:\n      '${NUWAX_LABEL_ONE}': '${NUWAX_LABEL_TWO}'\n",
                "  sequence:\n    image: alpine\n",
                "    environment: ['${NUWAX_LABEL_ONE}=${NUWAX_LABEL_TWO}']\n",
                "    labels: ['${NUWAX_LABEL_ONE}=${NUWAX_LABEL_TWO}']\n"
            ),
        )?;
        let config = load_compose_config_with_env(&compose, &env_path)?;
        let backend = config.services.0["backend"].as_ref().unwrap();
        let labels = serde_json::to_value(&backend.labels)?;
        assert_eq!(labels["${NUWAX_LABEL_ONE}"], "duplicate");
        assert_eq!(labels["${NUWAX_LABEL_TWO}"], "second");
        let environment = serde_json::to_value(&backend.environment)?;
        assert_eq!(environment["${NUWAX_LABEL_ONE}"], "duplicate");
        let sequence = config.services.0["sequence"].as_ref().unwrap();
        assert_eq!(
            serde_json::to_value(&sequence.environment)?,
            serde_json::json!(["duplicate=duplicate"])
        );
        assert_eq!(
            serde_json::to_value(&sequence.labels)?,
            serde_json::json!(["duplicate=duplicate"])
        );
        Ok(())
    }

    #[test]
    fn windows_utf8_bom_is_accepted_without_exporting_values() -> Result<()> {
        let directory = tempfile::tempdir()?;
        let path = directory.path().join("windows.env");
        std::fs::write(
            &path,
            "\u{feff}DEVICE_ID=v1:bom\r\nDEVICE_INFO_OS=windows\r\n",
        )?;
        let values = load_env_values(&path)?;
        assert_eq!(values["DEVICE_ID"], "v1:bom");
        assert_eq!(values["DEVICE_INFO_OS"], "windows");
        Ok(())
    }

    #[test]
    fn native_host_precedence_is_preserved_except_for_managed_device_values() {
        let values = HashMap::from([
            ("MYSQL_PASSWORD".to_string(), "from-file".to_string()),
            ("DEVICE_ID".to_string(), "v1:from-file".to_string()),
        ]);
        assert_eq!(
            compose_env_value("MYSQL_PASSWORD", &values, |_| Some("from-host".to_string())),
            Some("from-host".to_string())
        );
        assert_eq!(
            compose_env_value("DEVICE_ID", &values, |_| Some("v1:old-host".to_string())),
            Some("v1:from-file".to_string())
        );
        assert_eq!(
            compose_env_value("DEVICE_FIELDS_DISK_SERIAL", &values, |_| Some(
                "old-disk".to_string()
            )),
            None
        );
        assert_eq!(
            compose_env_value("MYSQL_PASSWORD", &values, |_| None),
            Some("from-file".to_string())
        );
    }

    #[test]
    fn file_interpolation_is_local_and_custom_files_do_not_cross_contaminate() -> Result<()> {
        let directory = tempfile::tempdir()?;
        let key = format!(
            "NUWAX_COMPOSE_{}",
            directory
                .path()
                .file_name()
                .unwrap()
                .to_string_lossy()
                .replace(['-', '.'], "_")
        );
        assert!(std::env::var_os(&key).is_none());
        let compose = directory.path().join("compose.yml");
        std::fs::write(
            &compose,
            format!(
                "services:\n  backend:\n    image: alpine:${{{key}}}\n    environment:\n      - DEVICE_FIELDS_DISK_SERIAL=${{DEVICE_FIELDS_DISK_SERIAL}}\n"
            ),
        )?;
        let first = directory.path().join("one.env");
        let second = directory.path().join("two.env");
        std::fs::write(&first, format!("{key}=first\n"))?;
        std::fs::write(&second, format!("{key}=second\n"))?;
        let one = load_compose_config_with_env(&compose, &first)?;
        let two = load_compose_config_with_env(&compose, &second)?;
        assert_eq!(
            one.services.0["backend"].as_ref().unwrap().image.as_deref(),
            Some("alpine:first")
        );
        assert_eq!(
            two.services.0["backend"].as_ref().unwrap().image.as_deref(),
            Some("alpine:second")
        );
        assert!(std::env::var_os(&key).is_none());
        Ok(())
    }

    #[test]
    fn invalidation_reloads_a_warm_cache_after_file_repair() -> Result<()> {
        let directory = tempfile::tempdir()?;
        let compose = directory.path().join("compose.yml");
        let env_path = directory.path().join("custom.env");
        std::fs::write(
            &compose,
            "services:\n  backend:\n    image: alpine:${DEVICE_INFO_FINGERPRINT_VERSION}\n",
        )?;
        std::fs::write(&env_path, "DEVICE_INFO_FINGERPRINT_VERSION=1\n")?;
        let manager = DockerManager::with_project(compose, env_path.clone(), None)?;
        assert_eq!(
            manager.load_compose_config()?.services.0["backend"]
                .as_ref()
                .unwrap()
                .image
                .as_deref(),
            Some("alpine:1")
        );
        std::fs::write(&env_path, "DEVICE_INFO_FINGERPRINT_VERSION=2\n")?;
        manager.invalidate_compose_config_cache();
        assert_eq!(
            manager.load_compose_config()?.services.0["backend"]
                .as_ref()
                .unwrap()
                .image
                .as_deref(),
            Some("alpine:2")
        );
        Ok(())
    }

    #[test]
    fn malformed_environment_is_reported_instead_of_partially_exported() -> Result<()> {
        let directory = tempfile::tempdir()?;
        let path = directory.path().join("invalid.env");
        std::fs::write(&path, "DEVICE_ID=valid\nBAD=\"unfinished\n")?;
        assert!(load_env_values(&path).is_err());
        Ok(())
    }
}
