//! 路径处理工具
//!
//! 提供跨平台的路径处理功能，支持 WSL2、Windows 和 POSIX 路径格式。

use crate::container::environment::{HostOs, PathFormat};
use std::path::{Path, PathBuf};
use tracing::debug;

/// 路径处理错误
#[derive(Debug, thiserror::Error)]
pub enum PathUtilsError {
    #[error("Path processing error: {0}")]
    InvalidPath(String),
}

/// 跨平台路径处理器
#[derive(Debug, Clone)]
pub struct PathProcessor {
    pub host_os: HostOs,
    pub path_format: PathFormat,
}

impl PathProcessor {
    /// 创建新的路径处理器
    pub fn new(host_os: HostOs, path_format: PathFormat) -> Self {
        Self {
            host_os,
            path_format,
        }
    }

    /// 解析和规范化路径
    /// 根据环境将输入路径转换为适合的格式
    pub fn normalize_path(&self, input_path: &str) -> Result<String, PathUtilsError> {
        let path = input_path.trim();

        if path.is_empty() {
            return Err(PathUtilsError::InvalidPath("Path cannot be empty".to_string()));
        }

        debug!(
            "🔍 Normalizing path: '{}' (current environment: {:?})",
            path, self.path_format
        );

        // 1. 初步清理路径
        let cleaned_path = self.clean_path(path);

        // 2. 根据环境转换路径格式
        let formatted_path = match self.path_format {
            PathFormat::Wsl2 => self.to_wsl2_format(&cleaned_path),
            PathFormat::Windows => self.to_windows_format(&cleaned_path),
            PathFormat::Posix => self.to_posix_format(&cleaned_path),
        };

        debug!("✅ Path normalization complete: '{}'", formatted_path);
        Ok(formatted_path)
    }

    /// 检查是否为 bind mount 路径
    pub fn is_bind_mount_path(&self, path: &str) -> bool {
        let path = path.trim();

        if path.is_empty() {
            return false;
        }

        // 绝对路径（POSIX 格式）
        if path.starts_with('/') && !path.starts_with("//") {
            return true;
        }

        // Windows 绝对路径（C:\, D:\ 等）
        if path.len() >= 3
            && path.chars().nth(1).unwrap_or_default() == ':'
            && (path.chars().nth(2).unwrap_or_default() == '\\'
                || path.chars().nth(2).unwrap_or_default() == '/')
        {
            return true;
        }

        // WSL2 路径格式
        if path.starts_with("/mnt/") || path.starts_with("/c/") || path.starts_with("/d/") {
            return true;
        }

        // 相对路径（包含路径分隔符）
        if path.contains('/') || path.contains('\\') {
            // 排除仅包含文件名的情况
            return !Path::new(path).file_name().map_or(false, |f| {
                !Path::new(f).extension().map_or(false, |ext| {
                    // 排除带扩展名的文件
                    ext.len() > 0
                })
            });
        }

        false
    }

    /// 将路径转换为相对于工作目录的绝对路径
    pub fn to_absolute_path(&self, path: &str, work_dir: &Path) -> Result<PathBuf, PathUtilsError> {
        let normalized_path = self.normalize_path(path)?;
        let path_buf = PathBuf::from(&normalized_path);

        if path_buf.is_absolute() {
            Ok(path_buf)
        } else {
            // 相对路径：相对于工作目录
            let absolute = work_dir.join(path_buf);
            Ok(absolute)
        }
    }

    /// 清理路径（移除多余的 ./ 和 //）
    fn clean_path(&self, path: &str) -> String {
        let mut components = Vec::new();

        for component in Path::new(path).components() {
            match component {
                std::path::Component::CurDir => {
                    // 跳过当前目录 .
                    continue;
                }
                std::path::Component::ParentDir => {
                    // 处理父目录 ..
                    if let Some(last) = components.last() {
                        if last != &std::path::Component::RootDir {
                            components.pop();
                        }
                    }
                }
                _ => {
                    components.push(component);
                }
            }
        }

        let cleaned = components
            .iter()
            .map(|c| c.as_os_str().to_string_lossy())
            .collect::<Vec<_>>()
            .join(std::path::MAIN_SEPARATOR_STR);

        // 确保空路径返回 "."
        if cleaned.is_empty() {
            ".".to_string()
        } else {
            cleaned
        }
    }

    /// 转换为 WSL2 格式
    fn to_wsl2_format(&self, path: &str) -> String {
        let path = path.replace('\\', "/");

        // 如果是 Windows 绝对路径（C:\...），转换为 WSL2 格式
        if path.len() >= 3
            && path.chars().nth(1).unwrap_or_default() == ':'
            && (path.chars().nth(2).unwrap_or_default() == '/'
                || path.chars().nth(2).unwrap_or_default() == '\\')
        {
            let drive_letter = path
                .chars()
                .nth(0)
                .unwrap_or_default()
                .to_lowercase()
                .next()
                .unwrap_or_default();
            let rest = &path[3..];
            return format!("/mnt/{}{}", drive_letter, rest);
        }

        // 如果已经是 WSL2 格式，返回
        if path.starts_with("/mnt/") || path.starts_with("/c/") || path.starts_with("/d/") {
            return path;
        }

        // 其他格式保持不变
        path
    }

