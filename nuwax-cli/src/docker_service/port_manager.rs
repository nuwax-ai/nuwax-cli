use super::error::{DockerServiceError, DockerServiceResult};
use client_core::constants::docker::DOCKER_SOCKET_PATH;
use ducker::docker::{container::DockerContainer, util::new_local_docker_connection};
use nom::{
    IResult, Parser,
    branch::alt,
    bytes::complete::{tag, take_until, take_while1},
    character::complete::char,
    combinator::map,
    multi::many0,
    sequence::{delimited, pair},
};
use serde_yaml::Value;
use std::collections::HashMap;
use std::env;
use std::fs;
use std::net::{SocketAddr, TcpListener};
use std::path::Path;
use tracing::{debug, error, info, warn};

/// 端口映射信息
#[derive(Debug, Clone)]
pub struct PortMapping {
    /// 主机端口
    pub host_port: u16,
    /// 容器端口
    pub container_port: u16,
    /// 协议类型 (tcp/udp)
    pub protocol: String,
    /// 服务名称
    pub service_name: String,
}

/// 端口冲突检查结果
#[derive(Debug)]
pub struct PortConflictReport {
    /// 有冲突的端口
    pub conflicted_ports: Vec<PortConflict>,
    /// 检查的端口总数
    pub total_checked: usize,
    /// 是否有冲突
    pub has_conflicts: bool,
}

/// 端口冲突详情
#[derive(Debug)]
pub struct PortConflict {
    /// 端口号
    pub port: u16,
    /// 服务名称
    pub service_name: String,
    /// 端口映射信息
    pub mapping: String,
}

/// 环境变量解析结果
#[derive(Debug, Clone)]
enum VarExpansion {
    /// 普通文本
    Text(String),
    /// 变量替换 ${VAR_NAME}
    Variable(String),
    /// 带默认值的变量 ${VAR_NAME:-default}
    VariableWithDefault(String, String),
}

/// 解析变量名（字母、数字、下划线、连字符）
fn var_name(input: &str) -> IResult<&str, &str> {
    take_while1(|c: char| c.is_alphanumeric() || c == '_' || c == '-').parse(input)
}

/// 解析 ${VAR_NAME} 格式的变量
fn parse_braced_var(input: &str) -> IResult<&str, VarExpansion> {
    map(delimited(tag("${"), var_name, char('}')), |var_name| {
        VarExpansion::Variable(var_name.to_string())
    })
    .parse(input)
}

/// 解析 ${VAR_NAME:-default} 格式的变量（带默认值）
fn parse_braced_var_with_default(input: &str) -> IResult<&str, VarExpansion> {
    map(
        delimited(
            tag("${"),
            pair(var_name, pair(tag(":-"), take_until("}"))),
            char('}'),
        ),
        |(var_name, (_, default_value))| {
            VarExpansion::VariableWithDefault(var_name.to_string(), default_value.to_string())
        },
    )
    .parse(input)
}

/// 解析 $VAR_NAME 格式的变量（不带花括号）
fn parse_simple_var(input: &str) -> IResult<&str, VarExpansion> {
    map(pair(char('$'), var_name), |(_, var_name)| {
        VarExpansion::Variable(var_name.to_string())
    })
    .parse(input)
}

/// 解析普通文本（非变量部分）
fn parse_text(input: &str) -> IResult<&str, VarExpansion> {
    map(take_while1(|c: char| c != '$'), |text: &str| {
        VarExpansion::Text(text.to_string())
    })
    .parse(input)
}

/// 解析单个 $ 字符（当它不是变量的开始时）
fn parse_dollar(input: &str) -> IResult<&str, VarExpansion> {
    map(char('$'), |_| VarExpansion::Text("$".to_string())).parse(input)
}

/// 解析环境变量和文本的混合内容
fn parse_env_string(input: &str) -> IResult<&str, Vec<VarExpansion>> {
    many0(alt((
        parse_braced_var_with_default, // 优先匹配带默认值的格式
        parse_braced_var,              // 然后匹配普通花括号格式
        parse_simple_var,              // 再匹配简单格式
        parse_text,                    // 最后匹配普通文本
        parse_dollar,                  // 处理单独的 $ 字符
    )))
    .parse(input)
}

