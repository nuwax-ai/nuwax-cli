/// Docker相关路径常量
pub mod docker {
    use std::path::{Path, PathBuf};

    /// docker-compose.yml文件名
    pub const COMPOSE_FILE_NAME: &str = "docker-compose.yml";

    /// Docker工作目录名
    pub const DOCKER_DIR_NAME: &str = "docker";

    /// 环境变量文件名
    pub const ENV_FILE_NAME: &str = ".env";

    /// Docker镜像目录名
    pub const IMAGES_DIR_NAME: &str = "images";

    /// 数据目录名
    pub const DATA_DIR_NAME: &str = "data";

    /// 应用程序目录名
    pub const APP_DIR_NAME: &str = "app";

    /// 配置目录名
    pub const CONFIG_DIR_NAME: &str = "config";

    /// 上传目录名
    pub const UPLOAD_DIR_NAME: &str = "upload";

    /// 备份目录名
    pub const BACKUPS_DIR_NAME: &str = "backups";

    /// 日志目录名
    pub const LOGS_DIR_NAME: &str = "logs";

    /// 服务数据目录结构
    pub mod data_dirs {
        /// MySQL数据目录
        pub const MYSQL_DATA_DIR: &str = "data/mysql";

        /// Redis数据目录
        pub const REDIS_DATA_DIR: &str = "data/redis";

        /// Milvus数据目录
        pub const MILVUS_DATA_DIR: &str = "data/milvus";

        /// Milvus数据存储目录
        pub const MILVUS_DATA_STORAGE_DIR: &str = "data/milvus/data";

        /// Milvus etcd数据目录
        pub const MILVUS_ETCD_DATA_DIR: &str = "data/milvus/etcd";
    }

    /// 服务日志目录结构
    pub mod log_dirs {
        /// Agent日志目录
        pub const AGENT_LOG_DIR: &str = "logs/agent";

        /// MySQL日志目录
        pub const MYSQL_LOG_DIR: &str = "logs/mysql";

        /// Redis日志目录
        pub const REDIS_LOG_DIR: &str = "logs/redis";

        /// Milvus日志目录
        pub const MILVUS_LOG_DIR: &str = "logs/milvus";
    }

    /// 服务端口相关常量
    pub mod ports {
        /// 默认frontend服务端口
        pub const DEFAULT_FRONTEND_PORT: u16 = 80;

        /// 默认backend服务端口
        pub const DEFAULT_BACKEND_PORT: u16 = 8080;

        /// 默认backend调试端口
        pub const DEFAULT_BACKEND_DEBUG_PORT: u16 = 5005;

        /// 默认MySQL端口
        pub const DEFAULT_MYSQL_PORT: u16 = 3306;

        /// 默认Redis端口
        pub const DEFAULT_REDIS_PORT: u16 = 6379;

        /// 默认Milvus端口
        pub const DEFAULT_MILVUS_PORT: u16 = 19530;

        /// 默认Milvus管理端口
        pub const DEFAULT_MILVUS_MANAGEMENT_PORT: u16 = 9091;

        /// 默认etcd端口
        pub const DEFAULT_ETCD_PORT: u16 = 2379;

        /// 默认MinIO API端口
        pub const DEFAULT_MINIO_API_PORT: u16 = 9000;

        /// 默认MinIO控制台端口
        pub const DEFAULT_MINIO_CONSOLE_PORT: u16 = 9001;

        /// 默认日志平台端口
        pub const DEFAULT_LOG_PLATFORM_PORT: u16 = 8097;

        /// 默认Quickwit端口
        pub const DEFAULT_QUICKWIT_PORT: u16 = 7280;

        /// 默认Quickwit管理端口
        pub const DEFAULT_QUICKWIT_ADMIN_PORT: u16 = 7281;

        /// 默认视频分析主服务端口
        pub const DEFAULT_VIDEO_ANALYSIS_MASTER_PORT: u16 = 8989;

