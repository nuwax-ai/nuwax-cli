use crate::app::CliApp;
use crate::cli::AutoUpgradeDeployCommand;
use crate::commands::{backup, docker_service, update};
use crate::docker_service::health_check::HealthChecker;
use anyhow::{Context, Result};
use client_core::constants::sql;
use client_core::container::DockerManager;
use client_core::mysql_executor::{MySqlConfig, MySqlExecutor};
use client_core::sql_diff::generate_live_schema_diff_multi;
use client_core::sql_diff::parse_schema_template;
use client_core::upgrade_strategy::UpgradeStrategy;
use client_core::utils::archive::{self, ArchiveFormat};
use rust_i18n::t;
use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use tokio::time::sleep;
use tracing::{debug, error, info, warn};

/// 创建 DockerManager（统一处理 config_file 和 project_name）
///
/// # 参数
/// - `config_file`: 可选的自定义 docker-compose 配置文件路径
/// - `project_name`: 可选的 docker-compose 项目名称
///
/// # 返回
/// 返回配置好的 DockerManager Arc 引用
fn create_docker_manager(
    configured: &Arc<DockerManager>,
    config_file: &Option<PathBuf>,
    project_name: &Option<String>,
) -> Result<Arc<DockerManager>> {
    docker_service::select_docker_manager(configured, config_file.clone(), project_name.clone())
}

/// A package root is independent of the location of the operator's secrets file.
/// Archive extraction currently supports the canonical package compose only;
/// reject unsupported overrides before stopping or replacing any deployment.
struct DeploymentContext {
    manager: Arc<DockerManager>,
    package_root: PathBuf,
}

impl DeploymentContext {
    fn new(
        configured: &Arc<DockerManager>,
        config_file: &Option<PathBuf>,
        project_name: &Option<String>,
    ) -> Result<Self> {
        let manager = create_docker_manager(configured, config_file, project_name)?;
        let package_root = absolute_deployment_path(Path::new("docker"))?;
        Self::validate_paths(
            manager.get_compose_file(),
            manager.get_env_file(),
            &package_root,
        )?;
        Ok(Self {
            manager,
            package_root,
        })
    }

    fn validate_paths(compose: &Path, env: &Path, package_root: &Path) -> Result<()> {
        if absolute_deployment_path(compose)? != package_root.join("docker-compose.yml") {
            anyhow::bail!(
                "Automatic package deployment only supports {}/docker-compose.yml; custom Compose overrides cannot safely be applied by this package extractor",
                package_root.display()
            );
        }
        if absolute_deployment_path(env)? == package_root.join("docker-compose.yml") {
            anyhow::bail!("The selected environment file must not be the package Compose file");
        }
        Ok(())
    }
}

fn absolute_deployment_path(path: &Path) -> Result<PathBuf> {
    let absolute = if path.is_absolute() {
        path.to_path_buf()
    } else {
        std::env::current_dir()?.join(path)
    };
    let mut normalized = PathBuf::new();
    for component in absolute.components() {
        match component {
            std::path::Component::ParentDir => {
                normalized.pop();
            }
            std::path::Component::CurDir => {}
            other => normalized.push(other.as_os_str()),
        }
    }
    Ok(normalized)
}

/// A directory move invalidates aliases whose link or resolved target is inside
/// the package root. System aliases above that root (such as /var) remain valid.
fn validate_offline_env_aliases(env_path: &Path, package_root: &Path) -> Result<()> {
    let root = absolute_deployment_path(package_root)?;
    let canonical_root = if root.exists() {
        fs::canonicalize(&root).context("Failed to resolve the offline package root")?
    } else {
        let parent = root
            .parent()
            .context("Offline package root has no parent")?;
        let name = root
            .file_name()
            .context("Offline package root has no name")?;
        fs::canonicalize(parent)
            .context("Failed to resolve the offline package parent")?
            .join(name)
    };
    let selected = absolute_deployment_path(env_path)?;
    for ancestor in selected.ancestors() {
        let metadata = match fs::symlink_metadata(ancestor) {
            Ok(metadata) => metadata,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
            Err(error) => {
                return Err(error).context("Failed to inspect the selected environment path");
            }
        };
        if !metadata.file_type().is_symlink() {
            continue;
        }
        let target = fs::canonicalize(ancestor)
            .context("Offline environment aliases must resolve before replacing the package")?;
        let location = match (ancestor.parent(), ancestor.file_name()) {
            (Some(parent), Some(name)) => fs::canonicalize(parent)
                .context("Failed to resolve an environment alias parent")?
                .join(name),
            _ => ancestor.to_path_buf(),
        };
        if location.starts_with(&canonical_root) || target.starts_with(&canonical_root) {
            anyhow::bail!(
                "Offline package replacement cannot preserve an environment alias stored in or targeting the package root; use a regular environment file inside docker/ or an external alias whose real target is outside docker/"
            );
        }
    }
    Ok(())
}

fn read_package_env_defaults(path: &Path) -> Result<String> {
    crate::utils::read_archive_entries(path, &[".env"])?
        .remove(".env")
        .map(String::from_utf8)
        .transpose()
        .context("Package .env is not valid UTF-8")
        .map(|value| value.unwrap_or_default())
}

/// 离线完整包在停止旧服务前必须能提供全部库表模板（多库清单，Fail Fast）。
/// 每个清单文件恰好一份、内容通过 parse_schema_template 校验（唯一 USE + 非空表集）。
fn validate_offline_archive_sql(archive_path: &Path) -> Result<()> {
    let manifest_bytes =
        crate::utils::read_archive_entries(archive_path, &["config/mysql-schema-manifest.json"])?
            .remove("config/mysql-schema-manifest.json");
    let manifest = manifest_bytes
        .map(|bytes| -> Result<_> {
            let text =
                String::from_utf8(bytes).context("Offline schema manifest is not valid UTF-8")?;
            client_core::mysql_manifest::parse_schema_manifest(&text)
        })
        .transpose()?;
    let expected: Vec<PathBuf> = match manifest.as_ref() {
        Some(manifest) => manifest
            .schemas
            .iter()
            .map(|schema| PathBuf::from(&schema.path))
            .collect(),
        None => sql::SCHEMA_SQL_FILES
            .iter()
            .map(|file| {
                Path::new(file)
                    .strip_prefix("docker")
                    .unwrap_or(Path::new(file))
                    .to_path_buf()
            })
            .collect(),
    };
    let is_target = |path: &Path| -> Option<usize> { expected.iter().position(|e| e == path) };

    let mut found: Vec<Option<String>> = vec![None; expected.len()];
    let mut record = |path: &Path, content: String| -> Result<()> {
        if let Some(index) = is_target(path) {
            if found[index].is_some() {
                return Err(anyhow::anyhow!(
                    "Duplicate {} in offline archive",
                    expected[index].display()
                ));
            }
            found[index] = Some(content);
        }
        Ok(())
    };

    match archive::detect_format_by_magic(archive_path)? {
        ArchiveFormat::Zip => {
            let file = fs::File::open(archive_path)?;
            let mut zip = zip::ZipArchive::new(file)?;
            for idx in 0..zip.len() {
                let mut entry = zip.by_index(idx)?;
                let raw_path = entry
                    .enclosed_name()
                    .ok_or_else(|| anyhow::anyhow!("Unsafe archive entry: {}", entry.name()))?;
                let path = raw_path
                    .strip_prefix("docker")
                    .unwrap_or(raw_path.as_ref())
                    .to_path_buf();
                if is_target(&path).is_some() {
                    let mut content = String::new();
                    entry.read_to_string(&mut content)?;
                    record(&path, content)?;
                }
            }
        }
        ArchiveFormat::TarGz => {
            let file = fs::File::open(archive_path)?;
            let decoder = flate2::read::GzDecoder::new(file);
            let mut tar = tar::Archive::new(decoder);
            for entry in tar.entries()? {
                let mut entry = entry?;
                let raw_path = entry.path()?.to_path_buf();
                let path = raw_path
                    .strip_prefix("docker")
                    .unwrap_or(raw_path.as_ref())
                    .to_path_buf();
                if is_target(&path).is_some() {
                    let mut content = String::new();
                    entry.read_to_string(&mut content)?;
                    record(&path, content)?;
                }
            }
        }
    }

    for (template_path, content) in expected.iter().zip(found) {
        let content = content.ok_or_else(|| {
            anyhow::anyhow!("Offline archive is missing {}", template_path.display())
        })?;
        let template = parse_schema_template(&content)
            .with_context(|| format!("Invalid schema template {}", template_path.display()))?;
        if let Some(manifest) = manifest.as_ref()
            && let Some(schema) = manifest
                .schemas
                .iter()
                .find(|schema| Path::new(&schema.path) == template_path.as_path())
            && template.database != schema.database
        {
            anyhow::bail!(
                "Offline schema {} USE does not match its manifest database",
                template_path.display()
            );
        }
    }
    Ok(())
}

/// 更新配置文件中的版本号并持久化
///
/// 使用 Arc::make_mut 来获取可变引用，如果 Arc 有多个引用会自动克隆
///
/// # 参数
/// - `config`: 应用配置的可变 Arc 引用
/// - `version`: 新的版本号字符串
///
/// # 错误
/// 如果保存配置文件失败会返回错误
fn update_config_version(
    config: &mut Arc<client_core::config::AppConfig>,
    config_path: &Path,
    version: &str,
) -> Result<()> {
    let config_mut = Arc::make_mut(config);
    config_mut.write_docker_versions(version.to_string());

    config_mut
        .save_to_file(config_path)
        .context(t!("auto_upgrade_deploy.save_config_failed"))?;

    info!(
        version = version,
        "✅ Updated and saved the version in configuration file"
    );
    Ok(())
}

async fn wait_for_mysql_connection(compose_path: &Path, env_path: &Path) -> Result<MySqlExecutor> {
    let compose = compose_path
        .to_str()
        .ok_or_else(|| anyhow::anyhow!("Compose path is not valid UTF-8"))?;
    let env = env_path
        .to_str()
        .ok_or_else(|| anyhow::anyhow!("Compose env path is not valid UTF-8"))?;
    // root 管理连接：多库 Live Diff 需要建库/授权与跨库 DDL 权限（应用账号不具备）
    let config = MySqlConfig::for_container_admin(Some(compose), Some(env))
        .await
        .context("Failed to resolve MySQL admin connection from Compose")?;
    let executor = MySqlExecutor::new(config);
    let timeout = Duration::from_secs(sql::MYSQL_READY_TIMEOUT);
    let deadline = tokio::time::Instant::now() + timeout;
    let mut last_error = "no connection attempt completed".to_string();
    loop {
        let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
        if remaining.is_zero() {
            return Err(anyhow::anyhow!(
                "MySQL was not connectable within {}s: {last_error}",
                timeout.as_secs()
            ));
        }
        let attempt_timeout = remaining.min(Duration::from_secs(10));
        match tokio::time::timeout(attempt_timeout, executor.test_connection()).await {
            Ok(Ok(())) => return Ok(executor),
            Ok(Err(error)) => last_error = error.to_string(),
            Err(_) => last_error = "connection attempt timed out".to_string(),
        }
        debug!(error = %last_error, "Waiting for MySQL to accept SQL connections");
        let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
        sleep(remaining.min(Duration::from_secs(2))).await;
    }
}

