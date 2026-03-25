use thiserror::Error;

#[derive(Error, Debug)]
pub enum DuckError {
    #[error("{}", t!("error.config"))]
    Config(#[from] toml::de::Error),

    #[error("{}", t!("error.duckdb", details = 0))]
    DuckDb(String),

    #[error("{}", t!("error.http"))]
    Http(#[from] reqwest::Error),

    #[error("{}", t!("error.io"))]
    Io(#[from] std::io::Error),

    #[error("{}", t!("error.uuid"))]
    Uuid(#[from] uuid::Error),

    #[error("{}", t!("error.serde"))]
    Serde(#[from] serde_json::Error),

    #[error("{}", t!("error.join"))]
    Join(#[from] tokio::task::JoinError),

    #[error("{}", t!("error.zip"))]
    Zip(#[from] zip::result::ZipError),

    #[error("{}", t!("error.walkdir"))]
    WalkDir(#[from] walkdir::Error),

    #[error("{}", t!("error.strip_prefix"))]
    StripPrefix(#[from] std::path::StripPrefixError),

    #[error("{}", t!("error.template", details = 0))]
    Template(String),

    #[error("{}", t!("error.docker", details = 0))]
    Docker(String),

    #[error("{}", t!("error.backup", details = 0))]
    Backup(String),

    #[error("{}", t!("error.upgrade", details = 0))]
    Upgrade(String),

    #[error("{}", t!("error.client_not_registered"))]
    ClientNotRegistered,

    #[error("{}", t!("error.invalid_response", details = 0))]
    InvalidResponse(String),

    #[error("{}", t!("error.custom", details = 0))]
    Custom(String),

    #[error("{}", t!("error.config_not_found"))]
    ConfigNotFound,

    #[error("{}", t!("error.api", details = 0))]
    Api(String),

    #[error("{}", t!("error.docker_service", details = 0))]
    DockerService(String),

    #[error("{}", t!("error.bad_request", details = 0))]
    BadRequest(String),

    #[error("{}", t!("error.version_parse", details = 0))]
    VersionParse(String),

    #[error("{}", t!("error.service_upgrade_parse", details = 0))]
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
