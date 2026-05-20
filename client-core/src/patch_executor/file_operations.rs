// client-core/src/patch_executor/file_operations.rs
//! 文件操作执行器
//!
//! 负责安全的文件替换、删除和回滚操作

use super::error::{PatchExecutorError, Result};
use fs_extra::dir;
use remove_dir_all::remove_dir_all;
use std::path::{Path, PathBuf};
use tempfile::{NamedTempFile, TempDir};
use tokio::fs;
use tracing::{debug, info, warn};
use walkdir::WalkDir;

/// 文件操作执行器
pub struct FileOperationExecutor {
    /// 工作目录
    work_dir: PathBuf,
    /// 备份目录（用于回滚）
    backup_dir: Option<TempDir>,
    /// 补丁源目录
    patch_source: Option<PathBuf>,
}

impl FileOperationExecutor {
    /// 创建新的文件操作执行器
    pub fn new(work_dir: PathBuf) -> Result<Self> {
        if !work_dir.exists() {
            return Err(PatchExecutorError::path_error(format!(
                "Working directory does not exist: {work_dir:?}"
            )));
        }

        debug!("Creating file operation executor, working directory: {:?}", work_dir);

        Ok(Self {
            work_dir,
            backup_dir: None,
            patch_source: None,
        })
    }

    /// 启用备份模式（支持回滚）
    pub fn enable_backup(&mut self) -> Result<()> {
        self.backup_dir = Some(TempDir::new()?);
        info!("File operation backup mode enabled");
        Ok(())
    }

    /// 设置补丁源目录
    pub fn set_patch_source(&mut self, patch_source: &Path) -> Result<()> {
        if !patch_source.exists() {
            return Err(PatchExecutorError::path_error(format!(
                "Patch source directory does not exist: {patch_source:?}"
            )));
        }

        self.patch_source = Some(patch_source.to_owned());
        debug!("Setting patch source directory: {:?}", patch_source);
        Ok(())
    }

    /// 执行文件替换操作
    pub async fn replace_files(&self, files: &[String]) -> Result<()> {
        info!("Starting to replace {} files", files.len());

        for file_path in files {
            self.replace_single_file(file_path).await?;
        }

        info!("File replacement completed");
        Ok(())
    }

    /// 执行目录替换操作
    pub async fn replace_directories(&self, directories: &[String]) -> Result<()> {
        info!("Starting to replace {} directories", directories.len());

        for dir_path in directories {
            self.replace_single_directory(dir_path).await?;
        }

        info!("Directory replacement completed");
        Ok(())
    }

    /// 执行删除操作
    pub async fn delete_items(&self, items: &[String]) -> Result<()> {
        info!("Starting to delete {} items", items.len());

        for item_path in items {
            self.delete_single_item(item_path).await?;
        }

        info!("Delete operation completed");
        Ok(())
    }

    /// 替换单个文件
    async fn replace_single_file(&self, file_path: &str) -> Result<()> {
        let target_path = self.work_dir.join(file_path);

        // 获取补丁源路径
        let source_path = self.get_patch_source_path(file_path)?;

        // 创建备份
        if let Some(backup_dir) = &self.backup_dir
            && target_path.exists() {
                let backup_path = backup_dir.path().join(file_path);
                if let Some(parent) = backup_path.parent() {
                    fs::create_dir_all(parent).await?;
                }
                fs::copy(&target_path, &backup_path).await?;
                debug!("Backed up file: {} -> {:?}", file_path, backup_path);
            }

        // 原子性替换
        self.atomic_file_replace(&source_path, &target_path).await?;

        info!("File replaced: {}", file_path);
        Ok(())
    }

    /// 替换单个目录
    async fn replace_single_directory(&self, dir_path: &str) -> Result<()> {
        let target_path = self.work_dir.join(dir_path);

        // 获取补丁源路径
        let source_path = self.get_patch_source_path(dir_path)?;

        // 创建备份
        if let Some(backup_dir) = &self.backup_dir
            && target_path.exists() {
                let backup_path = backup_dir.path().join(dir_path);
                self.backup_directory(&target_path, &backup_path).await?;
                debug!("Backed up directory: {} -> {:?}", dir_path, backup_path);
            }

        // 删除目标目录
        if target_path.exists() {
            self.safe_remove_directory(&target_path).await?;
        }

        // 复制新目录
        self.copy_directory(&source_path, &target_path).await?;

        info!("Directory replaced: {}", dir_path);
        Ok(())
    }

