// client-core/src/patch_executor/mod.rs
//! 增量升级补丁执行器模块
//!
//! 本模块负责处理增量升级的核心逻辑，包括：
//! - 文件操作执行器：安全的文件替换、删除和回滚
//! - 补丁包处理器：下载、验证和解压补丁包
//! - 主补丁执行器：协调整个补丁应用流程

pub mod error;
pub mod file_operations;
pub mod patch_processor;

// 重新导出主要接口
pub use error::PatchExecutorError;
pub use file_operations::FileOperationExecutor;
pub use patch_processor::PatchProcessor;

use crate::api_types::{PatchOperations, PatchPackageInfo};
use std::path::{Path, PathBuf};
use tracing::{debug, error, info, warn};

/// 主补丁执行器
///
/// 负责协调整个补丁应用流程，包括下载、验证、解压和应用补丁
pub struct PatchExecutor {
    /// 工作目录
    work_dir: PathBuf,
    /// 文件操作执行器
    file_executor: FileOperationExecutor,
    /// 补丁处理器
    patch_processor: PatchProcessor,
    /// 是否启用了备份
    backup_enabled: bool,
}

impl PatchExecutor {
    /// 创建新的补丁执行器
    pub fn new(work_dir: PathBuf) -> Result<Self, PatchExecutorError> {
        let file_executor = FileOperationExecutor::new(work_dir.clone())?;
        let patch_processor = PatchProcessor::new()?;

        Ok(Self {
            work_dir,
            file_executor,
            patch_processor,
            backup_enabled: false,
        })
    }

    /// 启用备份模式（支持回滚）
    pub fn enable_backup(&mut self) -> Result<(), PatchExecutorError> {
        self.file_executor.enable_backup()?;
        self.backup_enabled = true;
        info!("📦 已启用补丁执行备份模式");
        Ok(())
    }

    /// 应用补丁包
    ///
    /// # 参数
    /// * `patch_info` - 补丁包信息
    /// * `operations` - 补丁操作定义
    /// * `progress_callback` - 进度回调函数
    pub async fn apply_patch<F>(
        &mut self,
        patch_info: &PatchPackageInfo,
        operations: &PatchOperations,
        progress_callback: F,
    ) -> Result<(), PatchExecutorError>
    where
        F: Fn(f64) + Send + Sync,
    {
        info!("🔄 开始应用增量补丁...");
        progress_callback(0.0);

        // 验证前置条件
        self.validate_preconditions(operations)?;
        progress_callback(0.05);

        // 执行补丁应用流程
        match self
            .execute_patch_pipeline(patch_info, operations, &progress_callback)
            .await
        {
            Ok(_) => {
                progress_callback(1.0);
                info!("✅ 增量补丁应用完成");
                Ok(())
            }
            Err(e) => {
                error!("❌ 补丁应用失败: {}", e);

                // 根据错误类型决定是否回滚
                if e.requires_rollback() && self.backup_enabled {
                    warn!("🔄 开始自动回滚...");
                    if let Err(rollback_err) = self.rollback().await {
                        error!("❌ 回滚失败: {}", rollback_err);
                        return Err(PatchExecutorError::rollback_failed(format!(
                            "原始错误: {e}, 回滚错误: {rollback_err}"
                        )));
                    }
                    info!("✅ 自动回滚完成");
                }

                Err(e)
            }
        }
    }

    /// 验证前置条件
    fn validate_preconditions(
        &self,
        operations: &PatchOperations,
    ) -> Result<(), PatchExecutorError> {
        debug!("验证补丁应用前置条件");

        // 验证工作目录存在且可写
        if !self.work_dir.exists() {
            return Err(PatchExecutorError::path_error(format!(
                "工作目录不存在: {:?}",
                self.work_dir
            )));
        }

        // 验证操作不为空
        let total_operations = operations.total_operations();

        if total_operations == 0 {
            return Err(PatchExecutorError::custom("补丁操作为空"));
        }

        debug!("前置条件验证通过，共 {} 个操作", total_operations);
        Ok(())
    }

