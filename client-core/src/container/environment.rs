//! 环境检测模块
//!
//! 用于检测当前运行环境，包括操作系统、路径格式等，
//! 为跨平台兼容提供支持。
//!
//! 注意：本项目通过 Docker API 与容器引擎通信（使用 bollard 库），
//! 支持 Docker 和 Podman（Docker 兼容模式），因此不需要区分底层容器引擎类型。

use std::env;
use std::sync::OnceLock;
use tokio::process::Command;
use tracing::{debug, info, warn};

/// 全局存储检测到的 Docker Compose 命令类型
static COMPOSE_COMMAND_TYPE: OnceLock<ComposeCommandType> = OnceLock::new();

/// Docker Compose 命令类型
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub enum ComposeCommandType {
    /// 使用 docker compose 子命令（Docker 20.10.13+）
    DockerComposeSubcommand,
    /// 使用独立的 docker-compose 命令
    DockerComposeStandalone,
    /// 未检测（默认值）
    #[default]
    Unknown,
}

/// 检测 docker compose 命令类型（执行命令检测）
pub async fn detect_compose_command_type() -> ComposeCommandType {
    info!("🔍 检测 Docker Compose 命令类型...");

    // 1. 尝试 docker compose version（新语法）
    let output = Command::new("docker")
        .args(["compose", "version"])
        .output()
        .await;

    if let Ok(output) = output {
        if output.status.success() {
            let version_info = String::from_utf8_lossy(&output.stdout);
            info!(
                "   ✅ 使用 docker compose 子命令: {}",
                version_info.trim()
            );
            return ComposeCommandType::DockerComposeSubcommand;
        }
        debug!(
            "   docker compose version 返回非零退出码: {:?}",
            output.status
        );
    }

    // 2. 回退到 docker-compose --version（旧语法）
    debug!("   尝试 docker-compose 独立命令...");
    let output = Command::new("docker-compose")
        .arg("--version")
        .output()
        .await;

    if let Ok(output) = output {
        if output.status.success() {
            let version_info = String::from_utf8_lossy(&output.stdout);
            info!(
                "   ✅ 使用 docker-compose 独立命令: {}",
                version_info.trim()
            );
            return ComposeCommandType::DockerComposeStandalone;
        }
    }

    warn!("   ⚠️ 未检测到可用的 Docker Compose 命令");
    ComposeCommandType::Unknown
}

/// 设置全局 Docker Compose 命令类型（仅能设置一次）
///
/// 在命令入口处（如 main.rs）调用 detect_compose_command_type() 后，
/// 使用此函数存储检测结果，后续无需再次检测。
pub fn set_compose_command_type(compose_type: ComposeCommandType) {
    if COMPOSE_COMMAND_TYPE.set(compose_type).is_err() {
        debug!("Compose 命令类型已设置，忽略重复设置");
    }
}

/// 获取已检测的 Docker Compose 命令类型
///
/// 返回已检测的命令类型，如果未检测则返回 Unknown
pub fn get_compose_command_type() -> ComposeCommandType {
    COMPOSE_COMMAND_TYPE
        .get()
        .copied()
        .unwrap_or(ComposeCommandType::Unknown)
}

/// 主机操作系统类型
#[derive(Debug, Clone, PartialEq)]
pub enum HostOs {
    /// Windows + WSL2 环境
    WindowsWsl2,
    /// 原生 Windows 环境
    WindowsNative,
    /// 原生 Linux 环境
    LinuxNative,
    /// macOS 环境
    MacOs,
}

impl HostOs {
    /// 获取显示名称
    pub fn display_name(&self) -> &'static str {
        match self {
            HostOs::WindowsWsl2 => "Windows (WSL2)",
            HostOs::WindowsNative => "Windows (Native)",
            HostOs::LinuxNative => "Linux",
            HostOs::MacOs => "macOS",
        }
    }

    /// 检查是否为 Windows 环境（包括 WSL2 和原生）
    pub fn is_windows(&self) -> bool {
        matches!(self, HostOs::WindowsWsl2 | HostOs::WindowsNative)
    }

    /// 检查是否为 WSL2 环境
    pub fn is_wsl2(&self) -> bool {
        matches!(self, HostOs::WindowsWsl2)
    }

    /// 检查是否需要提前创建挂载目录
    ///
    /// Windows 环境下（包括 WSL2 和原生 Windows），容器引擎不会自动创建
    /// docker-compose.yml 中定义的挂载目录，需要提前手动创建。
    pub fn needs_early_mount_check(&self) -> bool {
        self.is_windows()
    }
}

