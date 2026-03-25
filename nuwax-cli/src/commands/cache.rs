use crate::app::CliApp;
use crate::cli::CacheCommand;
use anyhow::Result;
use rust_i18n::t;
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
    info!("{}", t!("cache.clear_start"));

    let cache_dir = Path::new(&app.config.cache.cache_dir);

    if !cache_dir.exists() {
        info!("{}", t!("cache.cache_dir_not_exists", path = cache_dir.display()));
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
                        warn!("{}", t!("cache.delete_dir_failed", path = path.display(), error = e.to_string()));
                    } else {
                        total_deleted += 1;
                        info!("{}", t!("cache.deleted", path = path.display()));
                    }
                }
                Err(e) => {
                    warn!("{}", t!("cache.calc_dir_size_failed", path = path.display(), error = e.to_string()));
                }
            }
        } else if path.is_file() {
            match path.metadata() {
                Ok(metadata) => {
                    total_size_freed += metadata.len();
                    if let Err(e) = fs::remove_file(&path) {
                        warn!("{}", t!("cache.delete_file_failed", path = path.display(), error = e.to_string()));
                    } else {
                        total_deleted += 1;
                        info!("{}", t!("cache.deleted", path = path.display()));
                    }
                }
                Err(e) => {
                    warn!("{}", t!("cache.get_metadata_failed", path = path.display(), error = e.to_string()));
                }
            }
        }
    }

    info!("{}", t!("cache.clear_complete"));
    info!("{}", t!("cache.deleted_items", count = total_deleted));
    info!(
        "{}",
        t!("cache.freed_space", size = format!("{:.2}", total_size_freed as f64 / 1024.0 / 1024.0))
    );

    Ok(())
}

/// 显示缓存使用情况
async fn show_cache_status(app: &CliApp) -> Result<()> {
    info!("{}", t!("cache.status_title"));
    info!("{}", t!("cache.status_separator"));

    let cache_dir = Path::new(&app.config.cache.cache_dir);
    let download_dir = Path::new(&app.config.cache.download_dir);

    if !cache_dir.exists() {
        info!("{}", t!("cache.cache_dir_not_exists", path = cache_dir.display()));
        return Ok(());
    }

    info!("{}", t!("cache.cache_root", path = cache_dir.display()));

    // 计算总大小
    match calculate_directory_size(cache_dir) {
        Ok(total_size) => {
            info!("{}", t!("cache.total_size", size = format!("{:.2}", total_size as f64 / 1024.0 / 1024.0)));
        }
        Err(e) => {
            warn!("{}", t!("cache.calc_total_size_failed", error = e.to_string()));
        }
    }

    // 显示下载目录详情
    if download_dir.exists() {
        info!("{}", t!("cache.download_cache_title"));

        if let Ok(entries) = fs::read_dir(download_dir) {
            let mut version_count = 0;
            for entry in entries {
                if let Ok(entry) = entry {
                    let path = entry.path();
                    if path.is_dir() {
                        version_count += 1;
                        let version_name = path.file_name().unwrap().to_string_lossy();

                        match calculate_directory_size(&path) {
                            Ok(size) => {
                                info!(
                                    "{}",
                                    t!("cache.version_size",
                                        version = version_name,
                                        size = format!("{:.2}", size as f64 / 1024.0 / 1024.0))
                                );
                            }
                            Err(_) => {
                                info!("{}", t!("cache.version_size_failed", version = version_name));
                            }
                        }
                    }
                }
            }

            if version_count == 0 {
                info!("{}", t!("cache.no_version_cache"));
            }
        }
    } else {
        info!("{}", t!("cache.download_cache_not_exists"));
    }

    Ok(())
}

/// 清理下载缓存（保留最新的指定数量版本）
async fn clean_downloads(app: &CliApp, keep: u32) -> Result<()> {
    info!("{}", t!("cache.clean_start", keep = keep));

    let download_dir = Path::new(&app.config.cache.download_dir);

    if !download_dir.exists() {
        info!("{}", t!("cache.download_dir_not_exists", path = download_dir.display()));
        return Ok(());
    }

    // 收集所有版本目录
    let mut versions = Vec::new();

    if let Ok(entries) = fs::read_dir(download_dir) {
        for entry in entries {
            if let Ok(entry) = entry {
                let path = entry.path();
                if path.is_dir() {
                    let version_name = path.file_name().unwrap().to_string_lossy().to_string();

                    // 获取目录修改时间作为排序依据
                    if let Ok(metadata) = path.metadata() {
                        if let Ok(modified) = metadata.modified() {
                            versions.push((version_name, path, modified));
                        }
                    }
                }
            }
        }
    }

    // 按修改时间降序排序（最新的在前）
    versions.sort_by(|a, b| b.2.cmp(&a.2));

    info!("{}", t!("cache.found_versions", count = versions.len()));

    let mut deleted_count = 0;
    let mut freed_space = 0u64;

    // 删除超出保留数量的版本
    for (i, (version_name, path, _)) in versions.iter().enumerate() {
        if i >= keep as usize {
            match calculate_directory_size(path) {
                Ok(size) => {
                    freed_space += size;
                    if let Err(e) = fs::remove_dir_all(path) {
                        warn!("{}", t!("cache.delete_version_failed", version = version_name, error = e.to_string()));
                    } else {
                        info!("{}", t!("cache.deleted_version", version = version_name));
                        deleted_count += 1;
                    }
                }
                Err(e) => {
                    warn!("{}", t!("cache.calc_version_size_failed", version = version_name, error = e.to_string()));
                }
            }
        } else {
            info!("{}", t!("cache.keep_version", version = version_name));
        }
    }

    info!("{}", t!("cache.clean_complete"));
    info!("{}", t!("cache.deleted_versions", count = deleted_count));
    info!(
        "{}",
        t!("cache.freed_space", size = format!("{:.2}", freed_space as f64 / 1024.0 / 1024.0))
    );

    Ok(())
}

/// 计算目录大小
fn calculate_directory_size(dir: &Path) -> Result<u64> {
    let mut total_size = 0;

    for entry in WalkDir::new(dir) {
        match entry {
            Ok(entry) => {
                if entry.file_type().is_file() {
                    if let Ok(metadata) = entry.metadata() {
                        total_size += metadata.len();
                    }
                }
            }
            Err(e) => {
                warn!("{}", t!("cache.walk_dir_error", error = e.to_string()));
            }
        }
    }

    Ok(total_size)
}
