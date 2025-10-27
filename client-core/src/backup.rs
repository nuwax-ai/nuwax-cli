use crate::{
    container::DockerManager,
    database::{BackupRecord, BackupStatus, BackupType, Database},
    error::DuckError,
};
use anyhow::Result;
use chrono::Utc;
use flate2::Compression;
use flate2::read::GzDecoder;
use flate2::write::GzEncoder;
use std::{fs::File, sync::Arc};
use std::path::{Path, PathBuf};
use tar::Archive;
use tar::Builder;
use tracing::{debug, error, info};
use walkdir::WalkDir;

/// 备份管理器
#[derive(Debug, Clone)]
pub struct BackupManager {
    storage_dir: PathBuf,
    database: Arc<Database>,
    docker_manager: Arc<DockerManager>,
}

/// 备份选项
#[derive(Debug, Clone)]
pub struct BackupOptions {
    /// 备份类型
    pub backup_type: BackupType,
    /// 服务版本
    pub service_version: String,
    /// 工作目录
    pub work_dir: PathBuf,
    /// 要备份的文件或目录列表
    pub source_paths: Vec<PathBuf>,
    /// 压缩级别 (0-9)
    pub compression_level: u32,
}

/// 恢复选项
#[derive(Debug, Clone)]
pub struct RestoreOptions {
    /// 目标目录
    pub target_dir: PathBuf,
    /// 是否强制覆盖
    pub force_overwrite: bool,
}

impl BackupManager {
    /// 创建新的备份管理器
    pub fn new(
        storage_dir: PathBuf,
        database: Arc<Database>,
        docker_manager: Arc<DockerManager>,
    ) -> Result<Self> {
        if !storage_dir.exists() {
            std::fs::create_dir_all(&storage_dir)?;
        }

        Ok(Self {
            storage_dir,
            database,
            docker_manager,
        })
    }

    /// 创建备份
    pub async fn create_backup(&self, options: BackupOptions) -> Result<BackupRecord> {
        // 检查所有源路径是否存在
        let need_backup_paths = options.source_paths;

        // 生成备份文件名（人类易读格式）
        let timestamp = Utc::now().format("%Y-%m-%d_%H-%M-%S");
        let backup_type_str = match options.backup_type {
            BackupType::Manual => "manual",
            BackupType::PreUpgrade => "pre-upgrade",
        };

        let backup_filename = format!(
            "backup_{}_v{}_{}.tar.gz",
            backup_type_str, options.service_version, timestamp
        );

        let backup_path = self.storage_dir.join(&backup_filename);

        info!("开始创建备份: {}", backup_path.display());

        // 执行备份
        match self
            .perform_backup(&need_backup_paths, &backup_path, options.compression_level)
            .await
        {
            Ok(_) => {
                info!("备份创建成功: {}", backup_path.display());

                // 记录到数据库
                let record_id = self
                    .database
                    .create_backup_record(
                        backup_path.to_string_lossy().to_string(),
                        options.service_version,
                        options.backup_type,
                        BackupStatus::Completed,
                    )
                    .await?;

                // 获取创建的记录
                self.database
                    .get_backup_by_id(record_id)
                    .await?
                    .ok_or_else(|| anyhow::anyhow!("无法获取刚创建的备份记录"))
            }
            Err(e) => {
                error!("备份创建失败: {}", e);

                // 记录失败到数据库
                self.database
                    .create_backup_record(
                        backup_path.to_string_lossy().to_string(),
                        options.service_version,
                        options.backup_type,
                        BackupStatus::Failed,
                    )
                    .await?;

                Err(e)
            }
        }
    }

    /// 执行实际的备份操作
    ///
    /// 支持备份目录和单个文件：
    /// - 当传入目录路径时，将递归备份该目录下的所有文件
    /// - 当传入文件路径时，将直接备份该文件
    async fn perform_backup(
        &self,
        source_paths: &[PathBuf],
        backup_path: &Path,
        compression_level: u32,
    ) -> Result<()> {
        // 确保备份目录存在
        if let Some(parent) = backup_path.parent() {
            tokio::fs::create_dir_all(parent).await?;
        }

        // 在后台线程中执行压缩操作，避免阻塞异步运行时
        let source_paths = source_paths.to_vec();
        let backup_path = backup_path.to_path_buf();

        tokio::task::spawn_blocking(move || {
            let file = File::create(&backup_path)?;
            let compression = Compression::new(compression_level);
            let encoder = GzEncoder::new(file, compression);
            let mut archive = Builder::new(encoder);

            // 遍历所有源路径并添加到归档中
            for source_path in &source_paths {
                if source_path.is_file() {
                    // 直接处理单个文件
                    add_file_to_archive(&mut archive, source_path, None)?;
                } else if source_path.is_dir() {
                    let dir_name = source_path
                        .file_name()
                        .ok_or_else(|| anyhow::anyhow!("无法获取目录名"))?
                        .to_string_lossy()
                        .to_string();

                    // 递归处理目录
                    for entry in WalkDir::new(source_path) {
                        let entry = entry.map_err(|e| anyhow::anyhow!("遍历目录失败: {e}"))?;
                        let path = entry.path();

                        if path.is_file() {
                            add_file_to_archive(
                                &mut archive,
                                path,
                                Some((source_path, &dir_name)),
                            )?;
                        }
                    }
                } else {
                    //可能是新增的文件或者目录,这里无法备份,只打印日志
                    info!("文件或者目录不存在,无需备份: {}", source_path.display());
                }
            }

            archive
                .finish()
                .map_err(|e| anyhow::anyhow!("完成归档失败: {e}"))?;

            Ok::<(), anyhow::Error>(())
        })
        .await??;

        Ok(())
    }

