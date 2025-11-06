// 模块声明
mod command;
mod config;
mod environment;
mod image;
mod path_utils;
mod service;
pub mod types;
pub mod volumes;

mod modern_docker;

// 重新导出公共API
pub use environment::{
    HostOs, PathFormat, RuntimeEnvironment, detect_runtime_environment,
};
pub use path_utils::{PathProcessor, PathUtilsError};
pub use types::{DockerManager, ServiceConfig, ServiceInfo, ServiceStatus};