/// 端口管理器 - 负责检测和管理端口冲突
#[derive(Debug, Clone)]
pub struct PortManager {
    /// 保留端口列表
    reserved_ports: Vec<u16>,
    /// 环境变量缓存
    env_vars: HashMap<String, String>,
}

impl PortManager {
    /// 创建新的端口管理器
    pub fn new() -> Self {
        Self {
            reserved_ports: Vec::new(),
            env_vars: HashMap::new(),
        }
    }

    /// 从.env文件加载环境变量
    pub fn load_env_file(&mut self, env_file_path: &Path) -> DockerServiceResult<()> {
        if !env_file_path.exists() {
            warn!(
                ".env文件不存在: {}, 跳过环境变量加载",
                env_file_path.display()
            );
            return Ok(());
        }

        info!("开始加载环境变量文件: {}", env_file_path.display());

        let content = fs::read_to_string(env_file_path).map_err(|e| {
            DockerServiceError::Configuration(format!(
                "无法读取.env文件 {}: {}",
                env_file_path.display(),
                e
            ))
        })?;

        info!("成功读取.env文件内容 ({} 字符)", content.len());

        // 清空现有环境变量缓存
        self.env_vars.clear();

        // 解析.env文件
        for (line_num, line) in content.lines().enumerate() {
            let line = line.trim();

            // 跳过空行和注释行
            if line.is_empty() || line.starts_with('#') {
                continue;
            }

            // 解析 KEY=VALUE 格式
            if let Some((key, value)) = line.split_once('=') {
                let key = key.trim().to_string();
                let value = value.trim();

                // 移除值两边的引号
                let value = if (value.starts_with('"') && value.ends_with('"'))
                    || (value.starts_with('\'') && value.ends_with('\''))
                {
                    &value[1..value.len() - 1]
                } else {
                    value
                };

                self.env_vars.insert(key.clone(), value.to_string());
                info!("第{}行: 加载环境变量: {} = {}", line_num + 1, key, value);
            } else {
                warn!("第{}行: 无效的环境变量格式: {}", line_num + 1, line);
            }
        }

        info!("环境变量加载完成: 总共 {} 个变量", self.env_vars.len());
        info!("已加载的环境变量列表: {:?}", self.env_vars);
        Ok(())
    }

    /// 替换字符串中的环境变量（使用 nom 解析器）
    /// 支持 ${VAR_NAME} 和 ${VAR_NAME:-default} 格式
    fn expand_env_vars(&self, input: &str) -> String {
        match parse_env_string(input) {
            Ok((remaining, expansions)) => {
                let mut result = String::new();

                // 处理解析出的各个部分
                for expansion in expansions {
                    match expansion {
                        VarExpansion::Text(text) => {
                            result.push_str(&text);
                        }
                        VarExpansion::Variable(var_name) => {
                            if let Some(value) = self.env_vars.get(&var_name) {
                                result.push_str(value);
                            } else if let Ok(value) = env::var(&var_name) {
                                result.push_str(&value);
                            } else {
                                warn!("环境变量 {} 未定义", var_name);
                                // 保持原始格式
                                result.push_str(&format!("${{{var_name}}}"));
                            }
                        }
                        VarExpansion::VariableWithDefault(var_name, default_value) => {
                            if let Some(value) = self.env_vars.get(&var_name) {
                                result.push_str(value);
                            } else if let Ok(value) = env::var(&var_name) {
                                result.push_str(&value);
                            } else {
                                debug!(
                                    "环境变量 {} 未定义，使用默认值: {}",
                                    var_name, default_value
                                );
                                result.push_str(&default_value);
                            }
                        }
                    }
                }

                // 如果还有剩余字符，追加到结果末尾
                if !remaining.is_empty() {
                    result.push_str(remaining);
                }

                result
            }
            Err(_) => {
                // 如果解析失败，返回原始字符串
                warn!("环境变量解析失败，返回原始字符串: {}", input);
                input.to_string()
            }
        }
    }