/// 路径格式类型
#[derive(Debug, Clone, PartialEq)]
pub enum PathFormat {
    /// WSL2 格式：/mnt/c/...
    Wsl2,
    /// Windows 格式：C:\...
    Windows,
    /// POSIX 格式：/...
    Posix,
}

impl PathFormat {
    pub fn display_name(&self) -> &'static str {
        match self {
            PathFormat::Wsl2 => "WSL2",
            PathFormat::Windows => "Windows",
            PathFormat::Posix => "POSIX",
        }
    }
}

/// 运行时环境信息
#[derive(Debug, Clone)]
pub struct RuntimeEnvironment {
    pub host_os: HostOs,
    pub path_format: PathFormat,
}

impl RuntimeEnvironment {
    /// 获取环境摘要信息
    pub fn summary(&self) -> String {
        format!(
            "{} ({})",
            self.host_os.display_name(),
            self.path_format.display_name()
        )
    }

    /// 检查是否需要特殊处理
    ///
    /// 在 Windows 环境下（包括 WSL2 和原生 Windows），容器引擎
    /// （Docker Desktop 或 Podman Desktop）都不会自动创建
    /// docker-compose.yml 中定义的挂载目录，因此需要提前主动创建这些目录。
    ///
    /// 在 Linux/macOS 环境下，系统会自动创建挂载目录，无需特殊处理。
    pub fn needs_special_handling(&self) -> bool {
        self.host_os.is_windows()
    }

    /// 检查是否为 WSL2 环境
    pub fn is_wsl2(&self) -> bool {
        self.host_os.is_wsl2()
    }
}

/// 检测当前运行环境
pub fn detect_runtime_environment() -> RuntimeEnvironment {
    debug!("🔍 开始检测运行时环境...");

    // 检测主机操作系统
    let host_os = detect_host_os();
    debug!("   主机 OS: {:?}", host_os);

    // 检测路径格式
    let path_format = detect_path_format(&host_os);
    debug!("   路径格式: {:?}", path_format);

    let env = RuntimeEnvironment {
        host_os,
        path_format,
    };

    info!("✅ 运行环境检测完成: {}", env.summary());

    if env.needs_special_handling() {
        info!("⚠️  检测到 Windows 环境，需要提前创建挂载目录");
    }

    env
}

/// 检测主机操作系统
fn detect_host_os() -> HostOs {
    // 检查是否为 WSL2
    if is_running_in_wsl() {
        return HostOs::WindowsWsl2;
    }

    // 检测原生操作系统
    match std::env::consts::OS {
        "windows" => HostOs::WindowsNative,
        "linux" => HostOs::LinuxNative,
        "macos" => HostOs::MacOs,
        other => {
            debug!("未知操作系统: {}, 假设为 Linux", other);
            HostOs::LinuxNative
        }
    }
}

/// 检测是否在 WSL2 中运行
fn is_running_in_wsl() -> bool {
    // 方法 1: 检查 /proc/version
    if let Ok(version) = std::fs::read_to_string("/proc/version") {
        if version.to_lowercase().contains("microsoft") {
            debug!("检测到 WSL 标记在 /proc/version 中");
            return true;
        }
    }

    // 方法 2: 检查 WSL 环境变量
    if env::var("WSL_DISTRO_NAME").is_ok() {
        debug!("检测到 WSL_DISTRO_NAME 环境变量");
        return true;
    }

    if env::var("WSLENV").is_ok() {
        debug!("检测到 WSLENV 环境变量");
        return true;
    }

    false
}

