use anyhow::Result;
use client_core::{
    constants::docker::get_docker_work_dir, upgrade_strategy::UpgradeStrategy, utils::archive,
};
use std::io::{Read, Write};
use std::time::Instant;
use tracing::{error, info};
use zip::read::ZipFile;

// 导入匹配器模块
pub mod env_manager;

// 重新导出匹配器模块
// pub use matcher::*;

/// 判断是否应该跳过某个文件（智能过滤）
///
/// 跳过的文件类型：
/// - macOS 系统文件：__MACOSX, .DS_Store, ._*
/// - 版本控制文件：.git/, .gitignore, .gitattributes
/// - 临时文件：.tmp, .temp, .bak
/// - IDE 文件：.vscode/, .idea/
///
/// 保留的重要配置文件：
/// - Docker 配置：.env, .env.*, .dockerignore
/// - 其他配置：.editorconfig, .prettier*, .eslint*
fn should_skip_file(file_name: &str) -> bool {
    // 跳过 macOS 系统文件和临时文件
    if file_name.starts_with("__MACOSX")
        || file_name.ends_with(".DS_Store")
        || file_name.starts_with("._")
        || file_name.ends_with(".tmp")
        || file_name.ends_with(".temp")
        || file_name.ends_with(".bak")
    {
        return true;
    }

    // 跳过版本控制相关文件
    if file_name.starts_with(".git/")
        || file_name == ".gitignore"
        || file_name == ".gitattributes"
        || file_name == ".gitmodules"
    {
        return true;
    }

    // 跳过 IDE 和编辑器配置目录
    if file_name.starts_with(".vscode/")
        || file_name.starts_with(".idea/")
        || file_name.starts_with(".vs/")
    {
        return true;
    }

    // 保留重要的配置文件（即使以.开头）
    if file_name == ".env"
        || file_name.starts_with(".env.")
        || file_name == ".dockerignore"
        || file_name == ".editorconfig"
        || file_name.starts_with(".prettier")
        || file_name.starts_with(".eslint")
    {
        return false;
    }

    // 其他以.开头的文件，谨慎起见也保留（除非明确知道要跳过）
    false
}

/// # Nuwax Cli  日志系统使用说明
///
/// 本项目遵循 Rust CLI 应用的日志最佳实践：
///
/// ## 基本原则
/// 1. **库代码只使用 `tracing` 宏**：`info!()`, `warn!()`, `error!()`, `debug!()`
/// 2. **应用入口控制日志配置**：在 `main.rs` 中调用 `setup_logging()`
/// 3. **用户界面输出与日志分离**：备份列表等用户友好信息通过其他方式输出
///
/// ## 日志配置选项
///
/// ### 命令行参数
/// - `-v, --verbose`：启用详细日志模式（DEBUG 级别）
///
/// ### 环境变量
/// - `RUST_LOG`：标准的 Rust 日志级别控制（如 `debug`, `info`, `warn`, `error`）
/// - `DUCK_LOG_FILE`：日志文件路径，设置后日志输出到文件而非终端
///
/// ## 使用示例
///
/// ```bash
/// # 标准日志输出到终端
/// nuwax-cli auto-backup status
///
/// # 详细日志输出到终端
/// nuwax-cli -v auto-backup status
///
/// # 日志输出到文件
/// DUCK_LOG_FILE=duck.log nuwax-cli auto-backup status
///
/// # 使用 RUST_LOG 控制特定模块的日志级别
/// RUST_LOG=duck_cli::commands::auto_backup=debug nuwax-cli auto-backup status
/// ```
///
/// ## 作为库使用
///
/// 当 nuwax-cli 作为库被其他项目使用时，可以：
/// 1. 让使用者完全控制日志配置（推荐）
/// 2. 或调用 `setup_minimal_logging()` 进行最小化配置
///
/// ## 日志格式
/// - **终端输出**：人类可读格式，不显示模块路径
/// - **文件输出**：包含完整模块路径和更多调试信息
///
/// 带进度显示的文件复制
#[allow(dead_code)]
pub fn copy_with_progress<R: Read, W: Write>(
    mut reader: R,
    mut writer: W,
    total_size: u64,
    file_name: &str,
) -> std::io::Result<u64> {
    let mut buf = [0u8; 8192]; // 8KB 缓冲区
    let mut copied = 0u64;
    let mut last_percent = 0;

    loop {
        let bytes_read = reader.read(&mut buf)?;
        if bytes_read == 0 {
            break;
        }

        writer.write_all(&buf[..bytes_read])?;
        copied += bytes_read as u64;

        // 显示大文件的复制进度（每10%或每100MB显示一次）
        if total_size > 100 * 1024 * 1024 {
            // 只对大于100MB的文件显示详细进度
            let percent = if total_size > 0 {
                (copied * 100) / total_size
            } else {
                0
            };
            let mb_copied = copied as f64 / 1024.0 / 1024.0;
            let mb_total = total_size as f64 / 1024.0 / 1024.0;

            // 每10%或每100MB更新一次进度
            if (percent != last_percent && percent % 10 == 0)
                || (copied % (100 * 1024 * 1024) == 0 && copied > 0)
            {
                info!(
                    "     ⏳ {} 复制进度: {:.1}% ({:.1}/{:.1} MB)",
                    file_name, percent as f64, mb_copied, mb_total
                );
                last_percent = percent;
            }
        }
    }

    Ok(copied)
}

