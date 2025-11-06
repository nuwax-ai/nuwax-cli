use crate::project_info::{metadata, version_info};
use clap::{Args, Parser, Subcommand};
use std::path::PathBuf;

/// 升级相关参数
#[derive(Args, Debug)]
pub struct UpgradeArgs {
    /// 强制重新下载（用于文件损坏时）,会重新下载完整的服务包
    #[arg(long)]
    pub force: bool,

    /// 只检查是否有可用的升级版本，不执行下载
    #[arg(long)]
    pub check: bool,
}

/// 自动备份相关命令
#[derive(Subcommand, Debug)]
pub enum AutoBackupCommand {
    /// 立即执行一次手动备份
    Run,
    /// 显示备份状态和历史记录
    Status,
}

/// 自动升级部署相关命令
#[derive(Subcommand, Debug)]
pub enum AutoUpgradeDeployCommand {
    /// 立即执行自动升级部署
    Run {
        /// 指定frontend服务的端口号（默认80端口）
        #[arg(
            long,
            help = "指定frontend服务的端口号，对应docker-compose.yml中的FRONTEND_HOST_PORT变量（默认: 80端口）"
        )]
        port: Option<u16>,
        /// 指定自定义的docker-compose配置文件路径
        #[arg(
            long,
            help = "指定自定义的docker-compose配置文件路径（默认: docker/docker-compose.yml）"
        )]
        config: Option<PathBuf>,
        /// 指定docker-compose的项目名称
        #[arg(
            short = 'p',
            long,
            help = "指定docker-compose的项目名称（默认: 从compose文件读取或使用'docker'）"
        )]
        project: Option<String>,
    },
    /// 显示当前自动升级配置
    Status,
}

/// 客户端更新相关命令
#[derive(Subcommand, Debug)]
pub enum CheckUpdateCommand {
    /// 检查最新版本信息
    Check,
    /// 安装指定版本或最新版本
    Install {
        /// 指定版本号（如不指定则安装最新版本）
        #[arg(long)]
        version: Option<String>,
        /// 强制重新安装（即使当前已是最新版本）
        #[arg(long)]
        force: bool,
    },
}

#[derive(Subcommand, Debug)]
pub enum DockerServiceCommand {
    /// 启动Docker服务
    Start {
        /// 指定docker-compose的项目名称
        #[arg(
            short = 'p',
            long,
            help = "指定docker-compose的项目名称（默认: 从compose文件读取或使用'docker'）"
        )]
        project: Option<String>,
    },
    /// 停止Docker服务
    Stop {
        /// 指定docker-compose的项目名称
        #[arg(
            short = 'p',
            long,
            help = "指定docker-compose的项目名称（默认: 从compose文件读取或使用'docker'）"
        )]
        project: Option<String>,
    },
    /// 重启Docker服务
    Restart {
        /// 指定docker-compose的项目名称
        #[arg(
            short = 'p',
            long,
            help = "指定docker-compose的项目名称（默认: 从compose文件读取或使用'docker'）"
        )]
        project: Option<String>,
    },
    /// 检查服务状态
    Status {
        /// 指定docker-compose的项目名称
        #[arg(
            short = 'p',
            long,
            help = "指定docker-compose的项目名称（默认: 从compose文件读取或使用'docker'）"
        )]
        project: Option<String>,
    },
    /// 重启指定容器
    RestartContainer {
        /// 容器名称
        container_name: String,
    },
    /// 加载Docker镜像
    LoadImages,
    /// 设置镜像标签
    SetupTags,
    /// 显示架构信息
    ArchInfo,
    /// 列出Docker镜像（使用ducker）
    ListImages,
    /// 检查并创建docker-compose.yml中的挂载目录
    CheckMountDirs,
}

/// 缓存管理相关命令
#[derive(Subcommand, Debug)]
pub enum CacheCommand {
    /// 清理所有缓存文件
    Clear,
    /// 显示缓存使用情况
    Status,
    /// 清理下载缓存（保留最新版本）
    CleanDownloads {
        /// 保留的版本数量
        #[arg(long, default_value = "3", help = "保留的版本数量")]
        keep: u32,
    },
}

/// Nuwax Cli ent CLI - Docker 服务管理和升级工具
#[derive(Parser)]
#[command(name = "nuwax-cli")]
#[command(about = metadata::PROJECT_DESCRIPTION)]
#[command(version = version_info::CLI_VERSION)]
#[command(long_about = metadata::display::DESCRIPTION_LONG)]
#[command(author = metadata::PROJECT_AUTHORS)]
pub struct Cli {
    /// 配置文件路径
    #[arg(short, long, default_value = "config.toml")]
    pub config: PathBuf,

    /// 详细输出
    #[arg(short, long)]
    pub verbose: bool,

    #[command(subcommand)]
    pub command: Commands,
}

#[derive(Subcommand)]
pub enum Commands {
    /// 显示服务状态和版本信息
    Status,
    /// 首次使用时初始化客户端，创建配置文件和数据库
    Init {
        /// 如果配置文件已存在，强制覆盖
        #[arg(long)]
        force: bool,
    },
    /// 检查客户端更新
    #[command(subcommand)]
    CheckUpdate(CheckUpdateCommand),
    /// 显示当前API配置信息
    ApiInfo,
    /// 下载Docker服务文件
    Upgrade {
        #[command(flatten)]
        args: UpgradeArgs,
    },
    /// 列出所有备份
    ListBackups,
    /// 从备份恢复
    Rollback {
        /// 备份 ID（可选，不提供时将显示交互式选择界面）
        backup_id: Option<i64>,
        /// 强制覆盖
        #[arg(long)]
        force: bool,
        /// 输出 JSON 格式的备份列表（用于 GUI 集成）
        #[arg(long)]
        list_json: bool,
        /// 是否回滚数据,默认不会滚数据文件
        #[arg(long, default_value = "false", help = "是否回滚数据文件，默认不回滚")]
        rollback_data: bool,
    },
    /// Docker服务相关命令
    #[command(subcommand)]
    DockerService(DockerServiceCommand),

    /// 🐋 一个用于管理 Docker 容器的终端应用
    Ducker {
        /// 传递给ducker的参数
        #[arg(trailing_var_arg = true, allow_hyphen_values = true)]
        args: Vec<String>,
    },

    /// 自动备份管理
    #[command(subcommand)]
    AutoBackup(AutoBackupCommand),

    /// 自动升级部署
    #[command(subcommand)]
    AutoUpgradeDeploy(AutoUpgradeDeployCommand),

    /// 缓存管理
    #[command(subcommand)]
    Cache(CacheCommand),

    /// 对比两个SQL文件并生成差异SQL
    DiffSql {
        /// 旧版本SQL文件路径
        #[arg(help = "旧版本SQL文件路径")]
        old_sql: PathBuf,
        /// 新版本SQL文件路径
        #[arg(help = "新版本SQL文件路径")]
        new_sql: PathBuf,
        /// 旧版本号（可选）
        #[arg(long, help = "旧版本号，用于生成差异描述")]
        old_version: Option<String>,
        /// 新版本号（可选）
        #[arg(long, help = "新版本号，用于生成差异描述")]
        new_version: Option<String>,
        /// 输出文件名（可选，默认为upgrade_diff.sql）
        #[arg(long, default_value = "upgrade_diff.sql", help = "差异SQL输出文件名")]
        output: String,
    },
}
