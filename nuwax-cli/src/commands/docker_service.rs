use std::path::PathBuf;

use crate::app::CliApp;
use crate::cli::DockerServiceCommand;
use crate::docker_service::{ContainerStatus, DockerService};
use anyhow::Result;
use client_core::upgrade_strategy::UpgradeStrategy;
use tracing::{error, info, warn};

/// 运行 Docker 服务相关命令的统一入口
pub async fn run_docker_service_command(app: &CliApp, cmd: DockerServiceCommand) -> Result<()> {
    match cmd {
        DockerServiceCommand::Start { project } => {
            info!("▶️  启动 Docker 服务...");
            start_docker_services(app, None, project).await
        }
        DockerServiceCommand::Stop { project } => {
            info!("⏹️  停止 Docker 服务...");
            stop_docker_services(app, None, project).await
        }
        DockerServiceCommand::Restart { project } => {
            info!("🔄 重启 Docker 服务...");
            restart_docker_services(app, None, project).await
        }
        DockerServiceCommand::Status { project } => {
            info!("📊 检查 Docker 服务状态...");
            check_docker_services_status_with_project(app, project).await
        }
        DockerServiceCommand::RestartContainer { container_name } => {
            info!("🔄 重启容器: {}", container_name);
            restart_container(app, &container_name).await
        }
        DockerServiceCommand::LoadImages => {
            info!("📦 加载 Docker 镜像...");
            load_docker_images(app).await
        }
        DockerServiceCommand::SetupTags => {
            info!("🏷️  设置镜像标签...");
            setup_image_tags(app).await
        }
        DockerServiceCommand::ArchInfo => {
            info!("🏗️  系统架构信息:");
            show_architecture_info(app).await
        }
        DockerServiceCommand::ListImages => {
            info!("🔍 列出 Docker 镜像:");
            let docker_service_manager =
                DockerService::new(app.config.clone(), app.docker_manager.clone())?;
            let images = docker_service_manager
                .list_docker_images_with_ducker()
                .await?;
            info!("Docker 镜像列表:");
            for image in images {
                info!("  {}", image);
            }
            Ok(())
        }
        DockerServiceCommand::CheckMountDirs => {
            info!("🔍 检查并创建docker-compose.yml中的挂载目录...");
            let docker_service_manager =
                DockerService::new(app.config.clone(), app.docker_manager.clone())?;
            docker_service_manager
                .ensure_compose_mount_directories()
                .await?;
            info!("✅ 挂载目录检查完成");
            Ok(())
        }
    }
}

/// 部署 Docker 服务
pub async fn deploy_docker_services(app: &CliApp, frontend_port: Option<u16>, config_file: Option<PathBuf>, project_name: Option<String>) -> Result<()> {
    info!("🚀 开始部署 Docker 服务...");

    // 如果指定了端口，先设置端口配置
    if let Some(port) = frontend_port {
        info!("🔧 配置frontend端口: {}", port);
        set_frontend_port(port).await?;
    }

    // 创建 Docker 服务管理器
    let mut docker_service_manager = if let Some(compose_path) = config_file {
        // 使用自定义的compose文件路径创建DockerManager
        let env_path = client_core::constants::docker::get_env_file_path();
        let custom_docker_manager = std::sync::Arc::new(
            client_core::container::DockerManager::with_project(&compose_path, &env_path, project_name)?
        );
        DockerService::new(app.config.clone(), custom_docker_manager)?
    } else {
        // 如果没有指定config文件，但有project name，创建带project name的DockerManager
        if let Some(project_name) = project_name {
            let custom_docker_manager = std::sync::Arc::new(
                client_core::container::DockerManager::with_project(
                    client_core::constants::docker::get_compose_file_path(),
                    client_core::constants::docker::get_env_file_path(),
                    Some(project_name),
                )?
            );
            DockerService::new(app.config.clone(), custom_docker_manager)?
        } else {
            // 使用默认的DockerManager
            DockerService::new(app.config.clone(), app.docker_manager.clone())?
        }
    };

    // 显示系统信息
    let arch = docker_service_manager.get_architecture();
    info!("检测到系统架构: {}", arch.display_name());
    info!(
        "工作目录: {}",
        docker_service_manager.get_work_dir().display()
    );

    // 执行完整的部署流程
    match docker_service_manager.deploy_services().await {
        Ok(_) => {
            info!("✅ Docker 服务部署成功!");

            // 显示服务状态
            if let Ok(report) = docker_service_manager.health_check().await {
                info!("📊 服务状态概览:");
                info!("  • 整体状态: {}", report.finalize().display_name());
                info!(
                    "  • 运行中容器: {}/{}",
                    report.get_running_count(), report.get_total_count()
                );

                if !report.containers.is_empty() {
                    info!("  • 容器详情:");
                    for container in &report.containers {
                        info!(
                            "    - {} ({}) - {}",
                            container.name,
                            container.image,
                            container.status.display_name()
                        );
                    }
                }
            }
        }
        Err(e) => {
            error!("❌ Docker 服务部署失败: {:?}", e);
            return Err(anyhow::anyhow!(format!("Docker 服务部署失败: {e:?}")));
        }
    }

    Ok(())
}