    /// 检查端口是否可用（实际检测系统端口占用）
    pub fn is_port_available(&self, port: u16) -> bool {
        // 检查是否在保留端口列表中
        if self.reserved_ports.contains(&port) {
            return false;
        }

        // 先检查 0.0.0.0（所有接口），这是最严格的检查
        // 如果能绑定 0.0.0.0，说明端口确实可用
        match TcpListener::bind(SocketAddr::from(([0, 0, 0, 0], port))) {
            Ok(listener) => {
                // 显式drop以立即释放端口
                drop(listener);
                true
            }
            Err(_) => {
                // 如果 0.0.0.0 绑定失败，再尝试 127.0.0.1
                // 这可以检测是否只是权限问题（某些系统上普通用户无法绑定 0.0.0.0）
                match TcpListener::bind(SocketAddr::from(([127, 0, 0, 1], port))) {
                    Ok(listener) => {
                        drop(listener);
                        // 能绑定本地回环但不能绑定所有接口，可能是权限限制
                        // 这种情况下我们认为端口可用（但可能需要提醒用户）
                        warn!("端口 {} 只能绑定到 127.0.0.1，可能存在权限限制", port);
                        true
                    }
                    Err(_) => {
                        // 连本地回环都绑定不了，端口确实被占用
                        false
                    }
                }
            }
        }
    }

    /// 获取可用端口
    #[allow(dead_code)]
    pub fn get_available_port(&self, preferred_port: u16) -> DockerServiceResult<u16> {
        if self.is_port_available(preferred_port) {
            Ok(preferred_port)
        } else {
            // 简单的端口递增策略
            for port in (preferred_port + 1)..=(preferred_port + 100) {
                if self.is_port_available(port) {
                    return Ok(port);
                }
            }
            Err(DockerServiceError::Configuration(format!(
                "无法找到从 {preferred_port} 开始的可用端口"
            )))
        }
    }

    /// 保留端口
    #[allow(dead_code)]
    pub fn reserve_port(&mut self, port: u16) {
        if !self.reserved_ports.contains(&port) {
            self.reserved_ports.push(port);
        }
    }

    /// 从docker-compose.yml文件中解析端口映射
    pub async fn parse_compose_ports(
        &mut self,
        compose_file_path: &Path,
    ) -> DockerServiceResult<Vec<PortMapping>> {
        info!(
            "开始解析docker-compose端口映射: {}",
            compose_file_path.display()
        );

        // 只有在环境变量缓存为空时才加载.env文件（避免重复加载）
        if self.env_vars.is_empty() {
            if let Some(parent_dir) = compose_file_path.parent() {
                let env_file = parent_dir.join(".env");
                if env_file.exists() {
                    info!(
                        "检测到环境变量缓存为空，加载.env文件: {}",
                        env_file.display()
                    );
                    if let Err(e) = self.load_env_file(&env_file) {
                        error!("在parse_compose_ports中加载环境变量文件失败: {}", e);
                        return Err(e);
                    }
                } else {
                    warn!("环境变量缓存为空，但未找到.env文件: {}", env_file.display());
                }
            }
        } else {
            info!(
                "环境变量缓存不为空（{}个变量），跳过.env文件加载",
                self.env_vars.len()
            );
        }

        let content = std::fs::read_to_string(compose_file_path).map_err(|e| {
            DockerServiceError::Configuration(format!(
                "无法读取docker-compose文件 {}: {}",
                compose_file_path.display(),
                e
            ))
        })?;

        let yaml: Value = serde_yaml::from_str(&content).map_err(|e| {
            DockerServiceError::Configuration(format!("解析docker-compose文件失败: {e}"))
        })?;

        let mut port_mappings = Vec::new();

        if let Some(services) = yaml.get("services").and_then(|s| s.as_mapping()) {
            for (service_name, service_config) in services {
                let service_name = service_name.as_str().unwrap_or("unknown").to_string();

                if let Some(ports) = service_config.get("ports").and_then(|p| p.as_sequence()) {
                    for port_def in ports {
                        if let Some(port_mapping) =
                            self.parse_port_definition(port_def, &service_name)?
                        {
                            port_mappings.push(port_mapping);
                        }
                    }
                }
            }
        }

        info!("解析完成，找到 {} 个端口映射", port_mappings.len());
        Ok(port_mappings)
    }