        /// 默认MCP代理端口
        pub const DEFAULT_MCP_PROXY_PORT: u16 = 8020;
    }

    /// 环境变量名称常量
    pub mod env_vars {
        /// Frontend服务主机端口环境变量
        pub const FRONTEND_HOST_PORT: &str = "FRONTEND_HOST_PORT";

        /// Backend应用端口环境变量
        pub const APP_PORT: &str = "APP_PORT";

        /// Backend调试端口环境变量
        pub const APP_DEBUG_PORT: &str = "APP_DEBUG_PORT";

        /// MySQL端口环境变量
        pub const MYSQL_PORT: &str = "MYSQL_PORT";

        /// Redis端口环境变量
        pub const REDIS_PORT: &str = "REDIS_PORT";

        /// Milvus端口环境变量
        pub const MILVUS_PORT: &str = "MILVUS_PORT";

        /// 日志平台主机端口环境变量
        pub const LOG_PLATFORM_HOST_PORT: &str = "LOG_PLATFORM_HOST_PORT";

        /// 视频分析主服务主机端口环境变量
        pub const VIDEO_ANALYSIS_MASTER_HOST_PORT: &str = "VIDEO_ANALYSIS_MASTER_HOST_PORT";

        /// 主应用端口环境变量（视频分析）
        pub const MASTER_APP_PORT: &str = "MASTER_APP_PORT";
    }

    /// Docker socket路径（跨平台支持）
    /// Unix/Linux/macOS: /var/run/docker.sock
    /// Windows: \\.\pipe\docker_engine
    #[cfg(unix)]
    pub const DOCKER_SOCKET_PATH: &str = "/var/run/docker.sock";

    #[cfg(windows)]
    pub const DOCKER_SOCKET_PATH: &str = r"\\.\pipe\docker_engine";

    /// 获取默认的docker-compose.yml文件路径（跨平台）
    pub fn get_compose_file_path() -> PathBuf {
        Path::new(".").join(DOCKER_DIR_NAME).join(COMPOSE_FILE_NAME)
    }

    /// 获取Docker工作目录路径（跨平台）
    pub fn get_docker_work_dir() -> PathBuf {
        Path::new(".").join(DOCKER_DIR_NAME)
    }

    /// 获取默认compose文件路径的字符串表示（用于向后兼容）
    pub fn get_compose_file_path_str() -> String {
        get_compose_file_path().to_string_lossy().to_string()
    }

    /// 获取环境变量文件路径（跨平台）
    pub fn get_env_file_path() -> PathBuf {
        Path::new(".").join(DOCKER_DIR_NAME).join(ENV_FILE_NAME)
    }

    /// 获取环境变量文件路径的字符串表示（用于向后兼容）
    pub fn get_env_file_path_str() -> String {
        get_env_file_path().to_string_lossy().to_string()
    }

    /// 获取Docker镜像目录路径（跨平台）
    pub fn get_images_dir_path() -> PathBuf {
        Path::new(".").join(DOCKER_DIR_NAME).join(IMAGES_DIR_NAME)
    }

    /// 获取数据目录路径（跨平台）
    pub fn get_data_dir_path() -> PathBuf {
        Path::new(".").join(DOCKER_DIR_NAME).join(DATA_DIR_NAME)
    }

    /// 获取应用程序目录路径（跨平台）
    pub fn get_app_dir_path() -> PathBuf {
        Path::new(".").join(DOCKER_DIR_NAME).join(APP_DIR_NAME)
    }

    /// 获取配置目录路径（跨平台）
    pub fn get_config_dir_path() -> PathBuf {
        Path::new(".").join(DOCKER_DIR_NAME).join(CONFIG_DIR_NAME)
    }

    /// 获取上传目录路径（跨平台）
    pub fn get_upload_dir_path() -> PathBuf {
        Path::new(".").join(DOCKER_DIR_NAME).join(UPLOAD_DIR_NAME)
    }

