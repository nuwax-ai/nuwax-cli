use crate::project_info::{metadata, version_info};
use clap::{Args, Parser, Subcommand};
use std::path::PathBuf;

/// Upgrade related parameters
#[derive(Args, Debug)]
pub struct UpgradeArgs {
    /// Force re-download (for corrupted files), will download complete service package
    #[arg(long)]
    pub force: bool,

    /// Only check for available upgrade versions, do not download
    #[arg(long)]
    pub check: bool,
}

/// Auto backup related commands
#[derive(Subcommand, Debug)]
pub enum AutoBackupCommand {
    /// Execute a manual backup immediately
    Run,
    /// Show backup status and history
    Status,
}

/// Auto upgrade deploy related commands
#[derive(Subcommand, Debug)]
pub enum AutoUpgradeDeployCommand {
    /// Execute auto upgrade deploy immediately
    Run {
        /// Specify frontend service port (default: port 80)
        #[arg(
            long,
            help = "Specify frontend service port, corresponds to FRONTEND_HOST_PORT variable in docker-compose.yml (default: port 80)"
        )]
        port: Option<u16>,
        /// Specify custom docker-compose config file path
        #[arg(
            long,
            help = "Specify custom docker-compose config file path (default: docker/docker-compose.yml)"
        )]
        config: Option<PathBuf>,
        /// Specify docker-compose project name
        #[arg(
            short = 'p',
            long,
            help = "Specify docker-compose project name (default: read from compose file or use 'docker')"
        )]
        project: Option<String>,
    },
    /// Show current auto upgrade configuration
    Status,
}

/// Client update related commands
#[derive(Subcommand, Debug)]
pub enum CheckUpdateCommand {
    /// Check latest version information
    Check,
    /// Install specified version or latest version
    Install {
        /// Specify version number (install latest if not specified)
        #[arg(long)]
        version: Option<String>,
        /// Force reinstall even if already latest version
        #[arg(long)]
        force: bool,
    },
}

#[derive(Subcommand, Debug)]
pub enum DockerServiceCommand {
    /// Start Docker services
    Start {
        /// Specify docker-compose project name
        #[arg(
            short = 'p',
            long,
            help = "Specify docker-compose project name (default: read from compose file or use 'docker')"
        )]
        project: Option<String>,
    },
    /// Stop Docker services
    Stop {
        /// Specify docker-compose project name
        #[arg(
            short = 'p',
            long,
            help = "Specify docker-compose project name (default: read from compose file or use 'docker')"
        )]
        project: Option<String>,
    },
    /// Restart Docker services
    Restart {
        /// Specify docker-compose project name
        #[arg(
            short = 'p',
            long,
            help = "Specify docker-compose project name (default: read from compose file or use 'docker')"
        )]
        project: Option<String>,
    },
    /// Check service status
    Status {
        /// Specify docker-compose project name
        #[arg(
            short = 'p',
            long,
            help = "Specify docker-compose project name (default: read from compose file or use 'docker')"
        )]
        project: Option<String>,
    },
    /// Restart specified container
    RestartContainer {
        /// Container name
        container_name: String,
    },
    /// Load Docker images
    LoadImages,
    /// Setup image tags
    SetupTags,
    /// Show architecture information
    ArchInfo,
    /// List Docker images (using ducker)
    ListImages,
    /// Check and create mount directories in docker-compose.yml
    CheckMountDirs,
}

/// Cache management related commands
#[derive(Subcommand, Debug)]
pub enum CacheCommand {
    /// Clear all cache files
    Clear,
    /// Show cache usage
    Status,
    /// Clean download cache (keep latest versions)
    CleanDownloads {
        /// Number of versions to keep
        #[arg(long, default_value = "3", help = "Number of versions to keep")]
        keep: u32,
    },
}

/// Nuwax CLI - Docker service management and upgrade tool
#[derive(Parser)]
#[command(name = "nuwax-cli")]
#[command(about = metadata::PROJECT_DESCRIPTION)]
#[command(version = version_info::CLI_VERSION)]
#[command(long_about = metadata::display::DESCRIPTION_LONG)]
#[command(author = metadata::PROJECT_AUTHORS)]
pub struct Cli {
    /// Configuration file path
    #[arg(short, long, default_value = "config.toml")]
    pub config: PathBuf,

    /// Verbose output
    #[arg(short, long)]
    pub verbose: bool,

    /// Language setting (zh-CN, zh-TW, en)
    #[arg(long, global = true)]
    pub lang: Option<String>,

    #[command(subcommand)]
    pub command: Commands,
}

#[derive(Subcommand)]
pub enum Commands {
    /// Show service status and version information
    Status,
    /// Initialize client on first use, create configuration file and database
    Init {
        /// Force overwrite if configuration file already exists
        #[arg(long)]
        force: bool,
    },
    /// Check client updates
    #[command(subcommand)]
    CheckUpdate(CheckUpdateCommand),
    /// Show current API configuration
    ApiInfo,
    /// Download Docker service files
    Upgrade {
        #[command(flatten)]
        args: UpgradeArgs,
    },
    /// List all backups
    ListBackups,
    /// Restore from backup
    Rollback {
        /// Backup ID (optional, will show interactive selection if not provided)
        backup_id: Option<i64>,
        /// Force overwrite
        #[arg(long)]
        force: bool,
        /// Output JSON format backup list (for GUI integration)
        #[arg(long)]
        list_json: bool,
        /// Whether to rollback data files, default no
        #[arg(long, default_value = "false", help = "Whether to rollback data files, default no")]
        rollback_data: bool,
    },
    /// Docker service related commands
    #[command(subcommand)]
    DockerService(DockerServiceCommand),

    /// 🐋 A terminal application for managing Docker containers
    Ducker {
        /// Arguments passed to ducker
        #[arg(trailing_var_arg = true, allow_hyphen_values = true)]
        args: Vec<String>,
    },

    /// Auto backup management
    #[command(subcommand)]
    AutoBackup(AutoBackupCommand),

    /// Auto upgrade deploy
    #[command(subcommand)]
    AutoUpgradeDeploy(AutoUpgradeDeployCommand),

    /// Cache management
    #[command(subcommand)]
    Cache(CacheCommand),

    /// Compare two SQL files and generate diff SQL
    DiffSql {
        /// Old version SQL file path
        #[arg(help = "Old version SQL file path")]
        old_sql: PathBuf,
        /// New version SQL file path
        #[arg(help = "New version SQL file path")]
        new_sql: PathBuf,
        /// Old version number (optional)
        #[arg(long, help = "Old version number for diff description")]
        old_version: Option<String>,
        /// New version number (optional)
        #[arg(long, help = "New version number for diff description")]
        new_version: Option<String>,
        /// Output file name (optional, default: upgrade_diff.sql)
        #[arg(long, default_value = "upgrade_diff.sql", help = "Diff SQL output file name")]
        output: String,
    },
}