/// 强制覆盖文件/目录：先删除再创建（彻底解决 Directory not empty 错误）
fn force_extract_file(
    entry: &mut ZipFile<std::fs::File>,
    target_path: &std::path::Path,
) -> Result<()> {
    // 如果目标存在，先彻底删除
    if target_path.exists() {
        if target_path.is_dir() {
            info!("🗑️  强制删除目录: {}", target_path.display());
            std::fs::remove_dir_all(target_path)?;
        } else {
            info!("🗑️  强制删除文件: {}", target_path.display());
            std::fs::remove_file(target_path)?;
        }
    }

    // 确保父目录存在
    if let Some(parent) = target_path.parent() {
        if !parent.exists() {
            std::fs::create_dir_all(parent)?;
        }
    }

    // 创建新文件/目录
    if entry.is_dir() {
        std::fs::create_dir_all(target_path).map_err(|e| {
            error!("❌ 目录创建失败: {} - 错误: {}", target_path.display(), e);
            e
        })?;
    } else {
        let mut outfile = std::fs::File::create(target_path).map_err(|e| {
            error!("❌ 文件创建失败: {} - 错误: {}", target_path.display(), e);
            e
        })?;
        std::io::copy(entry, &mut outfile).map_err(|e| {
            error!("❌ 文件写入失败: {} - 错误: {}", target_path.display(), e);
            e
        })?;
    }

    Ok(())
}

fn handle_extraction(
    entry: &mut ZipFile<std::fs::File>,
    dst: &std::path::Path,
    extracted_files: &mut usize,
    extracted_size: &mut u64,
) -> Result<()> {
    force_extract_file(entry, dst)?;
    *extracted_files += 1;
    *extracted_size += entry.size();
    Ok(())
}

/// 确保父目录存在
fn ensure_parent_dir(path: &std::path::Path) -> Result<()> {
    if let Some(parent) = path.parent() {
        if !parent.exists() {
            std::fs::create_dir_all(parent)?;
        }
    }
    Ok(())
}

/// 判断路径是否属于保护目录 (upload, data 等)
fn is_upload_directory_path(path: &std::path::Path) -> bool {
    // 判断 [upload, project_workspace, project_zips, project_nginx, project_init, data] 目录
    path.components().any(|component| {
        client_core::constants::docker::EXCLUDE_DIRS
            .iter()
            .any(|d| component.as_os_str() == *d)
    })
}

/// 安全删除 docker 目录，保留 upload 目录
fn safe_remove_docker_directory(output_dir: &std::path::Path) -> Result<()> {
    if !output_dir.exists() {
        return Ok(());
    }

    info!(
        "🧹 安全清理 docker 目录（保留 upload 目录）: {}",
        output_dir.display()
    );

    // 遍历 docker 目录，删除除了 upload 之外的所有内容
    for entry in std::fs::read_dir(output_dir)? {
        let entry = entry?;
        let path = entry.path();
        let file_name = entry.file_name();

        // 跳过 [upload, project_workspace, project_zips, project_nginx, project_init, data] 目录
        if client_core::constants::docker::EXCLUDE_DIRS
            .iter()
            .any(|d| file_name.as_os_str() == *d)
        {
            info!("🛡️ 保留目录: {}", path.display());
            continue;
        }

        // 删除其他文件或目录
        if path.is_dir() {
            info!("🗑️ 删除目录: {}", path.display());
            std::fs::remove_dir_all(&path)?;
        } else {
            info!("🗑️ 删除文件: {}", path.display());
            std::fs::remove_file(&path)?;
        }
    }

    info!("✅ docker 目录清理完成，upload 目录已保留");
    Ok(())
}

