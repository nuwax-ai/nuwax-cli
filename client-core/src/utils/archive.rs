//! 压缩归档文件辅助函数
//!
//! 使用 `infer` 库通过魔数（magic number）检测文件格式

use anyhow::Result;
use std::path::Path;

/// 压缩包格式
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ArchiveFormat {
    Zip,
    TarGz,
}

impl ArchiveFormat {
    /// 文件扩展名
    pub fn extension(&self) -> &str {
        match self {
            Self::Zip => ".zip",
            Self::TarGz => ".tar.gz",
        }
    }

    /// 从 infer 的 Type 转换
    fn from_infer_kind(kind: &infer::Type) -> Option<Self> {
        match kind.extension() {
            "zip" => Some(ArchiveFormat::Zip),
            "gz" | "tgz" => Some(ArchiveFormat::TarGz),
            _ => None,
        }
    }
}

/// 使用 infer 库通过魔数检测格式
pub fn detect_format_by_magic(path: &Path) -> Result<ArchiveFormat> {
    let kind = infer::get_from_path(path)?
        .ok_or_else(|| anyhow::anyhow!("无法识别文件格式"))?;

    ArchiveFormat::from_infer_kind(&kind)
        .ok_or_else(|| anyhow::anyhow!("不支持的文件格式: {}", kind.mime_type()))
}

/// 生成 Docker 服务包文件名
pub fn generate_docker_filename(arch: &str, format: ArchiveFormat) -> String {
    format!("docker-{}{}", arch, format.extension())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_generate_filename() {
        assert_eq!(
            generate_docker_filename("aarch64", ArchiveFormat::TarGz),
            "docker-aarch64.tar.gz"
        );
        assert_eq!(
            generate_docker_filename("x86_64", ArchiveFormat::Zip),
            "docker-x86_64.zip"
        );
    }

    #[test]
    fn test_archive_format_extension() {
        assert_eq!(ArchiveFormat::Zip.extension(), ".zip");
        assert_eq!(ArchiveFormat::TarGz.extension(), ".tar.gz");
    }
}
