// client-core/src/patch_executor/error.rs
//! 补丁执行器错误定义

use thiserror::Error;

/// 补丁执行器错误类型
#[derive(Debug, Error)]
pub enum PatchExecutorError {
    /// 文件操作错误
    #[error("文件操作失败: {0}")]
    IoError(#[from] std::io::Error),

    /// 路径错误
    #[error("路径错误: {path}")]
    PathError { path: String },

    /// 权限错误
    #[error("权限错误: {path}")]
    PermissionError { path: String },

    /// 原子操作失败
    #[error("原子操作失败: {reason}")]
    AtomicOperationFailed { reason: String },

    /// 回滚失败
    #[error("回滚失败: {reason}")]
    RollbackFailed { reason: String },

    /// 补丁下载失败
    #[error("补丁下载失败: {url}")]
    DownloadFailed { url: String },

    /// 补丁验证失败
    #[error("补丁验证失败: {reason}")]
    VerificationFailed { reason: String },

    /// 补丁解压失败
    #[error("补丁解压失败: {reason}")]
    ExtractionFailed { reason: String },

    /// 哈希校验失败
    #[error("哈希校验失败: 期望 {expected}, 实际 {actual}")]
    HashMismatch { expected: String, actual: String },

    /// 数字签名验证失败
    #[error("数字签名验证失败: {reason}")]
    SignatureVerificationFailed { reason: String },

    /// 不支持的操作
    #[error("不支持的操作: {operation}")]
    UnsupportedOperation { operation: String },

    /// 备份模式未启用
    #[error("备份模式未启用，无法执行回滚操作")]
    BackupNotEnabled,

    /// 补丁源目录未设置
    #[error("补丁源目录未设置")]
    PatchSourceNotSet,

    /// 临时文件操作错误
    #[error("临时文件操作错误: {0}")]
    TempFileError(#[from] tempfile::PersistError),

    /// HTTP 错误
    #[error("HTTP 请求错误: {0}")]
    HttpError(#[from] reqwest::Error),

    /// JSON 解析错误
    #[error("JSON 解析错误: {0}")]
    JsonError(#[from] serde_json::Error),

    /// ZIP 操作错误
    #[error("ZIP 操作错误: {0}")]
    ZipError(#[from] zip::result::ZipError),

    /// fs_extra 错误
    #[error("文件系统扩展操作错误: {0}")]
    FsExtraError(#[from] fs_extra::error::Error),

    /// 自定义错误
    #[error("补丁执行错误: {message}")]
    Custom { message: String },
}

impl PatchExecutorError {
    /// 创建自定义错误
    pub fn custom<S: Into<String>>(message: S) -> Self {
        Self::Custom {
            message: message.into(),
        }
    }

    /// 创建路径错误
    pub fn path_error<S: Into<String>>(path: S) -> Self {
        Self::PathError { path: path.into() }
    }

    /// 创建权限错误
    pub fn permission_error<S: Into<String>>(path: S) -> Self {
        Self::PermissionError { path: path.into() }
    }

    /// 创建原子操作失败错误
    pub fn atomic_operation_failed<S: Into<String>>(reason: S) -> Self {
        Self::AtomicOperationFailed {
            reason: reason.into(),
        }
    }

    /// 创建回滚失败错误
    pub fn rollback_failed<S: Into<String>>(reason: S) -> Self {
        Self::RollbackFailed {
            reason: reason.into(),
        }
    }

    /// 创建下载失败错误
    pub fn download_failed<S: Into<String>>(url: S) -> Self {
        Self::DownloadFailed { url: url.into() }
    }

    /// 创建验证失败错误
    pub fn verification_failed<S: Into<String>>(reason: S) -> Self {
        Self::VerificationFailed {
            reason: reason.into(),
        }
    }

    /// 创建解压失败错误
    pub fn extraction_failed<S: Into<String>>(reason: S) -> Self {
        Self::ExtractionFailed {
            reason: reason.into(),
        }
    }

    /// 创建哈希校验失败错误
    pub fn hash_mismatch<S: Into<String>>(expected: S, actual: S) -> Self {
        Self::HashMismatch {
            expected: expected.into(),
            actual: actual.into(),
        }
    }

    /// 创建数字签名验证失败错误
    pub fn signature_verification_failed<S: Into<String>>(reason: S) -> Self {
        Self::SignatureVerificationFailed {
            reason: reason.into(),
        }
    }

    /// 创建不支持的操作错误
    pub fn unsupported_operation<S: Into<String>>(operation: S) -> Self {
        Self::UnsupportedOperation {
            operation: operation.into(),
        }
    }

    /// 检查是否是可恢复的错误
    pub fn is_recoverable(&self) -> bool {
        match self {
            Self::IoError(_) => true,
            Self::HttpError(_) => true,
            Self::DownloadFailed { .. } => true,
            Self::TempFileError(_) => true,
            Self::VerificationFailed { .. } => false,
            Self::HashMismatch { .. } => false,
            Self::SignatureVerificationFailed { .. } => false,
            Self::PermissionError { .. } => false,
            Self::UnsupportedOperation { .. } => false,
            Self::BackupNotEnabled => false,
            Self::PatchSourceNotSet => false,
            _ => true,
        }
    }

    /// 检查是否需要回滚
    pub fn requires_rollback(&self) -> bool {
        match self {
            Self::VerificationFailed { .. } => false,
            Self::HashMismatch { .. } => false,
            Self::SignatureVerificationFailed { .. } => false,
            Self::DownloadFailed { .. } => false,
            Self::BackupNotEnabled => false,
            Self::PatchSourceNotSet => false,
            _ => true,
        }
    }
}

/// Result 类型别名
pub type Result<T> = std::result::Result<T, PatchExecutorError>;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_error_creation() {
        let error = PatchExecutorError::custom("test error");
        assert!(matches!(error, PatchExecutorError::Custom { .. }));
        assert_eq!(error.to_string(), "补丁执行错误: test error");
    }

    #[test]
    fn test_error_recoverability() {
        let recoverable = PatchExecutorError::download_failed("http://example.com");
        assert!(recoverable.is_recoverable());

        let non_recoverable = PatchExecutorError::hash_mismatch("abc", "def");
        assert!(!non_recoverable.is_recoverable());
    }

    #[test]
    fn test_rollback_requirement() {
        let requires_rollback = PatchExecutorError::atomic_operation_failed("test");
        assert!(requires_rollback.requires_rollback());

        let no_rollback = PatchExecutorError::verification_failed("test");
        assert!(!no_rollback.requires_rollback());
    }
}
