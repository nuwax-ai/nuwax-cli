/// Nuwax Cli  项目信息模块
///
/// 由于 nuwax-cli 是面向用户的主程序，项目元数据统一在这里定义
/// client-core 作为内部库，只提供技术性常量
///
/// 项目元数据（自动从 nuwax-cli 的 Cargo.toml 同步）
pub mod metadata {
    /// 项目名称（自动从 Cargo.toml 同步）
    pub const PROJECT_NAME: &str = env!("CARGO_PKG_NAME");

    /// 项目描述（自动从 Cargo.toml 同步）
    pub const PROJECT_DESCRIPTION: &str = env!("CARGO_PKG_DESCRIPTION");

    /// 项目作者（自动从 Cargo.toml 同步）
    pub const PROJECT_AUTHORS: &str = env!("CARGO_PKG_AUTHORS");

    /// 项目许可证（自动从 Cargo.toml 同步）
    pub const PROJECT_LICENSE: &str = env!("CARGO_PKG_LICENSE");

    /// 项目仓库地址（自动从 Cargo.toml 同步）
    pub const PROJECT_REPOSITORY: &str = env!("CARGO_PKG_REPOSITORY");

    /// 项目主页（自动从 Cargo.toml 同步）
    pub const PROJECT_HOMEPAGE: &str = env!("CARGO_PKG_HOMEPAGE");

    /// 项目文档地址（自动从 Cargo.toml 同步）
    pub const PROJECT_DOCUMENTATION: &str = env!("CARGO_PKG_HOMEPAGE");

    /// 用户友好的显示名称（手动维护，用于 UI 显示）
    pub mod display {
        /// 用户友好的项目名称
        pub const FRIENDLY_NAME: &str = "Nuwax Cli ent";

        /// CLI 工具的完整名称
        pub const CLI_FULL_NAME: &str = "Nuwax Cli ent CLI";

        /// 项目详细描述（比 Cargo.toml 中的描述更详细）
        pub const DESCRIPTION_LONG: &str = "一个自动化的 Docker 服务管理与升级平台客户端，支持 Docker Compose 服务的集中管理、自动备份、智能升级和运维监控";
    }

    /// 项目关键词
    pub const PROJECT_KEYWORDS: &[&str] = &[
        "docker",
        "service-management",
        "automation",
        "deployment",
        "backup",
        "upgrade",
        "monitoring",
    ];

    /// 项目分类
    pub const PROJECT_CATEGORIES: &[&str] = &[
        "command-line-utilities",
        "development-tools",
        "containerization",
    ];
}

/// 版本信息
pub mod version_info {
    /// CLI 版本（自动从 Cargo.toml 同步）
    pub const CLI_VERSION: &str = env!("CARGO_PKG_VERSION");

    /// 核心库版本（从 client-core 获取）
    pub const CORE_VERSION: &str = client_core::constants::version::version_info::CORE_VERSION;

    /// Docker 服务版本（从 client-core 获取）
    pub const DOCKER_SERVICE_VERSION: &str =
        client_core::constants::version::version_info::DEFAULT_DOCKER_SERVICE_VERSION;
}

/// 项目完整信息结构
#[derive(Debug, Clone)]
pub struct ProjectInfo {
    pub name: &'static str,
    pub full_name: &'static str,
    pub description: &'static str,
    pub description_long: &'static str,
    pub version: &'static str,
    pub authors: &'static str,
    pub license: &'static str,
    pub repository: &'static str,
    pub homepage: &'static str,
    pub documentation: &'static str,
    pub keywords: &'static [&'static str],
    pub categories: &'static [&'static str],
}

/// 获取完整的项目信息
pub fn get_project_info() -> ProjectInfo {
    ProjectInfo {
        name: metadata::PROJECT_NAME,
        full_name: metadata::display::CLI_FULL_NAME,
        description: metadata::PROJECT_DESCRIPTION,
        description_long: metadata::display::DESCRIPTION_LONG,
        version: version_info::CLI_VERSION,
        authors: metadata::PROJECT_AUTHORS,
        license: metadata::PROJECT_LICENSE,
        repository: metadata::PROJECT_REPOSITORY,
        homepage: metadata::PROJECT_HOMEPAGE,
        documentation: metadata::PROJECT_DOCUMENTATION,
        keywords: metadata::PROJECT_KEYWORDS,
        categories: metadata::PROJECT_CATEGORIES,
    }
}

/// 获取版本信息字符串
pub fn get_version_string() -> String {
    format!(
        "{} v{}",
        metadata::display::FRIENDLY_NAME,
        version_info::CLI_VERSION
    )
}

/// 获取完整的版本信息字符串（包含描述）
pub fn get_full_version_string() -> String {
    format!(
        "{} v{}\n{}",
        metadata::display::CLI_FULL_NAME,
        version_info::CLI_VERSION,
        metadata::PROJECT_DESCRIPTION
    )
}

/// 获取作者和许可证信息
pub fn get_copyright_info() -> String {
    format!(
        "© {} - Licensed under {}",
        metadata::PROJECT_AUTHORS,
        metadata::PROJECT_LICENSE
    )
}

/// 获取系统要求信息
pub fn get_system_requirements() -> SystemRequirements {
    use client_core::constants::version::version_info as core_version;

    SystemRequirements {
        min_docker_version: core_version::MIN_DOCKER_VERSION,
        min_compose_version: core_version::MIN_COMPOSE_VERSION,
        api_version: core_version::API_VERSION,
        config_format_version: core_version::CONFIG_FORMAT_VERSION,
        database_schema_version: core_version::DATABASE_SCHEMA_VERSION,
    }
}

/// 系统要求信息结构
#[derive(Debug, Clone)]
pub struct SystemRequirements {
    pub min_docker_version: &'static str,
    pub min_compose_version: &'static str,
    pub api_version: &'static str,
    pub config_format_version: &'static str,
    pub database_schema_version: &'static str,
}
