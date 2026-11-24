use crate::api_config::{ApiConfig, ApiEndpoints};
use crate::api_types::*;
use crate::config::DockerChannel;
use crate::constants::api as api_constants;
use crate::downloader::{DownloadProgress, DownloaderConfig, FileDownloader};
use crate::environment::Environment;
use crate::error::DuckError;
use crate::version::Version;
use anyhow::Result;
use reqwest::Client;
use std::path::Path;
use std::sync::Arc;
use std::time::Duration;
use tracing::{error, info};

/// API 客户端
///
/// 只负责从阿里云 OSS 读取版本清单并下载制品。原先指向自建 API 服务器的
/// 注册、公告、版本检查与上报接口已随服务器下线一并移除。
#[derive(Debug, Clone)]
pub struct ApiClient {
    client: Client,
    config: Arc<ApiConfig>,
}

impl Default for ApiClient {
    fn default() -> Self {
        Self::new()
    }
}

impl ApiClient {
    /// 创建新的 API 客户端
    pub fn new() -> Self {
        Self {
            client: crate::http_client::client_builder()
                .timeout(Duration::from_secs(60))
                .build()
                .expect("Failed to create HTTP client with timeout"),
            config: Arc::new(ApiConfig::default()),
        }
    }

    /// 获取当前API配置
    pub fn get_config(&self) -> &ApiConfig {
        &self.config
    }

    /// 构建 GET 请求
    fn build_request(&self, url: &str) -> reqwest::RequestBuilder {
        self.client.get(url)
    }

    /// 获取增强的服务清单（离线镜像通道，历史默认行为）
    ///
    /// 固定从 OSS 读取，支持通过 NUWAX_API_DOCKER_VERSION_URL 环境变量自定义 URL；
    /// NUWAX_CLI_ENV=test 时使用 beta/latest.json
    pub async fn get_enhanced_service_manifest(&self) -> Result<EnhancedServiceManifest> {
        self.get_service_manifest(DockerChannel::Images).await
    }

    /// 按镜像交付通道获取服务清单。
    ///
    /// 清单只来自阿里云 OSS：原自建 API 服务器已下线，此前 Images 通道在 OSS 读取
    /// 失败后会降级到服务器端的 `/api/v1/docker/upgrade/versions/latest.json`，现在两条
    /// 通道行为一致——读不到就直接失败。
    pub async fn get_service_manifest(
        &self,
        channel: DockerChannel,
    ) -> Result<EnhancedServiceManifest> {
        let (oss_url, url_source) = manifest_url_for(
            channel,
            std::env::var(manifest_url_env(channel)).ok().as_deref(),
            &Environment::from_env(),
            &self.config.endpoints,
        );

        info!("Fetching service manifest from {}: {}", url_source, oss_url);
        self.fetch_and_parse_manifest(&oss_url).await
    }

    /// 从指定 URL 获取并解析服务清单
    async fn fetch_and_parse_manifest(&self, url: &str) -> Result<EnhancedServiceManifest> {
        let response = self.build_request(url).send().await?;

        if response.status().is_success() {
            // 先获取原始json文本，解析为serde_json::Value，判断根对象是否有 platforms 字段
            let text = response.text().await?;
            let json_value: serde_json::Value = serde_json::from_str(&text).map_err(|e| {
                DuckError::Api(format!("Service manifest JSON parsing failed: {e}"))
            })?;

            let has_platforms = match &json_value {
                serde_json::Value::Object(map) => map.contains_key("platforms"),
                _ => false,
            };

            if has_platforms {
                // 有 platforms 字段，按增强格式解析
                match serde_json::from_value::<EnhancedServiceManifest>(json_value) {
                    Ok(manifest) => {
                        info!("Successfully parsed enhanced service manifest");
                        manifest.validate()?; // 进行数据验证
                        Ok(manifest)
                    }
                    Err(e) => {
                        error!("Failed to parse service upgrade - enhanced format: {}", e);
                        Err(anyhow::anyhow!(
                            "Failed to parse service upgrade - enhanced format: {}",
                            e
                        ))
                    }
                }
            } else {
                // 没有 platforms 字段，按旧格式解析并转换
                match serde_json::from_value::<ServiceManifest>(json_value) {
                    Ok(old_manifest) => {
                        info!(
                            "Successfully parsed legacy service manifest, converting to enhanced format"
                        );
                        let enhanced_manifest = EnhancedServiceManifest {
                            version: old_manifest.version.parse::<Version>()?,
                            release_date: old_manifest.release_date,
                            release_notes: old_manifest.release_notes,
                            packages: Some(old_manifest.packages),
                            platforms: None,
                            patch: None,
                        };
                        enhanced_manifest.validate()?;
                        Ok(enhanced_manifest)
                    }
                    Err(e) => {
                        error!("Failed to parse service upgrade - legacy format: {}", e);
                        Err(anyhow::anyhow!(
                            "Failed to parse service upgrade - legacy format: {}",
                            e
                        ))
                    }
                }
            }
        } else {
            let status = response.status();
            let text = response.text().await.unwrap_or_default();
            error!(
                "Failed to get enhanced service manifest: {} - {}",
                status, text
            );
            Err(anyhow::anyhow!(
                "Failed to get enhanced service manifest: {status} - {text}"
            ))
        }
    }

