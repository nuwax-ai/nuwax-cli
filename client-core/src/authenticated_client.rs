use crate::{ClientRegisterRequest, database::Database};
use anyhow::Result;
use reqwest::{Client, Method, RequestBuilder, Response};
use serde::Serialize;
use std::sync::Arc;
use tokio::sync::RwLock;
use tracing::{error, info, warn};

/// 认证客户端包装器
/// 自动处理client_id的设置和认证失败时的重新注册
#[derive(Debug, Clone)]
pub struct AuthenticatedClient {
    client: Client,
    database: Arc<Database>,
    server_base_url: String,
    client_id: Arc<RwLock<Option<String>>>,
}

impl AuthenticatedClient {
    /// 创建新的认证客户端
    pub async fn new(database: Arc<Database>, server_base_url: String) -> Result<Self> {
        let client = Client::new();

        // 从数据库获取当前的client_id
        let client_id = database.get_client_id().await?;

        Ok(Self {
            client,
            database,
            server_base_url,
            client_id: Arc::new(RwLock::new(client_id)),
        })
    }

    /// 检查URL是否是我们的服务器
    fn is_our_server(&self, url: &str) -> bool {
        url.starts_with(&self.server_base_url)
    }

    /// 检查是否是注册接口（不需要认证）
    fn is_register_endpoint(&self, url: &str) -> bool {
        url.contains("/clients/register")
    }

    /// 获取当前的client_id
    async fn get_client_id(&self) -> Option<String> {
        self.client_id.read().await.clone()
    }

    /// 更新client_id
    async fn set_client_id(&self, new_client_id: String) -> Result<()> {
        // 更新内存中的值
        *self.client_id.write().await = Some(new_client_id.clone());

        // 保存到数据库
        self.database.update_client_id(&new_client_id).await?;

        Ok(())
    }

    /// 自动注册客户端
    async fn auto_register(&self) -> Result<String> {
        info!("正在尝试自动注册客户端...");

        let request = ClientRegisterRequest {
            os: std::env::consts::OS.to_string(),
            arch: std::env::consts::ARCH.to_string(),
        };

        // 使用常量定义的注册端点
        let register_url = format!(
            "{}{}",
            self.server_base_url,
            crate::constants::api::endpoints::CLIENT_REGISTER
        );
        let response = self
            .client
            .post(&register_url)
            .json(&request)
            .send()
            .await?;

        if response.status().is_success() {
            let register_response: serde_json::Value = response.json().await?;
            if let Some(client_id) = register_response.get("client_id").and_then(|v| v.as_str()) {
                let client_id = client_id.to_string();
                info!("自动注册成功，获得客户端ID: {}", client_id);

                // 保存新的client_id
                self.set_client_id(client_id.clone()).await?;

                Ok(client_id)
            } else {
                Err(anyhow::anyhow!("注册响应格式无效"))
            }
        } else {
            let status = response.status();
            let text = response.text().await.unwrap_or_default();
            error!("客户端注册失败: {} - {}", status, text);
            Err(anyhow::anyhow!("注册失败: {status} - {text}"))
        }
    }

    /// 为请求添加认证头
    async fn add_auth_header(
        &self,
        mut request_builder: RequestBuilder,
        url: &str,
    ) -> RequestBuilder {
        // 只对我们的服务器且非注册接口添加认证头
        if self.is_our_server(url) && !self.is_register_endpoint(url) {
            if let Some(client_id) = self.get_client_id().await {
                request_builder = request_builder.header("X-Client-ID", client_id);
            }
        }
        request_builder
    }

    /// 执行请求，自动处理认证
    async fn execute_request(&self, method: Method, url: &str) -> Result<RequestBuilder> {
        let request_builder = self.client.request(method, url);
        Ok(self.add_auth_header(request_builder, url).await)
    }

    /// 执行带JSON body的请求
    async fn execute_request_with_json<T: Serialize>(
        &self,
        method: Method,
        url: &str,
        json: &T,
    ) -> Result<RequestBuilder> {
        let request_builder = self.client.request(method, url).json(json);
        Ok(self.add_auth_header(request_builder, url).await)
    }

    /// 发送请求并处理认证失败
    async fn send_with_retry(
        &self,
        request_builder: RequestBuilder,
        original_url: &str,
    ) -> Result<Response> {
        let response = request_builder.send().await?;

        // 检查是否是认证失败
        if response.status() == reqwest::StatusCode::UNAUTHORIZED
            && self.is_our_server(original_url)
            && !self.is_register_endpoint(original_url)
        {
            warn!("API请求认证失败 (401)，尝试自动重新注册...");

            // 尝试自动注册
            match self.auto_register().await {
                Ok(new_client_id) => {
                    info!("自动重新注册成功，客户端ID: {}，重试请求...", new_client_id);

                    // 重新从头构建请求，使用新的client_id
                    // 我们需要重新创建请求，因为原来的RequestBuilder已经被消费
                    let retry_request_builder = self
                        .client
                        .get(original_url)
                        .header("X-Client-ID", new_client_id);

                    let retry_response = retry_request_builder.send().await?;
                    Ok(retry_response)
                }
                Err(e) => {
                    error!("自动重新注册失败: {}", e);
                    Err(anyhow::anyhow!("认证失败且无法重新注册: {e}"))
                }
            }
        } else {
            Ok(response)
        }
    }

    /// GET请求
    pub async fn get(&self, url: &str) -> Result<RequestBuilder> {
        self.execute_request(Method::GET, url).await
    }

    /// POST请求
    pub async fn post(&self, url: &str) -> Result<RequestBuilder> {
        self.execute_request(Method::POST, url).await
    }

    /// PUT请求
    pub async fn put(&self, url: &str) -> Result<RequestBuilder> {
        self.execute_request(Method::PUT, url).await
    }

    /// DELETE请求
    pub async fn delete(&self, url: &str) -> Result<RequestBuilder> {
        self.execute_request(Method::DELETE, url).await
    }

    /// POST请求（带JSON）
    pub async fn post_json<T: Serialize>(&self, url: &str, json: &T) -> Result<Response> {
        let request_builder = self
            .execute_request_with_json(Method::POST, url, json)
            .await?;
        self.send_with_retry(request_builder, url).await
    }

    /// PUT请求（带JSON）
    pub async fn put_json<T: Serialize>(&self, url: &str, json: &T) -> Result<Response> {
        let request_builder = self
            .execute_request_with_json(Method::PUT, url, json)
            .await?;
        self.send_with_retry(request_builder, url).await
    }

    /// 发送请求（通用方法）
    pub async fn send(&self, request_builder: RequestBuilder, url: &str) -> Result<Response> {
        self.send_with_retry(request_builder, url).await
    }

    /// 获取原始的reqwest客户端（用于特殊情况）
    pub fn inner(&self) -> &Client {
        &self.client
    }

    /// 获取当前的client_id（只读）
    pub async fn current_client_id(&self) -> Option<String> {
        self.get_client_id().await
    }
}
