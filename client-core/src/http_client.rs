//! 统一的 HTTP 客户端构建入口。
//!
//! reqwest 默认只信任操作系统的根证书。Linux 上这份证书是 `ca-certificates` 软件包安装的
//! 一批文件，系统里没有它时连客户端都构建不出来。这里在系统根证书之外再合并一份编译进
//! 二进制的 Mozilla 根证书，两个来源一起信任：有系统证书时行为不变，没有时靠内置的这份。
//!
//! 所有发 HTTPS 请求的代码都应从这里取 [`ClientBuilder`]，不要直接调用
//! `reqwest::Client::builder()` 或 `reqwest::Client::new()`。

use reqwest::{Certificate, Client, ClientBuilder};
use std::time::Duration;
use tracing::warn;

/// 返回合并了内置根证书的 [`ClientBuilder`]，超时、User-Agent 等由调用方按需设置。
pub fn client_builder() -> ClientBuilder {
    Client::builder().tls_certs_merge(bundled_root_certificates())
}

/// 构建一个带全局超时的客户端。
pub fn build_client(timeout: Duration) -> Result<Client, reqwest::Error> {
    client_builder().timeout(timeout).build()
}

/// 把内置的 Mozilla 根证书（DER）逐个转成 reqwest 的 [`Certificate`]。
///
/// `Certificate::from_der` 只是包一层字节，正常情况下不会失败；万一失败就跳过那一张并告警，
/// 不让单张证书的问题影响其余根证书。
fn bundled_root_certificates() -> Vec<Certificate> {
    webpki_root_certs::TLS_SERVER_ROOT_CERTS
        .iter()
        .filter_map(|der| match Certificate::from_der(der.as_ref()) {
            Ok(certificate) => Some(certificate),
            Err(error) => {
                warn!(
                    error = %error,
                    "Skipping a built-in root certificate that reqwest could not load"
                );
                None
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_bundled_root_certificate_loads() {
        let certificates = bundled_root_certificates();

        assert_eq!(
            certificates.len(),
            webpki_root_certs::TLS_SERVER_ROOT_CERTS.len()
        );
        assert!(!certificates.is_empty());
    }

    #[test]
    fn client_builds_with_bundled_roots_merged() {
        // reqwest 在 build 时才会把每张根证书交给 rustls 解析，任何一张有问题这里都会失败
        let client = build_client(Duration::from_secs(1)).expect("HTTP client should build");

        drop(client);
    }
}