    /// 删除单个项目
    async fn delete_single_item(&self, item_path: &str) -> Result<()> {
        let target_path = self.work_dir.join(item_path);

        if !target_path.exists() {
            warn!("Delete target does not exist, skipping: {}", item_path);
            return Ok(());
        }

        // 创建备份
        if let Some(backup_dir) = &self.backup_dir {
            let backup_path = backup_dir.path().join(item_path);
            if target_path.is_dir() {
                self.backup_directory(&target_path, &backup_path).await?;
            } else {
                if let Some(parent) = backup_path.parent() {
                    fs::create_dir_all(parent).await?;
                }
                fs::copy(&target_path, &backup_path).await?;
            }
            debug!("Backed up item for deletion: {} -> {:?}", item_path, backup_path);
        }

        // 执行删除
        if target_path.is_dir() {
            self.safe_remove_directory(&target_path).await?;
        } else {
            fs::remove_file(&target_path).await?;
        }

        info!("Deleted: {}", item_path);
        Ok(())
    }

    /// 获取补丁源文件路径
    fn get_patch_source_path(&self, relative_path: &str) -> Result<PathBuf> {
        let patch_source = self
            .patch_source
            .as_ref()
            .ok_or(PatchExecutorError::PatchSourceNotSet)?;

        let source_path = patch_source.join(relative_path);

        if !source_path.exists() {
            return Err(PatchExecutorError::path_error(format!(
                "Patch source file does not exist: {source_path:?}"
            )));
        }

        Ok(source_path)
    }

    /// 原子性文件替换
    async fn atomic_file_replace(&self, source: &Path, target: &Path) -> Result<()> {
        // 确保目标目录存在
        if let Some(parent) = target.parent() {
            fs::create_dir_all(parent).await?;
        }

        // 使用临时文件实现原子性替换
        let temp_file = NamedTempFile::new_in(target.parent().unwrap_or_else(|| Path::new(".")))?;

        // 复制内容
        let source_content = fs::read(source).await?;
        fs::write(temp_file.path(), source_content).await?;

        // 原子性移动
        temp_file.persist(target)?;

        debug!("Atomic replacement completed: {:?} -> {:?}", source, target);
        Ok(())
    }

    /// 安全删除目录（跨平台兼容）
    async fn safe_remove_directory(&self, path: &Path) -> Result<()> {
        let path_clone = path.to_owned();
        tokio::task::spawn_blocking(move || remove_dir_all(&path_clone))
            .await
            .map_err(|e| PatchExecutorError::custom(format!("Delete directory task failed: {e}")))??;

        debug!("Safely deleted directory: {:?}", path);
        Ok(())
    }

    /// 复制目录
    async fn copy_directory(&self, source: &Path, target: &Path) -> Result<()> {
        let source_clone = source.to_owned();
        let target_clone = target.to_owned();

        tokio::task::spawn_blocking(move || {
            let options = dir::CopyOptions::new().overwrite(true).copy_inside(true);

            // 确保目标目录的父目录存在
            if let Some(parent) = target_clone.parent() {
                std::fs::create_dir_all(parent)
                    .map_err(|e| PatchExecutorError::custom(format!("Failed to create target parent directory: {e}")))?;
            }

            // 如果目标目录不存在，创建它
            if !target_clone.exists() {
                std::fs::create_dir_all(&target_clone)
                    .map_err(|e| PatchExecutorError::custom(format!("Failed to create target directory: {e}")))?;
            }

            // 复制源目录内容到目标目录
            dir::copy(
                &source_clone,
                target_clone.parent().unwrap_or(&target_clone),
                &options,
            )
            .map_err(|e| PatchExecutorError::custom(format!("Directory copy failed: {e}")))?;

            Ok::<(), PatchExecutorError>(())
        })
        .await
        .map_err(|e| PatchExecutorError::custom(format!("Failed to copy directory task: {e}")))??;

        debug!("Directory copied: {:?} -> {:?}", source, target);
        Ok(())
    }

