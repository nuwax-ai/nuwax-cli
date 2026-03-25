// 国际化支持
#[macro_use]
extern crate rust_i18n;
i18n!("../locales", fallback = "en");

// 私有模块声明
mod app;
mod cli;
mod commands;
mod docker_service;
mod docker_utils;
mod init;
pub mod project_info; // 公开项目信息模块
pub mod ui_support; // 公开UI支持模块
mod utils;

// 通过 pub use 精确控制对外暴露的接口
pub use app::CliApp;
pub use cli::{Cli, Commands};
pub use commands::{
    auto_upgrade_deploy::check_and_install_nuwax_cli_update_early, run_diff_sql,
    run_status_details, show_client_version,
}; // 导出status相关函数和diff-sql函数
pub use docker_service::{
    ContainerStatus, DockerService, DockerServiceManager, get_architecture_suffix,
    get_system_architecture, health_check,
};
pub use init::run_init;
pub use utils::{extract_docker_service, setup_logging}; // 导出解压函数和匹配器

// 重新导出核心功能
pub use client_core::{config_manager::ConfigManager, database_manager::DatabaseManager};

// 导出UI支持函数和类型
pub use ui_support::*;

// 导出国际化宏和函数
pub use rust_i18n::{set_locale, t};
