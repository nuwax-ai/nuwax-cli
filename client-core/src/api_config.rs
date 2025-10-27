use crate::constants::api;
use serde::{Deserialize, Serialize};
/// API配置模块 - 内置服务器端点配置
use std::fmt;

/// API端点配置
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ApiEndpoints {
    /// 客户端注册端点
    pub client_register: String,
    /// 公告获取端点
    pub announcements: String,
    /// Docker版本检查端点
    pub docker_check_version: String,
    /// Docker版本列表更新端点
    pub docker_update_version_list: String,
    /// Docker完整服务包下载端点
    pub docker_download_full: String,
    /// 客户端自升级历史端点
    pub client_self_upgrade_history: String,
    /// 服务升级历史端点
    pub service_upgrade_history: String,
    /// 遥测数据上报端点
    pub telemetry: String,
}

/// API配置
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ApiConfig {
    /// 基础URL
    pub base_url: String,
    /// API端点
    pub endpoints: ApiEndpoints,
}

impl Default for ApiConfig {
    fn default() -> Self {
        Self {
            base_url: api::DEFAULT_BASE_URL.to_string(),
            endpoints: ApiEndpoints {
                client_register: api::endpoints::CLIENT_REGISTER.to_string(),
                announcements: api::endpoints::ANNOUNCEMENTS.to_string(),
                docker_check_version: api::endpoints::DOCKER_CHECK_VERSION.to_string(),
                docker_update_version_list: api::endpoints::DOCKER_UPDATE_VERSION_LIST.to_string(),
                docker_download_full: api::endpoints::DOCKER_DOWNLOAD_FULL.to_string(),
                client_self_upgrade_history: api::endpoints::CLIENT_SELF_UPGRADE_HISTORY
                    .to_string(),
                service_upgrade_history: api::endpoints::SERVICE_UPGRADE_HISTORY.to_string(),
                telemetry: api::endpoints::TELEMETRY.to_string(),
            },
        }
    }
}

impl ApiConfig {
    /// 获取完整的端点URL
    pub fn get_endpoint_url(&self, endpoint: &str) -> String {
        format!("{}{}", self.base_url, endpoint)
    }

    /// 获取客户端注册完整URL
    pub fn get_client_register_url(&self) -> String {
        self.get_endpoint_url(&self.endpoints.client_register)
    }

    /// 获取公告列表完整URL
    pub fn get_announcements_url(&self) -> String {
        self.get_endpoint_url(&self.endpoints.announcements)
    }

    /// 获取Docker版本检查完整URL
    pub fn get_docker_check_version_url(&self) -> String {
        self.get_endpoint_url(&self.endpoints.docker_check_version)
    }

    /// 获取Docker版本列表更新完整URL
    pub fn get_docker_update_version_list_url(&self) -> String {
        self.get_endpoint_url(&self.endpoints.docker_update_version_list)
    }

    /// 获取Docker完整服务包下载完整URL
    pub fn get_docker_download_full_url(&self) -> String {
        self.get_endpoint_url(&self.endpoints.docker_download_full)
    }

    /// 获取服务升级历史完整URL（替换service_name占位符）
    pub fn get_service_upgrade_history_url(&self, service_name: &str) -> String {
        let endpoint = self
            .endpoints
            .service_upgrade_history
            .replace("{service_name}", service_name);
        self.get_endpoint_url(&endpoint)
    }

    /// 获取遥测数据上报完整URL
    pub fn get_telemetry_url(&self) -> String {
        self.get_endpoint_url(&self.endpoints.telemetry)
    }

    /// 获取所有端点信息，用于CLI帮助显示
    pub fn get_endpoints_info(&self) -> Vec<(&str, String)> {
        vec![
            ("服务器地址", self.base_url.clone()),
            ("客户端注册", self.get_client_register_url()),
            ("获取公告", self.get_announcements_url()),
            ("检查Docker版本", self.get_docker_check_version_url()),
            ("Docker版本列表", self.get_docker_update_version_list_url()),
            ("下载Docker更新", self.get_docker_download_full_url()),
            ("上报遥测数据", self.get_telemetry_url()),
        ]
    }
}

impl fmt::Display for ApiConfig {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        writeln!(f, "当前API配置:")?;
        writeln!(f, "服务器地址: {}", self.base_url)?;
        writeln!(f, "\n主要端点:")?;
        for (name, url) in self.get_endpoints_info() {
            writeln!(f, "  {name}: {url}")?;
        }
        Ok(())
    }
}
