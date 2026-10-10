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
use std::time::Duration;
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

/// All selected environment aliases must resolve before stopping services.
/// The shared transaction protects both the alias and its target in place.
fn validate_offline_env_aliases(env_path: &Path, _package_root: &Path) -> Result<()> {
    let selected = absolute_deployment_path(env_path)?;
    for ancestor in selected.ancestors() {
        match fs::symlink_metadata(ancestor) {
            Ok(metadata) if metadata.file_type().is_symlink() => {
                fs::canonicalize(ancestor).with_context(|| {
                    format!(
                        "Offline environment alias must resolve before stopping services: {}",
                        ancestor.display(),
                    )
                })?;
            }
            Ok(_) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => {
                return Err(error).context("Failed to inspect the selected environment path");
            }
        }
    }
    Ok(())
}

#[cfg(test)]
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
        None => {
            let mut names = vec!["docker-compose.yml"];
            names.extend(sql::OPTIONAL_SCHEMA_SQL_FILES.iter().copied());
            let entries = crate::utils::read_archive_entries(archive_path, &names)?;
            let compose = entries
                .get("docker-compose.yml")
                .map(|bytes| std::str::from_utf8(bytes))
                .transpose()
                .context("Offline Compose file is not valid UTF-8")?;
            crate::utils::legacy_schema::schema_paths(compose, |path| entries.contains_key(path))?
                .into_iter()
                .map(PathBuf::from)
                .collect()
        }
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
            Ok(Err(error)) => {
                last_error = crate::format_error_with_env(&anyhow::Error::new(error), env_path)
            }
            Err(_) => last_error = "connection attempt timed out".to_string(),
        }
        debug!(error = %last_error, "Waiting for MySQL to accept SQL connections");
        let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
        sleep(remaining.min(Duration::from_secs(2))).await;
    }
}

struct StagedDeployment<'a> {
    had_existing_compose: bool,
    target_version: &'a str,
    database_starting: bool,
    replacement: Option<&'a mut crate::utils::package_replace::PackageReplacement>,
}

