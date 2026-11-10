use crate::constants::api;
use serde::{Deserialize, Serialize};
/// API配置模块 - 内置版本清单（阿里云 OSS）地址配置
use std::fmt;

/// 版本清单端点配置
///
/// 全部为 OSS 上的固定 JSON 地址，由 `NUWAX_CLI_ENV` 决定使用 prod 还是 beta 一份。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ApiEndpoints {
    /// Docker版本JSON (OSS) - 生产环境
    pub docker_version_oss_prod: String,

    /// Docker版本JSON (OSS) - 测试/发布环境
    pub docker_version_oss_beta: String,

    /// registry 瘦通道版本JSON (OSS) - 生产环境
    pub docker_registry_version_oss_prod: String,

    /// registry 瘦通道版本JSON (OSS) - 测试/发布环境
    pub docker_registry_version_oss_beta: String,
}

/// API配置
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ApiConfig {
    /// 版本清单端点
    pub endpoints: ApiEndpoints,
}

impl Default for ApiConfig {
    fn default() -> Self {
        Self {
            endpoints: ApiEndpoints {
                docker_version_oss_prod: api::endpoints::DOCKER_VERSION_OSS_PROD.to_string(),
                docker_version_oss_beta: api::endpoints::DOCKER_VERSION_OSS_BETA.to_string(),
                docker_registry_version_oss_prod: api::endpoints::DOCKER_REGISTRY_VERSION_OSS_PROD
                    .to_string(),
                docker_registry_version_oss_beta: api::endpoints::DOCKER_REGISTRY_VERSION_OSS_BETA
                    .to_string(),
            },
        }
    }
}

impl ApiConfig {
    /// 获取所有端点信息，用于CLI显示
    pub fn get_endpoints_info(&self) -> Vec<(&str, String)> {
        vec![
            (
                "Docker版本清单(离线镜像)",
                self.endpoints.docker_version_oss_prod.clone(),
            ),
            (
                "Docker版本清单(测试环境)",
                self.endpoints.docker_version_oss_beta.clone(),
            ),
            (
                "Docker版本清单(registry瘦通道)",
                self.endpoints.docker_registry_version_oss_prod.clone(),
            ),
            (
                "Docker版本清单(registry瘦通道, 测试环境)",
                self.endpoints.docker_registry_version_oss_beta.clone(),
            ),
        ]
    }
}

impl fmt::Display for ApiConfig {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        writeln!(f, "当前API配置:")?;
        writeln!(f, "版本清单来源: 阿里云 OSS")?;
        writeln!(f, "\n清单地址:")?;
        for (name, url) in self.get_endpoints_info() {
            writeln!(f, "  {name}: {url}")?;
        }
        Ok(())
    }
}
