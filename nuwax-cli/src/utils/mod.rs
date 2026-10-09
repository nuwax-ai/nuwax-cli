use anyhow::{Context, Result};
use client_core::{
    constants::docker::get_docker_work_dir, upgrade_strategy::UpgradeStrategy, utils::archive,
};
use std::io::{Read, Write};
use std::path::{Component, Path};
use std::time::Instant;
use tracing::{error, info};
use zip::read::ZipFile;

// 导入匹配器模块
pub mod device_env;
pub mod env_manager;
pub mod env_merge;
pub(crate) mod legacy_schema;

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

fn contains_unsafe_component(path: &Path) -> bool {
    path.components().any(|component| {
        matches!(
            component,
            Component::ParentDir | Component::RootDir | Component::Prefix(_)
        )
    })
}

pub fn validate_archive_paths(archive_path: &Path) -> Result<()> {
    let format = archive::detect_format_by_magic(archive_path)?;

    match format {
        client_core::utils::archive::ArchiveFormat::Zip => {
            let file = std::fs::File::open(archive_path)?;
            let mut archive = zip::ZipArchive::new(file)?;

            for i in 0..archive.len() {
                let file = archive.by_index(i)?;
                let raw_name = file.name().to_string();
                let Some(enclosed_name) = file.enclosed_name() else {
                    return Err(anyhow::anyhow!(
                        "Unsafe archive path detected: {}",
                        raw_name
                    ));
                };

                if contains_unsafe_component(&enclosed_name) {
                    return Err(anyhow::anyhow!(
                        "Unsafe archive path detected: {}",
                        raw_name
                    ));
                }
            }
        }
        client_core::utils::archive::ArchiveFormat::TarGz => {
            let tar_gz = std::fs::File::open(archive_path)?;
            let decoder = flate2::read::GzDecoder::new(tar_gz);
            let mut archive = tar::Archive::new(decoder);

            for entry in archive.entries()? {
                let entry = entry?;
                let entry_type = entry.header().entry_type();
                if entry_type.is_symlink() || entry_type.is_hard_link() {
                    return Err(anyhow::anyhow!(
                        "Archive links are not allowed: {}",
                        entry.path()?.display()
                    ));
                }

                let path = entry.path()?;
                if contains_unsafe_component(&path) {
                    return Err(anyhow::anyhow!(
                        "Unsafe archive path detected: {}",
                        path.display()
                    ));
                }
            }
        }
    }

    Ok(())
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
            let percent = (copied * 100).checked_div(total_size).unwrap_or(0);
            let mb_copied = copied as f64 / 1024.0 / 1024.0;
            let mb_total = total_size as f64 / 1024.0 / 1024.0;

            // 每10%或每100MB更新一次进度
            if (percent != last_percent && percent.is_multiple_of(10))
                || (copied.is_multiple_of(100 * 1024 * 1024) && copied > 0)
            {
                info!(
                    "     ⏳ {} copy progress: {:.1}% ({:.1}/{:.1} MB)",
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
            info!("🗑️  Force removing directory: {}", target_path.display());
            std::fs::remove_dir_all(target_path)?;
        } else {
            info!("🗑️  Force removing file: {}", target_path.display());
            std::fs::remove_file(target_path)?;
        }
    }

    // 确保父目录存在
    if let Some(parent) = target_path.parent()
        && !parent.exists()
    {
        std::fs::create_dir_all(parent)?;
    }

    // 创建新文件/目录
    if entry.is_dir() {
        std::fs::create_dir_all(target_path).map_err(|e| {
            error!(
                "❌ Failed to create directory: {} - error: {}",
                target_path.display(),
                e
            );
            e
        })?;
    } else {
        let mut outfile = std::fs::File::create(target_path).map_err(|e| {
            error!(
                "❌ Failed to create file: {} - error: {}",
                target_path.display(),
                e
            );
            e
        })?;
        std::io::copy(entry, &mut outfile).map_err(|e| {
            error!(
                "❌ Failed to write file: {} - error: {}",
                target_path.display(),
                e
            );
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
    // Read incoming defaults completely before replacing the live environment.
    let is_env_file = dst
        .file_name()
        .and_then(|name| name.to_str())
        .is_some_and(|name| name == ".env");
    if is_env_file && dst.exists() {
        let mut defaults = String::new();
        entry.read_to_string(&mut defaults)?;
        env_merge::merge_env_file_defaults(dst, &defaults)?;
        info!(
            "🛡️ Merged package .env defaults into existing configuration: {path}",
            path = dst.display()
        );
        *extracted_files += 1;
        *extracted_size += entry.size();
        return Ok(());
    }
    force_extract_file(entry, dst)?;
    *extracted_files += 1;
    *extracted_size += entry.size();
    Ok(())
}

/// 确保父目录存在
fn ensure_parent_dir(path: &std::path::Path) -> Result<()> {
    if let Some(parent) = path.parent()
        && !parent.exists()
    {
        std::fs::create_dir_all(parent)?;
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
fn safe_remove_docker_directory(
    output_dir: &std::path::Path,
    protected: &[std::path::PathBuf],
) -> Result<()> {
    if !output_dir.exists() {
        return Ok(());
    }

    info!(
        "🧹 Safely cleaning docker directory (keeping upload): {}",
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
            info!("🛡️ Keeping directory: {}", path.display());
            continue;
        }
        if is_protected_env_path(&path, protected) {
            continue;
        }
        if path.is_dir() && contains_protected_env_path(&path, protected) {
            safe_remove_docker_directory(&path, protected)?;
            continue;
        }

        // 删除其他文件或目录
        if path.is_dir() {
            info!("🗑️ Removing directory: {}", path.display());
            std::fs::remove_dir_all(&path)?;
        } else {
            info!("🗑️ Removing file: {}", path.display());
            std::fs::remove_file(&path)?;
        }
    }

    info!("✅ Docker directory cleanup completed, upload directory preserved");
    Ok(())
}

/// 解压Docker服务包 - 支持 ZIP 和 TAR.GZ
pub async fn extract_docker_service(
    archive_path: &std::path::Path,
    upgrade_strategy: &UpgradeStrategy,
) -> Result<()> {
    extract_docker_service_with_env(archive_path, upgrade_strategy, Path::new("docker/.env")).await
}

fn absolute_clean_path(path: &Path) -> Result<std::path::PathBuf> {
    let absolute = if path.is_absolute() {
        path.to_path_buf()
    } else {
        std::env::current_dir()?.join(path)
    };
    let mut normalized = std::path::PathBuf::new();
    for component in absolute.components() {
        match component {
            Component::ParentDir => {
                normalized.pop();
            }
            Component::CurDir => {}
            other => normalized.push(other.as_os_str()),
        }
    }
    // Preserve the leaf (an .env symlink alias), while resolving the existing
    // parent/ancestor. macOS /var and /private/var and Windows canonical drive
    // prefixes must identify the same protected file. Missing new directories
    // are appended only after resolving their nearest existing ancestor.
    let Some(leaf) = normalized.file_name() else {
        return std::fs::canonicalize(&normalized).map_err(Into::into);
    };
    let mut suffix = vec![leaf.to_os_string()];
    let mut ancestor = normalized
        .parent()
        .ok_or_else(|| anyhow::anyhow!("Path has no parent"))?;
    loop {
        match std::fs::canonicalize(ancestor) {
            Ok(mut canonical) => {
                for component in suffix.iter().rev() {
                    canonical.push(component);
                }
                return Ok(canonical);
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                let component = ancestor.file_name().ok_or_else(|| {
                    anyhow::anyhow!("No existing ancestor for {}", normalized.display())
                })?;
                suffix.push(component.to_os_string());
                ancestor = ancestor
                    .parent()
                    .ok_or_else(|| anyhow::anyhow!("Path has no existing parent"))?;
            }
            Err(error) => {
                return Err(error).with_context(|| {
                    format!("Failed to resolve parent of {}", normalized.display())
                });
            }
        }
    }
}

fn is_protected_env_path(path: &Path, protected: &[std::path::PathBuf]) -> bool {
    absolute_clean_path(path).is_ok_and(|absolute| protected.contains(&absolute))
        || std::fs::canonicalize(path).is_ok_and(|canonical| protected.contains(&canonical))
}

fn contains_protected_env_path(path: &Path, protected: &[std::path::PathBuf]) -> bool {
    absolute_clean_path(path)
        .is_ok_and(|absolute| protected.iter().any(|env| env.starts_with(&absolute)))
        || std::fs::canonicalize(path)
            .is_ok_and(|canonical| protected.iter().any(|env| env.starts_with(&canonical)))
}

/// Keep the selected environment file and its symlink target during extraction.
pub async fn extract_docker_service_with_env(
    archive_path: &Path,
    upgrade_strategy: &UpgradeStrategy,
    env_path: &Path,
) -> Result<()> {
    let mut protected = vec![absolute_clean_path(env_path)?];
    if env_path.exists() {
        protected.push(std::fs::canonicalize(env_path)?);
    }
    let extract_start = Instant::now();

    info!(
        "📦 Starting Docker service package extraction: {}",
        archive_path.display()
    );

    // 检查文件是否存在
    if !archive_path.exists() {
        return Err(anyhow::anyhow!(
            "{}",
            t!("utils.file_not_exists", path = archive_path.display())
        ));
    }

    // 检测文件格式
    let format = archive::detect_format_by_magic(archive_path)?;
    info!("✅ Detected archive format: {:?}", format);

    validate_archive_paths(archive_path)?;

    // 根据格式选择解压方法
    match format {
        client_core::utils::archive::ArchiveFormat::Zip => {
            extract_zip_archive(archive_path, upgrade_strategy, extract_start, &protected).await
        }
        client_core::utils::archive::ArchiveFormat::TarGz => {
            extract_tar_gz_archive(archive_path, upgrade_strategy, extract_start, protected).await
        }
    }
}

/// Resolve the same optional docker/ prefix supported by candidate inspection.
fn zip_entry_name(archive: &zip::ZipArchive<std::fs::File>, relative: &str) -> Result<String> {
    if contains_unsafe_component(Path::new(relative)) {
        anyhow::bail!("Unsafe critical package path: {relative}");
    }
    let prefixed = format!("docker/{relative}");
    let with_prefix = archive.file_names().any(|name| name == prefixed);
    let without_prefix = archive.file_names().any(|name| name == relative);
    match (with_prefix, without_prefix) {
        (true, false) => Ok(prefixed),
        (false, true) => Ok(relative.to_string()),
        (true, true) => anyhow::bail!("Duplicate critical package path: {relative}"),
        (false, false) => anyhow::bail!("Patch archive is missing critical file {relative}"),
    }
}

fn optional_zip_text(
    archive: &mut zip::ZipArchive<std::fs::File>,
    relative: &str,
) -> Result<Option<String>> {
    let prefixed = format!("docker/{relative}");
    if !archive
        .file_names()
        .any(|name| name == prefixed || name == relative)
    {
        return Ok(None);
    }
    let name = zip_entry_name(archive, relative)?;
    let mut entry = archive.by_name(&name)?;
    let mut text = String::new();
    entry
        .read_to_string(&mut text)
        .with_context(|| format!("Failed to read package {relative}"))?;
    Ok(Some(text))
}

/// Validate all forced release files before patch operations can mutate disk.
fn patch_critical_files(archive: &mut zip::ZipArchive<std::fs::File>) -> Result<Vec<String>> {
    let mut files = match optional_zip_text(archive, "config/mysql-schema-manifest.json")? {
        Some(text) => {
            let manifest = client_core::mysql_manifest::parse_schema_manifest(&text)
                .context("Invalid patch mysql-schema-manifest.json; refusing legacy fallback")?;
            let mut files = vec![
                "config/mysql-schema-manifest.json".to_string(),
                "docker-compose.yml".to_string(),
                "DELIVERY_MANIFEST.json".to_string(),
            ];
            files.extend(manifest.referenced_paths());
            files
        }
        None => {
            let compose = match optional_zip_text(archive, "docker-compose.yml")? {
                Some(compose) => Some(compose),
                None => {
                    match std::fs::read_to_string(get_docker_work_dir().join("docker-compose.yml"))
                    {
                        Ok(compose) => Some(compose),
                        Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
                        Err(error) => {
                            return Err(error).context("Failed to read retained legacy Compose");
                        }
                    }
                }
            };
            let mut paths = legacy_schema::schema_paths(compose.as_deref(), |path| {
                archive
                    .file_names()
                    .any(|name| name == path || name == format!("docker/{path}"))
                    || get_docker_work_dir().join(path).exists()
            })?;
            legacy_schema::parse_templates(&paths, |path| {
                optional_zip_text(archive, path)?
                    .ok_or_else(|| anyhow::anyhow!("Patch archive is missing critical file {path}"))
            })?;
            if let Some(compose) = compose {
                for path in legacy_schema::component_entrypoints(&compose)? {
                    if archive
                        .file_names()
                        .any(|name| name == path || name == format!("docker/{path}"))
                    {
                        paths.push(path.to_string());
                    }
                }
            }
            paths
        }
    };
    if let Some(text) = optional_zip_text(archive, "DELIVERY_MANIFEST.json")? {
        let delivery = client_core::container::preflight::parse_delivery_manifest(&text)?;
        files.push("DELIVERY_MANIFEST.json".to_string());
        files.push(delivery.compose.path);
        files.extend(delivery.mysql.files.into_keys());
        files.extend(
            delivery
                .components
                .into_values()
                .flat_map(|component| component.artifacts.into_keys()),
        );
    } else if optional_zip_text(archive, "docker-compose.yml")?.is_some() {
        files.push("docker-compose.yml".to_string());
    }
    files.sort();
    files.dedup();
    for relative in &files {
        let name = zip_entry_name(archive, relative)?;
        let entry = archive.by_name(&name)?;
        if entry.is_dir() || entry.size() == 0 {
            anyhow::bail!("Patch critical file {relative} must be a non-empty regular file");
        }
    }
    Ok(files)
}

/// 解压 ZIP 格式归档
async fn extract_zip_archive(
    zip_path: &std::path::Path,
    upgrade_strategy: &UpgradeStrategy,
    extract_start: Instant,
    protected: &[std::path::PathBuf],
) -> Result<()> {
    // 打开ZIP文件
    let file = std::fs::File::open(zip_path)?;
    let mut archive = zip::ZipArchive::new(file)?;

    info!(
        "✅ ZIP opened successfully, contains {} files",
        archive.len()
    );

    match upgrade_strategy {
        UpgradeStrategy::FullUpgrade { .. } => {
            // 目标解压目录
            let output_dir = std::path::Path::new("docker");
            // 如果目标目录已存在，安全清理它（保留upload目录）
            if output_dir.exists() {
                safe_remove_docker_directory(output_dir, protected)?;
            } else {
                // 创建输出目录
                std::fs::create_dir_all(output_dir)?;
            }

            // 统计解压进度
            let mut extracted_files = 0;
            let mut extracted_size = 0u64;
            let total_files = archive.len();

            info!("🚀 Starting extraction of {} files...", total_files);

            for i in 0..archive.len() {
                let mut file = archive.by_index(i)?;
                let file_name = file.name().to_string();
                let enclosed_name = file.enclosed_name().ok_or_else(|| {
                    anyhow::anyhow!("Unsafe archive path detected: {}", file_name)
                })?;

                // 跳过系统文件和临时文件
                if should_skip_file(&file_name) {
                    info!("⏩ Skipping file: {}", file_name);
                    continue;
                }

                // 处理路径：移除可能的顶层docker目录前缀
                let clean_path = if enclosed_name.starts_with("docker") {
                    // 如果ZIP内已有docker/前缀，移除它
                    enclosed_name
                        .strip_prefix("docker")
                        .unwrap_or(&enclosed_name)
                } else {
                    enclosed_name.as_path()
                };

                let target_path = output_dir.join(clean_path);

                if is_protected_env_path(&target_path, protected) && target_path.exists() {
                    continue;
                }

                // Another configured deployment may still use the package's
                // default .env. Keep its values while adding incoming defaults.
                if !file.is_dir()
                    && target_path.file_name().is_some_and(|name| name == ".env")
                    && target_path.is_file()
                {
                    handle_extraction(
                        &mut file,
                        &target_path,
                        &mut extracted_files,
                        &mut extracted_size,
                    )?;
                    continue;
                }

                // 检查是否为 upload 目录路径
                if is_upload_directory_path(&target_path) {
                    // 如果 upload 目录已存在，跳过解压以保护用户数据
                    // 如果 upload 目录不存在，正常解压以创建目录结构
                    if target_path.exists() {
                        info!(
                            "🛡️ Keeping existing upload directory, skipping extraction: {}",
                            target_path.display()
                        );
                        continue;
                    } else {
                        info!(
                            "📁 Creating new upload directory structure: {}",
                            target_path.display()
                        );
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
                            "📁 Extraction progress: {}% ({}/{} files, {:.1} MB)",
                            percentage,
                            extracted_files,
                            total_files,
                            extracted_size as f64 / 1024.0 / 1024.0
                        );
                    }
                }
            }

            let elapsed = extract_start.elapsed();
            info!("🎉 Docker service package extraction completed!");
            info!("   📁 Extracted files: {}", extracted_files);
            info!(
                "   📏 Total data size: {:.1} MB",
                extracted_size as f64 / 1024.0 / 1024.0
            );
            info!("   ⏱️  Elapsed: {:.2} seconds", elapsed.as_secs_f64());
        }
        UpgradeStrategy::PatchUpgrade { patch_info, .. } => {
            legacy_schema::validate_changed_paths(&patch_info.get_changed_files())?;
            if optional_zip_text(&mut archive, "docker-compose.yml")?.is_none()
                && !legacy_schema::can_retain("docker-compose.yml", &patch_info.get_changed_files())
            {
                anyhow::bail!("Patch changes Compose but does not include its replacement");
            }
            let critical_files = patch_critical_files(&mut archive)?;
            // 增量升级：根据操作的文件和目录进行操作
            let change_files = patch_info.get_changed_files();
            let work_dir = get_docker_work_dir();
            let upgrade_change_file_or_dir = change_files
                .iter()
                .map(|path| work_dir.join(path))
                .collect::<Vec<_>>();

            // 清理即将被替换或删除的文件/目录（跳过upload目录）
            for file_or_dir in upgrade_change_file_or_dir {
                if is_upload_directory_path(&file_or_dir)
                    || contains_protected_env_path(&file_or_dir, protected)
                {
                    info!(
                        "🛡️ Keeping upload directory, skipping deletion: {}",
                        file_or_dir.display()
                    );
                    continue;
                }

                if file_or_dir.is_file() {
                    std::fs::remove_file(file_or_dir)?;
                } else if file_or_dir.is_dir() {
                    std::fs::remove_dir_all(file_or_dir)?;
                } else {
                    info!(
                        "File or directory does not exist, skipping: {}",
                        file_or_dir.display()
                    );
                }
            }

            let operations = patch_info.operations.clone();
            // 统计解压进度
            let mut extracted_files = 0;
            let mut extracted_size = 0u64;
            let total_files = archive.len();

            info!("🚀 Starting extraction of {} files...", total_files);

            //根据 operations 的 replace, delete 进行操作
            if let Some(replace) = operations.replace {
                let replace_files = replace.files;
                let replace_dirs = replace.directories;

                // 处理替换文件
                for file in replace_files {
                    let zip_path = format!("docker/{}", file.trim_start_matches('/'));
                    info!("🔍 Locating file: {} -> {}", file, zip_path);

                    let mut entry = archive.by_name(&zip_path).map_err(|e| {
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

                    if is_protected_env_path(&dst, protected)
                        && dst.file_name().is_none_or(|name| name != ".env")
                    {
                        continue;
                    }

                    // 检查是否为保护目录路径
                    if is_upload_directory_path(&dst) {
                        // 如果保护目录已存在，跳过解压以保护用户数据；
                        // 例外：`.env` 是文件且属于用户配置——不下钻跳过，
                        // 交给 handle_extraction 走"合并补键"路径（P1#2）
                        let is_user_env_file =
                            dst.is_file() && dst.file_name().is_some_and(|n| n == ".env");
                        if dst.exists() && !is_user_env_file {
                            info!(
                                "🛡️ Keeping existing directory, skipping replacement: {}",
                                dst.display()
                            );
                            continue;
                        } else if !dst.exists() {
                            info!(
                                "📁 Creating new protected directory structure: {}",
                                dst.display()
                            );
                        }
                    }

                    // 强制覆盖：先删除再解压（彻底解决 Directory not empty 错误）；
                    // .env 已存在时由 handle_extraction 走合并路径（保留用户值、补新键）
                    handle_extraction(&mut entry, &dst, &mut extracted_files, &mut extracted_size)?;
                }

                // 处理替换目录
                for dir in replace_dirs {
                    let zip_dir_path = format!("docker/{}", dir.trim_start_matches('/'));
                    info!("📁 Processing directory: {} -> {}", dir, zip_dir_path);

                    // 清理现有目录（跳过保护目录）
                    let target_dir = work_dir.join(&dir);
                    if is_upload_directory_path(&target_dir) && target_dir.exists() {
                        info!(
                            "🛡️ Keeping existing directory, skipping directory replacement: {}",
                            target_dir.display()
                        );
                        continue;
                    }

                    if target_dir.exists() && !contains_protected_env_path(&target_dir, protected) {
                        info!("🗑️  Force removing directory: {}", target_dir.display());
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

                            if is_protected_env_path(&dst, protected) {
                                continue;
                            }
                            if entry.is_dir() && contains_protected_env_path(&dst, protected) {
                                std::fs::create_dir_all(&dst)?;
                                continue;
                            }

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
                    if is_upload_directory_path(&path)
                        || contains_protected_env_path(&path, protected)
                    {
                        info!(
                            "🛡️ Keeping upload directory, skipping file deletion: {}",
                            path.display()
                        );
                        continue;
                    }
                    info!("🗑️ Removing file: {}", path.display());
                    if path.is_file() {
                        std::fs::remove_file(&path)?;
                    } else if path.exists() {
                        std::fs::remove_file(&path).or_else(|_| std::fs::remove_dir_all(&path))?;
                    } else {
                        info!("File does not exist, skipping: {}", path.display());
                    }
                }
                // 删除目录（跳过upload目录）
                for dir in delete.directories {
                    let path = work_dir.join(dir);
                    if is_upload_directory_path(&path)
                        || contains_protected_env_path(&path, protected)
                    {
                        info!(
                            "🛡️ Keeping upload directory, skipping directory deletion: {}",
                            path.display()
                        );
                        continue;
                    }
                    info!("🗑️ Removing directory: {}", path.display());
                    if path.is_dir() {
                        std::fs::remove_dir_all(&path)?;
                    } else if path.exists() {
                        std::fs::remove_file(&path).or_else(|_| std::fs::remove_dir_all(&path))?;
                    } else {
                        info!("Directory does not exist, skipping: {}", path.display());
                    }
                }
            }

            // Apply the same complete release that passed candidate validation,
            // even when the patch operation list omits its metadata or artifacts.
            for critical_file in &critical_files {
                let zip_path = zip_entry_name(&archive, critical_file)?;
                let dst_path = work_dir.join(critical_file);

                match archive.by_name(&zip_path) {
                    Ok(mut entry) => {
                        info!("🔧 Force updating critical file: {}", critical_file);
                        force_extract_file(&mut entry, &dst_path)?;
                        info!("✅ Critical file updated: {}", critical_file);
                    }
                    Err(_) => {
                        // C04/F07: patch 必须携带全部关键 schema/清单文件——缺文件时
                        // 保留的只会是磁盘上的旧版本，Live Diff 将对比错误目标。
                        // 显式失败并引导改用完整包，而不是静默沿用旧文件。
                        return Err(anyhow::anyhow!(
                            "Patch archive is missing critical schema file {zip_path}; \
                             refusing to keep a stale schema (deploy with a full package instead)"
                        ));
                    }
                }
            }
        }
        UpgradeStrategy::NoUpgrade { .. } => {
            // 无需升级,不应该走到这里的解压逻辑
            return Err(anyhow::anyhow!(
                "{}",
                t!("utils.no_upgrade_extract_unsupported")
            ));
        }
    }

    Ok(())
}

/// 解压 TAR.GZ 格式归档
async fn extract_tar_gz_archive(
    tar_gz_path: &std::path::Path,
    upgrade_strategy: &UpgradeStrategy,
    extract_start: Instant,
    protected: Vec<std::path::PathBuf>,
) -> Result<()> {
    let tar_gz_path = tar_gz_path.to_path_buf();
    let strategy = upgrade_strategy.clone();

    tokio::task::spawn_blocking(move || {
        extract_tar_gz_blocking(&tar_gz_path, &strategy, extract_start, &protected)
    })
    .await
    .map_err(|e| anyhow::anyhow!("{}", t!("utils.extract_task_failed", error = e.to_string())))?
}

/// TAR.GZ 解压实现（阻塞）
fn extract_tar_gz_blocking(
    tar_gz_path: &std::path::Path,
    upgrade_strategy: &UpgradeStrategy,
    extract_start: Instant,
    protected: &[std::path::PathBuf],
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
                safe_remove_docker_directory(output_dir, protected)?;
            } else {
                std::fs::create_dir_all(output_dir)?;
            }

            info!("🚀 Starting TAR.GZ extraction...");

            for entry in archive.entries()? {
                let mut entry: tar::Entry<flate2::read::GzDecoder<std::fs::File>> = entry?;
                let path = entry.path()?;
                let entry_type = entry.header().entry_type();
                if entry_type.is_symlink() || entry_type.is_hard_link() {
                    return Err(anyhow::anyhow!(
                        "Archive links are not allowed: {}",
                        path.display()
                    ));
                }
                if contains_unsafe_component(&path) {
                    return Err(anyhow::anyhow!(
                        "Unsafe archive path detected: {}",
                        path.display()
                    ));
                }

                // 跳过 __MACOSX 等系统文件
                if should_skip_tar_entry(&path) {
                    continue;
                }

                // 移除 docker/ 前缀（如果存在）
                let clean_path = path.strip_prefix("docker").unwrap_or(&path);
                let target_path = output_dir.join(clean_path);

                if is_protected_env_path(&target_path, protected) && target_path.exists() {
                    continue;
                }

                if entry_type.is_file()
                    && target_path.file_name().is_some_and(|name| name == ".env")
                    && target_path.is_file()
                {
                    let mut defaults = String::new();
                    entry.read_to_string(&mut defaults)?;
                    env_merge::merge_env_file_defaults(&target_path, &defaults)?;
                    extracted_files += 1;
                    extracted_size += entry.size();
                    continue;
                }

                // 保护 upload 目录
                if is_upload_directory_path(&target_path) && target_path.exists() {
                    info!(
                        "🛡️ Keeping existing directory, skipping: {}",
                        target_path.display()
                    );
                    continue;
                }

                // 确保父目录存在
                if let Some(parent) = target_path.parent()
                    && !parent.exists()
                {
                    std::fs::create_dir_all(parent)?;
                }

                // 解压文件
                entry.unpack(&target_path)?;
                extracted_files += 1;
                extracted_size += entry.size();

                // 每解压10%的文件显示进度
                if extracted_files % 10 == 0 {
                    info!(
                        "📁 Extraction progress: {} files ({:.1} MB)",
                        extracted_files,
                        extracted_size as f64 / 1024.0 / 1024.0
                    );
                }
            }

            let elapsed = extract_start.elapsed();
            info!("🎉 Docker service package extraction completed!");
            info!("   📁 Extracted files: {}", extracted_files);
            info!(
                "   📏 Total data size: {:.1} MB",
                extracted_size as f64 / 1024.0 / 1024.0
            );
            info!("   ⏱️  Elapsed: {:.2} seconds", elapsed.as_secs_f64());
        }
        UpgradeStrategy::PatchUpgrade { .. } => {
            // 增量升级目前不支持 TAR.GZ
            return Err(anyhow::anyhow!("{}", t!("utils.tar_gz_patch_unsupported")));
        }
        UpgradeStrategy::NoUpgrade { .. } => {
            return Err(anyhow::anyhow!(
                "{}",
                t!("utils.no_upgrade_extract_unsupported")
            ));
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
        // 日志走 stderr：stdout 留给数据（如 device-info --json 的机器可读输出），
        // 管道 `nuwax-cli device-info --json | jq` 才不会被日志行污染
        fmt()
            .with_env_filter(env_filter)
            .with_target(false) // 不显示模块路径
            .with_thread_names(false) // 不显示线程名
            .with_line_number(false) // 不显示行号
            .without_time() // 不显示时间戳
            .compact() // 使用紧凑格式
            .with_writer(std::io::stderr)
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

/// 判断归档内是否存在某路径（精确条目，或其下任意后代条目——files-only
/// 归档可能没有显式目录条目，只有 `prefix/file` 形式的文件）
pub fn archive_contains(archive_path: &std::path::Path, path: &str) -> Result<bool> {
    let prefix = format!("{path}/");
    match archive::detect_format_by_magic(archive_path)? {
        client_core::utils::archive::ArchiveFormat::Zip => {
            let file = std::fs::File::open(archive_path)?;
            let mut zip = zip::ZipArchive::new(file)?;
            for index in 0..zip.len() {
                let name = zip.by_index(index)?.name().to_string();
                let normalized = name.strip_prefix("docker/").unwrap_or(&name).to_string();
                if normalized == path || normalized.starts_with(&prefix) {
                    return Ok(true);
                }
            }
            Ok(false)
        }
        client_core::utils::archive::ArchiveFormat::TarGz => {
            let file = std::fs::File::open(archive_path)?;
            let decoder = flate2::read::GzDecoder::new(file);
            let mut tar = tar::Archive::new(decoder);
            for entry in tar.entries()? {
                let name = entry?.path()?.to_string_lossy().into_owned();
                let normalized = name.strip_prefix("docker/").unwrap_or(&name).to_string();
                if normalized == path || normalized.starts_with(&prefix) {
                    return Ok(true);
                }
            }
            Ok(false)
        }
    }
}

/// 按名称读取归档内少量条目（不整包解压、不重复下载）。
///
/// 供停服务前的候选包预检使用：条目名匹配兼容带/不带 `docker/` 前缀；
/// 不存在的条目不出现在返回值中，由调用方决定缺失是否致命。
/// ZIP 读取走 CRC 校验，损坏条目报错。
pub fn read_archive_entries(
    archive_path: &std::path::Path,
    names: &[&str],
) -> Result<std::collections::HashMap<String, Vec<u8>>> {
    let mut found = std::collections::HashMap::new();

    match archive::detect_format_by_magic(archive_path)? {
        client_core::utils::archive::ArchiveFormat::Zip => {
            let file = std::fs::File::open(archive_path)?;
            let mut zip = zip::ZipArchive::new(file)?;
            for index in 0..zip.len() {
                let mut entry = zip.by_index(index)?;
                let raw_path = entry
                    .enclosed_name()
                    .ok_or_else(|| anyhow::anyhow!("Unsafe archive entry: {}", entry.name()))?
                    .to_path_buf();
                let normalized = raw_path
                    .strip_prefix("docker")
                    .unwrap_or(raw_path.as_path())
                    .to_path_buf();
                if let Some(wanted_name) = names
                    .iter()
                    .find(|name| std::path::Path::new(*name) == normalized)
                {
                    let mut bytes = Vec::with_capacity(entry.size() as usize);
                    std::io::Read::read_to_end(&mut entry, &mut bytes)?;
                    if found.insert(wanted_name.to_string(), bytes).is_some() {
                        anyhow::bail!("Duplicate package entry: {wanted_name}");
                    }
                }
            }
        }
        client_core::utils::archive::ArchiveFormat::TarGz => {
            let file = std::fs::File::open(archive_path)?;
            let decoder = flate2::read::GzDecoder::new(file);
            let mut tar = tar::Archive::new(decoder);
            for entry in tar.entries()? {
                let mut entry = entry?;
                let raw_path = entry.path()?.to_path_buf();
                let normalized = raw_path
                    .strip_prefix("docker")
                    .unwrap_or(raw_path.as_path())
                    .to_path_buf();
                if let Some(wanted_name) = names
                    .iter()
                    .find(|name| std::path::Path::new(*name) == normalized)
                {
                    let mut bytes = Vec::new();
                    std::io::Read::read_to_end(&mut entry, &mut bytes)?;
                    if found.insert(wanted_name.to_string(), bytes).is_some() {
                        anyhow::bail!("Duplicate package entry: {wanted_name}");
                    }
                }
            }
        }
    }
    Ok(found)
}

#[cfg(test)]
mod env_path_tests {
    use super::*;

    #[test]
    fn missing_env_parents_use_the_existing_physical_ancestor() -> Result<()> {
        let root = tempfile::tempdir()?;
        let expected = std::fs::canonicalize(root.path())?.join("not-created/nested/operator.env");
        assert_eq!(
            absolute_clean_path(&root.path().join("not-created/nested/operator.env"))?,
            expected
        );
        Ok(())
    }

    #[cfg(unix)]
    #[test]
    fn directory_aliases_match_protected_env_and_leaf_symlink_is_preserved() -> Result<()> {
        use std::os::unix::fs::symlink;
        let root = tempfile::tempdir()?;
        let physical = root.path().join("physical");
        std::fs::create_dir(&physical)?;
        let alias = root.path().join("alias");
        symlink(&physical, &alias)?;
        let actual = physical.join("operator.env");
        std::fs::write(&actual, "EXISTING=operator\n")?;
        let selected = alias.join("operator.env");
        let protected = vec![
            absolute_clean_path(&selected)?,
            std::fs::canonicalize(&selected)?,
        ];
        assert!(is_protected_env_path(&actual, &protected));
        assert!(contains_protected_env_path(&physical, &protected));
        let leaf = alias.join(".env");
        symlink("operator.env", &leaf)?;
        assert_eq!(
            absolute_clean_path(&leaf)?,
            std::fs::canonicalize(&physical)?.join(".env")
        );
        assert_ne!(absolute_clean_path(&leaf)?, std::fs::canonicalize(&leaf)?);
        Ok(())
    }
}