/// 启动 Docker 服务
pub async fn start_docker_services(app: &CliApp, config_file: Option<PathBuf>, project_name: Option<String>) -> Result<()> {
    info!("▶️ 启动 Docker 服务...");

    let mut docker_service_manager = if let Some(compose_path) = config_file {
        // 使用自定义的compose文件路径创建DockerManager
        let env_path = client_core::constants::docker::get_env_file_path();
        let custom_docker_manager = std::sync::Arc::new(
            client_core::container::DockerManager::with_project(&compose_path, &env_path, project_name)?
        );
        DockerService::new(app.config.clone(), custom_docker_manager)?
    } else {
        // 如果没有指定config文件，但有project name，创建带project name的DockerManager
        if let Some(project_name) = project_name {
            let custom_docker_manager = std::sync::Arc::new(
                client_core::container::DockerManager::with_project(
                    client_core::constants::docker::get_compose_file_path(),
                    client_core::constants::docker::get_env_file_path(),
                    Some(project_name),
                )?
            );
            DockerService::new(app.config.clone(), custom_docker_manager)?
        } else {
            // 使用默认的DockerManager
            DockerService::new(app.config.clone(), app.docker_manager.clone())?
        }
    };

    match docker_service_manager.start_services().await {
        Ok(_) => {
            info!("✅ Docker 服务启动成功!");
        }
        Err(e) => {
            error!("❌ Docker 服务启动失败: {}", e);
            return Err(e.into());
        }
    }

    Ok(())
}

