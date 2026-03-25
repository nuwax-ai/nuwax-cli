use thiserror::Error;

/// Docker 服务相关的错误类型
#[derive(Error, Debug)]
pub enum DockerServiceError {
    #[error("{}", t!("docker_error.architecture_detection", details = 0))]
    ArchitectureDetection(String),

    #[error("{}", t!("docker_error.image_loading", details = 0))]
    ImageLoading(String),

    #[error("{}", t!("docker_error.environment_check", details = 0))]
    EnvironmentCheck(String),

    #[error("{}", t!("docker_error.service_management", details = 0))]
    ServiceManagement(String),

    #[error("{}", t!("docker_error.directory_setup", details = 0))]
    DirectorySetup(String),

    #[error("{}", t!("docker_error.configuration", details = 0))]
    Configuration(String),

    #[error("{}", t!("docker_error.health_check", details = 0))]
    HealthCheck(String),

    #[error("{}", t!("docker_error.port_management", details = 0))]
    PortManagement(String),

    #[error("{}", t!("docker_error.docker_command", details = 0))]
    DockerCommand(String),

    #[error("{}", t!("docker_error.file_system", details = 0))]
    FileSystem(String),

    #[error("{}", t!("docker_error.timeout", operation = operation, seconds = timeout_seconds))]
    Timeout {
        operation: String,
        timeout_seconds: u64,
    },

    #[error("{}", t!("docker_error.insufficient_resources", details = 0))]
    InsufficientResources(String),

    #[error("{}", t!("docker_error.missing_dependency", details = 0))]
    MissingDependency(String),

    #[error("{}", t!("docker_error.network", details = 0))]
    Network(String),

    #[error("{}", t!("docker_error.permission", details = 0))]
    Permission(String),

    #[error("{}", t!("docker_error.unknown", details = 0))]
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
