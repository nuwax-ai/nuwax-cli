use crate::docker_service::error::{DockerServiceError, DockerServiceResult};
use tracing::{info, warn};

/// 支持的系统架构
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Architecture {
    /// x86_64 架构
    Amd64,
    /// ARM64 架构  
    Arm64,
}

impl Architecture {
    /// 获取架构对应的字符串标识
    pub fn as_str(&self) -> &'static str {
        match self {
            Architecture::Amd64 => "amd64",
            Architecture::Arm64 => "arm64",
        }
    }

    /// 获取架构的显示名称
    pub fn display_name(&self) -> &'static str {
        match self {
            Architecture::Amd64 => "x86_64 (AMD64)",
            Architecture::Arm64 => "ARM64 (AArch64)",
        }
    }

    /// 从字符串解析架构
    pub fn from_str(s: &str) -> Option<Self> {
        match s.to_lowercase().as_str() {
            "amd64" | "x86_64" | "x86-64" => Some(Architecture::Amd64),
            "arm64" | "aarch64" | "arm" => Some(Architecture::Arm64),
            _ => None,
        }
    }
}

impl std::fmt::Display for Architecture {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.as_str())
    }
}

/// 检测当前系统的架构
pub fn detect_architecture() -> Architecture {
    let arch = std::env::consts::ARCH;
    info!("检测到系统架构: {}", arch);

    let detected = match arch {
        "x86_64" => Architecture::Amd64,
        "aarch64" => Architecture::Arm64,
        _ => {
            warn!("未知架构 '{}', 默认使用 amd64", arch);
            Architecture::Amd64
        }
    };

    info!("映射到支持的架构: {}", detected.display_name());
    detected
}

/// 验证架构是否有效
#[allow(dead_code)]
pub fn validate_architecture(arch: Architecture) -> DockerServiceResult<()> {
    match arch {
        Architecture::Amd64 | Architecture::Arm64 => {
            info!("架构 {} 受支持", arch.display_name());
            Ok(())
        }
    }
}

/// 获取架构相关的镜像匹配模式
#[allow(dead_code)]
pub fn get_image_pattern(arch: Architecture) -> String {
    format!("*-{}.tar", arch.as_str())
}

/// 检查指定架构的镜像是否存在
#[allow(dead_code)]
pub fn check_architecture_images_exist(
    images_dir: &std::path::Path,
    arch: Architecture,
) -> DockerServiceResult<Vec<std::path::PathBuf>> {
    if !images_dir.exists() {
        return Err(DockerServiceError::FileSystem(format!(
            "镜像目录不存在: {}",
            images_dir.display()
        )));
    }

    let pattern = get_image_pattern(arch);
    let mut found_images = Vec::new();

    // 扫描目录查找匹配的镜像文件
    if let Ok(entries) = std::fs::read_dir(images_dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_file() {
                if let Some(file_name) = path.file_name().and_then(|n| n.to_str()) {
                    // 检查文件名是否匹配架构模式
                    if file_name.ends_with(&format!("-{}.tar", arch.as_str())) {
                        found_images.push(path);
                    }
                }
            }
        }
    }

    if found_images.is_empty() {
        warn!(
            "未找到架构 {} 的镜像文件 (模式: {})",
            arch.display_name(),
            pattern
        );
    } else {
        info!(
            "找到 {} 个架构 {} 的镜像文件",
            found_images.len(),
            arch.display_name()
        );
    }

    Ok(found_images)
}

/// 获取可用的架构列表
#[allow(dead_code)]
pub fn get_available_architectures(
    images_dir: &std::path::Path,
) -> DockerServiceResult<std::collections::HashMap<Architecture, Vec<std::path::PathBuf>>> {
    let mut result = std::collections::HashMap::new();

    for &arch in &[Architecture::Amd64, Architecture::Arm64] {
        let images = check_architecture_images_exist(images_dir, arch)?;
        if !images.is_empty() {
            result.insert(arch, images);
        }
    }

    if result.is_empty() {
        return Err(DockerServiceError::ArchitectureDetection(format!(
            "在目录 {} 中未找到任何支持架构的镜像文件",
            images_dir.display()
        )));
    }

    info!("可用架构统计:");
    for (arch, images) in &result {
        info!("  {} -> {} 个镜像文件", arch.display_name(), images.len());
    }

    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::tempdir;

    #[test]
    fn test_architecture_from_str() {
        assert_eq!(Architecture::from_str("amd64"), Some(Architecture::Amd64));
        assert_eq!(Architecture::from_str("x86_64"), Some(Architecture::Amd64));
        assert_eq!(Architecture::from_str("arm64"), Some(Architecture::Arm64));
        assert_eq!(Architecture::from_str("aarch64"), Some(Architecture::Arm64));
        assert_eq!(Architecture::from_str("unknown"), None);
    }

    #[test]
    fn test_architecture_display() {
        assert_eq!(Architecture::Amd64.as_str(), "amd64");
        assert_eq!(Architecture::Arm64.as_str(), "arm64");
        assert_eq!(format!("{}", Architecture::Amd64), "amd64");
    }

    #[test]
    fn test_detect_architecture() {
        // 这个测试会根据运行环境返回不同结果
        let arch = detect_architecture();
        assert!(matches!(arch, Architecture::Amd64 | Architecture::Arm64));
    }

    #[test]
    fn test_check_architecture_images_exist() {
        let temp_dir = tempdir().unwrap();
        let images_dir = temp_dir.path().join("images");
        fs::create_dir_all(&images_dir).unwrap();

        // 创建测试镜像文件
        fs::write(images_dir.join("test-amd64.tar"), b"fake image").unwrap();
        fs::write(images_dir.join("another-amd64.tar"), b"fake image").unwrap();
        fs::write(images_dir.join("service-arm64.tar"), b"fake image").unwrap();

        let amd64_images =
            check_architecture_images_exist(&images_dir, Architecture::Amd64).unwrap();
        assert_eq!(amd64_images.len(), 2);

        let arm64_images =
            check_architecture_images_exist(&images_dir, Architecture::Arm64).unwrap();
        assert_eq!(arm64_images.len(), 1);
    }
}