/// 停止 Docker 服务
pub async fn stop_docker_services(app: &CliApp, config_file: Option<PathBuf>, project_name: Option<String>) -> Result<()> {
    let docker_service_manager = if let Some(compose_path) = config_file {
        // 使用自定义的compose文件路径创建DockerManager
        let env_path = client_core::constants::docker::get_env_file_path();
        let custom_docker_manager = std::sync::Arc::new(
            client_core::container::DockerManager::with_project(&compose_path, &env_path, project_name)?
        );
        DockerService::new(app.config.clone(), custom_docker_manager)?
    } else {
        // 如果没有指定config文件，但有project name，创建带project name的DockerManager
        if let Some(project_name) = project_name {
            let custom_docker_manager = std::sync::Arc::new(
                client_core::container::DockerManager::with_project(
                    client_core::constants::docker::get_compose_file_path(),
                    client_core::constants::docker::get_env_file_path(),
                    Some(project_name),
                )?
            );
            DockerService::new(app.config.clone(), custom_docker_manager)?
        } else {
            // 使用默认的DockerManager
            DockerService::new(app.config.clone(), app.docker_manager.clone())?
        }
    };

    match docker_service_manager.stop_services().await {
        Ok(_) => {
            info!("✅ Docker 服务已停止");
        }
        Err(e) => {
            error!("❌ Docker 服务停止失败: {}", e);
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
    use tokio::time::{sleep, Duration, Instant};

    info!("🔍 检查Docker服务状态...");

    // 1. 创建 DockerManager（用于 HealthChecker）
    let docker_manager = if let Some(ref compose_path) = config_file {
        let env_path = client_core::constants::docker::get_env_file_path();
        std::sync::Arc::new(
            client_core::container::DockerManager::with_project(
                compose_path,
                &env_path,
                project_name.clone(),
            )?
        )
    } else if let Some(ref proj_name) = project_name {
        std::sync::Arc::new(
            client_core::container::DockerManager::with_project(
                client_core::constants::docker::get_compose_file_path(),
                client_core::constants::docker::get_env_file_path(),
                Some(proj_name.clone()),
            )?
        )
    } else {
        app.docker_manager.clone()
    };

    // 2. 检查服务是否在运行
    let health_checker = HealthChecker::new(docker_manager);
    let report = health_checker.health_check().await?;
    let running_count = report.get_running_count();

    if running_count == 0 {
        info!("ℹ️ Docker服务未运行，无需停止");
        return Ok(true);
    }

    info!("🔍 发现 {} 个运行中的服务", running_count);

    // 3. 执行停止命令
    info!("🛑 停止Docker服务...");
    stop_docker_services(app, config_file.clone(), project_name.clone()).await?;

    // 4. 等待服务完全停止（使用 HealthChecker 精确检查）
    info!("⏳ 等待Docker服务完全停止...");
    
    let start_time = Instant::now();
    let timeout_duration = Duration::from_secs(timeout::SERVICE_STOP_TIMEOUT);
    let check_interval = Duration::from_secs(timeout::SERVICE_CHECK_INTERVAL);
    
    loop {
        // 每次循环都重新检查服务状态
        let report = health_checker.health_check().await?;
        let running_count = report.get_running_count();
        
        if running_count == 0 {
            info!("✅ Docker服务已成功停止");
            return Ok(true);
        }
        
        // 检查是否超时
        if start_time.elapsed() >= timeout_duration {
            warn!(
                timeout_seconds = timeout::SERVICE_STOP_TIMEOUT,
                running_count = running_count,
                "⚠️ 等待服务停止超时，还有 {} 个服务在运行，但可以继续",
                running_count
            );
            
            // 显示哪些服务还在运行
            info!("📋 仍在运行的服务:");
            for container in &report.containers {
                if container.status.is_healthy() {
                    info!("  • {} ({})", container.name, container.image);
                }
            }
            
            return Ok(false);
        }
        
        info!("⏳ 还有 {} 个服务在运行，继续等待...", running_count);
        sleep(check_interval).await;
    }
}

/// 重启 Docker 服务
pub async fn restart_docker_services(app: &CliApp, config_file: Option<PathBuf>, project_name: Option<String>) -> Result<()> {
    info!("🔄 重启 Docker 服务...");

    let mut docker_service_manager = if let Some(compose_path) = config_file {
        // 使用自定义的compose文件路径创建DockerManager
        let env_path = client_core::constants::docker::get_env_file_path();
        let custom_docker_manager = std::sync::Arc::new(
            client_core::container::DockerManager::with_project(&compose_path, &env_path, project_name)?
        );
        DockerService::new(app.config.clone(), custom_docker_manager)?
    } else {
        // 如果没有指定config文件，但有project name，创建带project name的DockerManager
        if let Some(project_name) = project_name {
            let custom_docker_manager = std::sync::Arc::new(
                client_core::container::DockerManager::with_project(
                    client_core::constants::docker::get_compose_file_path(),
                    client_core::constants::docker::get_env_file_path(),
                    Some(project_name),
                )?
            );
            DockerService::new(app.config.clone(), custom_docker_manager)?
        } else {
            // 使用默认的DockerManager
            DockerService::new(app.config.clone(), app.docker_manager.clone())?
        }
    };

    match docker_service_manager.restart_services().await {
        Ok(_) => {
            info!("✅ Docker 服务重启成功!");
        }
        Err(e) => {
            error!("❌ Docker 服务重启失败: {}", e);
            return Err(e.into());
        }
    }

    Ok(())
}

/// 重启单个容器
pub async fn restart_container(app: &CliApp, container_name: &str) -> Result<()> {
    info!("🔄 重启容器: {}", container_name);

    let docker_service_manager =
        DockerService::new(app.config.clone(), app.docker_manager.clone())?;

    match docker_service_manager
        .restart_container(container_name)
        .await
    {
        Ok(_) => {
            info!("✅ 容器 {} 重启成功!", container_name);
        }
        Err(e) => {
            error!("❌ 容器 {} 重启失败: {}", container_name, e);
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
pub async fn check_docker_services_status_with_project(app: &CliApp, project_name: Option<String>) -> Result<()> {
    info!("📊 检查 Docker 服务状态...");

    // 创建支持项目名称的 DockerService
    let docker_service_manager = if let Some(project_name) = project_name {
        let custom_docker_manager = std::sync::Arc::new(
            client_core::container::DockerManager::with_project(
                client_core::constants::docker::get_compose_file_path(),
                client_core::constants::docker::get_env_file_path(),
                Some(project_name),
            )?
        );
        DockerService::new(app.config.clone(), custom_docker_manager)?
    } else {
        DockerService::new(app.config.clone(), app.docker_manager.clone())?
    };

    match docker_service_manager.health_check().await {
        Ok(report) => {
            info!("=== Docker 服务状态报告 ===");
            info!(
                "检查时间: {}",
                report.check_time.format("%Y-%m-%d %H:%M:%S UTC")
            );
            info!("整体状态: {}", report.finalize().display_name());
            info!(
                "运行统计: {}/{} 个容器正在运行",
                report.get_running_count(), report.get_total_count()
            );

            if !report.containers.is_empty() {
                info!("容器详情:");
                for container in &report.containers {
                    let status_icon = match container.status {
                        ContainerStatus::Running => "🟢",
                        ContainerStatus::Stopped => "🔴",
                        ContainerStatus::Starting => "🟡",
                        ContainerStatus::Completed => "✅",
                        ContainerStatus::Unknown => "⚪",
                    };

                    info!(
                        "  {} {} ({})",
                        status_icon,
                        container.name,
                        container.status.display_name()
                    );
                    info!("     镜像: {}", container.image);

                    if !container.ports.is_empty() {
                        info!("     端口: {}", container.ports.join(", "));
                    }
                }
            }

            if !report.errors.is_empty() {
                warn!("⚠️ 错误信息:");
                for error in &report.errors {
                    warn!("  • {}", error);
                }
            }

            // 显示访问信息
            if report.finalize().is_healthy() {
                use client_core::constants::docker::ports;
                info!("🌐 服务访问信息:");
                info!(
                    "  • 前端页面: http://localhost:{}",
                    ports::DEFAULT_FRONTEND_PORT
                );
                info!(
                    "  • 后端API: http://localhost:{}",
                    ports::DEFAULT_BACKEND_PORT
                );
                info!(
                    "  • 管理界面: http://localhost:{} (如果配置)",
                    ports::DEFAULT_MINIO_API_PORT
                );
                info!("  📝 注意: 如果使用了自定义端口参数，请使用相应的端口访问");
            }
        }
        Err(e) => {
            error!("❌ 获取服务状态失败: {:?}", e);
            return Err(anyhow::anyhow!(format!("获取服务状态失败: {e:?}")));
        }
    }

    Ok(())
}

/// 加载 Docker 镜像
pub async fn load_docker_images(app: &CliApp) -> Result<()> {
    info!("📦 加载 Docker 镜像...");

    let docker_service_manager =
        DockerService::new(app.config.clone(), app.docker_manager.clone())?;

    // 显示架构信息
    let arch = docker_service_manager.get_architecture();
    info!("当前系统架构: {}", arch.display_name());

    match docker_service_manager.load_images().await {
        Ok(result) => {
            info!("📦 镜像加载完成!");
            info!("  • 成功加载: {} 个镜像", result.success_count());
            info!("  • 加载失败: {} 个镜像", result.failure_count());

            if !result.loaded_images.is_empty() {
                info!("✅ 成功加载的镜像:");
                for image in &result.loaded_images {
                    info!("  • {}", image);
                }
            }

            if !result.failed_images.is_empty() {
                warn!("❌ 加载失败的镜像:");
                for (image, error) in &result.failed_images {
                    warn!("  • {}: {}", image, error);
                }
            }
        }
        Err(e) => {
            error!("❌ 镜像加载失败: {}", e);
            return Err(e.into());
        }
    }

    Ok(())
}

/// 设置镜像标签
pub async fn setup_image_tags(app: &CliApp) -> Result<()> {
    info!("🏷️ 设置镜像标签...");

    let docker_service_manager =
        DockerService::new(app.config.clone(), app.docker_manager.clone())?;

    // 先加载镜像以获取实际的镜像映射
    info!("📦 检查已加载的镜像...");
    let load_result = docker_service_manager.load_images().await?;

    if load_result.image_mappings.is_empty() {
        warn!("⚠️ 未找到已加载的镜像映射，请先运行 load-images 命令");
        return Ok(());
    }

    // 使用基于映射的新方法
    match docker_service_manager
        .setup_image_tags_with_mappings(&load_result.image_mappings)
        .await
    {
        Ok(result) => {
            info!("🏷️ 镜像标签设置完成!");
            info!("  • 成功设置: {} 个标签", result.success_count());
            info!("  • 设置失败: {} 个标签", result.failure_count());

            if !result.tagged_images.is_empty() {
                info!("✅ 成功设置的标签:");
                for (original, target) in &result.tagged_images {
                    info!("  • {} → {}", original, target);
                }
            }

            if !result.failed_tags.is_empty() {
                warn!("❌ 设置失败的标签:");
                for (original, target, error) in &result.failed_tags {
                    warn!("  • {} → {}: {}", original, target, error);
                }
            }
        }
        Err(e) => {
            error!("❌ 镜像标签设置失败: {}", e);
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
    let upgrade_file_zip: Option<PathBuf> = match &upgrade_strategy {
        UpgradeStrategy::FullUpgrade {
            target_version,
            download_type,
            ..
        } => {
            // 强制升级策略，直接解压并覆盖现有文件
            info!("📦 开始解压Docker服务包...");

            let base_version = target_version.base_version_string();

            let zip_path = app.config.get_version_download_file_path(
                &base_version,
                &download_type.to_string(),
                None,
            );
            Some(zip_path)
        }
        UpgradeStrategy::PatchUpgrade { target_version, .. } => {
            //增量升级
            let base_version = target_version.base_version_string();
            let full_version = target_version.to_string();

            let zip_path = app.config.get_version_download_file_path(
                &base_version,
                &full_version.to_string(),
                None,
            );
            Some(zip_path)
        }
        UpgradeStrategy::NoUpgrade { .. } => {
            // 无需升级
            None
        }
    };

    // 检查文件是否存在
    if let Some(file_zip) = upgrade_file_zip {
        if !file_zip.exists() {
            error!("❌ Docker服务包文件不存在: {}", file_zip.display());
            return Err(anyhow::anyhow!(format!(
                "Docker服务包文件不存在: {}",
                file_zip.display()
            )));
        }

        info!("📦 找到Docker服务包: {}", file_zip.display());

        // 使用utils中的解压函数
        crate::utils::extract_docker_service(&file_zip, &upgrade_strategy).await?;

        info!("✅ Docker服务包解压完成");
    }
    Ok(())
}

/// 获取系统架构信息
pub async fn show_architecture_info(_app: &CliApp) -> Result<()> {
    let arch = crate::docker_service::get_system_architecture();

    info!("🔧 系统架构信息:");
    info!("  • 架构类型: {}", arch.display_name());
    info!("  • 架构标识: {}", arch.as_str());
    info!(
        "  • 镜像后缀: {}",
        crate::docker_service::get_architecture_suffix(arch)
    );

    Ok(())
}

/// 设置frontend服务端口（使用新的环境变量管理器）
async fn set_frontend_port(port: u16) -> Result<()> {
    use crate::utils::env_manager::update_frontend_port;
    use client_core::constants::docker::get_env_file_path;

    let env_file_path = get_env_file_path();
    if !env_file_path.exists() {
        info!("   .env文件不存在，无需更新端口");
        return Ok(());
    }

    info!("🔧 开始更新.env文件中的前端端口: {}", port);
    info!("   .env文件路径: {}", env_file_path.display());

    // 使用新的环境变量管理器进行智能更新
    if let Err(e) = update_frontend_port(&env_file_path, port) {
        error!("❌ 更新端口配置失败: {}", e);
        return Err(anyhow::anyhow!("更新端口配置失败: {}", e));
    }

    info!("✅ 端口配置更新成功!");
    Ok(())
}
