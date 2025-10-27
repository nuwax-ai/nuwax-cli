use thiserror::Error;

#[derive(Error, Debug)]
pub enum DuckError {
    #[error("配置错误: {0}")]
    Config(#[from] toml::de::Error),

    #[error("DuckDB数据库错误: {0}")]
    DuckDb(String),

    #[error("HTTP 请求错误: {0}")]
    Http(#[from] reqwest::Error),

    #[error("IO 错误: {0}")]
    Io(#[from] std::io::Error),

    #[error("UUID 错误: {0}")]
    Uuid(#[from] uuid::Error),

    #[error("序列化错误: {0}")]
    Serde(#[from] serde_json::Error),

    #[error("任务执行错误: {0}")]
    Join(#[from] tokio::task::JoinError),

    #[error("ZIP 文件错误: {0}")]
    Zip(#[from] zip::result::ZipError),

    #[error("目录遍历错误: {0}")]
    WalkDir(#[from] walkdir::Error),

    #[error("路径错误: {0}")]
    StripPrefix(#[from] std::path::StripPrefixError),

    #[error("进度条模板错误: {0}")]
    Template(String),

    #[error("Docker 命令执行失败: {0}")]
    Docker(String),

    #[error("备份操作失败: {0}")]
    Backup(String),

    #[error("升级操作失败: {0}")]
    Upgrade(String),

    #[error("客户端未注册")]
    ClientNotRegistered,

    #[error("服务端响应无效: {0}")]
    InvalidResponse(String),

    #[error("自定义错误: {0}")]
    Custom(String),

    #[error("配置文件未找到")]
    ConfigNotFound,

    #[error("API请求失败: {0}")]
    Api(String),

    #[error("Docker服务错误: {0}")]
    DockerService(String),

    #[error("Bad Request: {0}")]
    BadRequest(String),

    #[error("版本号解析错误: {0}")]
    VersionParse(String),

    #[error("应用服务升级解析失败: {0}")]
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
