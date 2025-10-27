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
    info!("🧹 开始清理缓存文件...");

    let cache_dir = Path::new(&app.config.cache.cache_dir);

    if !cache_dir.exists() {
        info!("缓存目录不存在: {}", cache_dir.display());
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
                        warn!("删除目录失败 {}: {}", path.display(), e);
                    } else {
                        total_deleted += 1;
                        info!("已删除: {}", path.display());
                    }
                }
                Err(e) => {
                    warn!("计算目录大小失败 {}: {}", path.display(), e);
                }
            }
        } else if path.is_file() {
            match path.metadata() {
                Ok(metadata) => {
                    total_size_freed += metadata.len();
                    if let Err(e) = fs::remove_file(&path) {
                        warn!("删除文件失败 {}: {}", path.display(), e);
                    } else {
                        total_deleted += 1;
                        info!("已删除: {}", path.display());
                    }
                }
                Err(e) => {
                    warn!("获取文件元数据失败 {}: {}", path.display(), e);
                }
            }
        }
    }

    info!("🎉 缓存清理完成!");
    info!("   删除项目: {} 个", total_deleted);
    info!(
        "   释放空间: {:.2} MB",
        total_size_freed as f64 / 1024.0 / 1024.0
    );

    Ok(())
}

/// 显示缓存使用情况
async fn show_cache_status(app: &CliApp) -> Result<()> {
    info!("📊 缓存使用情况");
    info!("================");

    let cache_dir = Path::new(&app.config.cache.cache_dir);
    let download_dir = Path::new(&app.config.cache.download_dir);

    if !cache_dir.exists() {
        info!("缓存目录不存在: {}", cache_dir.display());
        return Ok(());
    }

    info!("缓存根目录: {}", cache_dir.display());

    // 计算总大小
    match calculate_directory_size(cache_dir) {
        Ok(total_size) => {
            info!("总大小: {:.2} MB", total_size as f64 / 1024.0 / 1024.0);
        }
        Err(e) => {
            warn!("计算缓存总大小失败: {}", e);
        }
    }

    // 显示下载目录详情
    if download_dir.exists() {
        info!("\n📥 下载缓存详情:");

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
                                    "   版本 {}: {:.2} MB",
                                    version_name,
                                    size as f64 / 1024.0 / 1024.0
                                );
                            }
                            Err(_) => {
                                info!("   版本 {}: (计算大小失败)", version_name);
                            }
                        }
                    }
                }
            }

            if version_count == 0 {
                info!("   (无版本缓存)");
            }
        }
    } else {
        info!("\n📥 下载缓存: 不存在");
    }

    Ok(())
}

/// 清理下载缓存（保留最新的指定数量版本）
async fn clean_downloads(app: &CliApp, keep: u32) -> Result<()> {
    info!("🧹 清理下载缓存 (保留最新 {} 个版本)...", keep);

    let download_dir = Path::new(&app.config.cache.download_dir);

    if !download_dir.exists() {
        info!("下载缓存目录不存在: {}", download_dir.display());
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

    info!("发现 {} 个版本缓存", versions.len());

    let mut deleted_count = 0;
    let mut freed_space = 0u64;

    // 删除超出保留数量的版本
    for (i, (version_name, path, _)) in versions.iter().enumerate() {
        if i >= keep as usize {
            match calculate_directory_size(path) {
                Ok(size) => {
                    freed_space += size;
                    if let Err(e) = fs::remove_dir_all(path) {
                        warn!("删除版本缓存失败 {}: {}", version_name, e);
                    } else {
                        info!("已删除版本缓存: {}", version_name);
                        deleted_count += 1;
                    }
                }
                Err(e) => {
                    warn!("计算版本缓存大小失败 {}: {}", version_name, e);
                }
            }
        } else {
            info!("保留版本缓存: {}", version_name);
        }
    }

    info!("🎉 下载缓存清理完成!");
    info!("   删除版本: {} 个", deleted_count);
    info!(
        "   释放空间: {:.2} MB",
        freed_space as f64 / 1024.0 / 1024.0
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
                warn!("遍历目录时出错: {}", e);
            }
        }
    }

    Ok(total_size)
}
