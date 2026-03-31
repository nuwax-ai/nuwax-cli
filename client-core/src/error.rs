use thiserror::Error;

#[derive(Error, Debug)]
pub enum DuckError {
    #[error("Configuration parse error: {0}")]
    Config(#[from] toml::de::Error),

    #[error("DuckDB error: {0}")]
    DuckDb(String),

    #[error("HTTP error: {0}")]
    Http(#[from] reqwest::Error),

    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),

    #[error("UUID error: {0}")]
    Uuid(#[from] uuid::Error),

    #[error("Serialization error: {0}")]
    Serde(#[from] serde_json::Error),

    #[error("Task join error: {0}")]
    Join(#[from] tokio::task::JoinError),

    #[error("ZIP error: {0}")]
    Zip(#[from] zip::result::ZipError),

    #[error("Directory walk error: {0}")]
    WalkDir(#[from] walkdir::Error),

    #[error("Path strip-prefix error: {0}")]
    StripPrefix(#[from] std::path::StripPrefixError),

    #[error("Template error: {0}")]
    Template(String),

    #[error("Docker error: {0}")]
    Docker(String),

    #[error("Backup error: {0}")]
    Backup(String),

    #[error("Upgrade error: {0}")]
    Upgrade(String),

    #[error("Client is not registered")]
    ClientNotRegistered,

    #[error("Invalid response: {0}")]
    InvalidResponse(String),

    #[error("Custom error: {0}")]
    Custom(String),

    #[error("Configuration file not found")]
    ConfigNotFound,

    #[error("API error: {0}")]
    Api(String),

    #[error("Docker service error: {0}")]
    DockerService(String),

    #[error("Bad request: {0}")]
    BadRequest(String),

    #[error("Version parse error: {0}")]
    VersionParse(String),

    #[error("Service upgrade parse error: {0}")]
    ServiceUpgradeParse(String),
}

// 为DuckDB错误实现From trait
impl From<duckdb::Error> for DuckError {
    fn from(err: duckdb::Error) -> Self {
        DuckError::DuckDb(err.to_string())
    }
}

#[cfg(feature = "indicatif")]
impl From<indicatif::style::TemplateError> for DuckError {
    fn from(err: indicatif::style::TemplateError) -> Self {
        DuckError::Template(err.to_string())
    }
}

impl DuckError {
    pub fn custom(msg: impl Into<String>) -> Self {
        Self::Custom(msg.into())
    }

    pub fn docker(msg: impl Into<String>) -> Self {
        Self::Docker(msg.into())
    }

    pub fn backup(msg: impl Into<String>) -> Self {
        Self::Backup(msg.into())
    }

    pub fn upgrade(msg: impl Into<String>) -> Self {
        Self::Upgrade(msg.into())
    }

    pub fn docker_service(msg: impl Into<String>) -> Self {
        Self::DockerService(msg.into())
    }
}