/// 解压Docker服务包 - 支持 ZIP 和 TAR.GZ
pub async fn extract_docker_service(
    archive_path: &std::path::Path,
    upgrade_strategy: &UpgradeStrategy,
) -> Result<()> {
    let extract_start = Instant::now();

    info!("📦 开始解压Docker服务包: {}", archive_path.display());

    // 检查文件是否存在
    if !archive_path.exists() {
        return Err(anyhow::anyhow!(
            "{}",
            t!("utils.file_not_exists", path = archive_path.display())
        ));
    }

    // 检测文件格式
    let format = archive::detect_format_by_magic(archive_path)?;
    info!("✅ 检测到文件格式: {:?}", format);

    // 根据格式选择解压方法
    match format {
        client_core::utils::archive::ArchiveFormat::Zip => {
            extract_zip_archive(archive_path, upgrade_strategy, extract_start).await
        }
        client_core::utils::archive::ArchiveFormat::TarGz => {
            extract_tar_gz_archive(archive_path, upgrade_strategy, extract_start).await
        }
    }
}

/// 解压 ZIP 格式归档
async fn extract_zip_archive(
    zip_path: &std::path::Path,
    upgrade_strategy: &UpgradeStrategy,
    extract_start: Instant,
) -> Result<()> {
    // 打开ZIP文件
    let file = std::fs::File::open(zip_path)?;
    let mut archive = zip::ZipArchive::new(file)?;

    info!("✅ ZIP文件打开成功，包含 {} 个文件", archive.len());

    match upgrade_strategy {
        UpgradeStrategy::FullUpgrade { .. } => {
            // 目标解压目录
            let output_dir = std::path::Path::new("docker");
            // 如果目标目录已存在，安全清理它（保留upload目录）
            if output_dir.exists() {
                safe_remove_docker_directory(output_dir)?;
            } else {
                // 创建输出目录
                std::fs::create_dir_all(output_dir)?;
            }

            // 统计解压进度
            let mut extracted_files = 0;
            let mut extracted_size = 0u64;
            let total_files = archive.len();

            info!("🚀 开始解压 {} 个文件...", total_files);

            for i in 0..archive.len() {
                let mut file = archive.by_index(i)?;
                let file_name = file.name().to_string();

                // 跳过系统文件和临时文件
                if should_skip_file(&file_name) {
                    info!("⏩ 跳过文件: {}", file_name);
                    continue;
                }

                // 处理路径：移除可能的顶层docker目录前缀
                let clean_path = if file_name.starts_with("docker/") {
                    // 如果ZIP内已有docker/前缀，移除它
                    file_name.strip_prefix("docker/").unwrap_or(&file_name)
                } else {
                    &file_name
                };

                let target_path = output_dir.join(clean_path);

                // 检查是否为 upload 目录路径
                if is_upload_directory_path(&target_path) {
                    // 如果 upload 目录已存在，跳过解压以保护用户数据
                    // 如果 upload 目录不存在，正常解压以创建目录结构
                    if target_path.exists() {
                        info!(
                            "🛡️ 保护现有 upload 目录，跳过解压: {}",
                            target_path.display()
                        );
                        continue;
                    } else {
                        info!("📁 创建新的 upload 目录结构: {}", target_path.display());
                    }
                }

                if file.is_dir() {
                    // 创建目录
                    std::fs::create_dir_all(&target_path)?;
                } else {
                    // 强制覆盖：先删除再解压（彻底解决 Directory not empty 错误）
                    force_extract_file(&mut file, &target_path)?;

                    extracted_files += 1;
                    extracted_size += file.size();

                    // 每解压10%的文件显示进度
                    if extracted_files % (total_files / 10).max(1) == 0 {
                        let percentage = (extracted_files * 100) / total_files;
                        info!(
                            "📁 解压进度: {}% ({}/{} 文件, {:.1} MB)",
                            percentage,
                            extracted_files,
                            total_files,
                            extracted_size as f64 / 1024.0 / 1024.0
                        );
                    }
                }
            }

            let elapsed = extract_start.elapsed();
            info!("🎉 Docker服务包解压完成!");
            info!("   📁 解压文件: {} 个", extracted_files);
            info!(
                "   📏 总数据量: {:.1} MB",
                extracted_size as f64 / 1024.0 / 1024.0
            );
            info!("   ⏱️  耗时: {:.2} 秒", elapsed.as_secs_f64());
        }
        UpgradeStrategy::PatchUpgrade {
            patch_info,
            download_type: _,
            ..
        } => {
            // 增量升级：根据操作的文件和目录进行操作
            let change_files = patch_info.get_changed_files();
            let work_dir = get_docker_work_dir();
            let upgrade_change_file_or_dir = change_files
                .iter()
                .map(|path| work_dir.join(path))
                .collect::<Vec<_>>();

            // 清理即将被替换或删除的文件/目录（跳过upload目录）
            for file_or_dir in upgrade_change_file_or_dir {
                if is_upload_directory_path(&file_or_dir) {
                    info!("🛡️ 保护 upload 目录，跳过删除: {}", file_or_dir.display());
                    continue;
                }

                if file_or_dir.is_file() {
                    std::fs::remove_file(file_or_dir)?;
                } else if file_or_dir.is_dir() {
                    std::fs::remove_dir_all(file_or_dir)?;
                } else {
                    info!("文件/目录不存在，跳过: {}", file_or_dir.display());
                }
            }

            let operations = patch_info.operations.clone();
            // 统计解压进度
            let mut extracted_files = 0;
            let mut extracted_size = 0u64;
            let total_files = archive.len();

            info!("🚀 开始解压 {} 个文件...", total_files);

            //根据 operations 的 replace, delete 进行操作
            if let Some(replace) = operations.replace {
                let replace_files = replace.files;
                let replace_dirs = replace.directories;

                // 处理替换文件
                for file in replace_files {
                    let zip_path = format!("docker/{}", file.trim_start_matches('/'));
                    info!("🔍 查找文件: {} -> {}", file, zip_path);

                    let mut entry = archive
                        .by_name(&zip_path)
                        .map_err(|e| {
                            anyhow::anyhow!(
                                "{}",
                                t!(
                                    "utils.file_not_found_in_archive",
                                    path = zip_path,
                                    error = e.to_string()
                                )
                            )
                        })?;

                    let dst = work_dir.join(&file);

                    // 检查是否为保护目录路径
                    if is_upload_directory_path(&dst) {
                        // 如果保护目录已存在，跳过解压以保护用户数据
                        if dst.exists() {
                            info!("🛡️ 保护现有目录，跳过替换: {}", dst.display());
                            continue;
                        } else {
                            info!("📁 创建新的保护目录结构: {}", dst.display());
                        }
                    }

                    // 强制覆盖：先删除再解压（彻底解决 Directory not empty 错误）
                    force_extract_file(&mut entry, &dst)?;

                    extracted_files += 1;
                    extracted_size += entry.size();
                }

                // 处理替换目录
                for dir in replace_dirs {
                    let zip_dir_path = format!("docker/{}", dir.trim_start_matches('/'));
                    info!("📁 处理目录: {} -> {}", dir, zip_dir_path);

                    // 清理现有目录（跳过保护目录）
                    let target_dir = work_dir.join(&dir);
                    if is_upload_directory_path(&target_dir) && target_dir.exists() {
                        info!("🛡️ 保护现有目录，跳过目录替换: {}", target_dir.display());
                        continue;
                    }

                    if target_dir.exists() {
                        info!("🗑️  强制删除目录: {}", target_dir.display());
                        std::fs::remove_dir_all(&target_dir)?;
                    }

                    // 解压该目录下的所有条目
                    for i in 0..archive.len() {
                        let mut entry = archive.by_index(i)?;
                        let entry_name = entry.name();

                        if entry_name.starts_with(&zip_dir_path) {
                            let relative_path = entry_name
                                .strip_prefix(&zip_dir_path)
                                .unwrap_or("")
                                .trim_start_matches('/');

                            if relative_path.is_empty() && entry.is_dir() {
                                continue;
                            }

                            let dst = target_dir.join(relative_path);
                            ensure_parent_dir(&dst)?;

                            handle_extraction(
                                &mut entry,
                                &dst,
                                &mut extracted_files,
                                &mut extracted_size,
                            )?;
                        }
                    }
                }
            }
            if let Some(delete) = operations.delete {
                // 处理删除操作（跳过upload目录）
                for file in delete.files {
                    let path = work_dir.join(file);
                    if is_upload_directory_path(&path) {
                        info!("🛡️ 保护 upload 目录，跳过删除文件: {}", path.display());
                        continue;
                    }
                    info!("🗑️ 删除文件: {}", path.display());
                    if path.is_file() {
                        std::fs::remove_file(&path)?;
                    } else if path.exists() {
                        std::fs::remove_file(&path).or_else(|_| std::fs::remove_dir_all(&path))?;
                    } else {
                        info!("文件不存在，跳过: {}", path.display());
                    }
                }
                // 删除目录（跳过upload目录）
                for dir in delete.directories {
                    let path = work_dir.join(dir);
                    if is_upload_directory_path(&path) {
                        info!("🛡️ 保护 upload 目录，跳过删除目录: {}", path.display());
                        continue;
                    }
                    info!("🗑️ 删除目录: {}", path.display());
                    if path.is_dir() {
                        std::fs::remove_dir_all(&path)?;
                    } else if path.exists() {
                        std::fs::remove_file(&path).or_else(|_| std::fs::remove_dir_all(&path))?;
                    } else {
                        info!("目录不存在，跳过: {}", path.display());
                    }
                }
            }

            // 🔧 强制更新关键配置文件（无论是否在变更列表中）
            // 这些文件对于数据库升级至关重要，必须始终保持最新
            use client_core::constants::sql::CRITICAL_UPGRADE_FILES;
            for critical_file in CRITICAL_UPGRADE_FILES {
                let zip_path = format!("docker/{}", critical_file);
                let dst_path = work_dir.join(critical_file);

                match archive.by_name(&zip_path) {
                    Ok(mut entry) => {
                        info!("🔧 强制更新关键文件: {}", critical_file);
                        force_extract_file(&mut entry, &dst_path)?;
                        info!("✅ 关键文件已更新: {}", critical_file);
                    }
                    Err(_) => {
                        // 压缩包中没有这个文件，跳过
                        info!("⏭️  压缩包中不存在关键文件: {}", zip_path);
                    }
                }
            }
        }
        UpgradeStrategy::NoUpgrade { .. } => {
            // 无需升级,不应该走到这里的解压逻辑
            return Err(anyhow::anyhow!("{}", t!("utils.no_upgrade_extract_unsupported")));
        }
    }

    Ok(())
}