    /// 备份目录
    async fn backup_directory(&self, source: &Path, backup: &Path) -> Result<()> {
        if let Some(parent) = backup.parent() {
            fs::create_dir_all(parent).await?;
        }

        self.copy_directory(source, backup).await?;
        debug!("Directory backed up: {:?} -> {:?}", source, backup);
        Ok(())
    }

    /// 回滚操作
    pub async fn rollback(&self) -> Result<()> {
        if let Some(backup_dir) = &self.backup_dir {
            warn!("Starting file operation rollback...");

            // 遍历备份目录，恢复所有文件
            let backup_path = backup_dir.path().to_owned();
            let work_dir = self.work_dir.clone();

            tokio::task::spawn_blocking(move || {
                for entry in WalkDir::new(&backup_path) {
                    let entry = entry.map_err(|e| {
                        PatchExecutorError::custom(format!("Failed to traverse backup directory: {e}"))
                    })?;

                    let backup_file_path = entry.path();
                    if backup_file_path.is_file() {
                        // 计算相对路径
                        let relative_path =
                            backup_file_path.strip_prefix(&backup_path).map_err(|e| {
                                PatchExecutorError::custom(format!("Failed to calculate relative path: {e}"))
                            })?;

                        let target_path = work_dir.join(relative_path);

                        // 确保目标目录存在
                        if let Some(parent) = target_path.parent() {
                            std::fs::create_dir_all(parent).map_err(|e| {
                                PatchExecutorError::custom(format!("Failed to create rollback target directory: {e}"))
                            })?;
                        }

                        // 恢复文件
                        std::fs::copy(backup_file_path, &target_path).map_err(|e| {
                            PatchExecutorError::custom(format!("Failed to restore file: {e}"))
                        })?;

                        debug!("Restoring file: {:?} -> {:?}", backup_file_path, target_path);
                    }
                }

                Ok::<(), PatchExecutorError>(())
            })
            .await
            .map_err(|e| PatchExecutorError::custom(format!("Rollback task failed: {e}")))??;

            info!("File operation rollback completed");
        } else {
            return Err(PatchExecutorError::BackupNotEnabled);
        }

        Ok(())
    }

    /// 获取工作目录
    pub fn work_dir(&self) -> &Path {
        &self.work_dir
    }

    /// 检查是否启用了备份
    pub fn is_backup_enabled(&self) -> bool {
        self.backup_dir.is_some()
    }

