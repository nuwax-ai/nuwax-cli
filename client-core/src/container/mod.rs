// 模块声明
mod command;
mod config;
mod environment;
mod image;
mod path_utils;
mod service;
pub mod types;
pub mod volumes;

// 重新导出公共API
pub use environment::{
    ComposeCommandType, HostOs, PathFormat, RuntimeEnvironment, detect_compose_command_type,
    detect_runtime_environment, get_compose_command_type, set_compose_command_type,
};
pub use path_utils::{PathProcessor, PathUtilsError};
pub use types::{DockerManager, ServiceConfig, ServiceInfo, ServiceStatus};
