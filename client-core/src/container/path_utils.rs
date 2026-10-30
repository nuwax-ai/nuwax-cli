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
            return Err(PathUtilsError::InvalidPath(
                "Path cannot be empty".to_string(),
            ));
        }

        debug!(
            "🔍 Normalizing path: '{}' (current environment: {:?})",
            path, self.path_format
        );

        // Clamp foreign absolute-drive parents before mapping the drive under /mnt.
        let bytes = path.as_bytes();
        let drive_path = if bytes.len() >= 3
            && bytes[0].is_ascii_alphabetic()
            && bytes[1] == b':'
            && matches!(bytes[2], b'/' | b'\\')
        {
            let suffix = path[2..].replace('\\', "/");
            let suffix = format!("/{}", suffix.trim_start_matches('/'));
            let suffix = self.clean_path(&suffix).replace('\\', "/");
            Some(format!("{}{}", &path[..2], suffix))
        } else {
            None
        };
        let path = drive_path.as_deref().unwrap_or(path);

        // Convert drive/root syntax before native component parsing.
        let formatted_path = match self.path_format {
            PathFormat::Wsl2 => self.to_wsl2_format(path),
            PathFormat::Windows => self.to_windows_format(path),
            PathFormat::Posix => self.to_posix_format(path),
        };
        let cleaned_path = self.clean_path(&formatted_path);
        let formatted_path = self.convert_separators(&cleaned_path);

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
            return true;
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
        let mut cleaned = PathBuf::new();
        for component in Path::new(path).components() {
            match component {
                std::path::Component::CurDir => {}
                std::path::Component::ParentDir => match cleaned.components().next_back() {
                    Some(std::path::Component::Normal(_)) => {
                        cleaned.pop();
                    }
                    _ if !cleaned.has_root() => cleaned.push(component.as_os_str()),
                    _ => {}
                },
                _ => cleaned.push(component.as_os_str()),
            }
        }
        // 确保空路径返回 "."
        if cleaned.as_os_str().is_empty() {
            ".".to_string()
        } else {
            cleaned.to_string_lossy().into_owned()
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
            // path[2] is the separator '/', so path[2..] gives "/Users/test/data"
            let rest = &path[2..];
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
        // WSL2 路径转换为 Windows 路径（需要在替换斜杠之前处理）
        if let Some(rest) = path
            .strip_prefix("/mnt/")
            .or_else(|| path.strip_prefix("\\mnt\\"))
            && !rest.is_empty()
        {
            let drive_letter = rest
                .chars()
                .next()
                .unwrap_or_default()
                .to_uppercase()
                .next()
                .unwrap_or_default();
            // 跳过驱动器字母后的分隔符（如 / 或 \），然后转换剩余路径的分隔符
            let rest = rest[1..].trim_start_matches(['/', '\\']).replace('/', "\\");
            return format!("{}:\\{}", drive_letter, rest);
        }

        if let Some(rest) = path
            .strip_prefix("/c/")
            .or_else(|| path.strip_prefix("\\c\\"))
        {
            let rest = rest.trim_start_matches(['/', '\\']).replace('/', "\\");
            return format!("C:\\{}", rest);
        }

        if let Some(rest) = path
            .strip_prefix("/d/")
            .or_else(|| path.strip_prefix("\\d\\"))
        {
            let rest = rest.trim_start_matches(['/', '\\']).replace('/', "\\");
            return format!("D:\\{}", rest);
        }

        // 其他情况：替换斜杠为反斜杠
        path.replace('/', "\\")
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
            let rest = &path[2..];
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
        // convert_separators just converts slashes, doesn't do path format conversion
        let processor_wsl2 = PathProcessor::new(HostOs::WindowsWsl2, PathFormat::Wsl2);
        assert_eq!(
            processor_wsl2.convert_separators(r"C:\Users\test"),
            "C:/Users/test"
        );

        let processor_windows = PathProcessor::new(HostOs::WindowsNative, PathFormat::Windows);
        assert_eq!(
            processor_windows.convert_separators("/mnt/c/Users/test"),
            "\\mnt\\c\\Users\\test"
        );
    }

    #[test]
    fn test_clean_path() {
        let processor = PathProcessor::new(HostOs::LinuxNative, PathFormat::Posix);

        assert_eq!(processor.clean_path("./data"), "data");
        assert_eq!(
            processor.clean_path("data/./test"),
            Path::new("data").join("test").to_string_lossy()
        );
        assert_eq!(processor.clean_path("data/../test"), "test");
        // 空路径返回 "."
        assert_eq!(processor.clean_path("./"), ".");
    }

    #[test]
    fn test_needs_special_handling() {
        let processor_wsl2 = PathProcessor::new(HostOs::WindowsWsl2, PathFormat::Wsl2);
        assert!(processor_wsl2.needs_special_handling("/mnt/c/data"));

        let processor_posix = PathProcessor::new(HostOs::LinuxNative, PathFormat::Posix);
        assert!(!processor_posix.needs_special_handling("/data"));
    }

    #[test]
    fn relative_parents_and_absolute_roots_are_retained() {
        let processor = PathProcessor::new(HostOs::LinuxNative, PathFormat::Posix);
        assert_eq!(processor.normalize_path("../data").unwrap(), "../data");
        assert_eq!(
            processor.normalize_path("../../data").unwrap(),
            "../../data"
        );
        assert_eq!(
            processor.normalize_path("data/../../test").unwrap(),
            "../test"
        );
        assert_eq!(processor.normalize_path("/../data").unwrap(), "/data");
        assert_eq!(processor.normalize_path("/").unwrap(), "/");
    }

    #[test]
    fn windows_drive_to_posix_retains_separator() {
        let processor = PathProcessor::new(HostOs::LinuxNative, PathFormat::Posix);
        assert_eq!(
            processor.normalize_path("C:/Users/test").unwrap(),
            "/mnt/c/Users/test"
        );
        assert_eq!(processor.normalize_path("C:/").unwrap(), "/mnt/c");
    }

    #[test]
    fn absolute_drive_parents_cannot_escape_the_drive_mapping() {
        for (format, expected) in [
            (PathFormat::Posix, "/mnt/c/data"),
            (PathFormat::Wsl2, "/mnt/c/data"),
            (PathFormat::Windows, r"C:\data"),
        ] {
            let processor = PathProcessor::new(HostOs::LinuxNative, format);
            for input in ["C:/../../data", r"C:\..\..\data", "C://../../data"] {
                assert_eq!(
                    processor.normalize_path(input).unwrap(),
                    expected,
                    "{input}"
                );
            }
        }
    }

    #[cfg(windows)]
    #[test]
    fn native_windows_drive_unc_and_rooted_paths_retain_roots() {
        let processor = PathProcessor::new(HostOs::WindowsNative, PathFormat::Windows);
        for (input, expected) in [
            (r"C:\", r"C:\"),
            (r"C:\a\..\..\test", r"C:\test"),
            (r"\data", r"\data"),
            (r"\\server\share\a\..\test", r"\\server\share\test"),
        ] {
            assert_eq!(processor.normalize_path(input).unwrap(), expected);
        }
        assert_eq!(
            processor
                .to_absolute_path(r"\data", Path::new(r"C:\workspace"))
                .unwrap(),
            PathBuf::from(r"C:\data")
        );
    }
}