    /// Download a release-bound package into its final, atomically promoted cache.
    pub async fn download_service_package(
        &self,
        download_directory: &Path,
        identity: &crate::package_cache::PackageIdentity,
    ) -> Result<std::path::PathBuf> {
        let downloader = FileDownloader::new(DownloaderConfig {
            retain_completed_metadata: true,
            ..DownloaderConfig::default()
        });
        crate::package_cache::download_package::<fn(DownloadProgress)>(
            &self.client,
            &downloader,
            download_directory,
            identity,
            None,
        )
        .await
    }
}

/// 系统信息模块
/// 用于获取操作系统类型和版本等信息
#[allow(dead_code)]
pub mod system_info {
    use serde::{Deserialize, Serialize};

    #[derive(Debug, Clone, Serialize, Deserialize)]
    pub struct Info {
        os_type: String,
        version: String,
    }

    impl Info {
        pub fn os_type(&self) -> &str {
            &self.os_type
        }
        pub fn version(&self) -> &str {
            &self.version
        }
    }

    pub fn get() -> Info {
        Info {
            os_type: std::env::consts::OS.to_string(),
            version: std::env::consts::ARCH.to_string(),
        }
    }
}

/// 该通道清单 URL 的环境变量覆盖名（最高优先级）
fn manifest_url_env(channel: DockerChannel) -> &'static str {
    match channel {
        DockerChannel::Images => api_constants::NUWAX_API_DOCKER_VERSION_URL_ENV,
        DockerChannel::Registry => api_constants::NUWAX_API_DOCKER_REGISTRY_VERSION_URL_ENV,
    }
}