    /// 执行补丁应用管道
    async fn execute_patch_pipeline<F>(
        &mut self,
        patch_info: &PatchPackageInfo,
        operations: &PatchOperations,
        progress_callback: &F,
    ) -> Result<(), PatchExecutorError>
    where
        F: Fn(f64) + Send + Sync,
    {
        // 1. 下载并验证补丁包
        info!("📥 下载补丁包...");
        let patch_path = self.patch_processor.download_patch(patch_info).await?;
        progress_callback(0.25);

        // 2. 验证补丁完整性和签名
        info!("🔍 验证补丁完整性...");
        self.patch_processor
            .verify_patch_integrity(&patch_path, patch_info)
            .await?;
        progress_callback(0.35);

        // 3. 解压补丁包
        info!("📦 解压补丁包...");
        let extracted_path = self.patch_processor.extract_patch(&patch_path).await?;
        progress_callback(0.45);

        // 4. 验证解压后的文件结构
        info!("🔍 验证补丁文件结构...");
        self.validate_patch_structure(&extracted_path, operations)
            .await?;
        progress_callback(0.5);

        // 5. 应用补丁操作
        info!("🔧 应用补丁操作...");
        self.apply_patch_operations(&extracted_path, operations, progress_callback)
            .await?;

        Ok(())
    }

    /// 验证补丁文件结构
    async fn validate_patch_structure(
        &self,
        extracted_path: &Path,
        operations: &PatchOperations,
    ) -> Result<(), PatchExecutorError> {
        // 收集所有需要的文件
        let mut required_files = Vec::new();

        // 添加需要替换的文件
        if let Some(replace) = &operations.replace {
            for file in &replace.files {
                required_files.push(file.clone());
            }
            // 添加需要替换的目录（检查目录是否存在）
            for dir in &replace.directories {
                let dir_path = extracted_path.join(dir);
                if !dir_path.exists() || !dir_path.is_dir() {
                    return Err(PatchExecutorError::verification_failed(format!(
                        "补丁中缺少必需的目录: {dir}"
                    )));
                }
            }
        }

        // 验证文件结构
        self.patch_processor
            .validate_extracted_structure(&required_files)
            .await?;

        debug!("补丁文件结构验证通过");
        Ok(())
    }

    /// 应用补丁操作
    async fn apply_patch_operations<F>(
        &mut self,
        extracted_path: &Path,
        operations: &PatchOperations,
        progress_callback: &F,
    ) -> Result<(), PatchExecutorError>
    where
        F: Fn(f64) + Send + Sync,
    {
        // 设置补丁源目录
        self.file_executor.set_patch_source(extracted_path)?;

        // 计算总操作数用于进度计算
        let total_operations = operations.total_operations();

        let mut completed_operations = 0;

        let base_progress = 0.5; // 前面的步骤已经完成50%
        let operations_progress_range = 0.5; // 操作占50%进度

        // 执行文件替换
        if let Some(replace) = &operations.replace {
            // 如果有文件需要替换
            if !replace.files.is_empty() {
                info!("📄 替换 {} 个文件", &replace.files.len());
                self.file_executor.replace_files(&replace.files).await?;
                completed_operations += replace.files.len();
                let progress = base_progress
                    + (completed_operations as f64 / total_operations as f64)
                        * operations_progress_range;
                progress_callback(progress);
            }

            // 执行目录替换
            if !replace.directories.is_empty() {
                info!("📁 替换 {} 个目录", &replace.directories.len());
                self.file_executor
                    .replace_directories(&replace.directories)
                    .await?;
                completed_operations += replace.directories.len();
                let progress = base_progress
                    + (completed_operations as f64 / total_operations as f64)
                        * operations_progress_range;
                progress_callback(progress);
            }
        }

        // 执行删除操作
        if let Some(delete) = &operations.delete {
            // 如果有文件需要删除
            if !delete.files.is_empty() {
                info!("🗑️ 删除 {} 个项目", &delete.files.len());
                self.file_executor.delete_items(&delete.files).await?;
                completed_operations += &delete.files.len();
                let progress = base_progress
                    + (completed_operations as f64 / total_operations as f64)
                        * operations_progress_range;
                progress_callback(progress);
            }
            // 如果有目录需要删除
            if !delete.directories.is_empty() {
                info!("🗑️ 删除 {} 个目录", &delete.directories.len());
                self.file_executor.delete_items(&delete.directories).await?;
                completed_operations += &delete.directories.len();
                let progress = base_progress
                    + (completed_operations as f64 / total_operations as f64)
                        * operations_progress_range;
                progress_callback(progress);
            }
        }

        info!("✅ 补丁操作应用完成");
        Ok(())
    }