    /// 解析单个端口定义
    fn parse_port_definition(
        &self,
        port_def: &Value,
        service_name: &str,
    ) -> DockerServiceResult<Option<PortMapping>> {
        match port_def {
            Value::String(port_str) => {
                info!("解析端口定义 (原始): {} (服务: {})", port_str, service_name);
                debug!("当前环境变量缓存: {:?}", self.env_vars);

                // 先展开环境变量
                let port_str = self.expand_env_vars(port_str.trim());
                info!(
                    "解析端口定义 (展开环境变量后): {} (服务: {})",
                    port_str, service_name
                );

                // 格式: "8080:80" 或 "127.0.0.1:8080:80" 或 "8080:80/tcp"
                let port_str = port_str.trim();

                // 提取协议
                let (port_part, protocol) = if port_str.contains('/') {
                    let parts: Vec<&str> = port_str.split('/').collect();
                    (parts[0], parts.get(1).unwrap_or(&"tcp").to_string())
                } else {
                    (port_str, "tcp".to_string())
                };

                // 解析端口映射
                let ports: Vec<&str> = port_part.split(':').collect();
                match ports.len() {
                    2 => {
                        // "8080:80"
                        let host_port = ports[0].parse::<u16>().map_err(|_| {
                            DockerServiceError::Configuration(format!(
                                "无效的主机端口: {} (原始: {}) (服务: {})",
                                ports[0], port_str, service_name
                            ))
                        })?;
                        let container_port = ports[1].parse::<u16>().map_err(|_| {
                            DockerServiceError::Configuration(format!(
                                "无效的容器端口: {} (原始: {}) (服务: {})",
                                ports[1], port_str, service_name
                            ))
                        })?;

                        Ok(Some(PortMapping {
                            host_port,
                            container_port,
                            protocol,
                            service_name: service_name.to_string(),
                        }))
                    }
                    3 => {
                        // "127.0.0.1:8080:80"
                        let host_port = ports[1].parse::<u16>().map_err(|_| {
                            DockerServiceError::Configuration(format!(
                                "无效的主机端口: {} (原始: {}) (服务: {})",
                                ports[1], port_str, service_name
                            ))
                        })?;
                        let container_port = ports[2].parse::<u16>().map_err(|_| {
                            DockerServiceError::Configuration(format!(
                                "无效的容器端口: {} (原始: {}) (服务: {})",
                                ports[2], port_str, service_name
                            ))
                        })?;

                        Ok(Some(PortMapping {
                            host_port,
                            container_port,
                            protocol,
                            service_name: service_name.to_string(),
                        }))
                    }
                    _ => {
                        warn!(
                            "无法解析端口定义: {} (原始: {}) (服务: {})",
                            port_part, port_str, service_name
                        );
                        Ok(None)
                    }
                }
            }
            Value::Number(port_num) => {
                // 仅容器端口，没有主机端口映射
                if let Some(port) = port_num.as_u64() {
                    if port <= 65535 {
                        // 这种情况下没有主机端口映射，不需要检查冲突
                        Ok(None)
                    } else {
                        Err(DockerServiceError::Configuration(format!(
                            "端口号超出范围: {port}"
                        )))
                    }
                } else {
                    Ok(None)
                }
            }
            _ => {
                warn!("未知的端口定义格式: {:?}", port_def);
                Ok(None)
            }
        }
    }

