use thiserror::Error;

/// Docker 服务相关的错误类型
#[derive(Error, Debug)]
pub enum DockerServiceError {
    #[error("Architecture detection failed: {0}")]
    ArchitectureDetection(String),

    #[error("Image loading failed: {0}")]
    ImageLoading(String),

    #[error("Environment check failed: {0}")]
    EnvironmentCheck(String),

    #[error("Service management failed: {0}")]
    ServiceManagement(String),

    #[error("Directory setup failed: {0}")]
    DirectorySetup(String),

    #[error("Configuration error: {0}")]
    Configuration(String),

    #[error("Health check failed: {0}")]
    HealthCheck(String),

    #[error("Port management failed: {0}")]
    PortManagement(String),

    #[error("Docker command failed: {0}")]
    DockerCommand(String),

    #[error("File system error: {0}")]
    FileSystem(String),

    #[error("Operation timed out: {operation} ({timeout_seconds}s)")]
    Timeout {
        operation: String,
        timeout_seconds: u64,
    },

    #[error("Insufficient resources: {0}")]
    InsufficientResources(String),

    #[error("Missing dependency: {0}")]
    MissingDependency(String),

    #[error("Network error: {0}")]
    Network(String),

    #[error("Permission error: {0}")]
    Permission(String),

    #[error("Unknown error: {0}")]
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