    /// 获取备份目录路径（跨平台）
    pub fn get_backups_dir_path() -> PathBuf {
        Path::new(".").join(DOCKER_DIR_NAME).join(BACKUPS_DIR_NAME)
    }

    /// 获取日志目录路径（跨平台）
    pub fn get_logs_dir_path() -> PathBuf {
        Path::new(".").join(DOCKER_DIR_NAME).join(LOGS_DIR_NAME)
    }

    /// 获取所有必需的Docker服务目录列表
    pub fn get_all_required_directories() -> Vec<&'static str> {
        vec![
            DATA_DIR_NAME,
            APP_DIR_NAME,
            data_dirs::MYSQL_DATA_DIR,
            data_dirs::REDIS_DATA_DIR,
            data_dirs::MILVUS_DATA_DIR,
            data_dirs::MILVUS_DATA_STORAGE_DIR,
            data_dirs::MILVUS_ETCD_DATA_DIR,
            LOGS_DIR_NAME,
            log_dirs::AGENT_LOG_DIR,
            log_dirs::MYSQL_LOG_DIR,
            log_dirs::REDIS_LOG_DIR,
            log_dirs::MILVUS_LOG_DIR,
            UPLOAD_DIR_NAME,
            CONFIG_DIR_NAME,
            BACKUPS_DIR_NAME,
        ]
    }
}

/// API服务相关常量
pub mod api {

    ///TODO: 默认API服务器地址
    pub const DEFAULT_BASE_URL: &str = "https://api-version.nuwax.com";
    // pub const DEFAULT_BASE_URL: &str = "http://192.168.2.244:3000";

    /// API版本前缀
    pub const VERSION_PREFIX: &str = "/api/v1";

    /// API端点路径
    pub mod endpoints {
        /// 客户端注册端点
        pub const CLIENT_REGISTER: &str = "/api/v1/clients/register";

        /// 公告获取端点
        pub const ANNOUNCEMENTS: &str = "/api/v1/clients/announcements";

        /// Docker版本检查端点
        pub const DOCKER_CHECK_VERSION: &str = "/api/v1/docker/checkVersion";

        /// Docker版本列表更新端点
        pub const DOCKER_UPDATE_VERSION_LIST: &str = "/api/v1/docker/updateVersionList";

        /// Docker完整服务包下载端点
        pub const DOCKER_DOWNLOAD_FULL: &str =
            "/api/v1/clients/downloads/docker/services/full/latest";

        /// 客户端自升级历史端点
        pub const CLIENT_SELF_UPGRADE_HISTORY: &str = "/api/v1/clients/self-upgrade-history";

        /// 服务升级历史端点（包含占位符）
        pub const SERVICE_UPGRADE_HISTORY: &str =
            "/api/v1/clients/services/{service_name}/upgrade-history";

        /// 遥测数据上报端点
        pub const TELEMETRY: &str = "/api/v1/clients/telemetry";

        /// OpenAPI文档端点
        pub const OPENAPI_DOCS: &str = "/api-docs/openapi.json";
    }

    /// HTTP相关常量
    pub mod http {
        /// 默认连接超时时间（秒）
        pub const DEFAULT_TIMEOUT: u64 = 30;

        /// 默认重试次数
        pub const DEFAULT_RETRY_COUNT: u8 = 3;

        /// User-Agent头
        pub const USER_AGENT: &str = "nuwax-cli/1.0";
    }
}

/// 备份相关常量
pub mod backup {
    use std::path::{Path, PathBuf};

    /// 数据目录名
    pub const DATA_DIR_NAME: &str = "data";

    /// 备份目录名
    pub const BACKUP_DIR_NAME: &str = "backups";

    /// 备份文件前缀
    pub const BACKUP_PREFIX: &str = "backup_";

    /// 备份文件扩展名
    pub const BACKUP_EXTENSION: &str = ".zip";

    /// 最小有效ZIP文件大小（字节）
    pub const MIN_ZIP_FILE_SIZE: u64 = 100;