/// 通道 + 环境变量覆盖 + 运行环境 → 清单 URL。
/// 选择规则集中在这个纯函数里，两条通道的优先级差异一目了然。
/// 运行环境判定复用 `Environment`，与配置/数据库文件名的 prod/test 切换同一套规则。
fn manifest_url_for(
    channel: DockerChannel,
    url_override: Option<&str>,
    environment: &Environment,
    endpoints: &ApiEndpoints,
) -> (String, String) {
    if let Some(url) = url_override {
        return (
            url.to_string(),
            format!("env {}", manifest_url_env(channel)),
        );
    }

    let beta = environment.is_testing();
    match (channel, beta) {
        (DockerChannel::Images, true) => (
            endpoints.docker_version_oss_beta.clone(),
            "beta OSS (test env)".to_string(),
        ),
        (DockerChannel::Images, false) => (
            endpoints.docker_version_oss_prod.clone(),
            "prod OSS".to_string(),
        ),
        (DockerChannel::Registry, true) => (
            endpoints.docker_registry_version_oss_beta.clone(),
            "registry beta OSS (test env)".to_string(),
        ),
        (DockerChannel::Registry, false) => (
            endpoints.docker_registry_version_oss_prod.clone(),
            "registry prod OSS".to_string(),
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // 创建测试用的API客户端
    fn create_test_api_client() -> ApiClient {
        ApiClient::new()
    }

    /// 离线通道的默认 URL 必须与新增 registry 通道之前逐字节一致
    #[test]
    fn manifest_urls_are_channel_scoped_and_offline_defaults_are_unchanged() {
        use crate::environment::Environment;
        let offline = Environment::Production;
        let test = Environment::Test;
        let endpoints = &ApiConfig::default().endpoints;
        let (url, source) = manifest_url_for(DockerChannel::Images, None, &offline, endpoints);
        assert_eq!(
            url,
            "https://nuwa-packages.oss-rg-china-mainland.aliyuncs.com/docker-version/prod/latest.json"
        );
        assert_eq!(source, "prod OSS");

        let (beta, beta_source) = manifest_url_for(DockerChannel::Images, None, &test, endpoints);
        assert_eq!(beta, endpoints.docker_version_oss_beta);
        assert_eq!(beta_source, "beta OSS (test env)");

        // 覆盖变量只要存在就生效（哪怕是空值），与引入 registry 通道之前一致
        let (override_empty, override_source) =
            manifest_url_for(DockerChannel::Images, Some(""), &offline, endpoints);
        assert_eq!(override_empty, "");
        assert_eq!(override_source, "env NUWAX_API_DOCKER_VERSION_URL");

        let (registry, source) =
            manifest_url_for(DockerChannel::Registry, None, &offline, endpoints);
        assert_eq!(
            registry,
            "https://nuwa-packages.oss-rg-china-mainland.aliyuncs.com/docker-version/prod/registry.json"
        );
        assert_eq!(source, "registry prod OSS");
        let (registry_beta, _) = manifest_url_for(DockerChannel::Registry, None, &test, endpoints);
        assert_eq!(registry_beta, endpoints.docker_registry_version_oss_beta);

        // 覆盖变量按通道隔离：瘦通道不能借用离线通道的覆盖
        assert_eq!(
            manifest_url_env(DockerChannel::Images),
            "NUWAX_API_DOCKER_VERSION_URL"
        );
        assert_eq!(
            manifest_url_env(DockerChannel::Registry),
            "NUWAX_API_DOCKER_REGISTRY_VERSION_URL"
        );
        let (override_url, override_source) = manifest_url_for(
            DockerChannel::Registry,
            Some("http://192.0.2.117/registry.json"),
            &offline,
            endpoints,
        );
        assert_eq!(override_url, "http://192.0.2.117/registry.json");
        assert_eq!(override_source, "env NUWAX_API_DOCKER_REGISTRY_VERSION_URL");
    }

    #[test]
    fn test_api_client_creation() {
        let client = create_test_api_client();

        // 清单地址仍然指向 OSS，不再依赖已下线的自建 API 服务器
        let config = client.get_config();
        assert!(
            config
                .endpoints
                .docker_version_oss_prod
                .starts_with("https://")
        );
        assert!(
            config
                .get_endpoints_info()
                .iter()
                .all(|(_, url)| url.contains("aliyuncs.com"))
        );
    }

    #[test]
    fn test_build_request_uses_get() {
        let client = create_test_api_client();
        let url = "http://test.example.com/latest.json";
        let request = client.build_request(url).build().unwrap();

        assert_eq!(request.method(), reqwest::Method::GET);
        assert_eq!(request.url().as_str(), url);
    }

    #[test]
    fn test_system_info() {
        let info = system_info::get();

        // 验证系统信息不为空
        assert!(!info.os_type().is_empty());
        assert!(!info.version().is_empty());

        // 验证返回的是合理的值
        let valid_os_types = ["windows", "macos", "linux"];
        assert!(valid_os_types.contains(&info.os_type()));

        let valid_archs = ["x86_64", "aarch64", "arm64"];
        assert!(valid_archs.contains(&info.version()));
    }

    #[test]
    fn test_system_info_serialization() {
        let info = system_info::get();

        // 测试序列化
        let serialized = serde_json::to_string(&info).unwrap();
        assert!(serialized.contains(info.os_type()));
        assert!(serialized.contains(info.version()));

        // 测试反序列化
        let deserialized: system_info::Info = serde_json::from_str(&serialized).unwrap();
        assert_eq!(deserialized.os_type(), info.os_type());
        assert_eq!(deserialized.version(), info.version());
    }

    // Task 1.5 验收标准测试
    #[tokio::test]
    async fn test_task_1_5_acceptance_criteria() {
        // 验收标准：API客户端能正常创建
        let client = create_test_api_client();
        assert!(!client.get_config().get_endpoints_info().is_empty());

        // 验收标准：两条通道的 prod/beta 清单地址齐备
        let endpoints = &client.get_config().endpoints;
        assert!(!endpoints.docker_version_oss_prod.is_empty());
        assert!(!endpoints.docker_version_oss_beta.is_empty());
        assert!(!endpoints.docker_registry_version_oss_prod.is_empty());
        assert!(!endpoints.docker_registry_version_oss_beta.is_empty());

        // 验收标准：超时和重试机制（内置在reqwest客户端中）
        // 这个在单元测试中难以验证，需要集成测试

        println!("Task 1.5: API Client Extension - Acceptance Criteria Test Passed");
        println!("   - New API client methods can be created normally");
        println!("   - Manifest endpoints are all configured");
    }
}