/// 检测路径格式
fn detect_path_format(host_os: &HostOs) -> PathFormat {
    match host_os {
        HostOs::WindowsWsl2 => PathFormat::Wsl2,
        HostOs::WindowsNative => PathFormat::Windows,
        HostOs::LinuxNative | HostOs::MacOs => PathFormat::Posix,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_host_os_display_name() {
        assert_eq!(HostOs::WindowsWsl2.display_name(), "Windows (WSL2)");
        assert_eq!(HostOs::WindowsNative.display_name(), "Windows (Native)");
        assert_eq!(HostOs::LinuxNative.display_name(), "Linux");
        assert_eq!(HostOs::MacOs.display_name(), "macOS");
    }

    #[test]
    fn test_host_os_is_windows() {
        assert!(HostOs::WindowsWsl2.is_windows());
        assert!(HostOs::WindowsNative.is_windows());
        assert!(!HostOs::LinuxNative.is_windows());
        assert!(!HostOs::MacOs.is_windows());
    }

    #[test]
    fn test_host_os_needs_early_mount_check() {
        assert!(HostOs::WindowsWsl2.needs_early_mount_check());
        assert!(HostOs::WindowsNative.needs_early_mount_check());
        assert!(!HostOs::LinuxNative.needs_early_mount_check());
        assert!(!HostOs::MacOs.needs_early_mount_check());
    }

    #[test]
    fn test_path_format_display_name() {
        assert_eq!(PathFormat::Wsl2.display_name(), "WSL2");
        assert_eq!(PathFormat::Windows.display_name(), "Windows");
        assert_eq!(PathFormat::Posix.display_name(), "POSIX");
    }

    #[test]
    fn test_runtime_environment_summary() {
        let env = RuntimeEnvironment {
            host_os: HostOs::WindowsWsl2,
            path_format: PathFormat::Wsl2,
        };

        assert_eq!(env.summary(), "Windows (WSL2) (WSL2)");
    }

    #[test]
    fn test_runtime_environment_is_wsl2() {
        let env_wsl2 = RuntimeEnvironment {
            host_os: HostOs::WindowsWsl2,
            path_format: PathFormat::Wsl2,
        };
        assert!(env_wsl2.is_wsl2());

        let env_linux = RuntimeEnvironment {
            host_os: HostOs::LinuxNative,
            path_format: PathFormat::Posix,
        };
        assert!(!env_linux.is_wsl2());
    }

    #[test]
    fn test_runtime_environment_needs_special_handling() {
        // Windows WSL2 环境 → 需要特殊处理
        let env_wsl2 = RuntimeEnvironment {
            host_os: HostOs::WindowsWsl2,
            path_format: PathFormat::Wsl2,
        };
        assert!(env_wsl2.needs_special_handling());

        // Windows 原生环境 → 需要特殊处理
        let env_windows_native = RuntimeEnvironment {
            host_os: HostOs::WindowsNative,
            path_format: PathFormat::Windows,
        };
        assert!(env_windows_native.needs_special_handling());

        // Linux 环境 → 不需要特殊处理
        let env_linux = RuntimeEnvironment {
            host_os: HostOs::LinuxNative,
            path_format: PathFormat::Posix,
        };
        assert!(!env_linux.needs_special_handling());

        // macOS 环境 → 不需要特殊处理
        let env_macos = RuntimeEnvironment {
            host_os: HostOs::MacOs,
            path_format: PathFormat::Posix,
        };
        assert!(!env_macos.needs_special_handling());
    }

    #[test]
    fn test_compose_command_type_default() {
        assert_eq!(ComposeCommandType::default(), ComposeCommandType::Unknown);
    }

    #[test]
    fn test_compose_command_type_equality() {
        assert_eq!(
            ComposeCommandType::DockerComposeSubcommand,
            ComposeCommandType::DockerComposeSubcommand
        );
        assert_eq!(
            ComposeCommandType::DockerComposeStandalone,
            ComposeCommandType::DockerComposeStandalone
        );
        assert_eq!(ComposeCommandType::Unknown, ComposeCommandType::Unknown);

        assert_ne!(
            ComposeCommandType::DockerComposeSubcommand,
            ComposeCommandType::DockerComposeStandalone
        );
        assert_ne!(
            ComposeCommandType::DockerComposeSubcommand,
            ComposeCommandType::Unknown
        );
    }

    #[test]
    fn test_compose_command_type_clone_copy() {
        let original = ComposeCommandType::DockerComposeSubcommand;
        let cloned = original.clone();
        let copied = original;

        assert_eq!(original, cloned);
        assert_eq!(original, copied);
    }

    #[test]
    fn test_compose_command_type_debug() {
        // 测试 Debug trait 实现
        let subcommand = ComposeCommandType::DockerComposeSubcommand;
        let standalone = ComposeCommandType::DockerComposeStandalone;
        let unknown = ComposeCommandType::Unknown;

        assert_eq!(format!("{:?}", subcommand), "DockerComposeSubcommand");
        assert_eq!(format!("{:?}", standalone), "DockerComposeStandalone");
        assert_eq!(format!("{:?}", unknown), "Unknown");
    }
}
