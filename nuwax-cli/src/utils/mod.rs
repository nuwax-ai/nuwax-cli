use anyhow::{Context, Result};
use client_core::{
    constants::docker::get_docker_work_dir, upgrade_strategy::UpgradeStrategy, utils::archive,
};
use std::io::{Read, Write};
use std::path::{Component, Path};
use tracing::info;

// 导入匹配器模块
pub mod device_env;
pub mod env_manager;
pub mod env_merge;
pub(crate) mod legacy_schema;
pub mod package_replace;

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

#[cfg(test)]
fn is_protected_env_path(path: &Path, protected: &[std::path::PathBuf]) -> bool {
    absolute_clean_path(path).is_ok_and(|absolute| protected.contains(&absolute))
        || std::fs::canonicalize(path).is_ok_and(|canonical| protected.contains(&canonical))
}

#[cfg(test)]
fn contains_protected_env_path(path: &Path, protected: &[std::path::PathBuf]) -> bool {
    absolute_clean_path(path)
        .is_ok_and(|absolute| protected.iter().any(|env| env.starts_with(&absolute)))
}

/// Shared transactional extraction for direct callers. Automatic deployment
/// prepares this same transaction before stopping the existing services.
pub async fn extract_docker_service_with_env(
    archive_path: &Path,
    upgrade_strategy: &UpgradeStrategy,
    env_path: &Path,
) -> Result<()> {
    let mut transaction = package_replace::PackageReplacement::prepare_async(
        archive_path,
        upgrade_strategy,
        Path::new("docker"),
        env_path,
        false,
    )
    .await?;
    transaction.apply()?;
    transaction.finish()
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