    /// 回滚补丁操作
    pub async fn rollback(&mut self) -> Result<(), PatchExecutorError> {
        if !self.backup_enabled {
            return Err(PatchExecutorError::BackupNotEnabled);
        }

        warn!("🔙 开始回滚补丁操作...");
        self.file_executor.rollback().await?;
        info!("✅ 补丁回滚完成");
        Ok(())
    }

    /// 获取工作目录
    pub fn work_dir(&self) -> &Path {
        &self.work_dir
    }

    /// 检查是否启用了备份
    pub fn is_backup_enabled(&self) -> bool {
        self.backup_enabled
    }

    /// 获取操作摘要
    pub fn get_operation_summary(&self, operations: &PatchOperations) -> String {
        let mut replace_file_count = 0;
        let mut replace_dir_count = 0;
        let mut delete_file_count = 0;
        let mut delete_dir_count = 0;
        if let Some(replace) = &operations.replace {
            replace_file_count = replace.files.len();
            replace_dir_count = replace.directories.len();
        }
        if let Some(delete) = &operations.delete {
            delete_file_count = delete.files.len();
            delete_dir_count = delete.directories.len();
        }
        let total = operations.total_operations();
        format!(
            "补丁操作摘要: 总共 {total} 个操作 (文件替换: {replace_file_count}, 目录替换: {replace_dir_count}, 文件删除: {delete_file_count}, 目录删除: {delete_dir_count})"
        )
    }

    /// 获取补丁处理器的临时目录（用于调试）
    pub fn temp_dir(&self) -> &Path {
        self.patch_processor.temp_dir()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::api_types::ReplaceOperations;
    use tempfile::TempDir;

    #[tokio::test]
    async fn test_patch_executor_creation() {
        let temp_dir = TempDir::new().unwrap();
        let executor = PatchExecutor::new(temp_dir.path().to_owned());
        assert!(executor.is_ok());
    }

    #[tokio::test]
    async fn test_enable_backup() {
        let temp_dir = TempDir::new().unwrap();
        let mut executor = PatchExecutor::new(temp_dir.path().to_owned()).unwrap();

        assert!(!executor.is_backup_enabled());
        let result = executor.enable_backup();
        assert!(result.is_ok());
        assert!(executor.is_backup_enabled());
    }

    #[tokio::test]
    async fn test_validate_preconditions() {
        let temp_dir = TempDir::new().unwrap();
        let executor = PatchExecutor::new(temp_dir.path().to_owned()).unwrap();

        // 测试有效的操作
        let valid_operations = PatchOperations {
            replace: Some(ReplaceOperations {
                files: vec!["test.txt".to_string()],
                directories: vec!["test_dir".to_string()],
            }),
            delete: Some(ReplaceOperations {
                files: vec!["test.txt".to_string()],
                directories: vec!["test_dir".to_string()],
            }),
        };

        let result = executor.validate_preconditions(&valid_operations);
        assert!(result.is_ok());

        // 测试空操作
        let empty_operations = PatchOperations {
            replace: Some(ReplaceOperations {
                files: vec![],
                directories: vec![],
            }),
            delete: Some(ReplaceOperations {
                files: vec![],
                directories: vec![],
            }),
        };

        let result = executor.validate_preconditions(&empty_operations);
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_operation_summary() {
        let temp_dir = TempDir::new().unwrap();
        let executor = PatchExecutor::new(temp_dir.path().to_owned()).unwrap();

        let operations = PatchOperations {
            replace: Some(ReplaceOperations {
                files: vec!["file1.txt".to_string(), "file2.txt".to_string()],
                directories: vec!["dir1".to_string()],
            }),
            delete: Some(ReplaceOperations {
                files: vec!["old_file.txt".to_string()],
                directories: vec![],
            }),
        };

        let summary = executor.get_operation_summary(&operations);
        assert!(summary.contains("总共 4 个操作"));
        assert!(summary.contains("文件替换: 2"));
        assert!(summary.contains("目录替换: 1"));
        assert!(summary.contains("删除: 1"));
    }

    #[tokio::test]
    async fn test_rollback_without_backup() {
        let temp_dir = TempDir::new().unwrap();
        let mut executor = PatchExecutor::new(temp_dir.path().to_owned()).unwrap();

        // 测试未启用备份时的回滚
        let result = executor.rollback().await;
        assert!(result.is_err());
        assert!(matches!(
            result.unwrap_err(),
            PatchExecutorError::BackupNotEnabled
        ));
    }
}
