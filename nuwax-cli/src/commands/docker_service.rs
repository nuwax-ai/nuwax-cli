use client_core::container::DockerManager;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use crate::app::CliApp;
use crate::cli::DockerServiceCommand;
use crate::docker_service::{ContainerStatus, DockerService};
use anyhow::Result;
use client_core::upgrade_strategy::UpgradeStrategy;
use rust_i18n::t;
use tracing::{error, info, warn};

/// 运行 Docker 服务相关命令的统一入口
pub async fn run_docker_service_command(app: &CliApp, cmd: DockerServiceCommand) -> Result<()> {
    match cmd {
        DockerServiceCommand::Start { project } => {
            info!("▶️  Starting Docker services...");
            start_docker_services(app, None, project).await
        }
        DockerServiceCommand::Stop { project } => {
            info!("⏹️  Stopping Docker services...");
            stop_docker_services(app, None, project).await
        }
        DockerServiceCommand::Restart { project } => {
            info!("🔄 Restarting Docker services...");
            restart_docker_services(app, None, project).await
        }
        DockerServiceCommand::Status { project } => {
            info!("📊 Checking Docker service status...");
            check_docker_services_status_with_project(app, project).await
        }
        DockerServiceCommand::RestartContainer { container_name } => {
            info!("🔄 Restarting container: {name}", name = container_name);
            restart_container(app, &container_name).await
        }
        DockerServiceCommand::LoadImages => {
            info!("📦 Loading Docker images...");
            load_docker_images(app).await
        }
        DockerServiceCommand::SetupTags => {
            info!("🏷️  Setting image tags...");
            setup_image_tags(app).await
        }
        DockerServiceCommand::ArchInfo => {
            info!("🏗️  System architecture info:");
            show_architecture_info(app).await
        }
        DockerServiceCommand::ListImages => {
            info!("🔍 Listing Docker images:");
            let docker_service_manager =
                DockerService::new(app.config.clone(), app.docker_manager.clone())?;
            let images = docker_service_manager
                .list_docker_images_with_ducker()
                .await?;
            info!("Docker image list:");
            for image in images {
                info!("  {}", image);
            }
            Ok(())
        }
        DockerServiceCommand::CheckMountDirs => {
            info!("🔍 Checking and creating mount directories in docker-compose.yml...");
            let docker_service_manager =
                DockerService::new(app.config.clone(), app.docker_manager.clone())?;
            docker_service_manager
                .ensure_compose_mount_directories()
                .await?;
            info!("✅ Mount directory check complete");
            Ok(())
        }
    }
}

/// Apply command overrides while retaining the configured environment file.
pub(super) fn select_docker_manager(
    configured: &Arc<DockerManager>,
    compose_override: Option<PathBuf>,
    project_override: Option<String>,
) -> Result<Arc<DockerManager>> {
    if compose_override.is_none() && project_override.is_none() {
        return Ok(configured.clone());
    }
    let compose_path =
        compose_override.unwrap_or_else(|| configured.get_compose_file().to_path_buf());
    Ok(Arc::new(DockerManager::with_project(
        compose_path,
        configured.get_env_file().to_path_buf(),
        project_override,
    )?))
}

fn inject_device_env(manager: &DockerManager) -> Result<()> {
    crate::utils::device_env::ensure_device_env_with_paths(
        manager.get_env_file(),
        manager.get_compose_file(),
        &client_core::constants::device_info::get_fingerprint_file_path(),
        false,
    )
    .map_err(|error| {
        anyhow::anyhow!(
            "{}",
            t!("device_info_cmd.inject_failed", error = error.to_string())
        )
    })?;
    manager.invalidate_compose_config_cache();
    Ok(())
}