async fn run_staged_deployment(
    app: &mut CliApp,
    frontend_port: Option<u16>,
    config_file: Option<PathBuf>,
    project_name: Option<String>,
    context: &DeploymentContext,
    had_existing_compose: bool,
    target_version: &str,
) -> Result<()> {
    // 部署前校验迁移计划：manifest v1（解析+结构+文件校验）或 legacy 固定清单
    // （缺文件/解析失败即 Fail Fast）；顺带收集库名供配置预检校验应用连接目标
    let migration_plan = resolve_migration_plan(&context.package_root)?;
    let (manifest, template_databases) = match &migration_plan {
        MigrationPlan::Manifest(manifest) => (Some(manifest), manifest.database_names()),
        MigrationPlan::Legacy => {
            let mut template_databases = Vec::with_capacity(sql::SCHEMA_SQL_FILES.len());
            for template_path in sql::SCHEMA_SQL_FILES {
                let path = context
                    .package_root
                    .join(template_path.trim_start_matches("docker/"));
                let content = fs::read_to_string(&path).with_context(|| {
                    format!(
                        "{}",
                        t!(
                            "auto_upgrade_deploy.schema_template_missing",
                            path = path.display().to_string()
                        )
                    )
                })?;
                let template = parse_schema_template(&content)
                    .with_context(|| format!("Invalid schema template {template_path}"))?;
                template_databases.push(template.database);
            }
            (None, template_databases)
        }
    };

    let docker_manager = &context.manager;
    docker_manager.invalidate_compose_config_cache();

    // C01/C03/C04: 候选 Compose + 合并后 .env 的预检（启动 mysql、修改数据库之前的最后一道闸）：
    // 必填键非空、应用连接与 manifest 逐条映射一致（legacy 则按约定键）、
    // 宿主产物完整 + 交付清单 hash（失败只报键名/库名/路径，不含任何凭据）
    let delivery = load_delivery_manifest(&context.package_root)?;
    client_core::container::preflight::preflight_deploy_config_at(
        docker_manager.get_compose_file(),
        docker_manager.get_env_file(),
        &context.package_root,
        &template_databases,
        manifest.map(std::convert::AsRef::as_ref),
        delivery.as_ref(),
    )
    .context("Deployment configuration preflight failed")?;
    if let Some(manifest) = manifest {
        client_core::container::preflight::validate_initdb_mount_contract_at(
            docker_manager.get_compose_file(),
            &context.package_root,
            manifest,
        )
        .context("Candidate compose violates the initdb mount contract")?;
    }

    if had_existing_compose {
        // stop_docker_services_and_wait 在没有运行容器时可能跳过 down；这里清理旧的已停止容器。
        docker_manager
            .stop_services()
            .await
            .context("Failed to remove stopped containers from the previous deployment")?;
    }
    docker_service::prepare_docker_services(app, frontend_port, config_file.clone(), project_name)
        .await?;
    // prepare_docker_services may update .env (frontend port).
    docker_manager.invalidate_compose_config_cache();

    let mysql_stage = docker_manager.get_service_dependency_closure("mysql")?;
    let all_services = docker_manager.get_compose_service_names().await?;
    info!("▶️ Starting MySQL and its Compose dependencies...");
    docker_manager
        .up_services(&["mysql".to_string()], false)
        .await?;
    let mysql_id_before = docker_manager.get_service_container_id("mysql").await?;
    let executor = wait_for_mysql_connection(
        docker_manager.get_compose_file(),
        docker_manager.get_env_file(),
    )
    .await?;

    // MySQL decides whether initdb/seeds apply from the actual data volume.
    // Once it is connectable, always perform idempotent bootstrap/Live Diff:
    // an empty host directory or a custom data mount cannot justify skipping it.
    info!("🔄 MySQL is connectable; applying live schema differences before applications start");
    execute_sql_diff_upgrade(&executor, &context.package_root).await?;

    let startup_layers = docker_manager.get_compose_startup_layers(&mysql_stage)?;
    for layer in startup_layers {
        info!(services = %layer.join(", "), "▶️ Starting next Compose dependency layer...");
        docker_manager
            .up_services_without_dependencies(&layer, true)
            .await?;
        docker_manager
            .wait_for_compose_services_ready(
                &layer,
                Duration::from_secs(client_core::constants::timeout::HEALTH_CHECK_TIMEOUT),
            )
            .await?;
    }

    let mysql_id_after = docker_manager.get_service_container_id("mysql").await?;
    if mysql_id_before != mysql_id_after {
        return Err(anyhow::anyhow!(
            "MySQL container changed while starting application services; refusing to mark deployment successful"
        ));
    }

    let mut all_service_names: Vec<String> = all_services.into_iter().collect();
    all_service_names.sort();
    docker_manager
        .wait_for_compose_services_ready(
            &all_service_names,
            Duration::from_secs(client_core::constants::timeout::HEALTH_CHECK_TIMEOUT),
        )
        .await?;
    let health_checker = HealthChecker::new(docker_manager.clone());
    health_checker
        .wait_for_services_ready(Duration::from_secs(
            client_core::constants::timeout::HEALTH_CHECK_INTERVAL,
        ))
        .await
        .context("Docker services did not become healthy after MySQL migration")?;

    let app_config_path = app.config_path.clone();
    update_config_version(&mut app.config, &app_config_path, target_version)?;
    info!("✅ Deployment completed after MySQL migration and service health checks");
    Ok(())
}

fn create_docker_backup_path() -> PathBuf {
    let timestamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs())
        .unwrap_or(0);
    PathBuf::from(format!(
        ".docker.offline-backup-{}-{timestamp}",
        std::process::id()
    ))
}

fn restore_docker_backup(backup_dir: &Path, docker_dir: &Path) -> Result<()> {
    if docker_dir.exists() {
        fs::remove_dir_all(docker_dir).context("Failed to remove partial docker directory")?;
    }

    if backup_dir.exists() {
        fs::rename(backup_dir, docker_dir)
            .context("Failed to restore previous docker directory")?;
    }

    Ok(())
}

fn restore_preserved_docker_dirs(backup_dir: &Path, docker_dir: &Path) -> Result<()> {
    if !backup_dir.exists() {
        return Ok(());
    }

    fs::create_dir_all(docker_dir).context("Failed to create docker directory")?;

    for dir_name in client_core::constants::docker::EXCLUDE_DIRS {
        let old_path = backup_dir.join(dir_name);
        if !old_path.exists() {
            continue;
        }

        let new_path = docker_dir.join(dir_name);
        if dir_name == ".env" && old_path.is_file() && new_path.is_file() {
            crate::utils::env_merge::merge_preserved_env_file(&old_path, &new_path)?;
            info!("🛡️ Preserved existing .env values and added missing package defaults");
            continue;
        }

        if new_path.exists() {
            if new_path.is_dir() {
                fs::remove_dir_all(&new_path).with_context(|| {
                    format!("Failed to replace preserved directory: {dir_name}")
                })?;
            } else {
                fs::remove_file(&new_path)
                    .with_context(|| format!("Failed to replace preserved file: {dir_name}"))?;
            }
        }

        fs::rename(&old_path, &new_path)
            .with_context(|| format!("Failed to restore preserved directory: {dir_name}"))?;
        info!("🛡️ Restored preserved docker directory: {dir_name}");
    }

    Ok(())
}

/// 迁移计划：包内 mysql-schema-manifest.json（v1 契约）或 legacy 固定清单
enum MigrationPlan {
    Manifest(Box<client_core::mysql_manifest::SchemaManifest>),
    Legacy,
}

/// 解析迁移计划：manifest 存在 → 解析 + 结构校验 + 文件校验（Fail Fast，
/// 不降级到扫描 SQL）；不存在 → legacy。`docker_root` 为包内 docker/ 目录。
fn resolve_migration_plan(docker_root: &Path) -> Result<MigrationPlan> {
    let manifest_relative = Path::new(sql::SCHEMA_MANIFEST_PATH)
        .strip_prefix("docker")
        .unwrap_or(Path::new(sql::SCHEMA_MANIFEST_PATH));
    let manifest_path = docker_root.join(manifest_relative);
    if !manifest_path.exists() {
        info!("📦 No mysql-schema-manifest.json in package; using legacy fixed schema list");
        return Ok(MigrationPlan::Legacy);
    }
    let text = fs::read_to_string(&manifest_path)
        .with_context(|| format!("Failed to read {}", manifest_path.display()))?;
    let manifest = client_core::mysql_manifest::parse_schema_manifest(&text)
        .with_context(|| format!("Invalid {}", manifest_path.display()))?;
    client_core::mysql_manifest::validate_manifest_files(&manifest, docker_root)
        .context("mysql-schema-manifest references invalid files")?;
    info!(
        databases = ?manifest.database_names(),
        schemas = manifest.schemas.len(),
        "📦 mysql-schema-manifest v1 loaded"
    );
    Ok(MigrationPlan::Manifest(Box::new(manifest)))
}

/// 读取包内交付清单（DELIVERY_MANIFEST.json v1）。
/// 文件不存在 → None（legacy 包）；存在但损坏/不支持 → Fail Fast。
fn load_delivery_manifest(
    docker_root: &Path,
) -> Result<Option<client_core::container::preflight::DeliveryManifest>> {
    let relative = Path::new(sql::DELIVERY_MANIFEST_PATH)
        .strip_prefix("docker")
        .unwrap_or(Path::new(sql::DELIVERY_MANIFEST_PATH));
    let path = docker_root.join(relative);
    if !path.exists() {
        return Ok(None);
    }
    let text =
        fs::read_to_string(&path).with_context(|| format!("Failed to read {}", path.display()))?;
    let delivery = client_core::container::preflight::parse_delivery_manifest(&text)
        .with_context(|| format!("Invalid {}", path.display()))?;
    Ok(Some(delivery))
}

/// 停止旧服务前的候选包预检（P1#3）：直接读取已下载/本地归档的少量条目
/// （compose / .env / schema manifest / 交付清单 / 关键 schema / 组件产物），
/// 不重复下载大包、不整包预解压。新增必填键缺失、坏清单、patch 缺关键
/// schema、连接映射错误都在停止旧服务之前失败（旧服务保持运行）。
fn preflight_candidate_package_at(
    archive_path: &Path,
    is_patch: bool,
    user_env_path: &Path,
) -> Result<()> {
    if is_patch && archive::detect_format_by_magic(archive_path)? != ArchiveFormat::Zip {
        anyhow::bail!(
            "Incremental package application requires ZIP; use a full package for TAR.GZ archives"
        );
    }
    let base_names = [
        "docker-compose.yml",
        ".env",
        "config/mysql-schema-manifest.json",
        "DELIVERY_MANIFEST.json",
    ];
    let entries =
        crate::utils::read_archive_entries(archive_path, &base_names).with_context(|| {
            format!(
                "Failed to read candidate package entries from {}",
                archive_path.display()
            )
        })?;

    let compose_bytes = entries
        .get("docker-compose.yml")
        .ok_or_else(|| anyhow::anyhow!("candidate package has no docker-compose.yml"))?;
    let compose_text = String::from_utf8(compose_bytes.clone())
        .context("candidate package docker-compose.yml is not valid UTF-8")?;

    // 合并 env：用户现值完全保留 + 包内默认补键；解析走与运行时相同的
    // Compose env-file 解析器（引号/行内注释/插值-未定义为空/优先级，F03）
    let user_env = match fs::read_to_string(user_env_path) {
        Ok(contents) => contents,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => String::new(),
        Err(error) => {
            return Err(error).with_context(|| {
                format!(
                    "Failed to read selected environment file {}",
                    user_env_path.display()
                )
            });
        }
    };
    let package_env = entries
        .get(".env")
        .map(|bytes| String::from_utf8(bytes.clone()))
        .transpose()
        .context("Candidate package .env is not valid UTF-8")?
        .unwrap_or_default();
    let values = client_core::container::preflight::compose_env_values_from_text(
        &crate::utils::env_merge::merge_env_contents(&user_env, &package_env)?,
    )?;

    let missing =
        client_core::container::preflight::missing_required_env_keys(&compose_text, &values)?;
    if !missing.is_empty() {
        return Err(anyhow::anyhow!(
            "candidate package requires environment keys that are missing or empty: {missing:?}; \
             define them in the selected environment file before upgrading"
        ));
    }

    // schema manifest：存在即必须可解析且引用文件都在归档内（不降级、不扫描 SQL）
    let manifest = match entries.get("config/mysql-schema-manifest.json") {
        Some(bytes) => {
            let text = String::from_utf8(bytes.clone())
                .context("mysql-schema-manifest.json is not valid UTF-8")?;
            let manifest = client_core::mysql_manifest::parse_schema_manifest(&text)
                .context("candidate package carries an invalid mysql-schema-manifest")?;
            let referenced = manifest.referenced_paths();
            let refs: Vec<&str> = referenced.iter().map(String::as_str).collect();
            let ref_entries = crate::utils::read_archive_entries(archive_path, &refs)?;
            for path in &refs {
                let bytes = ref_entries.get(*path).ok_or_else(|| {
                    anyhow::anyhow!(
                        "mysql-schema-manifest references {path}, missing from the package"
                    )
                })?;
                if bytes.is_empty() {
                    return Err(anyhow::anyhow!(
                        "mysql-schema-manifest references {path}, but the package entry is empty"
                    ));
                }
            }
            client_core::container::preflight::validate_application_connections(
                &compose_text,
                &values,
                &manifest,
            )
            .context("candidate package connection mapping does not match the migration targets")?;
            client_core::container::preflight::validate_initdb_mount_contract(
                &compose_text,
                &manifest,
            )
            .context("candidate package initdb mount contract is violated")?;
            Some(manifest)
        }
        None => {
            // legacy 包：patch 必须携带全部关键 schema（不能沿用磁盘旧文件）
            if is_patch {
                for critical in client_core::constants::sql::CRITICAL_UPGRADE_FILES {
                    let present = crate::utils::read_archive_entries(archive_path, &[critical])?
                        .contains_key(*critical);
                    if !present {
                        return Err(anyhow::anyhow!(
                            "patch package is missing critical schema file {critical}; \
                             refusing to keep a stale schema (use a full package)"
                        ));
                    }
                }
            }
            None
        }
    };
    let _ = manifest;

    // 交付清单：存在即校验（hash + release 身份 + 架构一致）
    let mut delivery = None;
    if let Some(bytes) = entries.get("DELIVERY_MANIFEST.json") {
        let text = String::from_utf8(bytes.clone())
            .context("DELIVERY_MANIFEST.json is not valid UTF-8")?;
        let parsed = client_core::container::preflight::parse_delivery_manifest(&text)
            .context("candidate package carries an invalid DELIVERY_MANIFEST")?;
        let mut needed: Vec<String> = parsed.mysql.files.keys().cloned().collect();
        needed.extend(
            parsed
                .components
                .values()
                .flat_map(|component| component.artifacts.keys().cloned()),
        );
        needed.push(parsed.compose.path.clone());
        let refs: Vec<&str> = needed.iter().map(String::as_str).collect();
        let delivery_entries = crate::utils::read_archive_entries(archive_path, &refs)?;
        client_core::container::preflight::verify_delivery_against_entries(
            &parsed,
            &delivery_entries,
            Some(crate::docker_service::get_system_architecture().as_str()),
        )
        .context("candidate package contents do not match its DELIVERY_MANIFEST")?;
        delivery = Some(parsed);
    }

    // 组件产物挂载存在性：候选 compose 挂载引用的组件路径必须在归档内
    let component_roots: Vec<String> = delivery
        .as_ref()
        .map(|parsed| {
            let mut roots: Vec<String> = parsed
                .components
                .values()
                .flat_map(|component| {
                    component
                        .artifacts
                        .keys()
                        .map(|path| path.split('/').next().unwrap_or(path).to_string())
                })
                .collect();
            roots.sort();
            roots.dedup();
            roots
        })
        .unwrap_or_else(|| vec!["im-app".to_string(), "repo-collab-app".to_string()]);
    let mounts = client_core::container::preflight::collect_bind_mount_sources(&compose_text)?;
    let component_mounts: Vec<String> = mounts
        .into_iter()
        .filter(|source| {
            component_roots
                .iter()
                .any(|root| source == root || source.starts_with(&format!("{root}/")))
        })
        .collect();
    if !component_mounts.is_empty() {
        let refs: Vec<&str> = component_mounts.iter().map(String::as_str).collect();
        let mount_entries = crate::utils::read_archive_entries(archive_path, &refs)?;
        for source in &component_mounts {
            // 文件形态（叶子含扩展名）要求精确条目；目录形态允许 files-only 归档
            //（无显式目录条目，仅有 prefix/file 后代）
            let looks_like_file = source
                .rsplit('/')
                .next()
                .is_some_and(|leaf| leaf.contains('.'));
            let present = mount_entries.contains_key(source.as_str())
                || (!looks_like_file && crate::utils::archive_contains(archive_path, source)?);
            if !present {
                return Err(anyhow::anyhow!(
                    "candidate compose mounts '{source}' but the package does not contain it"
                ));
            }
        }
    }

    info!(
        archive = %archive_path.display(),
        "✅ Candidate package preflight passed (required keys, schema manifest, connections, delivery, artifacts)"
    );
    Ok(())
}