/// 解压 TAR.GZ 格式归档
async fn extract_tar_gz_archive(
    tar_gz_path: &std::path::Path,
    upgrade_strategy: &UpgradeStrategy,
    extract_start: Instant,
) -> Result<()> {
    let tar_gz_path = tar_gz_path.to_path_buf();
    let strategy = upgrade_strategy.clone();

    tokio::task::spawn_blocking(move || {
        extract_tar_gz_blocking(&tar_gz_path, &strategy, extract_start)
    })
    .await
    .map_err(|e| anyhow::anyhow!("{}", t!("utils.extract_task_failed", error = e.to_string())))?
}

/// TAR.GZ 解压实现（阻塞）
fn extract_tar_gz_blocking(
    tar_gz_path: &std::path::Path,
    upgrade_strategy: &UpgradeStrategy,
    extract_start: Instant,
) -> Result<()> {
    use flate2::read::GzDecoder;
    use tar::Archive;

    let tar_gz = std::fs::File::open(tar_gz_path)?;
    let decoder = GzDecoder::new(tar_gz);
    let mut archive = Archive::new(decoder);

    let output_dir = std::path::Path::new("docker");
    let mut extracted_files = 0;
    let mut extracted_size = 0u64;

    match upgrade_strategy {
        UpgradeStrategy::FullUpgrade { .. } => {
            // 全量升级：清空 docker 目录（保留 upload 目录）
            if output_dir.exists() {
                safe_remove_docker_directory(output_dir)?;
            } else {
                std::fs::create_dir_all(output_dir)?;
            }

            info!("🚀 开始解压 TAR.GZ 文件...");

            for entry in archive.entries()? {
                let mut entry: tar::Entry<flate2::read::GzDecoder<std::fs::File>> = entry?;
                let path = entry.path()?;

                // 跳过 __MACOSX 等系统文件
                if should_skip_tar_entry(&path) {
                    continue;
                }

                // 移除 docker/ 前缀（如果存在）
                let clean_path = path.strip_prefix("docker").unwrap_or(&path);
                let target_path = output_dir.join(clean_path);

                // 保护 upload 目录
                if is_upload_directory_path(&target_path) && target_path.exists() {
                    info!("🛡️ 保护现有目录，跳过: {}", target_path.display());
                    continue;
                }

                // 确保父目录存在
                if let Some(parent) = target_path.parent() {
                    if !parent.exists() {
                        std::fs::create_dir_all(parent)?;
                    }
                }

                // 解压文件
                entry.unpack(&target_path)?;
                extracted_files += 1;
                extracted_size += entry.size();

                // 每解压10%的文件显示进度
                if extracted_files % 10 == 0 {
                    info!(
                        "📁 解压进度: {} 个文件 ({:.1} MB)",
                        extracted_files,
                        extracted_size as f64 / 1024.0 / 1024.0
                    );
                }
            }

            let elapsed = extract_start.elapsed();
            info!("🎉 Docker服务包解压完成!");
            info!("   📁 解压文件: {} 个", extracted_files);
            info!(
                "   📏 总数据量: {:.1} MB",
                extracted_size as f64 / 1024.0 / 1024.0
            );
            info!("   ⏱️  耗时: {:.2} 秒", elapsed.as_secs_f64());
        }
        UpgradeStrategy::PatchUpgrade { .. } => {
            // 增量升级目前不支持 TAR.GZ
            return Err(anyhow::anyhow!("{}", t!("utils.tar_gz_patch_unsupported")));
        }
        UpgradeStrategy::NoUpgrade { .. } => {
            return Err(anyhow::anyhow!("{}", t!("utils.no_upgrade_extract_unsupported")));
        }
    }

    Ok(())
}