/// 准备 Docker 服务环境和镜像，但不启动容器。
pub async fn prepare_docker_services(
    app: &CliApp,
    frontend_port: Option<u16>,
    config_file: Option<PathBuf>,
    project_name: Option<String>,
) -> Result<()> {
    info!("🚀 Preparing Docker service deployment...");

    let manager = select_docker_manager(&app.docker_manager, config_file, project_name)?;
    if let Some(port) = frontend_port {
        info!("🔧 Configuring frontend port: {port}");
        set_frontend_port(manager.get_env_file(), port).await?;
    }
    inject_device_env(&manager)?;
    let mut docker_service_manager = DockerService::new(app.config.clone(), manager)?;

    // 显示系统信息
    let arch = docker_service_manager.get_architecture();
    info!(
        "Detected system architecture: {arch}",
        arch = arch.display_name()
    );
    info!(
        "Working directory: {path}",
        path = docker_service_manager.get_work_dir().display()
    );

    // 只执行准备阶段；数据库迁移前不能等待全部服务健康。
    match docker_service_manager.prepare_services().await {
        Ok(_) => info!("✅ Docker services prepared successfully!"),
        Err(e) => {
            error!(
                "❌ Docker service preparation failed: {error}",
                error = format!("{:?}", e)
            );
            return Err(anyhow::anyhow!(t!(
                "docker_service_cmd.deploy_failed_msg",
                error = format!("{:?}", e)
            )));
        }
    }

    Ok(())
}

/// 启动 Docker 服务
pub async fn start_docker_services(
    app: &CliApp,
    config_file: Option<PathBuf>,
    project_name: Option<String>,
) -> Result<()> {
    info!("▶️ Starting Docker services...");

    let manager = select_docker_manager(&app.docker_manager, config_file, project_name)?;
    inject_device_env(&manager)?;
    let mut docker_service_manager = DockerService::new(app.config.clone(), manager)?;

    match docker_service_manager.start_services().await {
        Ok(_) => {
            info!("✅ Docker services started successfully!");
        }
        Err(e) => {
            error!(
                "❌ Docker service start failed: {error}",
                error = e.to_string()
            );
            return Err(e.into());
        }
    }

    Ok(())
}

/// 停止 Docker 服务
pub async fn stop_docker_services(
    app: &CliApp,
    config_file: Option<PathBuf>,
    project_name: Option<String>,
) -> Result<()> {
    let manager = select_docker_manager(&app.docker_manager, config_file, project_name)?;
    let docker_service_manager = DockerService::new(app.config.clone(), manager)?;

    match docker_service_manager.stop_services().await {
        Ok(_) => {
            info!("✅ Docker services stopped");
        }
        Err(e) => {
            error!(
                "❌ Docker service stop failed: {error}",
                error = e.to_string()
            );
            return Err(e.into());
        }
    }

    Ok(())
}

/// 停止 Docker 服务并等待确认（统一的公共方法）
///
/// 这是一个完整的停止流程，包括：
/// 1. 检查服务是否在运行
/// 2. 执行停止命令
/// 3. 等待服务完全停止
///
/// # 参数
/// - `app`: 应用实例
/// - `config_file`: 可选的 docker-compose 配置文件路径
/// - `project_name`: 可选的项目名称
///
/// # 返回
/// - `Ok(true)`: 服务已停止（或本来就没运行）
/// - `Ok(false)`: 等待停止超时，但可以继续
/// - `Err`: 发生错误
pub async fn stop_docker_services_and_wait(
    app: &CliApp,
    config_file: Option<PathBuf>,
    project_name: Option<String>,
) -> Result<bool> {
    use crate::docker_service::health_check::HealthChecker;
    use client_core::constants::timeout;
    use tokio::time::{Duration, Instant, sleep};

    info!("🔍 Checking Docker service status...");

    let docker_manager = select_docker_manager(
        &app.docker_manager,
        config_file.clone(),
        project_name.clone(),
    )?;

    // 2. 检查服务是否在运行
    let health_checker = HealthChecker::new(docker_manager);
    let report = health_checker.health_check().await?;
    let running_count = report.get_running_count();

    if running_count == 0 {
        info!("ℹ️ Docker services not running, no need to stop");
        return Ok(true);
    }

    info!("🔍 Found {count} running services", count = running_count);

    // 3. 执行停止命令
    info!("🛑 Stopping Docker services...");
    stop_docker_services(app, config_file.clone(), project_name.clone()).await?;

    // 4. 等待服务完全停止（使用 HealthChecker 精确检查）
    info!("⏳ Waiting for Docker services to stop completely...");

    let start_time = Instant::now();
    let timeout_duration = Duration::from_secs(timeout::SERVICE_STOP_TIMEOUT);
    let check_interval = Duration::from_secs(timeout::SERVICE_CHECK_INTERVAL);

    loop {
        // 每次循环都重新检查服务状态
        let report = health_checker.health_check().await?;
        let running_count = report.get_running_count();

        if running_count == 0 {
            info!("✅ Docker services stopped successfully");
            return Ok(true);
        }

        // 检查是否超时
        if start_time.elapsed() >= timeout_duration {
            warn!(
                timeout_seconds = timeout::SERVICE_STOP_TIMEOUT,
                running_count = running_count,
                "⚠️ Service stop timeout, {count} services still running, but can continue",
                count = running_count
            );

            // 显示哪些服务还在运行
            info!("📋 Still running services:");
            for container in &report.containers {
                if container.status.is_healthy() {
                    info!(
                        "  • {name} ({image})",
                        name = container.name,
                        image = container.image
                    );
                }
            }

            return Ok(false);
        }

        info!(
            "⏳ {count} services still running, waiting...",
            count = running_count
        );
        sleep(check_interval).await;
    }
}