    /// 智能检查端口冲突（考虑是否是已有服务占用）
    pub async fn smart_check_compose_port_conflicts(
        &mut self,
        compose_file_path: &Path,
        env_file_path: &Path,
    ) -> DockerServiceResult<PortConflictReport> {
        info!(
            "开始智能检查docker-compose文件的端口冲突: {}",
            compose_file_path.display()
        );

        // 自动加载.env文件
        if env_file_path.exists() {
            info!(
                "发现.env文件，正在加载环境变量: {}",
                env_file_path.display()
            );
            match self.load_env_file(&env_file_path) {
                Ok(_) => info!("✅ .env文件加载成功"),
                Err(e) => {
                    error!("❌ 加载.env文件失败: {}", e);
                    return Err(e);
                }
            }
        } else {
            warn!("❌ 未找到.env文件: {}", env_file_path.display());
        }

        // 显示当前环境变量状态
        info!("当前已加载的环境变量数量: {}", self.env_vars.len());

        // 解析docker-compose.yml中定义的服务
        let compose_services = self.parse_compose_services(compose_file_path)?;
        info!("当前docker-compose.yml中定义的服务: {:?}", compose_services);

        let port_mappings = self.parse_compose_ports(compose_file_path).await?;
        let mut conflicted_ports = Vec::new();
        let total_checked = port_mappings.len();

        // 尝试获取当前运行的容器信息
        let running_containers = self.get_running_containers().await;

        for mapping in &port_mappings {
            if !self.is_port_available(mapping.host_port) {
                // 端口被占用，检查是否是已有的相关服务
                let is_related_service = if let Ok(containers) = &running_containers {
                    self.is_port_used_by_compose_service(
                        mapping.host_port,
                        containers,
                        &mapping.service_name,
                        &compose_services,
                    )
                } else {
                    false
                };

                if is_related_service {
                    info!(
                        "端口 {} 被compose相关服务占用 (服务: {}) - 这是正常的",
                        mapping.host_port, mapping.service_name
                    );
                } else {
                    warn!(
                        "发现端口冲突: 端口 {} 被其他进程占用 (服务: {})",
                        mapping.host_port, mapping.service_name
                    );

                    conflicted_ports.push(PortConflict {
                        port: mapping.host_port,
                        service_name: mapping.service_name.clone(),
                        mapping: format!(
                            "{}:{}/{}",
                            mapping.host_port, mapping.container_port, mapping.protocol
                        ),
                    });
                }
            } else {
                debug!(
                    "端口 {} 可用 (服务: {})",
                    mapping.host_port, mapping.service_name
                );
            }
        }

        let has_conflicts = !conflicted_ports.is_empty();

        if has_conflicts {
            error!(
                "发现 {} 个真实端口冲突，共检查 {} 个端口",
                conflicted_ports.len(),
                total_checked
            );
        } else {
            info!(
                "智能端口检查完成，没有发现冲突，共检查 {} 个端口",
                total_checked
            );
        }

        Ok(PortConflictReport {
            conflicted_ports,
            total_checked,
            has_conflicts,
        })
    }

    /// 解析docker-compose.yml中定义的服务名称列表
    fn parse_compose_services(&self, compose_file_path: &Path) -> DockerServiceResult<Vec<String>> {
        let content = std::fs::read_to_string(compose_file_path).map_err(|e| {
            DockerServiceError::Configuration(format!(
                "无法读取docker-compose文件 {}: {}",
                compose_file_path.display(),
                e
            ))
        })?;

        let yaml: Value = serde_yaml::from_str(&content).map_err(|e| {
            DockerServiceError::Configuration(format!("解析docker-compose文件失败: {e}"))
        })?;

        let mut services = Vec::new();

        if let Some(services_section) = yaml.get("services").and_then(|s| s.as_mapping()) {
            for (service_name, _) in services_section {
                if let Some(name) = service_name.as_str() {
                    services.push(name.to_string());
                }
            }
        }

        Ok(services)
    }