/// 判断 TAR 条目是否应该跳过
fn should_skip_tar_entry(path: &std::path::Path) -> bool {
    let s = path.to_string_lossy();
    s.contains("__MACOSX") || s.contains(".DS_Store") || s.contains("._")
}

/// 设置日志记录系统
///
/// 这个函数遵循Rust CLI应用的最佳实践：
/// - 库代码只使用 tracing 宏记录日志
/// - 在应用入口配置日志输出行为
/// - 支持 RUST_LOG 环境变量控制日志级别
/// - 默认输出到stderr，避免与程序输出混淆
/// - 终端输出简洁格式，文件输出详细格式
pub fn setup_logging(verbose: bool) {
    #[allow(unused_imports)]
    use tracing_subscriber::{EnvFilter, fmt, util::SubscriberInitExt};

    // 根据verbose参数和环境变量确定日志级别
    let default_level = if verbose { "debug" } else { "info" };
    let env_filter = EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| EnvFilter::new(default_level))
        // 过滤掉第三方库的详细日志，减少噪音
        .add_directive("reqwest=warn".parse().unwrap())
        .add_directive("tokio=warn".parse().unwrap())
        .add_directive("hyper=warn".parse().unwrap());

    // 检查环境变量，决定是否输出到文件
    if let Ok(log_file) = std::env::var("DUCK_LOG_FILE") {
        // 输出到文件 - 使用详细格式便于调试
        let file = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(log_file)
            .expect("Failed to create log file");

        fmt()
            .with_env_filter(env_filter)
            .with_writer(file)
            .with_target(true)
            .with_thread_names(true)
            .with_line_number(true)
            .init();
    } else {
        // 输出到终端 - 使用简洁格式，用户友好
        fmt()
            .with_env_filter(env_filter)
            .with_target(false) // 不显示模块路径
            .with_thread_names(false) // 不显示线程名
            .with_line_number(false) // 不显示行号
            .without_time() // 不显示时间戳
            .compact() // 使用紧凑格式
            .init();
    }
}

/// 为库使用提供的简化日志初始化
///
/// 当nuwax-cli作为库使用时，可以调用此函数进行最小化的日志配置
/// 或者让库的使用者完全控制日志配置
#[allow(dead_code)]
pub fn setup_minimal_logging() {
    #[allow(unused_imports)]
    use tracing_subscriber::{EnvFilter, fmt, util::SubscriberInitExt};

    // 尝试初始化一个简单的订阅者
    // 如果已经有全局订阅者，这会返回错误，我们忽略它
    let _ = fmt()
        .with_env_filter(EnvFilter::from_default_env())
        .with_target(false)
        .compact() // 使用紧凑格式
        .try_init();
}