    /// 获取默认备份目录路径（跨平台）
    pub fn get_backup_dir() -> PathBuf {
        Path::new(".").join(DATA_DIR_NAME).join(BACKUP_DIR_NAME)
    }

    /// 获取默认备份存储目录（用于配置）
    pub fn get_default_storage_dir() -> PathBuf {
        Path::new(".").join(BACKUP_DIR_NAME)
    }
}

/// 更新升级相关常量
pub mod upgrade {
    use std::path::{Path, PathBuf};

    /// 数据目录名
    pub const DATA_DIR_NAME: &str = "data";

    /// 下载目录名
    pub const DOWNLOAD_DIR_NAME: &str = "downloads";

    /// 临时目录名
    pub const TEMP_DIR_NAME: &str = "temp";

    /// 下载的docker服务包文件名,老版本的文件名,包含x86_64 和 arm64 的docker镜像
    pub const DOCKER_SERVICE_PACKAGE: &str = "docker.zip";
    /// 下载的docker服务包文件名（arm64）
    pub const DOCKER_SERVICE_AARCH64_PACKAGE: &str = "docker-aarch64.zip";
    /// 下载的docker服务包文件名（x86_64）
    pub const DOCKER_SERVICE_X86_64_PACKAGE: &str = "docker-x86_64.zip";

    /// 默认更新包文件名
    pub const DEFAULT_UPDATE_PACKAGE: &str = "update.zip";

    /// 获取下载文件保存目录（跨平台）
    pub fn get_download_dir() -> PathBuf {
        Path::new(".").join(DATA_DIR_NAME).join(DOWNLOAD_DIR_NAME)
    }

    /// 获取临时解压目录（跨平台）
    pub fn get_temp_extract_dir() -> PathBuf {
        Path::new(".").join(DATA_DIR_NAME).join(TEMP_DIR_NAME)
    }
}

/// 文件格式相关常量
pub mod file_format {
    /// ZIP文件扩展名
    pub const ZIP_EXTENSION: &str = ".zip";

    /// TOML配置文件扩展名
    pub const TOML_EXTENSION: &str = ".toml";

    /// 数据库文件扩展名
    pub const DB_EXTENSION: &str = ".db";

    /// ZIP文件魔术字节 - 本地文件头
    pub const ZIP_MAGIC_LOCAL_HEADER: [u8; 4] = [0x50, 0x4B, 0x03, 0x04];

    /// ZIP文件魔术字节 - 中央目录结束记录
    pub const ZIP_MAGIC_CENTRAL_DIR_END: [u8; 4] = [0x50, 0x4B, 0x05, 0x06];

    /// ZIP文件魔术字节 - 数据描述符
    pub const ZIP_MAGIC_DATA_DESCRIPTOR: [u8; 4] = [0x50, 0x4B, 0x07, 0x08];

    /// ZIP文件通用魔术字节前缀（PK）
    pub const ZIP_MAGIC_PK_PREFIX: [u8; 2] = [0x50, 0x4B];
}

/// 超时时间常量（秒）
pub mod timeout {
    /// Docker服务停止等待超时时间
    pub const SERVICE_STOP_TIMEOUT: u64 = 30;

    /// Docker服务启动等待超时时间
    pub const SERVICE_START_TIMEOUT: u64 = 60;

    /// 升级部署时服务启动等待超时时间（更长，因为部署后启动需要更多时间）
    pub const DEPLOY_START_TIMEOUT: u64 = 90;

    /// Docker服务状态检查间隔时间
    pub const SERVICE_CHECK_INTERVAL: u64 = 2;

    /// Docker服务健康检查超时时间（用于启动后的健康检查）
    pub const HEALTH_CHECK_TIMEOUT: u64 = 180;

    /// Docker服务健康检查间隔时间
    pub const HEALTH_CHECK_INTERVAL: u64 = 5;

    /// 服务重启间隔等待时间
    pub const RESTART_INTERVAL: u64 = 2;

    /// 服务验证前等待时间（让服务稳定）
    pub const SERVICE_VERIFY_WAIT: u64 = 5;
}