/// 在线升级的候选包预检包装：解析策略对应本地包路径并执行归档预检
fn preflight_candidate_package(
    app: &CliApp,
    strategy: &UpgradeStrategy,
    user_env_path: &Path,
) -> Result<()> {
    let Some(path) = docker_service::package_path_for_strategy(app, strategy)? else {
        return Ok(());
    };
    if !path.exists() {
        return Err(anyhow::anyhow!(
            "upgrade package is not available after download: {}",
            path.display()
        ));
    }
    let is_patch = matches!(strategy, UpgradeStrategy::PatchUpgrade { .. });
    preflight_candidate_package_at(&path, is_patch, user_env_path)
}

/// 停止旧服务前的当前配置预检（C01）：必填键、库目标/连接映射、宿主产物完整性。
/// 路径来自最终选定的 DockerManager（--config / 项目配置覆盖生效，F06）。
/// 候选新包的同一预检在解压后、修改数据库前由 `run_staged_deployment` 再执行一次。
fn preflight_current_config(
    compose_path: &Path,
    env_path: &Path,
    docker_root: &Path,
) -> Result<()> {
    if !compose_path.is_file() {
        return Ok(());
    }
    let plan = resolve_migration_plan(docker_root)?;
    let (manifest, template_databases) = match &plan {
        MigrationPlan::Manifest(manifest) => (Some(manifest), manifest.database_names()),
        MigrationPlan::Legacy => (None, collect_existing_template_databases(docker_root)),
    };
    let delivery = load_delivery_manifest(docker_root)?;
    client_core::container::preflight::preflight_deploy_config_at(
        compose_path,
        env_path,
        docker_root,
        &template_databases,
        manifest.map(std::convert::AsRef::as_ref),
        delivery.as_ref(),
    )
    .context("Configuration preflight failed before stopping services")
}

/// 收集磁盘上现存且可解析的 schema 模板库名（宽容处理：旧部署可能尚无
/// 新增模板文件；完整校验在解压后的预检中执行，这里只为库目标校验提供输入）
fn collect_existing_template_databases(docker_root: &Path) -> Vec<String> {
    client_core::constants::sql::SCHEMA_SQL_FILES
        .iter()
        .filter_map(|relative| {
            fs::read_to_string(docker_root.join(relative.trim_start_matches("docker/")))
                .ok()
                .and_then(|content| parse_schema_template(&content).ok())
                .map(|template| template.database)
        })
        .collect()
}

/// Immutable in-memory snapshot of the selected operator environment.
/// Capture before stopping services; apply package defaults or restore the
/// original contents atomically to the same file, without secrets backup files.
struct OnlineEnvPreserve {
    env_path: PathBuf,
    preserved: Option<String>,
    permissions: Option<fs::Permissions>,
}

impl OnlineEnvPreserve {
    fn capture(env_path: &Path) -> Result<Self> {
        let preserved = match fs::read_to_string(env_path) {
            Ok(contents) => Some(contents),
            Err(error)
                if error.kind() == std::io::ErrorKind::NotFound
                    && fs::symlink_metadata(env_path).is_err() =>
            {
                None
            }
            Err(error) => {
                return Err(error).with_context(|| {
                    format!(
                        "Failed to preserve selected environment file {}",
                        env_path.display()
                    )
                });
            }
        };
        let permissions = if preserved.is_some() {
            Some(fs::metadata(env_path)?.permissions())
        } else {
            None
        };
        Ok(Self {
            env_path: env_path.to_path_buf(),
            preserved,
            permissions,
        })
    }

    fn merge_defaults(&self, defaults: &str) -> Result<()> {
        let merged = crate::utils::env_merge::merge_env_contents(
            self.preserved.as_deref().unwrap_or_default(),
            defaults,
        )?;
        crate::utils::env_merge::write_env_contents(
            &self.env_path,
            &merged,
            self.permissions.clone(),
        )
    }

    fn restore(&self) -> Result<()> {
        if let Some(contents) = self.preserved.as_ref() {
            crate::utils::env_merge::write_env_contents(
                &self.env_path,
                contents,
                self.permissions.clone(),
            )?;
        }
        Ok(())
    }
}

fn env_snapshot_for_strategy(
    strategy: &UpgradeStrategy,
    env_path: &Path,
) -> Result<Option<OnlineEnvPreserve>> {
    match strategy {
        UpgradeStrategy::NoUpgrade { .. } => Ok(None),
        _ => OnlineEnvPreserve::capture(env_path).map(Some),
    }
}

/// 运行自动升级部署相关命令的统一入口
pub async fn handle_auto_upgrade_deploy_command(
    app: &mut CliApp,
    cmd: AutoUpgradeDeployCommand,
) -> Result<()> {
    match cmd {
        AutoUpgradeDeployCommand::Run {
            port,
            config,
            project,
        } => {
            info!("🚀 Starting auto-upgrade deployment...");
            run_auto_upgrade_deploy(app, port, config, project).await
        }
        AutoUpgradeDeployCommand::Status => {
            info!("Show auto-upgrade deployment status");
            show_status(app).await
        }
        AutoUpgradeDeployCommand::OfflineDeploy {
            archive,
            version,
            port,
            config,
            project,
        } => {
            info!("📦 Starting offline deployment...");
            run_offline_deploy(app, archive, version, port, config, project).await
        }
    }
}

/// 执行自动升级部署流程
pub async fn run_auto_upgrade_deploy(
    app: &mut CliApp,
    frontend_port: Option<u16>,
    config_file: Option<PathBuf>,
    project_name: Option<String>,
) -> Result<()> {
    info!("🚀 Starting auto-upgrade deployment...");

    // 如果指定了端口，显示端口信息
    if let Some(port) = frontend_port {
        info!("🔌 Custom frontend port: {port}", port = port);
    }

    // 如果指定了配置文件，显示配置文件信息
    if let Some(config_path) = &config_file {
        info!(
            "📄 Custom docker-compose configuration file: {path}",
            path = config_path.display()
        );
    }

    // 注意：CLI版本检查已经在 main.rs 中优先处理，这里不再重复检查
    info!("✅ CLI version pre-check complete, starting upgrade deployment");

    let context = DeploymentContext::new(&app.docker_manager, &config_file, &project_name)?;
    let docker_manager = &context.manager;

    // 1. 获取最新版本信息并下载
    info!("📥 Downloading the latest Docker service version...");

    // 下载策略只读取一次 manifest；部署版本从同一策略中取得，避免版本与包不一致。
    let upgrade_args = crate::cli::UpgradeArgs {
        force: false,
        check: false,
    };
    let upgrade_strategy = update::run_upgrade(app, upgrade_args).await?;
    let target_version = match &upgrade_strategy {
        UpgradeStrategy::FullUpgrade { target_version, .. }
        | UpgradeStrategy::PatchUpgrade { target_version, .. }
        | UpgradeStrategy::NoUpgrade { target_version } => target_version.to_string(),
    };

    let had_existing_compose = docker_manager.get_compose_file().is_file();
    preflight_current_config(
        docker_manager.get_compose_file(),
        docker_manager.get_env_file(),
        &context.package_root,
    )?;
    // Initial installations also validate the candidate before extraction.
    preflight_candidate_package(app, &upgrade_strategy, docker_manager.get_env_file())
        .context("Candidate package preflight failed before stopping services")?;
    let package_path = docker_service::package_path_for_strategy(app, &upgrade_strategy)?;
    let package_defaults = package_path
        .as_deref()
        .map(read_package_env_defaults)
        .transpose()?
        .unwrap_or_default();
    let env_preserve = env_snapshot_for_strategy(&upgrade_strategy, docker_manager.get_env_file())?;
    if had_existing_compose {
        // 3. 🛑 停止服务并等待（使用统一的公共方法）
        let stopped = docker_service::stop_docker_services_and_wait(
            app,
            config_file.clone(),
            project_name.clone(),
        )
        .await?;
        if !stopped {
            return Err(anyhow::anyhow!(
                "Timed out stopping old services; refusing to upgrade MySQL"
            ));
        }
    }

    // 5. 🔍 提前检查并创建挂载目录（重要：Windows Podman Desktop 需要）
    info!("🔍 Checking and creating mount directories...");
    // DockerManager 已在停服前创建并用于预检（见上方 F06 注释），此处直接复用

    // 使用新的环境检测机制
    let runtime_env = docker_manager.get_runtime_environment();

    if runtime_env.needs_special_handling() {
        info!("⚠️ Windows Podman Desktop detected, pre-creating mount directories");
        info!("Environment info: {env}", env = runtime_env.summary());
        info!("Podman Desktop does not auto-create mount directories; manual creation is required");

        if let Err(e) = docker_manager.ensure_host_volumes_exist().await {
            warn!(
                "⚠️ Mount directory check/creation failed: {error}",
                error = e.to_string()
            );
            warn!("Continuing execution, but container startup may fail");
        } else {
            info!("✅ Mount directory check complete");
        }
    } else {
        info!(
            "ℹ️ Current environment: {env} (no special handling needed)",
            env = runtime_env.summary()
        );
    }

    // 6. 📦 解压新的Docker服务包（在服务停止后）
    info!("📦 Extracting Docker service package...");

    // Extraction is the sole owner of cleanup and protects the selected env.
    let extraction = match package_path.as_deref() {
        Some(path) => {
            crate::utils::extract_docker_service_with_env(
                path,
                &upgrade_strategy,
                docker_manager.get_env_file(),
            )
            .await
        }
        None => Ok(()),
    };
    match extraction {
        Ok(_) => {
            info!("✅ Docker service package extracted");

            // C01: 与包内 .env 合并（保留用户值、仅补新包新增键；NoUpgrade 无守卫）
            if let Some(env_preserve) = env_preserve.as_ref() {
                env_preserve.merge_defaults(&package_defaults)?;
            }

            // 🔧 自动修复关键脚本文件权限
            fix_script_permissions().await?;

            // 版本号在 MySQL 迁移及全部服务健康后提交。
        }
        Err(e) => {
            error!(
                "❌ Failed to extract Docker service package: {error}",
                error = e.to_string()
            );
            // C01: 解压失败原样还原用户 .env，保证失败不污染
            if let Some(env_preserve) = env_preserve.as_ref()
                && let Err(restore_error) = env_preserve.restore()
            {
                warn!(
                    "⚠️ Failed to restore preserved .env after extraction failure: {error}",
                    error = restore_error.to_string()
                );
            }
            return Err(e);
        }
    }

    run_staged_deployment(
        app,
        frontend_port,
        config_file,
        project_name,
        &context,
        had_existing_compose,
        &target_version,
    )
    .await
}