    /// 检查端口是否被compose文件中定义的相关服务使用
    fn is_port_used_by_compose_service(
        &self,
        port: u16,
        containers: &[DockerContainer],
        service_name: &str,
        compose_services: &[String],
    ) -> bool {
        for container in containers {
            // 首先检查容器是否与compose文件中定义的服务相关
            if let Some(related_service) =
                self.find_related_compose_service(&container.names, compose_services)
            {
                // 只有当容器对应的服务在compose文件中定义时，才检查端口
                if related_service == service_name {
                    // 检查容器的端口映射
                    if container.ports.contains(&port.to_string()) {
                        debug!(
                            "端口 {} 被compose服务 {} 的容器 {} 使用",
                            port, related_service, container.names
                        );
                        return true;
                    }
                }
            }
        }
        false
    }

    /// 找到容器对应的compose服务名称
    fn find_related_compose_service(
        &self,
        container_name: &str,
        compose_services: &[String],
    ) -> Option<String> {
        let container_lower = container_name.to_lowercase();

        for service_name in compose_services {
            let service_lower = service_name.to_lowercase();

            // 检查容器名称是否包含服务名称
            if container_lower.contains(&service_lower) {
                return Some(service_name.clone());
            }

            // 检查是否是docker-compose生成的容器名称格式
            // 通常格式为: {项目名}_{服务名}_{实例号} 或 {项目名}-{服务名}-{实例号}
            if container_lower.contains(&format!("_{service_lower}_"))
                || container_lower.contains(&format!("-{service_lower}-"))
                || container_lower.ends_with(&format!("_{service_lower}"))
                || container_lower.ends_with(&format!("-{service_lower}"))
            {
                return Some(service_name.clone());
            }
        }

        None
    }

    /// 获取当前运行的容器信息
    async fn get_running_containers(&self) -> Result<Vec<DockerContainer>, String> {
        match new_local_docker_connection(DOCKER_SOCKET_PATH, None).await {
            Ok(docker) => match DockerContainer::list(&docker).await {
                Ok(containers) => {
                    debug!("成功获取到 {} 个容器信息", containers.len());
                    Ok(containers)
                }
                Err(e) => {
                    warn!("获取容器列表失败: {}", e);
                    Err(format!("获取容器列表失败: {e}"))
                }
            },
            Err(e) => {
                warn!("连接Docker失败: {}", e);
                Err(format!("连接Docker失败: {e}"))
            }
        }
    }

    /// 显示智能端口冲突报告
    pub fn print_smart_conflict_report(&self, report: &PortConflictReport) {
        if report.has_conflicts {
            warn!("⚠️  发现真实端口冲突!");
            warn!("总计检查: {} 个端口映射", report.total_checked);
            warn!("冲突数量: {} 个", report.conflicted_ports.len());

            warn!("冲突详情:");
            for conflict in &report.conflicted_ports {
                warn!("  🔴 端口 {} 被其他进程占用", conflict.port);
                warn!("     服务: {}", conflict.service_name);
                warn!("     映射: {}", conflict.mapping);
            }

            info!("💡 解决建议:");
            info!("  1. 停止占用端口的其他进程");
            info!("  2. 修改docker-compose.yml中的端口映射");
            info!("  3. 使用以下命令查看端口占用情况:");

            for conflict in &report.conflicted_ports {
                info!("     lsof -i :{}", conflict.port);
            }
        } else {
            info!("✅ 智能端口检查通过，没有发现冲突");
            info!("总计检查: {} 个端口映射", report.total_checked);
            info!("💡 提示: 已跳过相关服务占用的端口");
        }
    }
}

impl Default for PortManager {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::env;

    #[test]
    fn test_nom_parse_simple_text() {
        let result = parse_env_string("hello world");
        assert!(result.is_ok());
        let (remaining, expansions) = result.unwrap();
        assert_eq!(remaining, "");
        assert_eq!(expansions.len(), 1);
        match &expansions[0] {
            VarExpansion::Text(text) => assert_eq!(text, "hello world"),
            _ => panic!("应该是文本"),
        }
    }