    /// 只恢复数据文件，保留配置文件的智能恢复
    pub async fn restore_data_from_backup_with_exculde(
        &self,
        backup_id: i64,
        target_dir: &Path,
        auto_start_service: bool,
        dirs_to_exculde: &[&str],
    ) -> Result<()> {
        // 获取备份记录
        let backup_record = self
            .database
            .get_backup_by_id(backup_id)
            .await?
            .ok_or_else(|| anyhow::anyhow!("备份记录不存在: {backup_id}"))?;

        let backup_path = PathBuf::from(&backup_record.file_path);
        if !backup_path.exists() {
            return Err(anyhow::anyhow!("备份文件不存在: {}", backup_path.display()));
        }

        info!("开始智能数据恢复: {}", backup_path.display());
        info!("目标目录: {}", target_dir.display());

        // 停止服务，准备恢复
        info!("正在停止服务...");
        self.docker_manager.stop_services().await?;

        // 清理现有数据目录，但保留配置文件
        self.clear_data_directories(target_dir, dirs_to_exculde)
            .await?;

        // 执行恢复
        self.perform_restore(&backup_path, target_dir, dirs_to_exculde)
            .await?;

        // 根据参数决定是否启动服务
        if auto_start_service {
            info!("数据恢复完成，正在启动服务...");
            self.docker_manager.start_services().await?;
            info!("数据已成功恢复并启动: {}", target_dir.display());
        } else {
            info!("数据恢复完成，启动服务已跳过（由上级流程控制）");
            info!("数据已成功恢复: {}", target_dir.display());
        }

        Ok(())
    }

    /// 只恢复 data 目录，保留 app 目录和配置文件
    pub async fn restore_data_directory_only(
        &self,
        backup_id: i64,
        target_dir: &Path,
        auto_start_service: bool,
        dirs_to_restore: &[&str],
    ) -> Result<()> {
        // 获取备份记录
        let backup_record = self
            .database
            .get_backup_by_id(backup_id)
            .await?
            .ok_or_else(|| anyhow::anyhow!("备份记录不存在: {backup_id}"))?;

        let backup_path = PathBuf::from(&backup_record.file_path);
        if !backup_path.exists() {
            return Err(anyhow::anyhow!("备份文件不存在: {}", backup_path.display()));
        }

        info!("开始 data 目录恢复: {}", backup_path.display());
        info!("目标目录: {}", target_dir.display());

        // 停止服务，准备恢复
        info!("正在停止服务...");
        self.docker_manager.stop_services().await?;

        // 只清理 data 目录，保留 app 目录和配置文件
        self.clear_data_directory_only(target_dir).await?;

        // 执行选择性恢复：只恢复 data 目录
        self.perform_selective_restore(&backup_path, target_dir, dirs_to_restore)
            .await?;

        // 根据参数决定是否启动服务
        if auto_start_service {
            info!("data 目录恢复完成，正在启动服务...");
            self.docker_manager.start_services().await?;
            info!("data 目录已成功恢复并启动: {}", target_dir.display());
        } else {
            info!("data 目录恢复完成，启动服务已跳过（由上级流程控制）");
            info!("data 目录已成功恢复: {}", target_dir.display());
        }

        Ok(())
    }

    /// 清理数据目录
    async fn clear_data_directories(
        &self,
        docker_dir: &Path,
        dirs_to_exculde: &[&str],
    ) -> Result<()> {
        let mut data_dirs_to_clear: Vec<String> = vec!["data".to_string(), "app".to_string()];
        // Filter out directories that should be excluded from clearing
        data_dirs_to_clear.retain(|dir| !dirs_to_exculde.contains(&dir.as_str()));

        for dir_name in data_dirs_to_clear.iter() {
            let dir_path = docker_dir.join(dir_name);
            if dir_path.exists() {
                info!("清理数据目录: {}", dir_path.display());
                tokio::fs::remove_dir_all(&dir_path).await?;
            }
        }

        info!("数据目录清理完成，配置文件已保留");
        Ok(())
    }

