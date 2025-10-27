use thiserror::Error;

/// Docker 服务相关的错误类型
#[derive(Error, Debug)]
pub enum DockerServiceError {
    #[error("架构检测失败: {0}")]
    ArchitectureDetection(String),

    #[error("镜像加载失败: {0}")]
    ImageLoading(String),

    #[error("环境检查失败: {0}")]
    EnvironmentCheck(String),

    #[error("服务管理失败: {0}")]
    ServiceManagement(String),

    #[error("目录设置失败: {0}")]
    DirectorySetup(String),

    #[error("配置错误: {0}")]
    Configuration(String),

    #[error("健康检查失败: {0}")]
    HealthCheck(String),

    #[error("端口管理失败: {0}")]
    PortManagement(String),

    #[error("Docker 命令执行失败: {0}")]
    DockerCommand(String),

    #[error("文件系统错误: {0}")]
    FileSystem(String),

    #[error("超时错误: {operation} 操作超时 ({timeout_seconds}秒)")]
    Timeout {
        operation: String,
        timeout_seconds: u64,
    },

    #[error("资源不足: {0}")]
    InsufficientResources(String),

    #[error("依赖缺失: {0}")]
    MissingDependency(String),

    #[error("网络错误: {0}")]
    Network(String),

    #[error("权限错误: {0}")]
    Permission(String),

    #[error("未知错误: {0}")]
    Unknown(String),
}

/// Docker 服务操作的结果类型
pub type DockerServiceResult<T> = Result<T, DockerServiceError>;

impl From<std::io::Error> for DockerServiceError {
    fn from(err: std::io::Error) -> Self {
        DockerServiceError::FileSystem(err.to_string())
    }
}

impl From<client_core::DuckError> for DockerServiceError {
    fn from(err: client_core::DuckError) -> Self {
        match err {
            client_core::DuckError::Docker(msg) => DockerServiceError::DockerCommand(msg),
            client_core::DuckError::Api(msg) => DockerServiceError::Network(msg),
            client_core::DuckError::Config(err) => {
                DockerServiceError::Configuration(err.to_string())
            }
            client_core::DuckError::Backup(msg) => DockerServiceError::FileSystem(msg),
            client_core::DuckError::Custom(msg) => DockerServiceError::Unknown(msg),
            _ => DockerServiceError::Unknown(err.to_string()),
        }
    }
}

impl From<DockerServiceError> for client_core::DuckError {
    fn from(err: DockerServiceError) -> Self {
        client_core::DuckError::DockerService(err.to_string())
    }
}