/// 预约延迟执行自动升级部署
#[allow(dead_code)]
pub async fn schedule_delayed_deploy(app: &mut CliApp, time: u32, unit: &str) -> Result<()> {
    // 计算延迟时间（转换为秒）
    let delay_seconds = match unit.to_lowercase().as_str() {
        "minutes" | "minute" | "min" => time * 60,
        "hours" | "hour" | "h" => time * 3600,
        "days" | "day" | "d" => time * 86400,
        _ => {
            error!("Unsupported time unit: {unit}", unit = unit);
            return Err(anyhow::anyhow!(t!(
                "auto_upgrade_deploy.unsupported_time_unit_error",
                unit = unit
            )));
        }
    };

    let delay_duration = Duration::from_secs(delay_seconds as u64);
    let scheduled_at = chrono::Utc::now() + chrono::Duration::seconds(delay_seconds as i64);

    // 创建升级任务记录
    let task = client_core::config_manager::AutoUpgradeTask {
        task_id: uuid::Uuid::new_v4().to_string(),
        task_name: format!("delayed_upgrade_{time}"),
        schedule_time: scheduled_at,
        upgrade_type: "delayed".to_string(),
        target_version: None, // 最新版本
        status: "pending".to_string(),
        progress: Some(0),
        error_message: None,
        created_at: chrono::Utc::now(),
        updated_at: chrono::Utc::now(),
    };

    {
        let config_manager =
            client_core::config_manager::ConfigManager::new_with_database(app.database.clone());
        config_manager.create_auto_upgrade_task(&task).await?
    };

    info!("⏰ Delayed auto-upgrade deployment has been scheduled");
    info!("Task ID: {id}", id = task.task_id);
    info!("Delay: {time} {unit}", time = time, unit = unit);
    println!(
        "   {}",
        t!(
            "auto_upgrade_deploy.estimated_exec_time",
            duration = format_duration(delay_duration)
        )
    );
    info!(
        "Planned execution time: {time}",
        time = scheduled_at.format("%Y-%m-%d %H:%M:%S UTC")
    );

    info!(
        "Scheduled delayed auto-upgrade deployment: {time} {unit}, task ID: {task_id}",
        time = time,
        unit = unit,
        task_id = task.task_id
    );

    // 更新任务状态为进行中
    {
        let config_manager =
            client_core::config_manager::ConfigManager::new_with_database(app.database.clone());
        config_manager
            .update_upgrade_task_status(&task.task_id, "in_progress", Some(0), None)
            .await?;
    }

    // 开始延迟等待
    info!("⏳ Waiting...");

    // 这里可以优化为后台任务，避免阻塞
    sleep(delay_duration).await;

    info!("🔔 Delay reached; starting auto-upgrade deployment");
    info!(
        "Delay reached; auto-upgrade deployment starting, task ID: {task_id}",
        task_id = task.task_id
    );

    // 执行自动升级部署
    match run_auto_upgrade_deploy(app, None, None, None).await {
        Ok(_) => {
            let config_manager =
                client_core::config_manager::ConfigManager::new_with_database(app.database.clone());
            config_manager
                .update_upgrade_task_status(&task.task_id, "completed", Some(100), None)
                .await?;
            info!("✅ Delayed upgrade deployment task completed");
        }
        Err(e) => {
            let config_manager =
                client_core::config_manager::ConfigManager::new_with_database(app.database.clone());
            config_manager
                .update_upgrade_task_status(&task.task_id, "failed", None, Some(&e.to_string()))
                .await?;
            error!(
                "Delayed upgrade deployment task failed: {error}",
                error = e.to_string()
            );
            return Err(e);
        }
    }

    Ok(())
}

/// 显示自动升级部署状态
pub async fn show_status(app: &mut CliApp) -> Result<()> {
    let config_manager =
        client_core::config_manager::ConfigManager::new_with_database(app.database.clone());

    info!("📊 Auto-upgrade deployment status:");
    info!("Feature status: implemented");
    info!("Process: download latest version -> smart backup -> deploy services -> start services");

    // 显示待执行的升级任务
    match config_manager.get_pending_upgrade_tasks().await {
        Ok(tasks) => {
            if tasks.is_empty() {
                info!("📋 Upgrade tasks: no pending upgrade tasks");
            } else {
                info!("📋 Pending upgrade tasks:");
                for task in tasks {
                    info!("- Task ID: {id}", id = task.task_id);
                    info!("Name: {name}", name = task.task_name);
                    info!("Type: {type_name}", type_name = task.upgrade_type);
                    info!("Status: {status}", status = task.status);
                    info!(
                        "Planned execution time: {time}",
                        time = task.schedule_time.format("%Y-%m-%d %H:%M:%S UTC")
                    );
                    if let Some(target_version) = &task.target_version {
                        info!("Target version: {version}", version = target_version);
                    }
                    if let Some(progress) = task.progress {
                        info!("Progress: {progress}%", progress = progress);
                    }
                    if let Some(error) = &task.error_message {
                        warn!("Error message: {error}", error = error);
                    }
                }
            }
        }
        Err(e) => {
            warn!(
                "⚠️ Failed to obtain upgrade task information: {error}",
                error = e.to_string()
            );
            info!("Note: Task query capability is limited in this version");
        }
    }

    // 显示当前Docker服务状态
    info!("🐳 Current Docker service status:");
    docker_service::check_docker_services_status(app).await?;

    // 显示最近的备份
    info!("📝 Recent backups:");
    backup::run_list_backups(app).await?;

    Ok(())
}

/// 格式化时间间隔为可读字符串
#[allow(dead_code)]
fn format_duration(duration: Duration) -> String {
    let seconds = duration.as_secs();

    if seconds >= 86400 {
        t!("auto_upgrade_deploy.days", count = seconds / 86400).to_string()
    } else if seconds >= 3600 {
        t!("auto_upgrade_deploy.hours", count = seconds / 3600).to_string()
    } else if seconds >= 60 {
        t!("auto_upgrade_deploy.minutes", count = seconds / 60).to_string()
    } else {
        t!("auto_upgrade_deploy.seconds", count = seconds).to_string()
    }
}

/// 归档差异SQL文件
///
/// # 参数
/// - `diff_sql_path`: 差异SQL文件路径
/// - `status`: 状态标识 ("executed", "failed", "no_exec")
async fn archive_diff_sql_file(diff_sql_path: &Path, status: &str) -> Result<()> {
    if !diff_sql_path.is_file() {
        return Ok(());
    }

    let parent = diff_sql_path.parent().unwrap_or(Path::new("."));
    let timestamp = chrono::Local::now().format("%Y%m%d_%H%M%S_%f");
    let new_name = format!("diff_sql_{}_{}.sql", status, timestamp);
    let new_path = parent.join(new_name);

    match fs::rename(diff_sql_path, &new_path) {
        Ok(_) => {
            let status_desc = match status {
                "executed" => t!("auto_upgrade_deploy.diff_sql_executed"),
                "failed" => t!("auto_upgrade_deploy.diff_sql_failed"),
                "no_exec" => t!("auto_upgrade_deploy.diff_sql_no_exec"),
                _ => t!("auto_upgrade_deploy.diff_sql_archived"),
            };
            info!(
                "📝 {status} diff SQL file: {path}",
                status = status_desc,
                path = new_path.display()
            );
            Ok(())
        }
        Err(e) => {
            warn!(
                "⚠️ Failed to archive diff SQL file: {error}",
                error = e.to_string()
            );
            Ok(()) // 归档失败不影响主流程
        }
    }
}

/// legacy 迁移路径（包内无 mysql-schema-manifest.json 的旧包）：
/// 固定清单逐模板解析（建库/授权原句随模板 preamble 透传）→ 逐库 Live Diff。
async fn legacy_live_diff(
    temp_sql_dir: &Path,
    executor: &MySqlExecutor,
    package_root: &Path,
) -> Result<client_core::sql_diff::MultiDbDiffResult> {
    let mut templates = Vec::with_capacity(sql::SCHEMA_SQL_FILES.len());
    for template_path in sql::SCHEMA_SQL_FILES {
        let source_path = package_root.join(template_path.trim_start_matches("docker/"));
        let content = fs::read_to_string(&source_path).with_context(|| {
            format!(
                "{}",
                t!(
                    "auto_upgrade_deploy.schema_template_missing",
                    path = source_path.display().to_string()
                )
            )
        })?;
        let template = parse_schema_template(&content)
            .with_context(|| format!("Invalid schema template {template_path}"))?;

        // 同一库出现在多个模板文件属于配置错误（diff 会重复建表）
        if templates
            .iter()
            .any(|existing: &client_core::sql_diff::SchemaTemplate| {
                existing.database == template.database
            })
        {
            return Err(anyhow::anyhow!(
                "Duplicate schema template for database `{}` in SCHEMA_SQL_FILES",
                template.database
            ));
        }

        let new_sql_path = temp_sql_dir.join(format!("{}_new.sql", template.database));
        if new_sql_path.exists() {
            fs::remove_file(&new_sql_path)?;
        }
        fs::copy(&source_path, &new_sql_path).context(t!(
            "auto_upgrade_deploy.copy_sql_failed",
            src = source_path.display(),
            dst = new_sql_path.display()
        ))?;
        info!(
            database = %template.database,
            tables = template.tables.len(),
            path = %new_sql_path.display(),
            "📄 Copied schema template for database"
        );

        templates.push(template);
    }

    info!("📊 Generating SQL differences based on online schema (legacy fixed list)...");
    let diff_result = generate_live_schema_diff_multi(executor, &templates, "target version")
        .await
        .context(t!("auto_upgrade_deploy.generate_live_diff_failed"))?;

    Ok(diff_result)
}

