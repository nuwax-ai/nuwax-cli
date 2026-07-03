// 国际化支持
#[macro_use]
extern crate rust_i18n;

// 初始化 i18n，fallback 到英文
i18n!("../locales", fallback = ["en", "zh-CN"]);

pub mod api;
pub mod api_config;
pub mod api_types;

pub mod utils;

// 环境检测模块
pub mod environment;

// 重新导出 api_types 中的主要类型以保持向后兼容
pub use api_types::*;
pub mod architecture;
pub mod authenticated_client;
pub mod backup;
pub mod config;
pub mod config_manager;
pub mod constants;
pub mod container;
pub mod database;
pub mod database_manager;
pub mod db;
pub mod downloader;
pub mod error;
pub mod mysql_executor;
pub mod patch_executor;
pub mod sql_diff;
pub mod upgrade;
pub mod upgrade_strategy;
pub mod version;
pub mod release_version;

pub use release_version::ReleaseVersion;
pub use database_manager::DatabaseManager;
pub use error::*;

// 导出 i18n 相关
pub use rust_i18n::{set_locale, t};

// 测试环境下自动初始化 rustls ring 加密提供者
// 确保所有测试中的 reqwest 客户端可以正常进行 HTTPS 通信
#[cfg(test)]
#[ctor::ctor(unsafe)]
fn init_test_crypto_provider() {
    let _ = rustls::crypto::ring::default_provider().install_default();
}
