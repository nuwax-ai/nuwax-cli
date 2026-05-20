use crate::app::CliApp;
use crate::cli::CacheCommand;
use anyhow::Result;
use std::fs;
use std::path::Path;
use tracing::{info, warn};
use walkdir::WalkDir;

/// 处理缓存命令
pub async fn handle_cache_command(app: &CliApp, cache_cmd: CacheCommand) -> Result<()> {
    match cache_cmd {
        CacheCommand::Clear => clear_cache(app).await,
        CacheCommand::Status => show_cache_status(app).await,
        CacheCommand::CleanDownloads { keep } => clean_downloads(app, keep).await,
    }
}

/// 清理所有缓存文件
async fn clear_cache(app: &CliApp) -> Result<()> {
    info!("🧹 Starting cache cleanup...");

    let cache_dir = Path::new(&app.config.cache.cache_dir);

    if !cache_dir.exists() {
        info!("Cache directory does not exist: {path}", path = cache_dir.display());
        return Ok(());
    }

    let mut total_deleted = 0;
    let mut total_size_freed = 0u64;

    // 遍历缓存目录
    for entry in fs::read_dir(cache_dir)? {
        let entry = entry?;
        let path = entry.path();

        if path.is_dir() {
            match calculate_directory_size(&path) {
                Ok(size) => {
                    total_size_freed += size;
                    if let Err(e) = fs::remove_dir_all(&path) {
                        warn!("Failed to delete directory {path}: {error}", path = path.display(), error = e.to_string());
                    } else {
                        total_deleted += 1;
                        info!("Deleted: {path}", path = path.display());
                    }
                }
                Err(e) => {
                    warn!("Failed to calculate directory size {path}: {error}", path = path.display(), error = e.to_string());
                }
            }
        } else if path.is_file() {
            match path.metadata() {
                Ok(metadata) => {
                    total_size_freed += metadata.len();
                    if let Err(e) = fs::remove_file(&path) {
                        warn!("Failed to delete file {path}: {error}", path = path.display(), error = e.to_string());
                    } else {
                        total_deleted += 1;
                        info!("Deleted: {path}", path = path.display());
                    }
                }
                Err(e) => {
                    warn!("Failed to get file metadata {path}: {error}", path = path.display(), error = e.to_string());
                }
            }
        }
    }

    info!("🎉 Cache cleanup completed!");
    info!("   Deleted items: {count}", count = total_deleted);
    info!("   Freed space: {size} MB", size = format!("{:.2}", total_size_freed as f64 / 1024.0 / 1024.0));

    Ok(())
}

/// 显示缓存使用情况
async fn show_cache_status(app: &CliApp) -> Result<()> {
    info!("📊 Cache Usage");
    info!("================");

    let cache_dir = Path::new(&app.config.cache.cache_dir);
    let download_dir = Path::new(&app.config.cache.download_dir);

    if !cache_dir.exists() {
        info!("Cache directory does not exist: {path}", path = cache_dir.display());
        return Ok(());
    }

    info!("Cache root directory: {path}", path = cache_dir.display());

    // 计算总大小
    match calculate_directory_size(cache_dir) {
        Ok(total_size) => {
            info!("Total size: {size} MB", size = format!("{:.2}", total_size as f64 / 1024.0 / 1024.0));
        }
        Err(e) => {
            warn!("Failed to calculate total cache size: {error}", error = e.to_string());
        }
    }

    // 显示下载目录详情
    if download_dir.exists() {
        info!("
📥 Download cache details:");

        if let Ok(entries) = fs::read_dir(download_dir) {
            let mut version_count = 0;
            for entry in entries.flatten() {
                let path = entry.path();
                if path.is_dir() {
                    version_count += 1;
                    let version_name = path.file_name().unwrap().to_string_lossy();

                    match calculate_directory_size(&path) {
                        Ok(size) => {
                            info!("   Version {version}: {size} MB", version = version_name, size = format!("{:.2}", size as f64 / 1024.0 / 1024.0));
                        }
                        Err(_) => {
                            info!("   Version {version}: (failed to calculate size)", version = version_name);
                        }
                    }
                }
            }

            if version_count == 0 {
                info!("   (No version cache)");
            }
        }
    } else {
        info!("
📥 Download cache: does not exist");
    }

    Ok(())
}

/// 清理下载缓存（保留最新的指定数量版本）
async fn clean_downloads(app: &CliApp, keep: u32) -> Result<()> {
    info!("🧹 Cleaning download cache (keeping latest {keep} versions)...", keep = keep);

    let download_dir = Path::new(&app.config.cache.download_dir);

    if !download_dir.exists() {
        info!("Download cache directory does not exist: {path}", path = download_dir.display());
        return Ok(());
    }

    // 收集所有版本目录
    let mut versions = Vec::new();

    if let Ok(entries) = fs::read_dir(download_dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                let version_name = path.file_name().unwrap().to_string_lossy().to_string();

                // 获取目录修改时间作为排序依据
                if let Ok(metadata) = path.metadata()
                    && let Ok(modified) = metadata.modified() {
                        versions.push((version_name, path, modified));
                    }
            }
        }
    }

    // 按修改时间降序排序（最新的在前）
    versions.sort_by_key(|b| std::cmp::Reverse(b.2));

    info!("Found {count} version caches", count = versions.len());

    let mut deleted_count = 0;
    let mut freed_space = 0u64;

    // 删除超出保留数量的版本
    for (i, (version_name, path, _)) in versions.iter().enumerate() {
        if i >= keep as usize {
            match calculate_directory_size(path) {
                Ok(size) => {
                    freed_space += size;
                    if let Err(e) = fs::remove_dir_all(path) {
                        warn!("Failed to delete version cache {version}: {error}", version = version_name, error = e.to_string());
                    } else {
                        info!("Deleted version cache: {version}", version = version_name);
                        deleted_count += 1;
                    }
                }
                Err(e) => {
                    warn!("Failed to calculate version cache size {version}: {error}", version = version_name, error = e.to_string());
                }
            }
        } else {
            info!("Keeping version cache: {version}", version = version_name);
        }
    }

    info!("🎉 Download cache cleanup completed!");
    info!("   Deleted versions: {count}", count = deleted_count);
    info!("   Freed space: {size} MB", size = format!("{:.2}", freed_space as f64 / 1024.0 / 1024.0));

    Ok(())
}

/// 计算目录大小
fn calculate_directory_size(dir: &Path) -> Result<u64> {
    let mut total_size = 0;

    for entry in WalkDir::new(dir) {
        match entry {
            Ok(entry) => {
                if entry.file_type().is_file()
                    && let Ok(metadata) = entry.metadata() {
                        total_size += metadata.len();
                    }
            }
            Err(e) => {
                warn!("Error walking directory: {error}", error = e.to_string());
            }
        }
    }

    Ok(total_size)
}