async fn execute_sql_diff_upgrade(executor: &MySqlExecutor, package_root: &Path) -> Result<()> {
    let temp_sql_dir = Path::new(sql::TEMP_SQL_DIR);
    let diff_sql_path = temp_sql_dir.join(sql::DIFF_SQL_FILE);

    // 如果 temp_sql 目录已存在，先归档到 history_sql
    if temp_sql_dir.exists() {
        let history_dir = Path::new("history_sql");
        if !history_dir.exists() {
            fs::create_dir_all(history_dir)?;
            info!(
                "📁 Creating history SQL directory: {path}",
                path = history_dir.display()
            );
        }

        // 生成带时间戳的目录名
        let timestamp = chrono::Local::now().format("%Y%m%d_%H%M%S");
        let archive_name = format!("temp_sql_{}", timestamp);
        let archive_path = history_dir.join(&archive_name);

        // 移动 temp_sql 目录到 history_sql（带降级处理）
        match fs::rename(temp_sql_dir, &archive_path) {
            Ok(_) => {
                info!(
                    "📦 Archived old temp_sql directory to: {path}",
                    path = archive_path.display()
                );
            }
            Err(e) => {
                warn!(
                    "⚠️ Failed to archive temp_sql directory: {error}, try to clean it directly",
                    error = e.to_string()
                );
                // 降级：直接删除旧目录
                if let Err(e2) = fs::remove_dir_all(temp_sql_dir) {
                    warn!(
                        "⚠️ Failed to clean temp_sql directory: {error}, continue execution",
                        error = e2.to_string()
                    );
                } else {
                    info!("✅ The old temp_sql directory has been cleaned");
                }
            }
        }
    }

    // 创建临时SQL目录
    fs::create_dir_all(temp_sql_dir)?;
    info!(
        "📁 Creating temp SQL directory: {path}",
        path = temp_sql_dir.display()
    );

    // 迁移计划：manifest v1（bootstrap → 授权 → 逐库 Live Diff）或 legacy 固定清单
    let migration_plan = resolve_migration_plan(package_root)?;
    let diff_result = match &migration_plan {
        MigrationPlan::Manifest(manifest) => {
            // manifest 路径：留档各库模板副本；bootstrap/授权独立于表差异执行
            for schema in &manifest.schemas {
                let source_path = package_root.join(&schema.path);
                let new_sql_path = temp_sql_dir.join(format!("{}_new.sql", schema.database));
                if new_sql_path.exists() {
                    fs::remove_file(&new_sql_path)?;
                }
                fs::copy(&source_path, &new_sql_path).context(t!(
                    "auto_upgrade_deploy.copy_sql_failed",
                    src = source_path.display(),
                    dst = new_sql_path.display()
                ))?;
                info!(
                    database = %schema.database,
                    path = %new_sql_path.display(),
                    "📄 Copied schema template for database"
                );
            }
            info!("📊 Manifest migration: bootstrap → permissions → per-database Live Diff...");
            let app_user = executor.app_user().unwrap_or_default();
            client_core::mysql_manifest::run_manifest_migration(
                executor,
                manifest,
                package_root,
                app_user,
            )
            .await
            .context(t!("auto_upgrade_deploy.generate_live_diff_failed"))?
        }
        MigrationPlan::Legacy => legacy_live_diff(temp_sql_dir, executor, package_root).await?,
    };

    info!(description = %diff_result.description, has_executable_sql = diff_result.has_executable_sql, has_warnings = diff_result.has_warnings, "📋 Difference generation completed");

    // 逐库保存在线架构快照（SHOW CREATE TABLE 原文）
    for section in &diff_result.sections {
        let old_sql_path = temp_sql_dir.join(format!("{}_old.sql", section.database));
        fs::write(&old_sql_path, &section.live_sql)?;
        info!(
            database = %section.database,
            path = %old_sql_path.display(),
            "📄 Saved online schema SQL file"
        );
    }

    // 保存差异SQL文件（无论是否有可执行SQL，都保存以便查看）
    fs::write(&diff_sql_path, &diff_result.diff_sql)
        .context(t!("auto_upgrade_deploy.save_diff_sql_failed"))?;
    info!(
        "📄 Diff SQL file saved: {path}",
        path = diff_sql_path.display()
    );

    // 判断差异类型并输出相应提示
    if !diff_result.has_executable_sql {
        // 没有可执行SQL（可能有警告，也可能完全无差异）
        if diff_result.has_warnings {
            // 只有人工处理提示，没有可执行的新增/修改 SQL。
            info!("⚠️ Schema difference detected: only manual changes (skipped)");
            info!("💡 Manual changes are listed in the diff file");
        } else {
            // 情况2：完全没有差异（既没有可执行SQL，也没有警告）
            info!("📄 No database schema differences; no upgrade required");
        }

        // 统一归档差异SQL文件
        archive_diff_sql_file(&diff_sql_path, "no_exec").await?;
        return Ok(());
    }

    // 情况3：有可执行的SQL语句（可能同时包含警告）
    // 再次确认是否真的有可执行的SQL语句（排除全是注释的情况）
    let executable_lines: Vec<&str> = diff_result
        .diff_sql
        .lines()
        .filter(|line| {
            let trimmed = line.trim();
            !trimmed.is_empty() && !trimmed.starts_with("--") && !trimmed.starts_with("/*")
        })
        .collect();

    if executable_lines.is_empty() {
        // 虽然 has_executable_sql 为 true，但实际没有可执行的SQL（可能是逻辑错误）
        warn!("⚠️ Difference detected but no executable SQL statements");
        archive_diff_sql_file(&diff_sql_path, "no_exec").await?;
        return Ok(());
    }

    info!(
        sql_lines = executable_lines.len(),
        has_warnings = diff_result.has_warnings,
        "🔄 Start database upgrade"
    );

    // 如果同时包含警告，提示用户注意（这是混合场景：既有新增/修改，又有删除）
    if diff_result.has_warnings {
        warn!("⚠️ Note: diff contains executable SQL and manual-change warnings");
        warn!("✓ Add/modify operations will execute normally");
        warn!("✗ Manual changes were skipped and must be reviewed separately");
        warn!("📄 See details: {path}", path = diff_sql_path.display());
    }

    info!("🚀 Starting one-pass diff SQL execution");
    match executor.execute_diff_sql_once(&diff_result.diff_sql).await {
        Ok(results) => {
            info!(
                executed_statements = results.len(),
                "✅ Database upgraded successfully"
            );
            for result in results {
                info!("  {}", result);
            }

            // 归档已执行的差异SQL文件
            archive_diff_sql_file(&diff_sql_path, "executed").await?;
        }
        Err(e) => {
            error!(error = %e, "❌ Database upgrade failed");
            // 归档执行失败的差异SQL文件
            archive_diff_sql_file(&diff_sql_path, "failed").await?;
            return Err(e);
        }
    }

    Ok(())
}

/// 自动修复关键脚本文件权限
async fn fix_script_permissions() -> Result<()> {
    info!("🔧 Fixing critical script file permissions...");

    // 需要修复权限的脚本文件列表
    let script_files = ["docker/config/docker-entrypoint.sh"];

    #[cfg(unix)]
    let mut fixed_count = 0;
    #[cfg(not(unix))]
    let fixed_count = 0;
    let mut total_count = 0;

    for script_path in script_files.iter() {
        let path = std::path::Path::new(script_path);

        if path.exists() {
            total_count += 1;

            // 检查当前权限
            match std::fs::metadata(path) {
                Ok(metadata) => {
                    #[cfg(not(unix))]
                    let _ = metadata;
                    #[cfg(unix)]
                    {
                        use std::os::unix::fs::PermissionsExt;
                        let current_mode = metadata.permissions().mode() & 0o777;

                        // 如果没有执行权限，添加执行权限
                        if current_mode & 0o111 == 0 {
                            info!(
                                "🔒 Fixed permissions: {path} (current: {current} -> target: 755)",
                                path = path.display(),
                                current = format!("{:o}", current_mode)
                            );

                            let new_permissions = std::fs::Permissions::from_mode(0o755);
                            if let Err(e) = std::fs::set_permissions(path, new_permissions) {
                                warn!(
                                    "⚠️ Failed to fix permission {path}: {error}",
                                    path = path.display(),
                                    error = e.to_string()
                                );
                            } else {
                                fixed_count += 1;
                                info!("✅ Permission fixed: {path}", path = path.display());
                            }
                        } else {
                            info!(
                                "✓ Permission is correct: {path} ({mode})",
                                path = path.display(),
                                mode = format!("{:o}", current_mode)
                            );
                        }
                    }

                    #[cfg(not(unix))]
                    {
                        info!(
                            "ℹ️ Non-Unix systems, skip permission fix: {path}",
                            path = path.display()
                        );
                    }
                }
                Err(e) => {
                    warn!(
                        "⚠️ Unable to read file metadata {path}: {error}",
                        path = path.display(),
                        error = e.to_string()
                    );
                }
            }
        } else {
            info!(
                "📄 The script file does not exist, skip: {path}",
                path = script_path
            );
        }
    }

    if total_count > 0 {
        info!(
            "🔧 Permission fix complete: {fixed}/{total} scripts fixed",
            fixed = fixed_count,
            total = total_count
        );
    } else {
        info!("📄 No script files found that require permission fixes");
    }

    Ok(())
}

/// 检查并安装 nuwax-cli 更新（独立函数，用于早期检查）
/// 这个函数可以在数据库初始化之前调用，避免数据库锁冲突
pub async fn check_and_install_nuwax_cli_update_early() -> Result<()> {
    use crate::commands::check_update::{check_for_updates, install_release};

    info!("🔍 Prioritizing nuwax-cli version check (before database initialization)...");

    // 检查更新
    let version_info = match check_for_updates().await {
        Ok(info) => {
            info!(
                "✅ Version check completed: current={current}, latest={latest}",
                current = info.current_version,
                latest = info.latest_version
            );
            info
        }
        Err(e) => {
            error!(
                "❌ Failed to check for updates: {error}",
                error = e.to_string()
            );
            return Err(e);
        }
    };

    // 如果有更新，进行安装
    if version_info.is_update_available {
        info!(
            "🚀 Found new version of nuwax-cli: {current} -> {latest}",
            current = version_info.current_version,
            latest = version_info.latest_version
        );
        info!("📥 Starting automatic update installation...");

        match install_release(
            &version_info.download_url.unwrap_or_default(),
            &version_info.latest_version,
        )
        .await
        {
            Ok(_) => {
                info!(
                    "✅ nuwax-cli updated successfully! The program will restart to use the new version"
                );
                std::process::exit(0);
            }
            Err(e) => {
                error!(
                    "❌ nuwax-cli automatic update failed: {error}",
                    error = e.to_string()
                );
                error!(
                    "Please check the network connection or run manually: nuwax-cli check-update install"
                );
                return Err(e);
            }
        }
    } else {
        info!("✅ nuwax-cli is already up to date");
    }

    Ok(())
}