    #[test]
    fn test_nom_parse_simple_variable() {
        let result = parse_env_string("${VAR_NAME}");
        assert!(result.is_ok());
        let (remaining, expansions) = result.unwrap();
        assert_eq!(remaining, "");
        assert_eq!(expansions.len(), 1);
        match &expansions[0] {
            VarExpansion::Variable(var_name) => assert_eq!(var_name, "VAR_NAME"),
            _ => panic!("应该是变量"),
        }
    }

    #[test]
    fn test_nom_parse_variable_with_default() {
        let result = parse_env_string("${VAR_NAME:-default_value}");
        assert!(result.is_ok());
        let (remaining, expansions) = result.unwrap();
        assert_eq!(remaining, "");
        assert_eq!(expansions.len(), 1);
        match &expansions[0] {
            VarExpansion::VariableWithDefault(var_name, default_value) => {
                assert_eq!(var_name, "VAR_NAME");
                assert_eq!(default_value, "default_value");
            }
            _ => panic!("应该是带默认值的变量"),
        }
    }

    #[test]
    fn test_nom_parse_mixed_content() {
        let result = parse_env_string("Hello ${USER}, your port is ${PORT:-8080}!");
        assert!(result.is_ok());
        let (remaining, expansions) = result.unwrap();

        assert_eq!(remaining, "");
        assert_eq!(expansions.len(), 5); // 包括末尾的感叹号

        match &expansions[0] {
            VarExpansion::Text(text) => assert_eq!(text, "Hello "),
            _ => panic!("第一个应该是文本"),
        }

        match &expansions[1] {
            VarExpansion::Variable(var_name) => assert_eq!(var_name, "USER"),
            _ => panic!("第二个应该是变量"),
        }

        match &expansions[2] {
            VarExpansion::Text(text) => assert_eq!(text, ", your port is "),
            _ => panic!("第三个应该是文本"),
        }

        match &expansions[3] {
            VarExpansion::VariableWithDefault(var_name, default_value) => {
                assert_eq!(var_name, "PORT");
                assert_eq!(default_value, "8080");
            }
            _ => panic!("第四个应该是带默认值的变量"),
        }

        match &expansions[4] {
            VarExpansion::Text(text) => assert_eq!(text, "!"),
            _ => panic!("第五个应该是文本（感叹号）"),
        }
    }

    #[test]
    fn test_expand_env_vars_with_nom() {
        let mut port_manager = PortManager::new();
        port_manager
            .env_vars
            .insert("TEST_VAR".to_string(), "test_value".to_string());

        // 测试简单变量替换
        let result = port_manager.expand_env_vars("${TEST_VAR}");
        assert_eq!(result, "test_value");

        // 测试带默认值的变量（存在）
        let result = port_manager.expand_env_vars("${TEST_VAR:-default}");
        assert_eq!(result, "test_value");

        // 测试带默认值的变量（不存在）
        let result = port_manager.expand_env_vars("${UNDEFINED_VAR:-8080}");
        assert_eq!(result, "8080");

        // 测试混合内容
        let result = port_manager.expand_env_vars("Value: ${TEST_VAR}, Port: ${PORT:-3000}");
        assert_eq!(result, "Value: test_value, Port: 3000");

        // 测试普通文本
        let result = port_manager.expand_env_vars("no variables here");
        assert_eq!(result, "no variables here");
    }

    #[test]
    fn test_expand_env_vars_system_env() {
        let port_manager = PortManager::new();

        // 设置一个系统环境变量
        unsafe {
            env::set_var("TEST_SYSTEM_VAR", "system_value");
        }

        let result = port_manager.expand_env_vars("${TEST_SYSTEM_VAR}");
        assert_eq!(result, "system_value");

        // 清理
        unsafe {
            env::remove_var("TEST_SYSTEM_VAR");
        }
    }

    #[test]
    fn test_expand_env_vars_undefined_variable() {
        let port_manager = PortManager::new();

        // 测试未定义的变量保持原样
        let result = port_manager.expand_env_vars("${UNDEFINED_VAR}");
        assert_eq!(result, "${UNDEFINED_VAR}");
    }
}
