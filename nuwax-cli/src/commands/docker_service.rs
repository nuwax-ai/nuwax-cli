use std::path::PathBuf;

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

/// 部署 Docker 服务
pub async fn deploy_docker_services(
    app: &CliApp,
    frontend_port: Option<u16>,
    config_file: Option<PathBuf>,
    project_name: Option<String>,
) -> Result<()> {
    info!("🚀 Starting Docker service deployment...");

    // 如果指定了端口，先设置端口配置
    if let Some(port) = frontend_port {
        info!("🔧 Configuring frontend port: {port}", port = port);
        set_frontend_port(port).await?;
    }

    // 创建 Docker 服务管理器
    let mut docker_service_manager = if let Some(compose_path) = config_file {
        // 使用自定义的compose文件路径创建DockerManager
        let env_path = client_core::constants::docker::get_env_file_path();
        let custom_docker_manager =
            std::sync::Arc::new(client_core::container::DockerManager::with_project(
                &compose_path,
                &env_path,
                project_name,
            )?);
        DockerService::new(app.config.clone(), custom_docker_manager)?
    } else {
        // 如果没有指定config文件，但有project name，创建带project name的DockerManager
        if let Some(project_name) = project_name {
            let custom_docker_manager =
                std::sync::Arc::new(client_core::container::DockerManager::with_project(
                    client_core::constants::docker::get_compose_file_path(),
                    client_core::constants::docker::get_env_file_path(),
                    Some(project_name),
                )?);
            DockerService::new(app.config.clone(), custom_docker_manager)?
        } else {
            // 使用默认的DockerManager
            DockerService::new(app.config.clone(), app.docker_manager.clone())?
        }
    };

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

    // 执行完整的部署流程
    match docker_service_manager.deploy_services().await {
        Ok(_) => {
            info!("✅ Docker services deployed successfully!");

            // 显示服务状态
            if let Ok(report) = docker_service_manager.health_check().await {
                info!("📊 Service status overview:");
                info!(
                    "  • Overall status: {status}",
                    status = report.finalize().display_name()
                );
                info!(
                    "  • Running containers: {running}/{total}",
                    running = report.get_running_count(),
                    total = report.get_total_count()
                );

                if !report.containers.is_empty() {
                    info!("  • Container details:");
                    for container in &report.containers {
                        info!(
                            "    - {name} ({image}) - {status}",
                            name = container.name,
                            image = container.image,
                            status = container.status.display_name()
                        );
                    }
                }
            }
        }
        Err(e) => {
            error!(
                "❌ Docker service deployment failed: {error}",
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

    let mut docker_service_manager = if let Some(compose_path) = config_file {
        // 使用自定义的compose文件路径创建DockerManager
        let env_path = client_core::constants::docker::get_env_file_path();
        let custom_docker_manager =
            std::sync::Arc::new(client_core::container::DockerManager::with_project(
                &compose_path,
                &env_path,
                project_name,
            )?);
        DockerService::new(app.config.clone(), custom_docker_manager)?
    } else {
        // 如果没有指定config文件，但有project name，创建带project name的DockerManager
        if let Some(project_name) = project_name {
            let custom_docker_manager =
                std::sync::Arc::new(client_core::container::DockerManager::with_project(
                    client_core::constants::docker::get_compose_file_path(),
                    client_core::constants::docker::get_env_file_path(),
                    Some(project_name),
                )?);
            DockerService::new(app.config.clone(), custom_docker_manager)?
        } else {
            // 使用默认的DockerManager
            DockerService::new(app.config.clone(), app.docker_manager.clone())?
        }
    };

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
    let docker_service_manager = if let Some(compose_path) = config_file {
        // 使用自定义的compose文件路径创建DockerManager
        let env_path = client_core::constants::docker::get_env_file_path();
        let custom_docker_manager =
            std::sync::Arc::new(client_core::container::DockerManager::with_project(
                &compose_path,
                &env_path,
                project_name,
            )?);
        DockerService::new(app.config.clone(), custom_docker_manager)?
    } else {
        // 如果没有指定config文件，但有project name，创建带project name的DockerManager
        if let Some(project_name) = project_name {
            let custom_docker_manager =
                std::sync::Arc::new(client_core::container::DockerManager::with_project(
                    client_core::constants::docker::get_compose_file_path(),
                    client_core::constants::docker::get_env_file_path(),
                    Some(project_name),
                )?);
            DockerService::new(app.config.clone(), custom_docker_manager)?
        } else {
            // 使用默认的DockerManager
            DockerService::new(app.config.clone(), app.docker_manager.clone())?
        }
    };

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

    // 1. 创建 DockerManager（用于 HealthChecker）
    let docker_manager = if let Some(ref compose_path) = config_file {
        let env_path = client_core::constants::docker::get_env_file_path();
        std::sync::Arc::new(client_core::container::DockerManager::with_project(
            compose_path,
            &env_path,
            project_name.clone(),
        )?)
    } else if let Some(ref proj_name) = project_name {
        std::sync::Arc::new(client_core::container::DockerManager::with_project(
            client_core::constants::docker::get_compose_file_path(),
            client_core::constants::docker::get_env_file_path(),
            Some(proj_name.clone()),
        )?)
    } else {
        app.docker_manager.clone()
    };

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

    let mut docker_service_manager = if let Some(compose_path) = config_file {
        // 使用自定义的compose文件路径创建DockerManager
        let env_path = client_core::constants::docker::get_env_file_path();
        let custom_docker_manager =
            std::sync::Arc::new(client_core::container::DockerManager::with_project(
                &compose_path,
                &env_path,
                project_name,
            )?);
        DockerService::new(app.config.clone(), custom_docker_manager)?
    } else {
        // 如果没有指定config文件，但有project name，创建带project name的DockerManager
        if let Some(project_name) = project_name {
            let custom_docker_manager =
                std::sync::Arc::new(client_core::container::DockerManager::with_project(
                    client_core::constants::docker::get_compose_file_path(),
                    client_core::constants::docker::get_env_file_path(),
                    Some(project_name),
                )?);
            DockerService::new(app.config.clone(), custom_docker_manager)?
        } else {
            // 使用默认的DockerManager
            DockerService::new(app.config.clone(), app.docker_manager.clone())?
        }
    };

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

    // 创建支持项目名称的 DockerService
    let docker_service_manager = if let Some(project_name) = project_name {
        let custom_docker_manager =
            std::sync::Arc::new(client_core::container::DockerManager::with_project(
                client_core::constants::docker::get_compose_file_path(),
                client_core::constants::docker::get_env_file_path(),
                Some(project_name),
            )?);
        DockerService::new(app.config.clone(), custom_docker_manager)?
    } else {
        DockerService::new(app.config.clone(), app.docker_manager.clone())?
    };

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

/// 解压Docker服务包, 并根据升级策略进行处理
pub async fn extract_docker_service_with_upgrade_strategy(
    app: &CliApp,
    upgrade_strategy: UpgradeStrategy,
) -> Result<()> {
    //区分升级策略,来进行解压
    let file_zip: PathBuf = match &upgrade_strategy {
        UpgradeStrategy::FullUpgrade {
            target_version,
            download_type,
            ..
        } => {
            // 强制升级策略，直接解压并覆盖现有文件
            info!("📦 Starting Docker service package extraction...");

            let base_version = target_version.base_version_string();

            app.config.get_version_download_file_path(
                &base_version,
                &download_type.to_string(),
                None,
            )?
        }
        UpgradeStrategy::PatchUpgrade { target_version, .. } => {
            //增量升级
            let base_version = target_version.base_version_string();
            let full_version = target_version.to_string();

            app.config.get_version_download_file_path(
                &base_version,
                &full_version.to_string(),
                None,
            )?
        }
        UpgradeStrategy::NoUpgrade { .. } => {
            // 无需升级
            return Ok(());
        }
    };

    info!(
        "📦 Found Docker service package: {path}",
        path = file_zip.display()
    );

    // 使用utils中的解压函数
    crate::utils::extract_docker_service(&file_zip, &upgrade_strategy).await?;

    info!("✅ Docker service package extraction complete");
    Ok(())
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
async fn set_frontend_port(port: u16) -> Result<()> {
    use crate::utils::env_manager::update_frontend_port;
    use client_core::constants::docker::get_env_file_path;

    let env_file_path = get_env_file_path();
    if !env_file_path.exists() {
        info!("   .env file not found, no need to update port");
        return Ok(());
    }

    info!("🔧 Updating frontend port in .env: {port}", port = port);
    info!("   .env file path: {path}", path = env_file_path.display());

    // 使用新的环境变量管理器进行智能更新
    if let Err(e) = update_frontend_port(&env_file_path, port) {
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