/// 重启 Docker 服务
pub async fn restart_docker_services(
    app: &CliApp,
    config_file: Option<PathBuf>,
    project_name: Option<String>,
) -> Result<()> {
    info!("🔄 Restarting Docker services...");

    let manager = select_docker_manager(&app.docker_manager, config_file, project_name)?;
    let mut docker_service_manager = DockerService::new(app.config.clone(), manager)?;

    match docker_service_manager.restart_services().await {
        Ok(_) => {
            info!("✅ Docker services restarted successfully!");
        }
        Err(e) => {
            error!(
                "❌ Docker service restart failed: {error}",
                error = e.to_string()
            );
            return Err(e.into());
        }
    }

    Ok(())
}

/// 重启单个容器
pub async fn restart_container(app: &CliApp, container_name: &str) -> Result<()> {
    info!("🔄 Restarting container: {name}", name = container_name);

    let docker_service_manager =
        DockerService::new(app.config.clone(), app.docker_manager.clone())?;

    match docker_service_manager
        .restart_container(container_name)
        .await
    {
        Ok(_) => {
            info!(
                "✅ Container {name} restarted successfully!",
                name = container_name
            );
        }
        Err(e) => {
            error!(
                "❌ Container {name} restart failed: {error}",
                name = container_name,
                error = e.to_string()
            );
            return Err(e.into());
        }
    }

    Ok(())
}

/// 检查 Docker 服务状态
pub async fn check_docker_services_status(app: &CliApp) -> Result<()> {
    check_docker_services_status_with_project(app, None).await
}

/// 检查 Docker 服务状态（支持项目名称）
pub async fn check_docker_services_status_with_project(
    app: &CliApp,
    project_name: Option<String>,
) -> Result<()> {
    info!("📊 Checking Docker service status...");

    let manager = select_docker_manager(&app.docker_manager, None, project_name)?;
    let docker_service_manager = DockerService::new(app.config.clone(), manager)?;

    match docker_service_manager.health_check().await {
        Ok(report) => {
            info!("=== Docker Service Status Report ===");
            info!(
                "Check time: {time}",
                time = report.check_time.format("%Y-%m-%d %H:%M:%S UTC")
            );
            info!(
                "Overall status: {status}",
                status = report.finalize().display_name()
            );
            info!(
                "Running stats: {running}/{total} containers running",
                running = report.get_running_count(),
                total = report.get_total_count()
            );

            if !report.containers.is_empty() {
                info!("Container details:");
                for container in &report.containers {
                    let status_icon = match container.status {
                        ContainerStatus::Running => "🟢",
                        ContainerStatus::Stopped => "🔴",
                        ContainerStatus::Starting => "🟡",
                        ContainerStatus::Completed => "✅",
                        ContainerStatus::Unknown => "⚪",
                    };

                    info!(
                        "  {icon} {name} ({status})",
                        icon = status_icon,
                        name = container.name,
                        status = container.status.display_name()
                    );
                    info!("     Image: {image}", image = container.image);

                    if !container.ports.is_empty() {
                        info!("     Ports: {ports}", ports = container.ports.join(", "));
                    }
                }
            }

            if !report.errors.is_empty() {
                warn!("⚠️ Error messages:");
                for error in &report.errors {
                    warn!("  • {error}", error = error);
                }
            }

            // 显示访问信息
            if report.finalize().is_healthy() {
                use client_core::constants::docker::ports;
                info!("🌐 Service access info:");
                info!(
                    "  • Frontend: http://localhost:{port}",
                    port = ports::DEFAULT_FRONTEND_PORT
                );
                info!(
                    "  • Backend API: http://localhost:{port}",
                    port = ports::DEFAULT_BACKEND_PORT
                );
                info!(
                    "  • Admin panel: http://localhost:{port} (if configured)",
                    port = ports::DEFAULT_MINIO_API_PORT
                );
                info!("  📝 Note: Use custom port if specified");
            }
        }
        Err(e) => {
            error!(
                "❌ Failed to get service status: {error}",
                error = format!("{:?}", e)
            );
            return Err(anyhow::anyhow!(t!(
                "docker_service_cmd.get_status_failed_msg",
                error = format!("{:?}", e)
            )));
        }
    }

    Ok(())
}