/// 离线部署入口函数
pub async fn run_offline_deploy(
    app: &mut CliApp,
    archive_path: PathBuf,
    version: String,
    frontend_port: Option<u16>,
    config_file: Option<PathBuf>,
    project_name: Option<String>,
) -> Result<()> {
    info!(
        "📦 Starting offline deployment from: {path}",
        path = archive_path.display()
    );

    let context = DeploymentContext::new(&app.docker_manager, &config_file, &project_name)?;
    let docker_manager = &context.manager;
    validate_offline_env_aliases(docker_manager.get_env_file(), &context.package_root)?;

    // 1. 验证文件存在
    if !archive_path.exists() {
        return Err(anyhow::anyhow!(
            "Archive file not found: {}. Please check the file path.",
            archive_path.display()
        ));
    }

    crate::utils::validate_archive_paths(&archive_path)
        .context("Archive package validation failed")?;
    validate_offline_archive_sql(&archive_path)
        .context("Offline package MySQL DDL preflight failed")?;

    // 2. 解析版本号
    let version: client_core::version::Version =
        version.parse().context("Invalid version format")?;

    info!(
        "   Target version: {version}",
        version = version.to_string()
    );

    let had_existing_compose = docker_manager.get_compose_file().is_file();
    preflight_current_config(
        docker_manager.get_compose_file(),
        docker_manager.get_env_file(),
        &context.package_root,
    )?;
    preflight_candidate_package_at(&archive_path, false, docker_manager.get_env_file())
        .context("Offline package preflight failed before stopping services")?;
    let package_defaults = read_package_env_defaults(&archive_path)?;
    let env_preserve = OnlineEnvPreserve::capture(docker_manager.get_env_file())?;
    if had_existing_compose {
        // 停止服务并等待
        let stopped = docker_service::stop_docker_services_and_wait(
            app,
            config_file.clone(),
            project_name.clone(),
        )
        .await?;
        if !stopped {
            return Err(anyhow::anyhow!(
                "Timed out stopping old services; refusing to upgrade MySQL"
            ));
        }
    }

    // 4. DockerManager 已在停服前创建（用于预检），此处直接复用

    // 5. 环境检测（Podman Desktop 需要预先创建挂载目录）
    let runtime_env = docker_manager.get_runtime_environment();
    if runtime_env.needs_special_handling() {
        info!("⚠️ Windows Podman Desktop detected, pre-creating mount directories");
        if let Err(e) = docker_manager.ensure_host_volumes_exist().await {
            warn!(
                "⚠️ Mount directory check/creation failed: {error}",
                error = e.to_string()
            );
        }
    }

    // 6. 解压（全量升级方式）
    info!("📦 Extracting Docker service package...");

    let upgrade_strategy = UpgradeStrategy::FullUpgrade {
        url: String::new(),
        hash: String::new(),
        signature: String::new(),
        target_version: version.clone(),
        download_type: client_core::upgrade_strategy::DownloadType::Full,
    };

    // 直接解压本地文件。先把旧 docker 目录改名备份，解压失败时恢复。
    let docker_dir = context.package_root.as_path();
    let backup_dir = create_docker_backup_path();
    let had_existing_docker_dir = docker_dir.exists();
    if had_existing_docker_dir {
        info!("🧹 Moving existing docker directory to temporary backup...");
        fs::rename(docker_dir, &backup_dir)
            .context("Failed to backup existing docker directory")?;
    }

    if let Err(e) = crate::utils::extract_docker_service_with_env(
        &archive_path,
        &upgrade_strategy,
        docker_manager.get_env_file(),
    )
    .await
    {
        warn!("⚠️ Extract failed, restoring previous docker directory");
        restore_docker_backup(&backup_dir, docker_dir)?;
        env_preserve.restore()?;
        return Err(e);
    }

    if had_existing_docker_dir {
        restore_preserved_docker_dirs(&backup_dir, docker_dir)?;
    }
    env_preserve.merge_defaults(&package_defaults)?;
    info!("✅ Docker service package extracted");

    let target_version = version.to_string();
    let deployment = async {
        fix_script_permissions().await?;
        run_staged_deployment(
            app,
            frontend_port,
            config_file,
            project_name,
            &context,
            had_existing_compose,
            &target_version,
        )
        .await
    }
    .await;
    if let Err(error) = deployment {
        if had_existing_docker_dir {
            error!(backup_path = %backup_dir.display(), "Deployment failed; old package files were retained for inspection. MySQL data was not rolled back");
        }
        return Err(error);
    }

    if had_existing_docker_dir
        && backup_dir.exists()
        && let Err(error) = fs::remove_dir_all(&backup_dir)
    {
        warn!(backup_path = %backup_dir.display(), %error, "Deployment succeeded, but old package directory could not be removed");
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{OnlineEnvPreserve, restore_preserved_docker_dirs, validate_offline_archive_sql};
    use anyhow::{Context, Result};
    use flate2::{Compression, write::GzEncoder};
    use std::{fs::File, io::Write, path::Path};

    fn valid_platform_sql() -> &'static str {
        "CREATE DATABASE IF NOT EXISTS agent_platform;\n\
         GRANT ALL PRIVILEGES ON agent_platform.* TO 'agent_platform'@'%';\n\
         USE agent_platform;\n\
         CREATE TABLE users (id INT);"
    }

    fn valid_im_sql() -> &'static str {
        "CREATE DATABASE IF NOT EXISTS `nuwax_im`;\n\
         USE `nuwax_im`;\n\
         CREATE TABLE im_users (id INT);"
    }

    fn write_archive(path: &Path, platform: Option<&str>, im: Option<&str>) -> Result<()> {
        let encoder = GzEncoder::new(File::create(path)?, Compression::default());
        let mut archive = tar::Builder::new(encoder);
        for (name, sql) in [
            ("docker/config/init_mysql.sql", platform),
            ("docker/config/init_mysql_im.sql", im),
        ] {
            if let Some(sql) = sql {
                let mut header = tar::Header::new_gnu();
                header.set_size(sql.len() as u64);
                header.set_mode(0o644);
                header.set_cksum();
                archive.append_data(&mut header, name, sql.as_bytes())?;
            }
        }
        archive.finish()?;
        archive.into_inner()?.finish()?.flush()?;
        Ok(())
    }

    #[test]
    fn tar_patch_is_rejected_during_candidate_preflight() -> Result<()> {
        let directory = tempfile::tempdir()?;
        let archive = directory.path().join("patch.tar.gz");
        write_archive(&archive, Some(valid_platform_sql()), Some(valid_im_sql()))?;
        let error =
            super::preflight_candidate_package_at(&archive, true, &directory.path().join(".env"))
                .err()
                .context("TAR.GZ incremental application must fail before extraction")?;
        assert!(error.to_string().contains("ZIP"));
        Ok(())
    }

    #[test]
    fn selected_env_defaults_and_restore_use_the_selected_file() -> Result<()> {
        let directory = tempfile::tempdir()?;
        let docker = directory.path().join("docker");
        std::fs::create_dir_all(docker.join("secrets"))?;
        let selected = docker.join("secrets/operator.env");
        let default = docker.join(".env");
        std::fs::write(&selected, "EXISTING=operator\n")?;
        std::fs::write(&default, "EXISTING=default\n")?;
        let snapshot = OnlineEnvPreserve::capture(&selected)?;
        snapshot.merge_defaults("EXISTING=package\nNEW_KEY=added\n")?;
        assert_eq!(
            std::fs::read_to_string(&selected)?,
            "EXISTING=operator\nNEW_KEY=added\n"
        );
        assert_eq!(std::fs::read_to_string(&default)?, "EXISTING=default\n");
        snapshot.restore()?;
        assert_eq!(std::fs::read_to_string(&selected)?, "EXISTING=operator\n");
        assert_eq!(
            std::fs::read_dir(directory.path())?.count(),
            1,
            "no secrets backup is left on disk"
        );
        Ok(())
    }

    #[test]
    fn absent_selected_env_is_created_from_defaults() -> Result<()> {
        let directory = tempfile::tempdir()?;
        let selected = directory.path().join("secrets/operator.env");
        let snapshot = OnlineEnvPreserve::capture(&selected)?;
        snapshot.merge_defaults("NEW_KEY=added\n")?;
        assert_eq!(std::fs::read_to_string(&selected)?, "NEW_KEY=added\n");
        Ok(())
    }

    #[test]
    fn context_rejects_custom_compose_before_package_changes() -> Result<()> {
        let directory = tempfile::tempdir()?;
        let root = directory.path().join("docker");
        let env = directory.path().join("secrets/operator.env");
        super::DeploymentContext::validate_paths(&root.join("docker-compose.yml"), &env, &root)?;
        assert!(
            super::DeploymentContext::validate_paths(&root.join("custom.yml"), &env, &root)
                .is_err()
        );
        assert!(
            !root.exists(),
            "context validation must not mutate package files"
        );
        Ok(())
    }

    #[cfg(unix)]
    #[test]
    fn offline_rejects_external_leaf_and_parent_aliases_into_moving_package() -> Result<()> {
        use std::os::unix::fs::symlink;
        let directory = tempfile::tempdir()?;
        let package = directory.path().join("docker");
        std::fs::create_dir_all(package.join("secrets"))?;
        let target = package.join("secrets/operator.env");
        std::fs::write(&target, "OPERATOR=fixture\n")?;
        let leaf = directory.path().join("external.env");
        symlink(&target, &leaf)?;
        assert!(super::validate_offline_env_aliases(&leaf, &package).is_err());

        let parent = directory.path().join("external-secrets");
        symlink(package.join("secrets"), &parent)?;
        assert!(
            super::validate_offline_env_aliases(&parent.join("operator.env"), &package).is_err()
        );
        assert_eq!(std::fs::read_to_string(&target)?, "OPERATOR=fixture\n");
        Ok(())
    }

    #[cfg(unix)]
    #[test]
    fn offline_accepts_plain_inner_env_and_aliases_with_stable_external_targets() -> Result<()> {
        use std::os::unix::fs::symlink;
        let directory = tempfile::tempdir()?;
        let package = directory.path().join("docker");
        std::fs::create_dir_all(package.join("secrets"))?;
        let inner = package.join("secrets/operator.env");
        std::fs::write(&inner, "OPERATOR=fixture\n")?;
        super::validate_offline_env_aliases(&inner, &package)?;

        let external = directory.path().join("stable.env");
        std::fs::write(&external, "OPERATOR=external-fixture\n")?;
        let alias = directory.path().join("external-alias.env");
        symlink(&external, &alias)?;
        super::validate_offline_env_aliases(&alias, &package)?;

        let inside_alias = package.join("inside-alias.env");
        symlink(&external, &inside_alias)?;
        assert!(super::validate_offline_env_aliases(&inside_alias, &package).is_err());

        // An alias above the package root is equivalent to macOS /var or /tmp:
        // its target is an ancestor of the root, not something moved with it.
        let system_parent = directory.path().join("system-parent-alias");
        symlink(directory.path(), &system_parent)?;
        super::validate_offline_env_aliases(
            &system_parent.join("docker/secrets/operator.env"),
            &system_parent.join("docker"),
        )?;
        Ok(())
    }

    #[test]
    fn no_upgrade_has_no_env_snapshot_to_restore_or_consume() -> Result<()> {
        let directory = tempfile::tempdir()?;
        let env = directory.path().join("operator.env");
        std::fs::write(&env, "EXISTING=operator\n")?;
        let strategy = client_core::upgrade_strategy::UpgradeStrategy::NoUpgrade {
            target_version: "0.0.92.0".parse()?,
        };
        assert!(super::env_snapshot_for_strategy(&strategy, &env)?.is_none());
        assert_eq!(std::fs::read_to_string(&env)?, "EXISTING=operator\n");
        assert_eq!(std::fs::read_dir(directory.path())?.count(), 1);
        Ok(())
    }

    #[tokio::test]
    async fn full_extraction_preserves_nested_selected_env_then_merges_new_keys() -> Result<()> {
        use client_core::upgrade_strategy::{DownloadType, UpgradeStrategy};
        let directory = tempfile::tempdir()?;
        let docker = directory.path().join("docker");
        std::fs::create_dir_all(docker.join("secrets"))?;
        let selected = docker.join("secrets/operator.env");
        std::fs::write(&selected, "EXISTING=operator\n")?;
        std::fs::write(docker.join("secrets/obsolete.conf"), "old")?;
        std::fs::write(docker.join(".env"), "EXISTING=default\n")?;
        let archive = directory.path().join("full.zip");
        let mut zip = zip::ZipWriter::new(File::create(&archive)?);
        for (path, contents) in [
            ("docker/.env", "EXISTING=package\nNEW_KEY=added\n"),
            (
                "docker/secrets/operator.env",
                "package-must-not-replace-operator",
            ),
            ("docker/secrets/new.conf", "new"),
        ] {
            zip.start_file(path, zip::write::SimpleFileOptions::default())?;
            zip.write_all(contents.as_bytes())?;
        }
        zip.finish()?;
        let snapshot = OnlineEnvPreserve::capture(&selected)?;
        let strategy = UpgradeStrategy::FullUpgrade {
            url: String::new(),
            hash: String::new(),
            signature: String::new(),
            target_version: "0.0.92.0".parse()?,
            download_type: DownloadType::Full,
        };
        let cwd = std::env::current_dir()?;
        std::env::set_current_dir(directory.path())?;
        let extracted =
            crate::utils::extract_docker_service_with_env(&archive, &strategy, &selected).await;
        std::env::set_current_dir(cwd)?;
        extracted?;
        assert_eq!(std::fs::read_to_string(&selected)?, "EXISTING=operator\n");
        assert!(!docker.join("secrets/obsolete.conf").exists());
        assert_eq!(
            std::fs::read_to_string(docker.join("secrets/new.conf"))?,
            "new"
        );
        snapshot.merge_defaults(&super::read_package_env_defaults(&archive)?)?;
        assert_eq!(
            std::fs::read_to_string(&selected)?,
            "EXISTING=operator\nNEW_KEY=added\n"
        );
        assert_eq!(
            std::fs::read_to_string(docker.join(".env"))?,
            "EXISTING=default\nNEW_KEY=added\n"
        );
        Ok(())
    }

    #[tokio::test]
    async fn full_tar_preserves_nested_selected_env_and_merges_unselected_defaults() -> Result<()> {
        use client_core::upgrade_strategy::{DownloadType, UpgradeStrategy};
        let directory = tempfile::tempdir()?;
        let docker = directory.path().join("docker");
        std::fs::create_dir_all(docker.join("secrets"))?;
        let selected = docker.join("secrets/operator.env");
        std::fs::write(&selected, "EXISTING=operator\n")?;
        std::fs::write(docker.join("secrets/obsolete.conf"), "old")?;
        std::fs::write(docker.join(".env"), "EXISTING=default\n")?;
        let archive = directory.path().join("full.tar.gz");
        let encoder = GzEncoder::new(File::create(&archive)?, Compression::default());
        let mut tar = tar::Builder::new(encoder);
        for (path, contents) in [
            ("docker/.env", "EXISTING=package\nNEW_KEY=added\n"),
            (
                "docker/secrets/operator.env",
                "package-must-not-replace-operator",
            ),
            ("docker/secrets/new.conf", "new"),
        ] {
            let mut header = tar::Header::new_gnu();
            header.set_size(contents.len() as u64);
            header.set_mode(0o600);
            header.set_cksum();
            tar.append_data(&mut header, path, contents.as_bytes())?;
        }
        tar.into_inner()?.finish()?;
        let snapshot = OnlineEnvPreserve::capture(&selected)?;
        let strategy = UpgradeStrategy::FullUpgrade {
            url: String::new(),
            hash: String::new(),
            signature: String::new(),
            target_version: "0.0.92.0".parse()?,
            download_type: DownloadType::Full,
        };
        let cwd = std::env::current_dir()?;
        std::env::set_current_dir(directory.path())?;
        let extracted =
            crate::utils::extract_docker_service_with_env(&archive, &strategy, &selected).await;
        std::env::set_current_dir(cwd)?;
        extracted?;
        assert_eq!(std::fs::read_to_string(&selected)?, "EXISTING=operator\n");
        assert!(!docker.join("secrets/obsolete.conf").exists());
        assert_eq!(
            std::fs::read_to_string(docker.join("secrets/new.conf"))?,
            "new"
        );
        snapshot.merge_defaults(&super::read_package_env_defaults(&archive)?)?;
        assert_eq!(
            std::fs::read_to_string(&selected)?,
            "EXISTING=operator\nNEW_KEY=added\n"
        );
        assert_eq!(
            std::fs::read_to_string(docker.join(".env"))?,
            "EXISTING=default\nNEW_KEY=added\n"
        );
        Ok(())
    }

    /// P1#2 回归：真实 ZIP patch 提取器必须把包内新 .env 键合并进用户旧值
    /// （旧缺陷：解压器跳过已存在的 .env，最后旧文件与自己合并，新键永不落地）
    #[tokio::test]
    async fn patch_extraction_merges_package_env_into_user_values() -> Result<()> {
        use anyhow::Context as _;
        use client_core::api_types::{PatchOperations, PatchPackageInfo, ReplaceOperations};
        use client_core::upgrade_strategy::DownloadType;
        use client_core::upgrade_strategy::UpgradeStrategy;

        let directory = tempfile::tempdir()?;
        let docker_dir = directory.path().join("docker");
        std::fs::create_dir_all(docker_dir.join("config"))?;
        std::fs::write(
            docker_dir.join(".env"),
            "MYSQL_PASSWORD=user-secret\nPORT=8080\n",
        )?;

        let archive_path = directory.path().join("patch.zip");
        let mut zip = zip::ZipWriter::new(File::create(&archive_path)?);
        zip.start_file("docker/.env", zip::write::SimpleFileOptions::default())?;
        zip.write_all(b"MYSQL_PASSWORD=package-default\nNEW_REQUIRED=package-value\n")?;
        zip.start_file(
            "docker/config/app.conf",
            zip::write::SimpleFileOptions::default(),
        )?;
        zip.write_all(b"content\n")?;
        // C04 契约：patch 必须携带全部关键 schema（缺文件会被提取器硬拒绝）
        for critical in client_core::constants::sql::CRITICAL_UPGRADE_FILES {
            zip.start_file(
                format!("docker/{critical}"),
                zip::write::SimpleFileOptions::default(),
            )?;
            zip.write_all(
                b"USE agent_platform;\nCREATE TABLE `t` (`id` bigint NOT NULL, PRIMARY KEY (`id`));\n",
            )?;
        }
        zip.finish()?;

        let strategy = UpgradeStrategy::PatchUpgrade {
            patch_info: PatchPackageInfo {
                url: String::new(),
                hash: None,
                signature: None,
                notes: None,
                operations: PatchOperations {
                    replace: Some(ReplaceOperations {
                        files: vec![".env".to_string(), "config/app.conf".to_string()],
                        directories: Vec::new(),
                    }),
                    delete: None,
                },
            },
            target_version: "0.0.90.0".parse().context("version")?,
            download_type: DownloadType::Patch,
        };

        // 提取器按相对 CWD 的 docker/ 工作；nextest 每测试独立进程，chdir 安全
        let cwd = std::env::current_dir()?;
        std::env::set_current_dir(directory.path())?;
        let extraction = crate::utils::extract_docker_service(&archive_path, &strategy).await;
        std::env::set_current_dir(cwd)?;
        extraction?;

        let merged = std::fs::read_to_string(docker_dir.join(".env"))?;
        assert!(
            merged.contains("MYSQL_PASSWORD=user-secret"),
            "用户值必须保留: {merged}"
        );
        assert!(merged.contains("PORT=8080"), "用户值必须保留: {merged}");
        assert!(
            merged.contains("NEW_REQUIRED=package-value"),
            "包内新键必须补齐: {merged}"
        );
        assert!(
            !merged.contains("package-default"),
            "包内占位值不得覆盖用户密钥: {merged}"
        );
        Ok(())
    }

    /// F03 回归：候选 .env 里 `${UNDEFINED}` 与 `KEY= # 行内注释` 都必须按
    /// 真实 Compose 语义解析为空 → 必填键缺失在停服前发现
    #[test]
    fn candidate_preflight_treats_undefined_interpolation_and_inline_comments_as_empty()
    -> Result<()> {
        let directory = tempfile::tempdir()?;
        let docker_dir = directory.path().join("docker");
        std::fs::create_dir_all(&docker_dir)?;

        let archive_path = directory.path().join("full.zip");
        let mut zip = zip::ZipWriter::new(File::create(&archive_path)?);
        zip.start_file(
            "docker-compose.yml",
            zip::write::SimpleFileOptions::default(),
        )?;
        zip.write_all(
            b"services:\n  backend:\n    environment:\n      - SECRET=${NEW_MUST_SET:?}\n",
        )?;
        // 包内 .env "定义"了该键，但值为未定义引用 / 纯行内注释——真实 Compose 均解析为空
        zip.start_file("docker/.env", zip::write::SimpleFileOptions::default())?;
        zip.write_all(b"NEW_MUST_SET=${UNDEFINED_FIXTURE_KEY}\n")?;
        zip.finish()?;

        let cwd = std::env::current_dir()?;
        std::env::set_current_dir(directory.path())?;
        let result =
            super::preflight_candidate_package_at(&archive_path, false, &docker_dir.join(".env"));
        std::env::set_current_dir(cwd)?;
        let error = result.expect_err("undefined interpolation must count as empty");
        assert!(
            error.to_string().contains("NEW_MUST_SET"),
            "错误必须指名键: {error}"
        );

        // 行内注释形态
        let mut zip = zip::ZipWriter::new(File::create(&archive_path)?);
        zip.start_file(
            "docker-compose.yml",
            zip::write::SimpleFileOptions::default(),
        )?;
        zip.write_all(
            b"services:\n  backend:\n    environment:\n      - SECRET=${NEW_MUST_SET:?}\n",
        )?;
        zip.start_file("docker/.env", zip::write::SimpleFileOptions::default())?;
        zip.write_all(b"NEW_MUST_SET= # fill before upgrade\n")?;
        zip.finish()?;
        let cwd = std::env::current_dir()?;
        std::env::set_current_dir(directory.path())?;
        let result =
            super::preflight_candidate_package_at(&archive_path, false, &docker_dir.join(".env"));
        std::env::set_current_dir(cwd)?;
        assert!(result.is_err(), "行内注释值必须解析为空并触发必填失败");
        Ok(())
    }

    /// F07 回归：patch 按 manifest 驱动强制更新全部关键引用文件——
    /// 即使 operations 不包含它们，最终磁盘也是目标版本；缺引用文件则硬失败
    #[tokio::test]
    async fn patch_extraction_updates_manifest_referenced_files_beyond_ops() -> Result<()> {
        use anyhow::Context as _;
        use client_core::api_types::{PatchOperations, PatchPackageInfo, ReplaceOperations};
        use client_core::upgrade_strategy::{DownloadType, UpgradeStrategy};

        let directory = tempfile::tempdir()?;
        let docker_dir = directory.path().join("docker");
        std::fs::create_dir_all(docker_dir.join("config"))?;
        // 磁盘上的旧版 schema/bootstrap（将被包内新版覆盖）
        std::fs::write(
            docker_dir.join("config/init_mysql.sql"),
            b"-- old platform schema\n",
        )?;
        std::fs::write(
            docker_dir.join("config/init_mysql_im.sql"),
            b"-- old im schema\n",
        )?;
        std::fs::write(
            docker_dir.join("config/init_mysql_databases.sql"),
            b"-- old bootstrap\n",
        )?;

        let manifest_json = br#"{"contract_version":1,"requires":{"cli_capability":"mysql-schema-manifest-v1"},
"mysql_target":{"service":"mysql","internal_port":3306},"application_connections":[],
"databases":[{"name":"agent_platform","bootstrap_only":false},{"name":"nuwax_im","bootstrap_only":false}],
"bootstrap":{"path":"config/init_mysql_databases.sql","idempotent":true,"initdb_target":"00_init_mysql_databases.sql"},
"permissions":{"path":"config/init_mysql_permissions.sh","user_env":"MYSQL_USER","databases":"bootstrap","initdb_target":"01_init_mysql_permissions.sh"},
"schemas":[
 {"database":"agent_platform","path":"config/init_mysql.sql","initdb_target":"10_init_mysql.sql"},
 {"database":"nuwax_im","path":"config/init_mysql_im.sql","initdb_target":"20_init_mysql_im.sql"}],
"first_install_seeds":[]}"#;
        let new_platform = b"USE agent_platform;\nCREATE TABLE `users_new` (`id` bigint NOT NULL, PRIMARY KEY (`id`));\n";
        let new_im = b"USE `nuwax_im`;\nCREATE TABLE `im_msg_new` (`id` bigint NOT NULL, PRIMARY KEY (`id`));\n";
        let new_bootstrap = b"CREATE DATABASE IF NOT EXISTS `agent_platform` CHARACTER SET utf8mb4 COLLATE utf8mb4_unicode_ci;\nCREATE DATABASE IF NOT EXISTS `nuwax_im` CHARACTER SET utf8mb4 COLLATE utf8mb4_0900_ai_ci;\n";
        let new_permissions = b"#!/bin/sh\nexit 0\n";

        let archive_path = directory.path().join("patch.zip");
        let mut zip = zip::ZipWriter::new(File::create(&archive_path)?);
        for (name, bytes) in [
            (
                "docker/config/mysql-schema-manifest.json",
                &manifest_json[..],
            ),
            ("docker/config/init_mysql_databases.sql", &new_bootstrap[..]),
            (
                "docker/config/init_mysql_permissions.sh",
                &new_permissions[..],
            ),
            ("docker/config/init_mysql.sql", &new_platform[..]),
            ("docker/config/init_mysql_im.sql", &new_im[..]),
            ("docker/config/app.conf", b"content\n"),
        ] {
            zip.start_file(name, zip::write::SimpleFileOptions::default())?;
            zip.write_all(bytes)?;
        }
        let compose = b"services: {}\n";
        zip.start_file(
            "docker/docker-compose.yml",
            zip::write::SimpleFileOptions::default(),
        )?;
        zip.write_all(compose)?;
        let delivery = test_delivery_manifest(
            compose,
            &[
                ("config/mysql-schema-manifest.json", &manifest_json[..]),
                ("config/init_mysql_databases.sql", &new_bootstrap[..]),
                ("config/init_mysql_permissions.sh", &new_permissions[..]),
                ("config/init_mysql.sql", &new_platform[..]),
                ("config/init_mysql_im.sql", &new_im[..]),
            ],
            &[],
        )?;
        zip.start_file(
            "docker/DELIVERY_MANIFEST.json",
            zip::write::SimpleFileOptions::default(),
        )?;
        zip.write_all(delivery.as_bytes())?;
        zip.finish()?;

        // ops 只声明 app.conf——schema/manifest 全不在变更清单里
        let strategy = UpgradeStrategy::PatchUpgrade {
            patch_info: PatchPackageInfo {
                url: String::new(),
                hash: None,
                signature: None,
                notes: None,
                operations: PatchOperations {
                    replace: Some(ReplaceOperations {
                        files: vec!["config/app.conf".to_string()],
                        directories: Vec::new(),
                    }),
                    delete: None,
                },
            },
            target_version: "0.0.91.0".parse().context("version")?,
            download_type: DownloadType::Patch,
        };

        let cwd = std::env::current_dir()?;
        std::env::set_current_dir(directory.path())?;
        let extraction = crate::utils::extract_docker_service(&archive_path, &strategy).await;
        std::env::set_current_dir(cwd)?;
        extraction.expect("patch extraction must succeed with manifest-driven criticals");

        // 未进 ops 的关键文件全部更新为目标版本
        assert_eq!(
            std::fs::read(docker_dir.join("config/init_mysql.sql"))?,
            &new_platform[..],
            "平台 schema 必须被强制更新"
        );
        assert_eq!(
            std::fs::read(docker_dir.join("config/init_mysql_im.sql"))?,
            &new_im[..],
            "IM schema 必须被强制更新"
        );
        assert_eq!(
            std::fs::read(docker_dir.join("config/init_mysql_databases.sql"))?,
            &new_bootstrap[..],
            "bootstrap 必须被强制更新"
        );
        assert!(
            std::fs::read_to_string(docker_dir.join("config/mysql-schema-manifest.json"))?
                .contains("mysql-schema-manifest-v1")
        );
        Ok(())
    }

    fn test_delivery_manifest(
        compose: &[u8],
        mysql_files: &[(&str, &[u8])],
        artifacts: &[(&str, &[u8])],
    ) -> Result<String> {
        let hash = client_core::device_info::fingerprint::sha256_hex;
        let files: std::collections::BTreeMap<_, _> = mysql_files
            .iter()
            .map(|(path, bytes)| (path.to_string(), hash(bytes)))
            .collect();
        let artifact_hashes: std::collections::BTreeMap<_, _> = artifacts
            .iter()
            .map(|(path, bytes)| (path.to_string(), hash(bytes)))
            .collect();
        let components = if artifacts.is_empty() {
            serde_json::json!({})
        } else {
            serde_json::json!({ "fixture": {
                "version": "fixture", "image_id": format!("sha256:{}", "1".repeat(64)), "config_id": format!("sha256:{}", "2".repeat(64)),
                "source": "registry.fixture/im:latest", "target": "registry.fixture/im:latest", "artifacts": artifact_hashes
            }})
        };
        let mut payload = serde_json::json!({"contract_version":1,"architecture":crate::docker_service::get_system_architecture().as_str(),"components":components,
            "mysql":{"manifest":"config/mysql-schema-manifest.json","files":files}, "compose":{"path":"docker-compose.yml","sha256":hash(compose)}});
        // serde_json's map is sorted by key, matching the producer's canonical form.
        let release = hash(serde_json::to_string(&payload)?.as_bytes());
        payload.as_object_mut().context("payload object")?.insert(
            "release_sha256".to_string(),
            serde_json::Value::String(release),
        );
        Ok(serde_json::to_string(&payload)?)
    }

    #[tokio::test]
    async fn patch_forces_delivery_and_compose_even_when_operations_omit_them() -> Result<()> {
        use anyhow::Context as _;
        use client_core::api_types::{PatchOperations, PatchPackageInfo, ReplaceOperations};
        use client_core::upgrade_strategy::{DownloadType, UpgradeStrategy};
        let directory = tempfile::tempdir()?;
        let docker = directory.path().join("docker");
        std::fs::create_dir_all(docker.join("config"))?;
        std::fs::write(
            docker.join("docker-compose.yml"),
            "services: {old: {image: fixture}}\n",
        )?;
        std::fs::write(docker.join("DELIVERY_MANIFEST.json"), "old release")?;
        let compose = b"services: {}\n";
        let platform = valid_platform_sql().as_bytes();
        let im = valid_im_sql().as_bytes();
        let delivery = test_delivery_manifest(
            compose,
            &[
                ("config/init_mysql.sql", platform),
                ("config/init_mysql_im.sql", im),
            ],
            &[],
        )?;
        let archive_path = directory.path().join("patch.zip");
        let mut zip = zip::ZipWriter::new(File::create(&archive_path)?);
        for (path, bytes) in [
            ("docker-compose.yml", &compose[..]),
            ("DELIVERY_MANIFEST.json", delivery.as_bytes()),
            ("config/init_mysql.sql", platform),
            ("config/init_mysql_im.sql", im),
        ] {
            zip.start_file(
                format!("docker/{path}"),
                zip::write::SimpleFileOptions::default(),
            )?;
            zip.write_all(bytes)?;
        }
        zip.finish()?;
        let strategy = UpgradeStrategy::PatchUpgrade {
            patch_info: PatchPackageInfo {
                url: String::new(),
                hash: None,
                signature: None,
                notes: None,
                operations: PatchOperations {
                    replace: Some(ReplaceOperations {
                        files: Vec::new(),
                        directories: Vec::new(),
                    }),
                    delete: None,
                },
            },
            target_version: "0.0.92.0".parse().context("version")?,
            download_type: DownloadType::Patch,
        };
        let cwd = std::env::current_dir()?;
        std::env::set_current_dir(directory.path())?;
        let result = crate::utils::extract_docker_service(&archive_path, &strategy).await;
        std::env::set_current_dir(cwd)?;
        result?;
        assert_eq!(std::fs::read(docker.join("docker-compose.yml"))?, compose);
        assert_eq!(
            std::fs::read_to_string(docker.join("DELIVERY_MANIFEST.json"))?,
            delivery
        );
        let manifest = client_core::container::preflight::parse_delivery_manifest(&delivery)?;
        client_core::container::preflight::verify_delivery_manifest(&manifest, &docker, None)?;
        Ok(())
    }

    #[tokio::test]
    async fn invalid_patch_manifest_fails_before_changing_disk() -> Result<()> {
        use anyhow::Context as _;
        use client_core::api_types::{PatchOperations, PatchPackageInfo, ReplaceOperations};
        use client_core::upgrade_strategy::{DownloadType, UpgradeStrategy};
        let directory = tempfile::tempdir()?;
        let docker = directory.path().join("docker");
        std::fs::create_dir_all(docker.join("config"))?;
        std::fs::write(docker.join("config/app.conf"), "old")?;
        let archive_path = directory.path().join("patch.zip");
        let mut zip = zip::ZipWriter::new(File::create(&archive_path)?);
        for (path, bytes) in [
            (
                "config/mysql-schema-manifest.json",
                &b"invalid manifest"[..],
            ),
            ("config/app.conf", &b"new"[..]),
            ("config/init_mysql.sql", valid_platform_sql().as_bytes()),
            ("config/init_mysql_im.sql", valid_im_sql().as_bytes()),
        ] {
            zip.start_file(
                format!("docker/{path}"),
                zip::write::SimpleFileOptions::default(),
            )?;
            zip.write_all(bytes)?;
        }
        zip.finish()?;
        let strategy = UpgradeStrategy::PatchUpgrade {
            patch_info: PatchPackageInfo {
                url: String::new(),
                hash: None,
                signature: None,
                notes: None,
                operations: PatchOperations {
                    replace: Some(ReplaceOperations {
                        files: vec!["config/app.conf".to_string()],
                        directories: Vec::new(),
                    }),
                    delete: None,
                },
            },
            target_version: "0.0.92.0".parse().context("version")?,
            download_type: DownloadType::Patch,
        };
        let cwd = std::env::current_dir()?;
        std::env::set_current_dir(directory.path())?;
        let result = crate::utils::extract_docker_service(&archive_path, &strategy).await;
        std::env::set_current_dir(cwd)?;
        assert!(result.is_err());
        assert_eq!(
            std::fs::read_to_string(docker.join("config/app.conf"))?,
            "old"
        );
        Ok(())
    }

    /// P1#3 回归：候选包预检在缺新必填键时失败（旧服务无需停止即可发现）
    #[test]
    fn candidate_package_preflight_rejects_missing_required_keys() -> Result<()> {
        let directory = tempfile::tempdir()?;
        let docker_dir = directory.path().join("docker");
        std::fs::create_dir_all(&docker_dir)?;
        std::fs::write(docker_dir.join(".env"), "EXISTING=1\n")?;

        let archive_path = directory.path().join("full.zip");
        let mut zip = zip::ZipWriter::new(File::create(&archive_path)?);
        zip.start_file(
            "docker-compose.yml",
            zip::write::SimpleFileOptions::default(),
        )?;
        zip.write_all(
            b"services:\n  backend:\n    environment:\n      - SECRET=${NEW_MUST_SET:?}\n",
        )?;
        zip.start_file("docker/.env", zip::write::SimpleFileOptions::default())?;
        zip.write_all(b"OTHER=2\n")?;
        zip.finish()?;

        let cwd = std::env::current_dir()?;
        std::env::set_current_dir(directory.path())?;
        let result =
            super::preflight_candidate_package_at(&archive_path, false, &docker_dir.join(".env"));
        std::env::set_current_dir(cwd)?;
        let error = result.expect_err("missing required key must fail the preflight");
        assert!(
            error.to_string().contains("NEW_MUST_SET"),
            "错误必须指名缺失键: {error}"
        );
        Ok(())
    }

    #[test]
    fn offline_archive_requires_all_schema_templates() -> Result<()> {
        let directory = tempfile::tempdir()?;
        let archive = directory.path().join("bundle.tar.gz");

        // 空包：缺少全部模板
        write_archive(&archive, None, None)?;
        assert!(validate_offline_archive_sql(&archive).is_err());

        // 缺 im 模板（对应"新 CLI + 旧包"组合，Fail Fast）
        write_archive(&archive, Some(valid_platform_sql()), None)?;
        assert!(validate_offline_archive_sql(&archive).is_err());

        // 模板缺 USE（无法归属库）
        write_archive(
            &archive,
            Some("CREATE TABLE users (id INT);"),
            Some(valid_im_sql()),
        )?;
        assert!(validate_offline_archive_sql(&archive).is_err());

        // 两模板齐全且合法
        write_archive(&archive, Some(valid_platform_sql()), Some(valid_im_sql()))?;
        validate_offline_archive_sql(&archive)?;
        Ok(())
    }

    #[test]
    fn offline_manifest_uses_declared_schema_paths_and_checks_database_identity() -> Result<()> {
        let directory = tempfile::tempdir()?;
        let archive_path = directory.path().join("bundle.zip");
        let manifest = serde_json::json!({
            "contract_version":1,"requires":{"cli_capability":"mysql-schema-manifest-v1"},
            "mysql_target":{"service":"mysql","internal_port":3306},"application_connections":[],
            "databases":[{"name":"custom_app","bootstrap_only":false}],
            "bootstrap":{"path":"config/bootstrap.sql","idempotent":true,"initdb_target":"00_bootstrap.sql"},
            "permissions":{"path":"config/permissions.sh","user_env":"MYSQL_USER","databases":"bootstrap","initdb_target":"01_permissions.sh"},
            "schemas":[{"database":"custom_app","path":"config/custom-schema.sql","initdb_target":"10_custom.sql"}],
            "first_install_seeds":[]
        });
        let write = |database: &str| -> Result<()> {
            let mut archive = zip::ZipWriter::new(File::create(&archive_path)?);
            archive.start_file(
                "docker/config/mysql-schema-manifest.json",
                zip::write::SimpleFileOptions::default(),
            )?;
            archive.write_all(serde_json::to_string(&manifest)?.as_bytes())?;
            archive.start_file(
                "docker/config/custom-schema.sql",
                zip::write::SimpleFileOptions::default(),
            )?;
            archive.write_all(
                format!("USE {database};\nCREATE TABLE example (id INT);\n").as_bytes(),
            )?;
            archive.finish()?;
            Ok(())
        };
        write("custom_app")?;
        validate_offline_archive_sql(&archive_path)?;
        write("other_app")?;
        assert!(validate_offline_archive_sql(&archive_path).is_err());
        Ok(())
    }

    #[test]
    fn offline_zip_archive_accepts_target_sql() -> Result<()> {
        let directory = tempfile::tempdir()?;
        let archive = directory.path().join("bundle.zip");
        let mut zip = zip::ZipWriter::new(File::create(&archive)?);
        for (name, sql) in [
            ("docker/config/init_mysql.sql", valid_platform_sql()),
            ("docker/config/init_mysql_im.sql", valid_im_sql()),
        ] {
            zip.start_file(name, zip::write::SimpleFileOptions::default())?;
            zip.write_all(sql.as_bytes())?;
        }
        zip.finish()?;
        validate_offline_archive_sql(&archive)?;

        // 归一化后同名的重复模板必须被拒绝（带/不带 docker/ 前缀）
        let duplicated = directory.path().join("duplicate.zip");
        let mut zip = zip::ZipWriter::new(File::create(&duplicated)?);
        for name in ["docker/config/init_mysql.sql", "config/init_mysql.sql"] {
            zip.start_file(name, zip::write::SimpleFileOptions::default())?;
            zip.write_all(valid_platform_sql().as_bytes())?;
        }
        zip.start_file(
            "config/init_mysql_im.sql",
            zip::write::SimpleFileOptions::default(),
        )?;
        zip.write_all(valid_im_sql().as_bytes())?;
        zip.finish()?;
        assert!(validate_offline_archive_sql(&duplicated).is_err());
        Ok(())
    }

    #[test]
    fn offline_upgrade_preserves_existing_values_and_adds_package_env_defaults() -> Result<()> {
        let directory = tempfile::tempdir()?;
        let backup = directory.path().join("docker.previous");
        let docker = directory.path().join("docker");
        std::fs::create_dir_all(&backup)?;
        std::fs::create_dir_all(&docker)?;
        std::fs::write(
            backup.join(".env"),
            "# operator config\nMYSQL_PASSWORD=existing-test-value\nFRONTEND_HOST_PORT=8091",
        )?;
        std::fs::write(
            docker.join(".env"),
            "MYSQL_PASSWORD=package-default\nSURREALDB_USER=package-user\nSURREALDB_PASSWORD=package-password\n",
        )?;

        restore_preserved_docker_dirs(&backup, &docker)?;

        let merged = std::fs::read_to_string(docker.join(".env"))?;
        assert!(merged.contains("# operator config\n"));
        assert!(merged.contains("MYSQL_PASSWORD=existing-test-value\n"));
        assert!(merged.contains("FRONTEND_HOST_PORT=8091\n"));
        assert!(merged.contains("SURREALDB_USER=package-user\n"));
        assert!(merged.contains("SURREALDB_PASSWORD=package-password\n"));
        assert!(!merged.contains("MYSQL_PASSWORD=package-default"));
        assert!(!backup.join(".env").exists());
        Ok(())
    }

    #[test]
    fn offline_upgrade_preserves_existing_service_logs() -> Result<()> {
        let directory = tempfile::tempdir()?;
        let backup = directory.path().join("docker.previous");
        let docker = directory.path().join("docker");
        let old_logs = backup.join("logs/rcoder/project_logs/SYSTEM");
        let package_logs = docker.join("logs/rcoder");
        std::fs::create_dir_all(&old_logs)?;
        std::fs::create_dir_all(&package_logs)?;
        std::fs::write(old_logs.join("api.log"), "existing service log\n")?;
        std::fs::write(
            package_logs.join("package-placeholder.log"),
            "package file\n",
        )?;

        restore_preserved_docker_dirs(&backup, &docker)?;

        assert_eq!(
            std::fs::read_to_string(docker.join("logs/rcoder/project_logs/SYSTEM/api.log"))?,
            "existing service log\n"
        );
        assert!(!docker.join("logs/rcoder/package-placeholder.log").exists());
        assert!(!backup.join("logs").exists());
        Ok(())
    }
}