    /// 获取补丁源目录
    pub fn patch_source(&self) -> Option<&Path> {
        self.patch_source.as_deref()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;
    use tokio::fs;

    #[tokio::test]
    async fn test_file_operation_executor_creation() {
        let temp_dir = TempDir::new().unwrap();
        let executor = FileOperationExecutor::new(temp_dir.path().to_owned());
        assert!(executor.is_ok());
    }

    #[tokio::test]
    async fn test_enable_backup() {
        let temp_dir = TempDir::new().unwrap();
        let mut executor = FileOperationExecutor::new(temp_dir.path().to_owned()).unwrap();

        assert!(!executor.is_backup_enabled());
        let result = executor.enable_backup();
        assert!(result.is_ok());
        assert!(executor.is_backup_enabled());
    }

    #[tokio::test]
    async fn test_invalid_work_dir() {
        let invalid_path = PathBuf::from("/nonexistent/path");
        let executor = FileOperationExecutor::new(invalid_path);
        assert!(executor.is_err());
    }

    #[tokio::test]
    async fn test_set_patch_source() {
        let temp_dir = TempDir::new().unwrap();
        let patch_source_dir = TempDir::new().unwrap();

        let mut executor = FileOperationExecutor::new(temp_dir.path().to_owned()).unwrap();
        let result = executor.set_patch_source(patch_source_dir.path());
        assert!(result.is_ok());
        assert_eq!(executor.patch_source(), Some(patch_source_dir.path()));
    }

    #[tokio::test]
    async fn test_atomic_file_replace() {
        let temp_dir = TempDir::new().unwrap();
        let executor = FileOperationExecutor::new(temp_dir.path().to_owned()).unwrap();

        // 创建源文件
        let source_file = temp_dir.path().join("source.txt");
        let content = "test content";
        fs::write(&source_file, content).await.unwrap();

        // 创建目标文件路径
        let target_file = temp_dir.path().join("target.txt");

        // 执行原子性替换
        executor
            .atomic_file_replace(&source_file, &target_file)
            .await
            .unwrap();

        // 验证目标文件内容
        let target_content = fs::read_to_string(&target_file).await.unwrap();
        assert_eq!(target_content, content);
    }

    #[tokio::test]
    async fn test_file_replacement_with_backup() {
        let temp_dir = TempDir::new().unwrap();
        let patch_source_dir = TempDir::new().unwrap();

        let mut executor = FileOperationExecutor::new(temp_dir.path().to_owned()).unwrap();
        executor.enable_backup().unwrap();
        executor.set_patch_source(patch_source_dir.path()).unwrap();

        // 创建原始文件
        let original_file = temp_dir.path().join("test.txt");
        let original_content = "original content";
        fs::write(&original_file, original_content).await.unwrap();

        // 创建补丁文件
        let patch_file = patch_source_dir.path().join("test.txt");
        let patch_content = "new content";
        fs::write(&patch_file, patch_content).await.unwrap();

        // 执行文件替换
        executor
            .replace_files(&["test.txt".to_string()])
            .await
            .unwrap();

        // 验证文件已被替换
        let new_content = fs::read_to_string(&original_file).await.unwrap();
        assert_eq!(new_content, patch_content);

        // 测试回滚
        executor.rollback().await.unwrap();

        // 验证文件已被恢复
        let restored_content = fs::read_to_string(&original_file).await.unwrap();
        assert_eq!(restored_content, original_content);
    }

    #[tokio::test]
    async fn test_directory_operations() {
        let temp_dir = TempDir::new().unwrap();
        let patch_source_dir = TempDir::new().unwrap();

        let mut executor = FileOperationExecutor::new(temp_dir.path().to_owned()).unwrap();
        executor.enable_backup().unwrap();
        executor.set_patch_source(patch_source_dir.path()).unwrap();

        // 创建原始目录和文件
        let original_dir = temp_dir.path().join("testdir");
        fs::create_dir_all(&original_dir).await.unwrap();
        fs::write(original_dir.join("file1.txt"), "original file1")
            .await
            .unwrap();

        // 创建补丁目录和文件
        let patch_dir = patch_source_dir.path().join("testdir");
        fs::create_dir_all(&patch_dir).await.unwrap();
        fs::write(patch_dir.join("file2.txt"), "new file2")
            .await
            .unwrap();

        // 执行目录替换
        executor
            .replace_directories(&["testdir".to_string()])
            .await
            .unwrap();

        // 验证目录已被替换
        assert!(!original_dir.join("file1.txt").exists());
        assert!(original_dir.join("file2.txt").exists());
        let new_content = fs::read_to_string(original_dir.join("file2.txt"))
            .await
            .unwrap();
        assert_eq!(new_content, "new file2");
    }

    #[tokio::test]
    async fn test_delete_operations() {
        let temp_dir = TempDir::new().unwrap();
        let mut executor = FileOperationExecutor::new(temp_dir.path().to_owned()).unwrap();
        executor.enable_backup().unwrap();

        // 创建要删除的文件
        let test_file = temp_dir.path().join("to_delete.txt");
        fs::write(&test_file, "delete me").await.unwrap();

        // 执行删除
        executor
            .delete_items(&["to_delete.txt".to_string()])
            .await
            .unwrap();

        // 验证文件已被删除
        assert!(!test_file.exists());

        // 测试回滚
        executor.rollback().await.unwrap();

        // 验证文件已被恢复
        assert!(test_file.exists());
        let restored_content = fs::read_to_string(&test_file).await.unwrap();
        assert_eq!(restored_content, "delete me");
    }
}