/// 加载 Docker 镜像
pub async fn load_docker_images(app: &CliApp) -> Result<()> {
    info!("📦 Loading Docker images...");

    let docker_service_manager =
        DockerService::new(app.config.clone(), app.docker_manager.clone())?;

    // 显示架构信息
    let arch = docker_service_manager.get_architecture();
    info!(
        "Current system architecture: {arch}",
        arch = arch.display_name()
    );

    match docker_service_manager.load_images().await {
        Ok(result) => {
            info!("📦 Image loading complete!");
            info!(
                "  • Successfully loaded: {count} images",
                count = result.success_count()
            );
            info!(
                "  • Failed to load: {count} images",
                count = result.failure_count()
            );

            if !result.loaded_images.is_empty() {
                info!("✅ Successfully loaded images:");
                for image in &result.loaded_images {
                    info!("  • {image}", image = image);
                }
            }

            if !result.failed_images.is_empty() {
                warn!("❌ Failed to load images:");
                for (image, error) in &result.failed_images {
                    warn!("  • {image}: {error}", image = image, error = error);
                }
            }
        }
        Err(e) => {
            error!("❌ Image loading failed: {error}", error = e.to_string());
            return Err(e.into());
        }
    }

    Ok(())
}

/// 设置镜像标签
pub async fn setup_image_tags(app: &CliApp) -> Result<()> {
    info!("🏷️  Setting image tags...");

    let docker_service_manager =
        DockerService::new(app.config.clone(), app.docker_manager.clone())?;

    // 先加载镜像以获取实际的镜像映射
    info!("📦 Checking loaded images...");
    let load_result = docker_service_manager.load_images().await?;

    if load_result.image_mappings.is_empty() {
        warn!("⚠️ No loaded image mappings found, please run load-images first");
        return Ok(());
    }

    // 使用基于映射的新方法
    match docker_service_manager
        .setup_image_tags_with_mappings(&load_result.image_mappings)
        .await
    {
        Ok(result) => {
            info!("🏷️ Image tag setup complete!");
            info!(
                "  • Successfully set: {count} tags",
                count = result.success_count()
            );
            info!(
                "  • Failed to set: {count} tags",
                count = result.failure_count()
            );

            if !result.tagged_images.is_empty() {
                info!("✅ Successfully tagged:");
                for (original, target) in &result.tagged_images {
                    info!(
                        "  • {original} → {target}",
                        original = original,
                        target = target
                    );
                }
            }

            if !result.failed_tags.is_empty() {
                warn!("❌ Failed to tag:");
                for (original, target, error) in &result.failed_tags {
                    warn!(
                        "  • {original} → {target}: {error}",
                        original = original,
                        target = target,
                        error = error
                    );
                }
            }
        }
        Err(e) => {
            error!("❌ Image tag setup failed: {error}", error = e.to_string());
            return Err(e.into());
        }
    }

    Ok(())
}