async fn run_staged_deployment(
    app: &mut CliApp,
    frontend_port: Option<u16>,
    config_file: Option<PathBuf>,
    project_name: Option<String>,
    context: &DeploymentContext,
    staged: &mut StagedDeployment<'_>,
) -> Result<()> {
    // 部署前校验迁移计划：manifest v1（解析+结构+文件校验）或 legacy 固定清单
    // （缺文件/解析失败即 Fail Fast）；顺带收集库名供配置预检校验应用连接目标
    let migration_plan = resolve_migration_plan(&context.package_root)?;
    let (manifest, template_databases) = match &migration_plan {
        MigrationPlan::Manifest(manifest) => (Some(manifest), manifest.database_names()),
        MigrationPlan::Legacy => (
            None,
            crate::utils::legacy_schema::disk_templates(
                &context.package_root,
                context.manager.get_compose_file(),
            )?
            .into_iter()
            .map(|template| template.database)
            .collect(),
        ),
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

    if staged.had_existing_compose {
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
    // MySQL initdb can alter data before Live Diff; from this boundary onward
    // retain package recovery files instead of automatically rolling files back.
    if let Some(replacement) = staged.replacement.as_deref_mut() {
        replacement.mark_database_starting()?;
    }
    staged.database_starting = true;
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
    update_config_version(&mut app.config, &app_config_path, staged.target_version)?;
    info!("✅ Deployment completed after MySQL migration and service health checks");
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
    preflight_candidate_package_with_compose_at(archive_path, is_patch, user_env_path, None, &[])
}

fn preflight_candidate_package_with_compose_at(
    archive_path: &Path,
    is_patch: bool,
    user_env_path: &Path,
    current_compose: Option<&Path>,
    changed_paths: &[String],
) -> Result<()> {
    crate::utils::legacy_schema::validate_changed_paths(changed_paths)?;
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

    let legacy_patch = is_patch
        && !entries.contains_key("config/mysql-schema-manifest.json")
        && !entries.contains_key("DELIVERY_MANIFEST.json");
    let compose_text = match entries.get("docker-compose.yml") {
        Some(bytes) => String::from_utf8(bytes.clone())
            .context("candidate package docker-compose.yml is not valid UTF-8")?,
        None if legacy_patch => {
            if !crate::utils::legacy_schema::can_retain("docker-compose.yml", changed_paths) {
                anyhow::bail!("Legacy patch changes Compose but does not include its replacement");
            }
            let path = current_compose.context(
                "Legacy patch without Compose requires the selected current Compose file",
            )?;
            fs::read_to_string(path)
                .with_context(|| format!("Failed to read current Compose {}", path.display()))?
        }
        None => anyhow::bail!("candidate package has no docker-compose.yml"),
    };

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
            let optional =
                crate::utils::read_archive_entries(archive_path, sql::OPTIONAL_SCHEMA_SQL_FILES)?;
            let paths = crate::utils::legacy_schema::schema_paths(Some(&compose_text), |path| {
                optional.contains_key(path)
                    || (legacy_patch
                        && current_compose
                            .and_then(Path::parent)
                            .is_some_and(|root| root.join(path).exists()))
            })?;
            let names: Vec<&str> = paths.iter().map(String::as_str).collect();
            let schemas = crate::utils::read_archive_entries(archive_path, &names)?;
            // A legacy patch may retain Compose, but never retain a stale schema.
            let templates = crate::utils::legacy_schema::parse_templates(&paths, |path| {
                let bytes = schemas.get(path).ok_or_else(|| {
                    anyhow::anyhow!("candidate package is missing required schema file {path}")
                })?;
                String::from_utf8(bytes.clone()).map_err(Into::into)
            })?;
            let databases = templates
                .into_iter()
                .map(|template| template.database)
                .collect::<Vec<_>>();
            client_core::container::preflight::validate_local_db_targets(&values, &databases)?;
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
            let retained = legacy_patch
                && crate::utils::legacy_schema::can_retain(source, changed_paths)
                && current_compose
                    .and_then(Path::parent)
                    .is_some_and(|root| root.join(source).exists());
            if !present && !retained {
                return Err(anyhow::anyhow!(
                    "candidate compose mounts '{source}' but the candidate deployment does not contain it"
                ));
            }
        }
    }

    if legacy_patch {
        // Directory presence alone is insufficient: operations may remove a
        // required child while leaving siblings available in the patch.
        let required = crate::utils::legacy_schema::component_entrypoints(&compose_text)?;
        let entries = crate::utils::read_archive_entries(archive_path, &required)?;
        for path in required {
            if entries.get(path).is_some_and(Vec::is_empty) {
                anyhow::bail!("Legacy patch supplies an empty component file {path}");
            }
            let supplied = entries.contains_key(path);
            let retained = crate::utils::legacy_schema::can_retain(path, changed_paths)
                && current_compose.and_then(Path::parent).is_some_and(|root| {
                    fs::metadata(root.join(path))
                        .is_ok_and(|metadata| metadata.is_file() && metadata.len() > 0)
                });
            if !supplied && !retained {
                anyhow::bail!("Legacy patch candidate is missing required component file {path}");
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
    current_compose: &Path,
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
    let changed_paths = match strategy {
        UpgradeStrategy::PatchUpgrade { patch_info, .. } => patch_info.get_changed_files(),
        _ => Vec::new(),
    };
    let is_patch = matches!(strategy, UpgradeStrategy::PatchUpgrade { .. });
    preflight_candidate_package_with_compose_at(
        &path,
        is_patch,
        user_env_path,
        Some(current_compose),
        &changed_paths,
    )
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
        MigrationPlan::Legacy => (
            None,
            crate::utils::legacy_schema::disk_templates(docker_root, compose_path)?
                .into_iter()
                .map(|template| template.database)
                .collect(),
        ),
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

/// Immutable in-memory snapshot of the selected operator environment.
/// Capture before stopping services; apply package defaults or restore the
/// original contents atomically to the same file, without secrets backup files.
#[cfg(test)]
struct OnlineEnvPreserve {
    env_path: PathBuf,
    preserved: Option<String>,
    permissions: Option<fs::Permissions>,
}

#[cfg(test)]
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

#[cfg(test)]
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

/// Record the version of the package being deployed, rather than the version
/// advertised by a server that may be behind the installed deployment.
fn deployment_target_version(strategy: &UpgradeStrategy, deployed_version: &str) -> String {
    match strategy {
        UpgradeStrategy::FullUpgrade { target_version, .. }
        | UpgradeStrategy::PatchUpgrade { target_version, .. } => target_version.to_string(),
        UpgradeStrategy::NoUpgrade { .. } => deployed_version.to_owned(),
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

    // 下载策略只读取一次 manifest；替包时记录策略目标，无替包时保留当前部署版本。
    let upgrade_args = crate::cli::UpgradeArgs {
        force: false,
        check: false,
    };
    let upgrade_strategy = update::run_upgrade(app, upgrade_args).await?;
    let target_version =
        deployment_target_version(&upgrade_strategy, &app.config.get_docker_versions());

    let had_existing_compose = docker_manager.get_compose_file().is_file();
    preflight_current_config(
        docker_manager.get_compose_file(),
        docker_manager.get_env_file(),
        &context.package_root,
    )?;
    // Initial installations also validate the candidate before extraction.
    preflight_candidate_package(
        app,
        &upgrade_strategy,
        docker_manager.get_env_file(),
        docker_manager.get_compose_file(),
    )
    .context("Candidate package preflight failed before stopping services")?;
    let package_path = docker_service::package_path_for_strategy(app, &upgrade_strategy)?;
    // Stage and probe affected managed directories before stopping services.
    // Neither the docker root nor any existing persistent tree is moved.
    let mut replacement = match package_path.as_deref() {
        Some(path) => Some(
            crate::utils::package_replace::PackageReplacement::prepare_async(
                path,
                &upgrade_strategy,
                &context.package_root,
                docker_manager.get_env_file(),
                true,
            )
            .await
            .context("Package replacement preflight failed before stopping services")?,
        ),
        None => None,
    };
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
                error = crate::format_error_with_env(&e, docker_manager.get_env_file())
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

    if let Some(replacement) = replacement.as_mut() {
        replacement.apply()?;
        if let Err(error) = fix_script_permissions().await {
            return replacement.recover_error(error);
        }
    }
    let mut staged = StagedDeployment {
        had_existing_compose,
        target_version: &target_version,
        database_starting: false,
        replacement: replacement.as_mut(),
    };
    let deployment = run_staged_deployment(
        app,
        frontend_port,
        config_file,
        project_name,
        &context,
        &mut staged,
    )
    .await;
    let database_starting = staged.database_starting;
    finish_package_deployment(replacement, deployment, database_starting)
}

/// Recover only managed release files before MySQL starts. From MySQL startup
/// (which includes initdb) onward, never restore database files or switch release
/// files underneath a running database automatically.
fn finish_package_deployment(
    mut replacement: Option<crate::utils::package_replace::PackageReplacement>,
    deployment: Result<()>,
    database_starting: bool,
) -> Result<()> {
    match deployment {
        Ok(()) => match replacement {
            Some(replacement) => replacement.finish(),
            None => Ok(()),
        },
        Err(error) => {
            if let Some(replacement) = replacement.as_mut() {
                if !database_starting {
                    return replacement.recover_error(error);
                }
                let recovery = replacement.retain_for_recovery();
                return Err(error.context(format!(
                    "Deployment failed after MySQL startup; managed package recovery files retained at {}; persistent data was not rolled back and deployment completion was not confirmed",
                    recovery.display(),
                )));
            }
            Err(error)
        }
    }
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
    let compose = fs::read_to_string(package_root.join("docker-compose.yml"))
        .context("Failed to read legacy Compose for schema selection")?;
    let paths = crate::utils::legacy_schema::schema_paths(Some(&compose), |path| {
        package_root.join(path).exists()
    })?;
    let templates = crate::utils::legacy_schema::parse_templates(&paths, |path| {
        fs::read_to_string(package_root.join(path)).map_err(Into::into)
    })?;
    for (template_path, template) in paths.iter().zip(&templates) {
        let source_path = package_root.join(template_path);
        let new_sql_path = temp_sql_dir.join(format!("{}_new.sql", template.database));
        if new_sql_path.exists() {
            fs::remove_file(&new_sql_path)?;
        }
        fs::copy(&source_path, &new_sql_path).context(t!(
            "auto_upgrade_deploy.copy_sql_failed",
            src = source_path.display(),
            dst = new_sql_path.display()
        ))?;
        info!(database = %template.database, tables = template.tables.len(),
            path = %new_sql_path.display(), "📄 Copied schema template for database");
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
    let upgrade_strategy = UpgradeStrategy::FullUpgrade {
        url: String::new(),
        hash: String::new(),
        signature: String::new(),
        target_version: version.clone(),
        download_type: client_core::upgrade_strategy::DownloadType::Full,
    };
    let mut replacement = crate::utils::package_replace::PackageReplacement::prepare_async(
        &archive_path,
        &upgrade_strategy,
        &context.package_root,
        docker_manager.get_env_file(),
        true,
    )
    .await
    .context("Offline package replacement preflight failed before stopping services")?;
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
                error = crate::format_error_with_env(&e, docker_manager.get_env_file())
            );
        }
    }

    replacement.apply()?;
    if let Err(error) = fix_script_permissions().await {
        return replacement.recover_error(error);
    }
    let target_version = version.to_string();
    let mut staged = StagedDeployment {
        had_existing_compose,
        target_version: &target_version,
        database_starting: false,
        replacement: Some(&mut replacement),
    };
    let deployment = run_staged_deployment(
        app,
        frontend_port,
        config_file,
        project_name,
        &context,
        &mut staged,
    )
    .await;
    let database_starting = staged.database_starting;
    finish_package_deployment(Some(replacement), deployment, database_starting)
}

#[cfg(test)]
mod tests {
    use super::{OnlineEnvPreserve, UpgradeStrategy, validate_offline_archive_sql};
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
    fn offline_accepts_external_leaf_and_parent_aliases_into_preserved_package() -> Result<()> {
        use std::os::unix::fs::symlink;
        let directory = tempfile::tempdir()?;
        let package = directory.path().join("docker");
        std::fs::create_dir_all(package.join("secrets"))?;
        let target = package.join("secrets/operator.env");
        std::fs::write(&target, "OPERATOR=fixture\n")?;
        let leaf = directory.path().join("external.env");
        symlink(&target, &leaf)?;
        super::validate_offline_env_aliases(&leaf, &package)?;

        let parent = directory.path().join("external-secrets");
        symlink(package.join("secrets"), &parent)?;
        super::validate_offline_env_aliases(&parent.join("operator.env"), &package)?;
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
        super::validate_offline_env_aliases(&inside_alias, &package)?;

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
    fn no_upgrade_with_older_manifest_keeps_deployed_version() -> Result<()> {
        let strategy = UpgradeStrategy::NoUpgrade {
            target_version: "9.71.1.0".parse()?,
        };
        assert_eq!(
            super::deployment_target_version(&strategy, "9.71.2.2"),
            "9.71.2.2"
        );
        Ok(())
    }

    #[test]
    fn no_upgrade_with_equal_manifest_preserves_version_string() -> Result<()> {
        let strategy = UpgradeStrategy::NoUpgrade {
            target_version: "9.71.2.0".parse()?,
        };
        // Config versions are operator-owned strings; do not normalize a
        // three-part version when redeploying the unchanged package.
        assert_eq!(
            super::deployment_target_version(&strategy, "9.71.2"),
            "9.71.2"
        );
        Ok(())
    }

    #[test]
    fn full_upgrade_records_selected_package_version() -> Result<()> {
        let strategy = UpgradeStrategy::FullUpgrade {
            url: String::new(),
            hash: String::new(),
            signature: String::new(),
            target_version: "9.71.3.0".parse()?,
            download_type: client_core::upgrade_strategy::DownloadType::Full,
        };
        assert_eq!(
            super::deployment_target_version(&strategy, "9.71.2.2"),
            "9.71.3.0"
        );
        Ok(())
    }

    #[test]
    fn patch_upgrade_records_selected_package_version() -> Result<()> {
        use client_core::api_types::{PatchOperations, PatchPackageInfo};
        let strategy = UpgradeStrategy::PatchUpgrade {
            patch_info: PatchPackageInfo {
                url: String::new(),
                hash: None,
                signature: None,
                notes: None,
                operations: PatchOperations {
                    replace: None,
                    delete: None,
                },
            },
            target_version: "9.71.2.3".parse()?,
            download_type: client_core::upgrade_strategy::DownloadType::Patch,
        };
        assert_eq!(
            super::deployment_target_version(&strategy, "9.71.2.2"),
            "9.71.2.3"
        );
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
    fn offline_legacy_archive_requires_platform_and_validates_present_im() -> Result<()> {
        let directory = tempfile::tempdir()?;
        let archive = directory.path().join("bundle.tar.gz");

        // 空包：缺少全部模板
        write_archive(&archive, None, None)?;
        assert!(validate_offline_archive_sql(&archive).is_err());

        // Legacy single-database releases remain supported by the new CLI.
        write_archive(&archive, Some(valid_platform_sql()), None)?;
        validate_offline_archive_sql(&archive)?;

        // Optional templates, when present, must still be valid.
        write_archive(
            &archive,
            Some(valid_platform_sql()),
            Some("CREATE TABLE im_users (id INT);"),
        )?;
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

    fn write_legacy_zip(path: &Path, entries: &[(&str, &str)]) -> Result<()> {
        let mut archive = zip::ZipWriter::new(File::create(path)?);
        for (name, content) in entries {
            archive.start_file(*name, zip::write::SimpleFileOptions::default())?;
            archive.write_all(content.as_bytes())?;
        }
        archive.finish()?;
        Ok(())
    }

    fn legacy_compose() -> &'static str {
        "services:\n  mysql:\n    image: mysql:8.0\n    volumes:\n      - ./config/init_mysql.sql:/docker-entrypoint-initdb.d/10_init_mysql.sql:ro\n      - ./config/init_mysql_data.sql:/docker-entrypoint-initdb.d/30_data.sql:ro\n"
    }

    #[test]
    fn legacy_full_zip_accepts_single_database_and_checks_mounted_extra_schema() -> Result<()> {
        let directory = tempfile::tempdir()?;
        let archive = directory.path().join("full.zip");
        let env = directory.path().join("operator.env");
        write_legacy_zip(
            &archive,
            &[
                ("docker/docker-compose.yml", legacy_compose()),
                ("docker/config/init_mysql.sql", valid_platform_sql()),
                (
                    "docker/config/init_mysql_data.sql",
                    "INSERT INTO users VALUES (1);",
                ),
            ],
        )?;
        validate_offline_archive_sql(&archive)?;
        super::preflight_candidate_package_at(&archive, false, &env)?;
        let aliased =
            legacy_compose().replace("./config/init_mysql.sql", "./config/./init_mysql.sql");
        write_legacy_zip(
            &archive,
            &[
                ("docker/docker-compose.yml", &aliased),
                ("docker/config/init_mysql.sql", valid_platform_sql()),
            ],
        )?;
        super::preflight_candidate_package_at(&archive, false, &env)?;

        let im_compose = format!(
            "{}      - ./config/init_mysql_im.sql:/docker-entrypoint-initdb.d/20_im.sql:ro\n",
            legacy_compose()
        );
        write_legacy_zip(
            &archive,
            &[
                ("docker/docker-compose.yml", &im_compose),
                ("docker/config/init_mysql.sql", valid_platform_sql()),
            ],
        )?;
        assert!(validate_offline_archive_sql(&archive).is_err());
        assert!(super::preflight_candidate_package_at(&archive, false, &env).is_err());
        Ok(())
    }

    #[test]
    fn legacy_no_upgrade_preflight_accepts_single_database_and_rejects_bad_or_missing_im()
    -> Result<()> {
        let directory = tempfile::tempdir()?;
        let root = directory.path().join("docker");
        std::fs::create_dir_all(root.join("config"))?;
        let compose = root.join("docker-compose.yml");
        let env = root.join(".env");
        std::fs::write(&compose, legacy_compose())?;
        std::fs::write(&env, "MYSQL_DATABASE=agent_platform\n")?;
        std::fs::write(root.join("config/init_mysql.sql"), valid_platform_sql())?;
        super::preflight_current_config(&compose, &env, &root)?;
        std::fs::write(
            root.join("config/init_mysql_im.sql"),
            "CREATE TABLE im_users (id INT);",
        )?;
        assert!(super::preflight_current_config(&compose, &env, &root).is_err());
        std::fs::remove_file(root.join("config/init_mysql_im.sql"))?;
        std::fs::write(
            &compose,
            format!(
                "{}      - ./config/init_mysql_im.sql:/docker-entrypoint-initdb.d/20_im.sql:ro\n",
                legacy_compose()
            ),
        )?;
        assert!(super::preflight_current_config(&compose, &env, &root).is_err());
        std::fs::write(root.join("config/init_mysql_im.sql"), valid_im_sql())?;
        super::preflight_current_config(&compose, &env, &root)?;
        Ok(())
    }

    #[tokio::test]
    async fn legacy_patch_reuses_current_compose_and_force_updates_single_schema() -> Result<()> {
        use client_core::api_types::{PatchOperations, PatchPackageInfo};
        use client_core::upgrade_strategy::{DownloadType, UpgradeStrategy};
        let directory = tempfile::tempdir()?;
        let root = directory.path().join("docker");
        std::fs::create_dir_all(root.join("config"))?;
        let compose = root.join("docker-compose.yml");
        let env = directory.path().join("secrets/operator.env");
        std::fs::create_dir_all(env.parent().context("env parent")?)?;
        std::fs::write(&env, "MYSQL_DATABASE=agent_platform\n")?;
        std::fs::write(&compose, legacy_compose())?;
        std::fs::write(root.join("config/init_mysql.sql"), "old schema")?;
        let archive = directory.path().join("patch.zip");
        write_legacy_zip(
            &archive,
            &[("docker/config/init_mysql.sql", valid_platform_sql())],
        )?;
        super::preflight_candidate_package_with_compose_at(
            &archive,
            true,
            &env,
            Some(&compose),
            &[],
        )?;
        let strategy = UpgradeStrategy::PatchUpgrade {
            patch_info: PatchPackageInfo {
                url: String::new(),
                hash: None,
                signature: None,
                notes: None,
                operations: PatchOperations {
                    replace: None,
                    delete: None,
                },
            },
            target_version: "0.0.90.1".parse()?,
            download_type: DownloadType::Patch,
        };
        let cwd = std::env::current_dir()?;
        std::env::set_current_dir(directory.path())?;
        let extracted =
            crate::utils::extract_docker_service_with_env(&archive, &strategy, &env).await;
        std::env::set_current_dir(cwd)?;
        extracted?;
        assert_eq!(std::fs::read_to_string(&compose)?, legacy_compose());
        assert_eq!(
            std::fs::read_to_string(root.join("config/init_mysql.sql"))?,
            valid_platform_sql()
        );
        assert!(
            super::preflight_candidate_package_with_compose_at(
                &archive,
                true,
                &env,
                Some(&compose),
                &["docker-compose.yml".to_string()],
            )
            .is_err(),
            "a patch deleting/replacing Compose cannot retain its old contents"
        );
        Ok(())
    }

    #[test]
    fn legacy_patch_rejects_operations_removing_retained_component_entrypoints() -> Result<()> {
        let directory = tempfile::tempdir()?;
        let root = directory.path().join("docker");
        std::fs::create_dir_all(root.join("im-app"))?;
        let compose = root.join("docker-compose.yml");
        let env = directory.path().join("operator.env");
        let im_compose = format!(
            "{}  im:\n    image: im\n    volumes:\n      - ./im-app:/app:ro\n",
            legacy_compose()
        );
        std::fs::write(&compose, im_compose)?;
        for name in [
            "nuwax-im-web-bootstrap.jar",
            "nuwax-im-gateway-bootstrap.jar",
        ] {
            std::fs::write(root.join("im-app").join(name), "jar bytes")?;
        }
        let archive = directory.path().join("patch.zip");
        write_legacy_zip(
            &archive,
            &[("docker/config/init_mysql.sql", valid_platform_sql())],
        )?;
        super::preflight_candidate_package_with_compose_at(
            &archive,
            true,
            &env,
            Some(&compose),
            &[],
        )?;
        for changed in ["im-app", "im-app/nuwax-im-web-bootstrap.jar"] {
            assert!(
                super::preflight_candidate_package_with_compose_at(
                    &archive,
                    true,
                    &env,
                    Some(&compose),
                    &[changed.to_string()],
                )
                .is_err()
            );
        }
        // A sibling entry cannot hide removal of a required jar in a directory mount.
        write_legacy_zip(
            &archive,
            &[
                ("docker/config/init_mysql.sql", valid_platform_sql()),
                ("docker/im-app/readme.txt", "patched readme"),
            ],
        )?;
        assert!(
            super::preflight_candidate_package_with_compose_at(
                &archive,
                true,
                &env,
                Some(&compose),
                &["im-app/nuwax-im-web-bootstrap.jar".to_string()],
            )
            .is_err()
        );
        assert_eq!(
            std::fs::read_to_string(root.join("im-app/nuwax-im-web-bootstrap.jar"))?,
            "jar bytes"
        );
        write_legacy_zip(
            &archive,
            &[
                ("docker/config/init_mysql.sql", valid_platform_sql()),
                ("docker/im-app/nuwax-im-web-bootstrap.jar", ""),
            ],
        )?;
        assert!(
            super::preflight_candidate_package_with_compose_at(
                &archive,
                true,
                &env,
                Some(&compose),
                &[],
            )
            .is_err(),
            "an explicitly empty replacement cannot fall back to the old jar"
        );
        Ok(())
    }

    #[tokio::test]
    async fn legacy_patch_force_restores_supplied_binary_jar_deleted_without_replace() -> Result<()>
    {
        use client_core::api_types::{PatchOperations, PatchPackageInfo, ReplaceOperations};
        use client_core::upgrade_strategy::{DownloadType, UpgradeStrategy};
        let directory = tempfile::tempdir()?;
        let root = directory.path().join("docker");
        std::fs::create_dir_all(root.join("config"))?;
        std::fs::create_dir_all(root.join("im-app"))?;
        let compose = root.join("docker-compose.yml");
        let env = root.join(".env");
        std::fs::write(
            &compose,
            format!(
                "{}  im:\n    image: im\n    volumes:\n      - ./im-app:/app:ro\n",
                legacy_compose()
            ),
        )?;
        std::fs::write(&env, "MYSQL_DATABASE=agent_platform\n")?;
        std::fs::write(root.join("config/init_mysql.sql"), valid_platform_sql())?;
        let web = "im-app/nuwax-im-web-bootstrap.jar";
        std::fs::write(root.join(web), "old jar")?;
        std::fs::write(
            root.join("im-app/nuwax-im-gateway-bootstrap.jar"),
            "unchanged jar",
        )?;
        let archive = directory.path().join("patch.zip");
        let mut zip = zip::ZipWriter::new(File::create(&archive)?);
        zip.start_file(
            "docker/config/init_mysql.sql",
            zip::write::SimpleFileOptions::default(),
        )?;
        zip.write_all(valid_platform_sql().as_bytes())?;
        zip.start_file(
            format!("docker/{web}"),
            zip::write::SimpleFileOptions::default(),
        )?;
        let jar = [0x50, 0x4b, 0xff, 0xfe];
        zip.write_all(&jar)?;
        zip.finish()?;
        super::preflight_candidate_package_with_compose_at(
            &archive,
            true,
            &env,
            Some(&compose),
            &[web.to_string()],
        )?;
        let strategy = UpgradeStrategy::PatchUpgrade {
            patch_info: PatchPackageInfo {
                url: String::new(),
                hash: None,
                signature: None,
                notes: None,
                operations: PatchOperations {
                    replace: None,
                    delete: Some(ReplaceOperations {
                        files: vec![web.to_string()],
                        directories: Vec::new(),
                    }),
                },
            },
            target_version: "0.0.90.1".parse()?,
            download_type: DownloadType::Patch,
        };
        let cwd = std::env::current_dir()?;
        std::env::set_current_dir(directory.path())?;
        let extracted =
            crate::utils::extract_docker_service_with_env(&archive, &strategy, &env).await;
        std::env::set_current_dir(cwd)?;
        extracted?;
        assert_eq!(std::fs::read(root.join(web))?, jar);
        assert_eq!(
            std::fs::read_to_string(root.join("im-app/nuwax-im-gateway-bootstrap.jar"))?,
            "unchanged jar"
        );
        Ok(())
    }

    #[test]
    fn legacy_patch_rejects_unsafe_operation_paths_before_retaining_compose() -> Result<()> {
        let directory = tempfile::tempdir()?;
        let compose = directory.path().join("docker-compose.yml");
        std::fs::write(&compose, legacy_compose())?;
        let archive = directory.path().join("patch.zip");
        write_legacy_zip(
            &archive,
            &[("docker/config/init_mysql.sql", valid_platform_sql())],
        )?;
        let env = directory.path().join("operator.env");
        for changed in [
            "config/../docker-compose.yml",
            ".//docker-compose.yml",
            "/docker-compose.yml",
            "C:\\docker-compose.yml",
            ".",
        ] {
            assert!(
                super::preflight_candidate_package_with_compose_at(
                    &archive,
                    true,
                    &env,
                    Some(&compose),
                    &[changed.to_string()],
                )
                .is_err()
            );
        }
        assert_eq!(std::fs::read_to_string(compose)?, legacy_compose());
        Ok(())
    }

    #[test]
    fn legacy_patch_requires_referenced_or_preexisting_im_from_candidate_archive() -> Result<()> {
        let directory = tempfile::tempdir()?;
        let root = directory.path().join("docker");
        std::fs::create_dir_all(root.join("config"))?;
        let compose = root.join("docker-compose.yml");
        let env = directory.path().join("operator.env");
        let archive = directory.path().join("patch.zip");
        write_legacy_zip(
            &archive,
            &[("docker/config/init_mysql.sql", valid_platform_sql())],
        )?;
        std::fs::write(
            &compose,
            format!(
                "{}      - ./config/init_mysql_im.sql:/docker-entrypoint-initdb.d/20_im.sql:ro\n",
                legacy_compose()
            ),
        )?;
        assert!(
            super::preflight_candidate_package_with_compose_at(
                &archive,
                true,
                &env,
                Some(&compose),
                &[],
            )
            .is_err()
        );
        std::fs::write(&compose, legacy_compose())?;
        std::fs::write(root.join("config/init_mysql_im.sql"), valid_im_sql())?;
        assert!(
            super::preflight_candidate_package_with_compose_at(
                &archive,
                true,
                &env,
                Some(&compose),
                &[],
            )
            .is_err()
        );
        write_legacy_zip(
            &archive,
            &[
                ("docker/config/init_mysql.sql", valid_platform_sql()),
                (
                    "docker/config/init_mysql_im.sql",
                    "CREATE TABLE im_users (id INT);",
                ),
            ],
        )?;
        assert!(
            super::preflight_candidate_package_with_compose_at(
                &archive,
                true,
                &env,
                Some(&compose),
                &[],
            )
            .is_err()
        );
        write_legacy_zip(
            &archive,
            &[
                ("docker/config/init_mysql.sql", valid_platform_sql()),
                ("docker/config/init_mysql_im.sql", valid_im_sql()),
            ],
        )?;
        super::preflight_candidate_package_with_compose_at(
            &archive,
            true,
            &env,
            Some(&compose),
            &[],
        )?;
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
        let docker = directory.path().join("docker");
        std::fs::create_dir(&docker)?;
        std::fs::write(
            docker.join(".env"),
            "# operator config\nMYSQL_PASSWORD=existing-test-value\nFRONTEND_HOST_PORT=8091",
        )?;
        let archive = directory.path().join("package.zip");
        let mut zip = zip::ZipWriter::new(File::create(&archive)?);
        zip.start_file("docker/.env", zip::write::SimpleFileOptions::default())?;
        zip.write_all(b"MYSQL_PASSWORD=package-default\nSURREALDB_USER=package-user\nSURREALDB_PASSWORD=package-password\n")?;
        zip.finish()?;
        let strategy = UpgradeStrategy::FullUpgrade {
            url: String::new(),
            hash: String::new(),
            signature: String::new(),
            target_version: "0.0.108".parse()?,
            download_type: client_core::upgrade_strategy::DownloadType::Full,
        };
        let mut replacement = crate::utils::package_replace::PackageReplacement::prepare(
            &archive,
            &strategy,
            &docker,
            &docker.join(".env"),
            true,
        )?;
        replacement.apply()?;
        replacement.finish()?;
        let merged = std::fs::read_to_string(docker.join(".env"))?;
        assert!(merged.starts_with(
            "# operator config\nMYSQL_PASSWORD=existing-test-value\nFRONTEND_HOST_PORT=8091\n"
        ));
        assert!(merged.contains("SURREALDB_USER=package-user\n"));
        assert!(merged.contains("SURREALDB_PASSWORD=package-password\n"));
        assert!(!merged.contains("MYSQL_PASSWORD=package-default"));
        Ok(())
    }

    #[tokio::test]
    async fn real_candidate_rejection_reuses_the_verified_completed_download() -> Result<()> {
        use std::io::Read;
        use std::net::TcpListener;
        use std::sync::{
            Arc,
            atomic::{AtomicBool, AtomicUsize, Ordering},
        };
        use std::time::Duration;
        struct Fixture {
            stop: Arc<AtomicBool>,
            thread: Option<std::thread::JoinHandle<()>>,
        }
        impl Drop for Fixture {
            fn drop(&mut self) {
                self.stop.store(true, Ordering::SeqCst);
                if let Some(thread) = self.thread.take() {
                    let _ = thread.join();
                }
            }
        }
        let directory = tempfile::tempdir()?;
        let source = directory.path().join("source.zip");
        let mut zip = zip::ZipWriter::new(File::create(&source)?);
        zip.start_file(
            "docker/docker-compose.yml",
            zip::write::SimpleFileOptions::default(),
        )?;
        zip.write_all(b"services:\n  backend:\n    image: fixture\n    environment:\n      - SECRET=${CODEX_CACHE_PREFLIGHT_REQUIRED:?required}\n")?;
        zip.start_file("docker/.env", zip::write::SimpleFileOptions::default())?;
        zip.write_all(b"CODEX_CACHE_PREFLIGHT_REQUIRED=\n")?;
        zip.start_file(
            "docker/config/init_mysql.sql",
            zip::write::SimpleFileOptions::default(),
        )?;
        zip.write_all(valid_platform_sql().as_bytes())?;
        zip.finish()?;
        let body = std::fs::read(&source)?;
        let size = body.len();
        let listener = TcpListener::bind("127.0.0.1:0")?;
        let url = format!("http://{}/package.zip", listener.local_addr()?);
        listener.set_nonblocking(true)?;
        let get_count = Arc::new(AtomicUsize::new(0));
        let transferred = Arc::new(AtomicUsize::new(0));
        let stop = Arc::new(AtomicBool::new(false));
        let thread_count = get_count.clone();
        let thread_bytes = transferred.clone();
        let thread_stop = stop.clone();
        let thread = std::thread::spawn(move || {
            while !thread_stop.load(Ordering::SeqCst) {
                let (mut stream, _) = match listener.accept() {
                    Ok(connection) => connection,
                    Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                        std::thread::sleep(Duration::from_millis(5));
                        continue;
                    }
                    Err(error) => panic!("fixture accept failed: {error}"),
                };
                stream
                    .set_nonblocking(false)
                    .expect("blocking fixture connection");
                stream
                    .set_read_timeout(Some(Duration::from_secs(2)))
                    .expect("read timeout");
                let mut request = Vec::new();
                let mut buffer = [0_u8; 2048];
                while !request.windows(4).any(|bytes| bytes == b"\r\n\r\n") {
                    let count = stream.read(&mut buffer).expect("HTTP request");
                    if count == 0 {
                        break;
                    }
                    request.extend_from_slice(&buffer[..count]);
                }
                let get = request.starts_with(b"GET ");
                let header = format!(
                    "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nAccept-Ranges: bytes\r\nETag: \"cache-preflight-one\"\r\nConnection: close\r\n\r\n",
                    body.len()
                );
                stream.write_all(header.as_bytes()).expect("HTTP header");
                if get {
                    thread_count.fetch_add(1, Ordering::SeqCst);
                    stream.write_all(&body).expect("HTTP body");
                    thread_bytes.fetch_add(body.len(), Ordering::SeqCst);
                }
            }
        });
        let fixture = Fixture {
            stop,
            thread: Some(thread),
        };
        let identity = client_core::package_cache::PackageIdentity::new(
            "0.0.108.0",
            client_core::architecture::Architecture::detect().as_str(),
            "full",
            &url,
            None,
        )?;
        let _ = rustls::crypto::ring::default_provider().install_default();
        let api = client_core::api::ApiClient::new(None, None);
        let cache = directory.path().join("cache");
        let selected_env = directory.path().join("operator.env");
        for _ in 0..2 {
            let candidate = api.download_service_package(&cache, &identity).await?;
            let error = super::preflight_candidate_package_at(&candidate, false, &selected_env)
                .err()
                .context("The actual candidate preflight must reject the missing key")?;
            assert!(format!("{error:#}").contains("CODEX_CACHE_PREFLIGHT_REQUIRED"));
        }
        assert_eq!(get_count.load(Ordering::SeqCst), 1);
        assert_eq!(transferred.load(Ordering::SeqCst), size);
        drop(fixture);
        Ok(())
    }

    #[test]
    fn deployment_failure_rolls_back_files_only_before_mysql_startup() -> Result<()> {
        for database_started in [false, true] {
            let directory = tempfile::tempdir()?;
            let docker = directory.path().join("docker");
            std::fs::create_dir_all(docker.join("data/mysql"))?;
            std::fs::write(docker.join("data/mysql/canary"), "unchanged database")?;
            std::fs::write(docker.join("docker-compose.yml"), "old compose")?;
            std::fs::write(docker.join(".env"), "OPERATOR=keep\n")?;
            let package = directory.path().join("package.zip");
            let mut zip = zip::ZipWriter::new(File::create(&package)?);
            zip.start_file(
                "docker/docker-compose.yml",
                zip::write::SimpleFileOptions::default(),
            )?;
            zip.write_all(b"new compose")?;
            zip.start_file("docker/.env", zip::write::SimpleFileOptions::default())?;
            zip.write_all(b"NEW_KEY=added\n")?;
            zip.finish()?;
            let strategy = UpgradeStrategy::FullUpgrade {
                url: String::new(),
                hash: String::new(),
                signature: String::new(),
                target_version: "0.0.108".parse()?,
                download_type: client_core::upgrade_strategy::DownloadType::Full,
            };
            let mut replacement = crate::utils::package_replace::PackageReplacement::prepare(
                &package,
                &strategy,
                &docker,
                &docker.join(".env"),
                true,
            )?;
            replacement.apply()?;
            let error = super::finish_package_deployment(
                Some(replacement),
                Err(anyhow::anyhow!("injected deployment failure")),
                database_started,
            )
            .err()
            .context("injected deployment failure must propagate")?;
            assert_eq!(
                std::fs::read_to_string(docker.join("data/mysql/canary"))?,
                "unchanged database"
            );
            if database_started {
                assert_eq!(
                    std::fs::read_to_string(docker.join("docker-compose.yml"))?,
                    "new compose"
                );
                assert!(format!("{error:#}").contains("MySQL startup"));
                assert_eq!(std::fs::read_dir(directory.path())?.count(), 3);
            } else {
                assert_eq!(
                    std::fs::read_to_string(docker.join("docker-compose.yml"))?,
                    "old compose"
                );
                assert_eq!(
                    std::fs::read_to_string(docker.join(".env"))?,
                    "OPERATOR=keep\n"
                );
                assert!(format!("{error:#}").contains("previous managed files restored"));
                assert_eq!(std::fs::read_dir(directory.path())?.count(), 2);
            }
        }
        Ok(())
    }
}