    /// 转换为 Windows 格式
    fn to_windows_format(&self, path: &str) -> String {
        let path = path.replace('/', "\\");

        // WSL2 路径转换为 Windows 路径
        if path.starts_with("/mnt/") {
            let rest = &path[5..]; // 移除 /mnt/
            if !rest.is_empty() {
                let drive_letter = rest
                    .chars()
                    .next()
                    .unwrap_or_default()
                    .to_uppercase()
                    .next()
                    .unwrap_or_default();
                let rest = &rest[1..];
                return format!("{}:\\{}", drive_letter, rest);
            }
        }

        if path.starts_with("/c/") {
            return format!("C:\\{}", &path[3..]);
        }

        if path.starts_with("/d/") {
            return format!("D:\\{}", &path[3..]);
        }

        path
    }

    /// 转换为 POSIX 格式
    fn to_posix_format(&self, path: &str) -> String {
        let path = path.replace('\\', "/");

        // Windows 绝对路径转换为 POSIX
        if path.len() >= 3
            && path.chars().nth(1).unwrap_or_default() == ':'
            && (path.chars().nth(2).unwrap_or_default() == '\\'
                || path.chars().nth(2).unwrap_or_default() == '/')
        {
            let drive_letter = path
                .chars()
                .nth(0)
                .unwrap_or_default()
                .to_lowercase()
                .next()
                .unwrap_or_default();
            let rest = &path[3..];
            return format!("/mnt/{}{}", drive_letter, rest);
        }

        // WSL2 格式保持不变
        if path.starts_with("/mnt/") || path.starts_with("/c/") || path.starts_with("/d/") {
            return path;
        }

        path
    }

    /// 转换路径分隔符
    pub fn convert_separators(&self, path: &str) -> String {
        match self.path_format {
            PathFormat::Wsl2 | PathFormat::Posix => path.replace('\\', "/"),
            PathFormat::Windows => path.replace('/', "\\"),
        }
    }

    /// 检查路径是否需要特殊处理
    pub fn needs_special_handling(&self, path: &str) -> bool {
        // Windows 路径或 WSL2 路径需要特殊处理
        self.is_bind_mount_path(path)
            && (self.path_format == PathFormat::Wsl2 || self.path_format == PathFormat::Windows)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_normalize_path_wsl2() {
        let processor = PathProcessor::new(HostOs::WindowsWsl2, PathFormat::Wsl2);

        // Windows 路径转换为 WSL2
        assert_eq!(
            processor.normalize_path(r"C:\Users\test\data").unwrap(),
            "/mnt/c/Users/test/data"
        );

        // 相对路径保持不变
        assert_eq!(processor.normalize_path("./data").unwrap(), "data");

        // 已经是 WSL2 格式
        assert_eq!(
            processor.normalize_path("/mnt/c/Users/test").unwrap(),
            "/mnt/c/Users/test"
        );
    }

    #[test]
    fn test_normalize_path_windows() {
        let processor = PathProcessor::new(HostOs::WindowsNative, PathFormat::Windows);

        // WSL2 路径转换为 Windows
        assert_eq!(
            processor.normalize_path("/mnt/c/Users/test").unwrap(),
            r"C:\Users\test"
        );

        // 路径分隔符转换
        assert_eq!(
            processor.normalize_path("C:/Users/test").unwrap(),
            r"C:\Users\test"
        );
    }

    #[test]
    fn test_is_bind_mount_path_wsl2() {
        let processor = PathProcessor::new(HostOs::WindowsWsl2, PathFormat::Wsl2);

        assert!(processor.is_bind_mount_path("/mnt/c/Users/test/data"));
        assert!(processor.is_bind_mount_path("/c/Users/test/data"));
        assert!(processor.is_bind_mount_path("/data/mysql"));
        assert!(processor.is_bind_mount_path("./data"));
        assert!(processor.is_bind_mount_path("../data"));
        assert!(processor.is_bind_mount_path(r"C:\data"));

        // 这些不是 bind mount
        assert!(!processor.is_bind_mount_path("volume_name"));
        assert!(!processor.is_bind_mount_path(""));
    }

    #[test]
    fn test_to_absolute_path() {
        let processor = PathProcessor::new(HostOs::LinuxNative, PathFormat::Posix);
        let work_dir = PathBuf::from("/workspace");

        // 绝对路径保持不变
        assert_eq!(
            processor.to_absolute_path("/data", &work_dir).unwrap(),
            PathBuf::from("/data")
        );

        // 相对路径相对于工作目录
        assert_eq!(
            processor.to_absolute_path("./data", &work_dir).unwrap(),
            PathBuf::from("/workspace/data")
        );
    }

    #[test]
    fn test_convert_separators() {
        let processor_wsl2 = PathProcessor::new(HostOs::WindowsWsl2, PathFormat::Wsl2);
        assert_eq!(
            processor_wsl2.convert_separators(r"C:\Users\test"),
            "/mnt/c/Users/test"
        );

        let processor_windows = PathProcessor::new(HostOs::WindowsNative, PathFormat::Windows);
        assert_eq!(
            processor_windows.convert_separators("/mnt/c/Users/test"),
            r"\mnt\c\Users\test"
        );
    }

    #[test]
    fn test_clean_path() {
        let processor = PathProcessor::new(HostOs::LinuxNative, PathFormat::Posix);

        assert_eq!(processor.clean_path("./data"), "data");
        assert_eq!(processor.clean_path("data/./test"), "data/test");
        assert_eq!(processor.clean_path("data/../test"), "test");
        assert_eq!(processor.clean_path("./"), "");
    }

    #[test]
    fn test_needs_special_handling() {
        let processor_wsl2 = PathProcessor::new(HostOs::WindowsWsl2, PathFormat::Wsl2);
        assert!(processor_wsl2.needs_special_handling("/mnt/c/data"));

        let processor_posix = PathProcessor::new(HostOs::LinuxNative, PathFormat::Posix);
        assert!(!processor_posix.needs_special_handling("/data"));
    }
}