    /// 只清理 data 目录，保留 app 目录和配置文件
    async fn clear_data_directory_only(&self, docker_dir: &Path) -> Result<()> {
        let data_dir = docker_dir.join("data");
        if data_dir.exists() {
            info!("清理 data 目录: {}", data_dir.display());
            tokio::fs::remove_dir_all(&data_dir).await?;
        }

        info!("data 目录清理完成，app 目录和配置文件已保留");
        Ok(())
    }

    /// 执行选择性恢复操作：只恢复指定的目录
    async fn perform_selective_restore(
        &self,
        backup_path: &Path,
        target_dir: &Path,
        dirs_to_restore: &[&str],
    ) -> Result<()> {
        use flate2::read::GzDecoder;
        use std::fs::File;
        use tar::Archive;

        // 确保目标目录存在
        tokio::fs::create_dir_all(target_dir).await?;

        let backup_path = backup_path.to_path_buf();
        let target_dir = target_dir.to_path_buf();
        let dirs_to_restore: Vec<String> = dirs_to_restore.iter().map(|s| s.to_string()).collect();

        // 在后台线程中执行解压操作
        tokio::task::spawn_blocking(move || {
            let file = File::open(&backup_path)?;
            let decoder = GzDecoder::new(file);
            let mut archive = Archive::new(decoder);

            // 遍历归档中的所有条目
            for entry in archive.entries()? {
                let mut entry =
                    entry.map_err(|e| DuckError::Backup(format!("读取归档条目失败: {e}")))?;

                // 获取条目路径
                let entry_path = entry
                    .path()
                    .map_err(|e| DuckError::Backup(format!("获取条目路径失败: {e}")))?;
                let entry_path_str = entry_path.to_string_lossy();

                // 检查是否是我们要恢复的目录
                let should_restore = dirs_to_restore
                    .iter()
                    .any(|dir| entry_path_str.starts_with(&format!("{dir}/")));

                if should_restore {
                    // 计算解压到的目标路径
                    let target_path = target_dir.join(&*entry_path);

                    // 确保父目录存在
                    if let Some(parent) = target_path.parent() {
                        std::fs::create_dir_all(parent)?;
                    }

                    // 解压文件
                    entry.unpack(&target_path).map_err(|e| {
                        DuckError::Backup(format!("解压文件失败 {}: {e}", target_path.display()))
                    })?;

                    debug!("恢复文件: {}", target_path.display());
                }
            }

            Ok::<(), DuckError>(())
        })
        .await??;

        Ok(())
    }

    /// 执行实际的恢复操作, 可以指定排除的目录,比如回滚恢复的时候,排除 data目录,不会滚数据
    async fn perform_restore(
        &self,
        backup_path: &Path,
        target_dir: &Path,
        dirs_to_exculde: &[&str],
    ) -> Result<()> {
        // 确保目标目录存在
        tokio::fs::create_dir_all(target_dir).await?;

        let backup_path = backup_path.to_path_buf();
        let target_dir = target_dir.to_path_buf();
        let dirs_to_exclude: Vec<String> = dirs_to_exculde.iter().map(|s| s.to_string()).collect();

        // 在后台线程中执行解压操作
        tokio::task::spawn_blocking(move || {
            let file = File::open(&backup_path)?;
            let decoder = GzDecoder::new(file);
            let mut archive = Archive::new(decoder);

            let mut debug_dirs = std::collections::HashSet::new();

            // 遍历归档中的所有条目
            for entry in archive.entries()? {
                let mut entry =
                    entry.map_err(|e| DuckError::Backup(format!("读取归档条目失败: {e}")))?;

                // 获取条目路径
                let entry_path = entry
                    .path()
                    .map_err(|e| DuckError::Backup(format!("获取条目路径失败: {e}")))?;
                let entry_path_str = entry_path.to_string_lossy();

                // Split path into components
                let path_components: Vec<&str> = entry_path_str.split('/').collect();

                // Check if this is a directory we want to exclude (first level)
                let should_exclude = if !path_components.is_empty() {
                    let first_level_dir = path_components[0];
                    debug_dirs.insert(first_level_dir.to_string());

                    dirs_to_exclude
                        .iter()
                        .any(|dir| dir.as_str() == first_level_dir)
                } else {
                    false // Not enough path components, don't exclude
                };

                if !should_exclude {
                    // 计算解压到的目标路径
                    let target_path = target_dir.join(&*entry_path);

                    // 确保父目录存在
                    if let Some(parent) = target_path.parent() {
                        std::fs::create_dir_all(parent)?;
                    }

                    // 解压文件
                    entry.unpack(&target_path).map_err(|e| {
                        DuckError::Backup(format!("解压文件失败 {}: {e}", target_path.display()))
                    })?;

                    debug!("恢复文件: {}", target_path.display());
                }
            }

            debug!("测试日志,恢复目录: {:?}", debug_dirs);

            Ok::<(), DuckError>(())
        })
        .await??;

        Ok(())
    }