/// 按升级策略解析已下载包的本地路径（不触发下载）。
/// `NoUpgrade` 返回 `None`。供停服务前的候选包预检读取少量条目使用。
pub fn package_path_for_strategy(
    app: &CliApp,
    upgrade_strategy: &UpgradeStrategy,
) -> Result<Option<PathBuf>> {
    use client_core::package_cache::{PackageIdentity, resolve_cached_package};
    let architecture = client_core::architecture::Architecture::detect();
    let (directory, identity) = match upgrade_strategy {
        UpgradeStrategy::FullUpgrade {
            target_version,
            download_type,
            url,
            hash,
            ..
        } => (
            app.config.get_version_download_dir(
                &target_version.base_version_string(),
                &download_type.to_string(),
            ),
            PackageIdentity::new(
                &target_version.to_string(),
                architecture.as_str(),
                "full",
                url,
                Some(hash),
            )?,
        ),
        UpgradeStrategy::PatchUpgrade {
            target_version,
            patch_info,
            ..
        } => (
            app.config.get_version_download_dir(
                &target_version.base_version_string(),
                &target_version.to_string(),
            ),
            PackageIdentity::new(
                &target_version.to_string(),
                architecture.as_str(),
                "patch",
                &patch_info.url,
                patch_info.hash.as_deref(),
            )?,
        ),
        UpgradeStrategy::NoUpgrade { .. } => return Ok(None),
    };
    Ok(Some(resolve_cached_package(&directory, &identity)?))
}

/// 获取系统架构信息
pub async fn show_architecture_info(_app: &CliApp) -> Result<()> {
    let arch = crate::docker_service::get_system_architecture();

    info!("🔧 System architecture info:");
    info!("  • Architecture type: {arch}", arch = arch.display_name());
    info!("  • Architecture ID: {id}", id = arch.as_str());
    info!(
        "  • Image suffix: {suffix}",
        suffix = crate::docker_service::get_architecture_suffix(arch)
    );

    Ok(())
}

/// 设置frontend服务端口（使用新的环境变量管理器）
async fn set_frontend_port(env_file_path: &Path, port: u16) -> Result<()> {
    use crate::utils::env_manager::update_frontend_port;
    if !env_file_path.exists() {
        info!("   .env file not found, no need to update port");
        return Ok(());
    }

    info!("🔧 Updating frontend port in .env: {port}", port = port);
    info!("   .env file path: {path}", path = env_file_path.display());

    // 使用新的环境变量管理器进行智能更新
    if let Err(e) = update_frontend_port(env_file_path, port) {
        error!(
            "❌ Port configuration update failed: {error}",
            error = e.to_string()
        );
        return Err(anyhow::anyhow!(t!(
            "docker_service_cmd.update_port_failed_msg",
            error = e.to_string()
        )));
    }

    info!("✅ Port configuration updated successfully!");
    Ok(())
}

#[cfg(test)]
mod custom_deployment_paths_tests {
    use super::*;

    #[test]
    fn command_overrides_retain_the_configured_environment_file() -> Result<()> {
        let directory = tempfile::tempdir()?;
        let compose = directory.path().join("deploy/prod.yml");
        let env_path = directory.path().join("secrets/prod.env");
        let configured = Arc::new(DockerManager::with_project(
            compose.clone(),
            env_path.clone(),
            None,
        )?);
        assert!(Arc::ptr_eq(
            &select_docker_manager(&configured, None, None)?,
            &configured
        ));
        let override_path = directory.path().join("override.yml");
        let selected = select_docker_manager(
            &configured,
            Some(override_path.clone()),
            Some("isolated-project".to_string()),
        )?;
        assert_eq!(selected.get_compose_file(), override_path);
        assert_eq!(selected.get_env_file(), env_path);
        let project_only =
            select_docker_manager(&configured, None, Some("project-only".to_string()))?;
        assert_eq!(project_only.get_compose_file(), compose);
        assert_eq!(project_only.get_env_file(), env_path);
        Ok(())
    }

    #[tokio::test]
    async fn frontend_port_updates_the_selected_custom_file() -> Result<()> {
        let directory = tempfile::tempdir()?;
        let path = directory.path().join("prod.env");
        std::fs::write(
            &path,
            "FRONTEND_HOST_PORT=80 # keep comment\nMYSQL_PASSWORD=\"synthetic\\$value\"\n",
        )?;
        set_frontend_port(&path, 8090).await?;
        assert_eq!(
            std::fs::read_to_string(&path)?,
            "FRONTEND_HOST_PORT=8090 # keep comment\nMYSQL_PASSWORD=\"synthetic\\$value\"\n"
        );
        Ok(())
    }
}