/// 网络相关常量
pub mod network {
    /// 本地回环地址
    pub const LOCALHOST_IPV4: &str = "127.0.0.1";

    /// 本地回环地址（IPv6）
    pub const LOCALHOST_IPV6: &str = "::1";

    /// 所有网络接口地址
    pub const ALL_INTERFACES: &str = "0.0.0.0";

    /// Docker端口映射格式示例
    pub const PORT_MAPPING_EXAMPLES: [&str; 3] = ["8080:80", "127.0.0.1:8080:80", "8080:80/tcp"];
}

/// 日志和输出相关常量
pub mod logging {
    use std::path::{Path, PathBuf};

    /// 默认日志级别
    pub const DEFAULT_LOG_LEVEL: &str = "info";

    /// 数据目录名
    pub const DATA_DIR_NAME: &str = "data";

    /// 日志目录名
    pub const LOG_DIR_NAME: &str = "logs";

    /// 获取日志文件保存目录（跨平台）
    pub fn get_log_dir() -> PathBuf {
        Path::new(".").join(DATA_DIR_NAME).join(LOG_DIR_NAME)
    }
}

/// Cron任务相关常量
pub mod cron {
    /// 默认自动备份cron表达式（每天凌晨2点）
    pub const DEFAULT_BACKUP_CRON: &str = "0 2 * * *";

    /// Cron表达式字段数量
    pub const CRON_FIELDS_COUNT: usize = 5;
}

/// 应用配置相关常量
pub mod config {
    use std::path::{Path, PathBuf};

    /// 数据目录名
    pub const DATA_DIR_NAME: &str = "data";

    /// 配置文件名
    pub const CONFIG_FILE_NAME: &str = "config.toml";

    /// 数据库文件名
    pub const DATABASE_FILE_NAME: &str = "duck_client.db";

    /// 缓存目录名
    pub const CACHE_DIR_NAME: &str = "cacheDuckData";

    /// 下载目录名
    pub const DOWNLOAD_DIR_NAME: &str = "download";

    /// 获取默认配置文件路径（跨平台）
    pub fn get_config_file_path() -> PathBuf {
        Path::new(".").join(DATA_DIR_NAME).join(CONFIG_FILE_NAME)
    }

    /// 获取数据库文件路径（跨平台）
    pub fn get_database_path() -> PathBuf {
        Path::new(".").join(DATA_DIR_NAME).join(DATABASE_FILE_NAME)
    }

    /// 获取默认缓存目录（跨平台）
    pub fn get_default_cache_dir() -> PathBuf {
        Path::new(".").join(CACHE_DIR_NAME)
    }

    /// 获取默认下载目录（跨平台）
    pub fn get_default_download_dir() -> PathBuf {
        get_default_cache_dir().join(DOWNLOAD_DIR_NAME)
    }
}

/// 技术版本信息常量
pub mod version {
    /// 版本信息（仅技术版本，项目信息在 nuwax-cli 中定义）
    pub mod version_info {
        /// 核心库版本（自动同步）
        pub const CORE_VERSION: &str = env!("CARGO_PKG_VERSION");

        /// Docker 服务版本（默认，手动维护）
        pub const DEFAULT_DOCKER_SERVICE_VERSION: &str = "0.0.1";

        /// 最小支持的 Docker 版本
        pub const MIN_DOCKER_VERSION: &str = "20.10.0";

        /// 最小支持的 Docker Compose 版本
        pub const MIN_COMPOSE_VERSION: &str = "2.0.0";

        /// API 版本
        pub const API_VERSION: &str = "v1";

        /// 配置格式版本
        pub const CONFIG_FORMAT_VERSION: &str = "1.0";

        /// 数据库架构版本
        pub const DATABASE_SCHEMA_VERSION: &str = "1.0";
    }
}

/// 更新检查相关常量
pub mod updates {
    /// 默认检查频率
    pub const DEFAULT_CHECK_FREQUENCY: &str = "daily";
}