    /// 获取所有备份记录
    pub async fn list_backups(&self) -> Result<Vec<BackupRecord>> {
        self.database.get_all_backups().await
    }

    /// 删除备份
    pub async fn delete_backup(&self, backup_id: i64) -> Result<()> {
        // 获取备份记录
        let backup_record = self
            .database
            .get_backup_by_id(backup_id)
            .await?
            .ok_or_else(|| DuckError::Backup(format!("备份记录不存在: {backup_id}")))?;

        let backup_path = PathBuf::from(&backup_record.file_path);

        // 删除文件
        if backup_path.exists() {
            tokio::fs::remove_file(&backup_path).await?;
            info!("删除备份文件: {}", backup_path.display());
        }

        // 从数据库中删除记录
        self.database.delete_backup_record(backup_id).await?;

        Ok(())
    }

    /// 检查并迁移备份存储目录
    pub async fn migrate_storage_directory(&self, new_storage_dir: &Path) -> Result<()> {
        if new_storage_dir == self.storage_dir {
            return Ok(()); // 没有变化
        }

        info!(
            "开始迁移备份存储目录: {} -> {}",
            self.storage_dir.display(),
            new_storage_dir.display()
        );

        // 创建新目录
        tokio::fs::create_dir_all(new_storage_dir).await?;

        // 获取所有备份记录
        let backups = self.list_backups().await?;

        for backup in backups {
            let old_path = PathBuf::from(&backup.file_path);
            if old_path.exists() {
                let filename = old_path
                    .file_name()
                    .ok_or_else(|| DuckError::Backup("无法获取备份文件名".to_string()))?;
                let new_path = new_storage_dir.join(filename);

                // 移动文件
                tokio::fs::rename(&old_path, &new_path).await?;
                info!(
                    "迁移备份文件: {} -> {}",
                    old_path.display(),
                    new_path.display()
                );

                // 更新数据库中的路径
                self.database
                    .update_backup_file_path(backup.id, new_path.to_string_lossy().to_string())
                    .await?;
            }
        }

        info!("备份存储目录迁移完成");
        Ok(())
    }

    /// 获取存储目录
    pub fn get_storage_dir(&self) -> &Path {
        &self.storage_dir
    }

    /// 估算目录大小
    pub async fn estimate_backup_size(&self, source_dir: &Path) -> Result<u64> {
        let source_dir = source_dir.to_path_buf();

        let total_size = tokio::task::spawn_blocking(move || {
            let mut total = 0u64;

            for entry in WalkDir::new(&source_dir).into_iter().flatten() {
                if entry.path().is_file() {
                    if let Ok(metadata) = entry.metadata() {
                        total += metadata.len();
                    }
                }
            }

            total
        })
        .await?;

        // 考虑压缩率，估算压缩后大小约为原大小的 30-50%
        Ok(total_size / 2)
    }
}

// 用于将文件添加到归档中
fn add_file_to_archive(
    archive: &mut Builder<GzEncoder<File>>,
    file_path: &Path,
    base_info: Option<(&Path, &str)>,
) -> Result<()> {
    let archive_path = if let Some((base_dir, dir_name)) = base_info {
        // 文件是目录的一部分，计算相对路径
        let relative_path = file_path
            .strip_prefix(base_dir)
            .map_err(|e| DuckError::Backup(format!("计算相对路径失败: {e}")))?;

        // 格式：{dir_name}/{relative_path}
        if cfg!(windows) {
            format!(
                "{}/{}",
                dir_name,
                relative_path.display().to_string().replace('\\', "/")
            )
        } else {
            format!("{}/{}", dir_name, relative_path.display())
        }
    } else {
        // 直接处理单个文件，保持原有路径结构
        let path_str = file_path.to_string_lossy().to_string();

        // 标准化路径分隔符为Unix风格
        let path_str = if cfg!(windows) {
            path_str.replace('\\', "/")
        } else {
            path_str
        };

        // 移除路径开头可能的 "./" 前缀
        if path_str.starts_with("./") {
            path_str[2..].to_string()
        } else {
            path_str
        }
    };

    debug!(
        "添加文件到归档: {} -> {}",
        file_path.display(),
        archive_path
    );

    archive
        .append_path_with_name(file_path, archive_path)
        .map_err(|e| DuckError::Backup(format!("添加文件到归档失败: {e}")))?;

    Ok(())
}
