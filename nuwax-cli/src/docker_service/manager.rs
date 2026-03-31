use crate::docker_service::architecture::{Architecture, detect_architecture};
use crate::docker_service::directory_permissions::DirectoryPermissionManager;
use crate::docker_service::error::{DockerServiceError, DockerServiceResult};
use crate::docker_service::health_check::{HealthChecker, HealthReport};
use crate::docker_service::image_loader::{ImageLoader, LoadResult, TagResult};
use crate::docker_service::port_manager::PortManager;
use crate::docker_service::script_permissions::ScriptPermissionManager;

use client_core::config::AppConfig;
use client_core::constants::timeout;
use client_core::container::DockerManager;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;
use tracing::{error, info, warn};

/// Docker 服务管理器
pub struct DockerServiceManager {
    #[allow(dead_code)]
    config: Arc<AppConfig>,
    docker_manager: Arc<DockerManager>,
    work_dir: PathBuf,
    architecture: Architecture,
    image_loader: ImageLoader,
    health_checker: HealthChecker,
    port_manager: PortManager,
    script_permission_manager: ScriptPermissionManager,
    directory_permission_manager: DirectoryPermissionManager,
}

impl DockerServiceManager {
    /// 创建新的 Docker 服务管理器
    pub fn new(
        config: Arc<AppConfig>,
        docker_manager: Arc<DockerManager>,
        work_dir: PathBuf,
    ) -> Self {
        let architecture = detect_architecture();

        // 由于 DockerManager 实现了 Clone，我们可以安全地克隆它
        let image_loader = ImageLoader::new(docker_manager.clone(), work_dir.clone())
            .expect("Failed to create image loader");
        let health_checker = HealthChecker::new(docker_manager.clone());

        Self {
            config,
            docker_manager,
            work_dir: work_dir.clone(),
            architecture,
            image_loader,
            health_checker,
            port_manager: PortManager::new(),
            script_permission_manager: ScriptPermissionManager::new(work_dir.clone()),
            directory_permission_manager: DirectoryPermissionManager::new(work_dir.clone()),
        }
    }

    /// 获取当前系统架构
    pub fn get_architecture(&self) -> Architecture {
        self.architecture
    }

    /// 获取工作目录
    pub fn get_work_dir(&self) -> &PathBuf {
        &self.work_dir
    }

    /// 执行完整的服务部署流程
    pub async fn deploy_services(&mut self) -> DockerServiceResult<()> {
        info!("Starting Docker service deployment...");

        // 1. 环境检查
        self.check_environment().await?;

        // 2. 自动检测 docker-compose.yml 并创建所有挂载目录
        self.docker_manager
            .ensure_host_volumes_exist()
            .await
            .map_err(|err| DockerServiceError::DirectorySetup(err.to_string()))?;

        // 3. 设置 MySQL 配置文件权限（644）
        self.directory_permission_manager
            .ensure_mysql_config_safe()?;

        // 4. 检查和修复脚本权限
        self.script_permission_manager
            .check_and_fix_script_permissions()
            .await?;

        // 5. 加载镜像并获取映射信息
        let load_result = self.load_images().await?;

        // 6. 使用ducker验证并设置镜像标签（推荐方法）
        self.setup_image_tags_with_ducker_validation(&load_result.image_mappings)
            .await?;

        // 7. 启动服务
        self.start_services().await?;

        info!("Docker service deployment completed");
        Ok(())
    }

    /// 环境检查
    pub async fn check_environment(&self) -> DockerServiceResult<()> {
        info!("Checking Docker environment...");

        // 跳过 Docker 状态检查，避免高磁盘 IO 问题
        // Docker 29+ 版本中，docker info、docker --version 等命令会扫描大量数据
        // 导致部署过程中磁盘 IO 飙升，影响性能
        // 如果 Docker 未运行，后续操作会自然暴露错误
        // self.docker_manager
        //     .check_docker_status()
        //     .await
        //     .map_err(|e| DockerServiceError::EnvironmentCheck(e.to_string()))?;

        // 检查工作目录
        if !self.work_dir.exists() {
            return Err(DockerServiceError::EnvironmentCheck(format!(
                "{}",
                t!("docker_service_manager.work_dir_not_exists", path = self.work_dir.display())
            )));
        }

        // 检查镜像目录
        let images_dir = self
            .work_dir
            .join(client_core::constants::docker::IMAGES_DIR_NAME);
        if !images_dir.exists() {
            return Err(DockerServiceError::EnvironmentCheck(format!(
                "{}",
                t!("docker_service_manager.images_dir_not_exists", path = images_dir.display())
            )));
        }

        // 检查 docker-compose.yml
        let compose_file = self
            .work_dir
            .join(client_core::constants::docker::COMPOSE_FILE_NAME);
        if !compose_file.exists() {
            return Err(DockerServiceError::EnvironmentCheck(format!(
                "{}",
                t!("docker_service_manager.compose_file_not_exists", path = compose_file.display())
            )));
        }

        // 环境信息提示（新增）
        let runtime_env = self.docker_manager.get_runtime_environment();
        if runtime_env.needs_special_handling() {
            info!("   Environment: {env} - special handling required", env = runtime_env.summary());
        } else {
            info!("   Environment: {env}", env = runtime_env.summary());
        }

        info!("Environment check passed");
        Ok(())
    }

    /// 检查并创建 docker-compose.yml 中所有挂载的目录
    pub async fn ensure_compose_mount_directories(&self) -> DockerServiceResult<()> {
        info!("🔍 Checking and creating mount directories from docker-compose.yml...");

        // 使用新的环境检测机制
        let runtime_env = self.docker_manager.get_runtime_environment();

        if runtime_env.needs_special_handling() {
            info!("⚠️ Windows Podman Desktop environment detected");
            info!("   Podman Desktop does not auto-create mount directories, creating proactively");
        }

        // 设置必要目录
        self.docker_manager
            .ensure_host_volumes_exist()
            .await
            .map_err(|err| DockerServiceError::DirectorySetup(err.to_string()))?;

        info!("✅ Mount directory check completed");
        Ok(())
    }

    /// 加载 Docker 镜像
    pub async fn load_images(&self) -> DockerServiceResult<LoadResult> {
        info!("Starting Docker image loading...");
        let result = self.image_loader.load_all_images().await?;

        if !result.is_all_successful() {
            warn!("Some image loading failed: success {success}, failed {failed}", success = result.success_count(), failed = result.failure_count());
        }

        Ok(result)
    }

    /// 基于实际镜像映射设置标签
    pub async fn setup_image_tags_with_mappings(
        &self,
        image_mappings: &[(String, String)],
    ) -> DockerServiceResult<TagResult> {
        info!("Starting image tag setup...");
        let result = self
            .image_loader
            .setup_image_tags_with_mappings(image_mappings)
            .await?;

        if !result.is_all_successful() {
            warn!("Some tag setup failed: success {success}, failed {failed}", success = result.success_count(), failed = result.failure_count());
        }

        Ok(result)
    }

    /// 基于 ducker 验证镜像后再设置标签（推荐使用）
    pub async fn setup_image_tags_with_ducker_validation(
        &self,
        image_mappings: &[(String, String)],
    ) -> DockerServiceResult<TagResult> {
        info!("Starting validated image tag setup...");
        let result = self
            .image_loader
            .setup_image_tags_with_validation(image_mappings)
            .await?;

        if !result.is_all_successful() {
            warn!("Some tag setup failed: success {success}, failed {failed}", success = result.success_count(), failed = result.failure_count());
        }

        Ok(result)
    }

    /// 使用 ducker 列出当前系统中的所有镜像
    pub async fn list_docker_images_with_ducker(&self) -> DockerServiceResult<Vec<String>> {
        info!("Using ducker to list images...");
        self.image_loader.list_images_with_ducker().await
    }

    /// 启动所有服务
    pub async fn start_services(&mut self) -> DockerServiceResult<()> {
        info!("Starting Docker Compose services...");

        // 1. 检查和修复脚本权限
        self.script_permission_manager
            .check_and_fix_script_permissions()
            .await?;

        // 2. 自动检测 docker-compose.yml 并创建所有挂载目录
        self.docker_manager
            .ensure_host_volumes_exist()
            .await
            .map_err(|err| DockerServiceError::DirectorySetup(err.to_string()))?;

        // 3. 设置 MySQL 配置文件权限（644）
        self.directory_permission_manager
            .ensure_mysql_config_safe()?;

        // 3. 检查端口冲突
        self.check_port_conflicts().await?;

        // 直接使用已配置的 DockerManager，无需切换目录
        let result = self.docker_manager.start_services().await;

        match result {
            Ok(_) => {
                // 等待服务就绪
                info!("Waiting for services to become ready...");
                let check_interval = Duration::from_secs(timeout::HEALTH_CHECK_INTERVAL);

                // 提前检查MySQL状态，如果发现问题立即修复
                // tokio::time::sleep(Duration::from_secs(10)).await; // 等待10秒让容器启动

                // // 检查并修复MySQL配置文件权限
                // if let Err(e) = self
                //     .directory_permission_manager
                match self
                    .health_checker
                    .wait_for_services_ready(check_interval)
                    .await
                {
                    Ok(report) => {
                        info!("All services started successfully!");
                        self.print_service_status(&report).await;
                    }
                    Err(e) => {
warn!("Wait for services failed or timed out: {error}", error = e.to_string());

                        // 即使超时也显示当前状态
                        if let Ok(report) = self.health_checker.health_check().await {
                            self.print_service_status_with_failures(&report).await;
                        }
                    }
                }

                Ok(())
            }
            Err(e) => {
                error!("Docker Compose start command failed, checking container status...");
error!("Error detail: {error}", error = format!("{e:?}"));

                // 基于 ducker 思路：即使 compose 失败，也要检查是否有部分容器成功启动
                match self.health_checker.health_check().await {
                    Ok(report) => {
                        if report.get_running_count() > 0 {
                            info!("🔍 {running}/{total} containers are running, entering health-check phase", running = report.get_running_count(), total = report.get_total_count());

                            // 有部分容器成功，进入健康检查阶段
                            let check_interval =
                                Duration::from_secs(timeout::HEALTH_CHECK_INTERVAL);

                            match self
                                .health_checker
                                .wait_for_services_ready(check_interval)
                                .await
                            {
                                Ok(final_report) => {
                                    info!("🎉 Some services eventually started successfully!");

                                    // // 执行容器启动后权限维护
                                    // if let Err(e) = self
                                    //     .directory_permission_manager
                                    //     .post_container_start_maintenance()
                                    //     .await
                                    // {
                                    //     warn!("容器启动后权限维护失败: {}", e);
                                    // }

                                    self.print_service_status(&final_report).await;
                                    return Ok(()); // 部分成功，返回 Ok
                                }
                                Err(_health_error) => {
                                    warn!("⏰ Health check timed out, but some services are still running");

                                    // // 检查MySQL容器状态，如果失败尝试权限修复
                                    // if (self.check_and_fix_mysql_if_failed(&report).await).is_err()
                                    // {
                                    //     warn!("MySQL权限修复失败，但继续执行");
                                    // }

                                    // // 即使超时也执行权限维护
                                    // if let Err(e) = self
                                    //     .directory_permission_manager
                                    //     .post_container_start_maintenance()
                                    //     .await
                                    // {
                                    //     warn!("容器启动后权限维护失败: {}", e);
                                    // }

                                    self.print_service_status_with_failures(&report).await;
                                    info!("You can inspect logs: nuwax-cli docker-service logs [service]");
                                    return Ok(()); // 部分成功，返回 Ok
                                }
                            }
                        } else {
                            error!("No running containers found");
                            self.print_detailed_error_analysis(&report, &e.to_string())
                                .await;
                        }
                    }
                    Err(e) => {
                        error!("❌ Failed to get container status details");
error!("Error detail: {error}", error = format!("{e:?}"));
                    }
                }

                Err(DockerServiceError::ServiceManagement(e.to_string()))
            }
        }
    }

    /// 停止所有服务
    pub async fn stop_services(&self) -> DockerServiceResult<()> {
        info!("Stopping Docker Compose services...");

        // 直接使用已配置的 DockerManager，无需切换目录
        let result = self.docker_manager.stop_services().await;

        match result {
            Ok(_) => {
                info!("Services stopped successfully");
                Ok(())
            }
            Err(e) => {
error!("Failed to stop services: {error}", error = e.to_string());
                Err(DockerServiceError::ServiceManagement(e.to_string()))
            }
        }
    }

    /// 重启所有服务
    pub async fn restart_services(&mut self) -> DockerServiceResult<()> {
        info!("Restarting Docker Compose services...");

        // 先停止服务
        self.stop_services().await?;

        // 等待一下确保服务完全停止
        tokio::time::sleep(Duration::from_secs(timeout::RESTART_INTERVAL)).await;

        // 重新启动服务（不重新部署，只是启动）
        self.start_services().await
    }

    /// 重启单个容器
    pub async fn restart_container(&self, container_name: &str) -> DockerServiceResult<()> {
        info!("Restarting container: {name}", name = container_name);

        // 直接使用已配置的 DockerManager，无需切换目录
        let result = self.docker_manager.restart_service(container_name).await;

        match result {
            Ok(_) => {
                info!("Container {name} restarted successfully", name = container_name);
                Ok(())
            }
            Err(e) => {
                error!("Container {name} restart failed: {error}", name = container_name, error = e.to_string());
                Err(DockerServiceError::ServiceManagement(e.to_string()))
            }
        }
    }

    /// 执行健康检查
    pub async fn health_check(&self) -> DockerServiceResult<HealthReport> {
        self.health_checker.health_check().await
    }

    /// 获取服务状态摘要
    pub async fn get_status_summary(&self) -> DockerServiceResult<String> {
        self.health_checker.get_status_summary().await
    }

    /// 打印服务状态信息
    async fn print_service_status(&self, report: &HealthReport) {
        info!("=== Service Status Overview ===");
        info!("Overall status: {status}", status = report.finalize().display_name());
        info!("Running containers: {running}/{total}", running = report.get_running_count(), total = report.get_total_count());

        if !report.containers.is_empty() {
            info!("Container details:");
            for container in &report.containers {
                info!(
                    "  • {name} - {status} ({image})",
                    name = container.name,
                    status = container.status.display_name(),
                    image = container.image
                );
            }
        }

        if !report.errors.is_empty() {
            warn!("Errors:");
            for error in &report.errors {
                warn!("  • {error}", error = error);
            }
        }

        // 显示访问信息
        if report.finalize().is_healthy() {
            info!("=== Service Access Info ===");
            use client_core::constants::docker::ports;
            info!("• Frontend: http://localhost:{port}", port = ports::DEFAULT_FRONTEND_PORT);
            info!("• Backend API: http://localhost:{port}", port = ports::DEFAULT_BACKEND_PORT);
            info!("• Service management complete. Ready to use.");
        }
    }

    /// 打印包含失败信息的服务状态
    async fn print_service_status_with_failures(&self, report: &HealthReport) {
        info!("=== Service Status Details ===");
        info!("Overall status: {status}", status = report.finalize().display_name());
        info!("Health summary: {running}/{total} containers healthy", running = report.get_running_count(), total = report.get_total_count());

        // 分类显示容器状态
        let running_containers: Vec<_> = report
            .containers
            .iter()
            .filter(|c| c.status.is_healthy())
            .collect();
        let failed_containers: Vec<_> = report
            .containers
            .iter()
            .filter(|c| !c.status.is_healthy() && !c.status.is_transitioning())
            .collect();
        let starting_containers: Vec<_> = report
            .containers
            .iter()
            .filter(|c| c.status.is_transitioning())
            .collect();

        if !running_containers.is_empty() {
            info!("✅ Running containers:");
            for container in running_containers {
                info!("  • {name} ({image})",
                        name = container.name,
                        image = container.image
                    );
            }
        }

        if !starting_containers.is_empty() {
            warn!("🔄 Starting containers:");
            for container in starting_containers {
                warn!(
                    "  • {name} - {status}",
                    name = container.name,
                    status = container.status.display_name()
                );
            }
        }

        if !failed_containers.is_empty() {
            error!("❌ Failed containers:");
            for container in failed_containers {
                error!(
                    "  • {name} - {status} ({image})",
                    name = container.name,
                    status = container.status.display_name(),
                    image = container.image
                );

                // 提供针对性的建议
                self.print_container_troubleshooting(&container.name, &container.image)
                    .await;
            }
        }

        // 显示部分成功时的访问信息
        if report.get_running_count() > 0 {
            info!("=== Available Service Access Info ===");
            use client_core::constants::docker::ports;

            let has_frontend = report
                .containers
                .iter()
                .any(|c| c.status.is_healthy() && c.name.contains("frontend"));
            let has_backend = report
                .containers
                .iter()
                .any(|c| c.status.is_healthy() && c.name.contains("backend"));

            if has_frontend {
                info!("• Frontend: http://localhost:{port}", port = ports::DEFAULT_FRONTEND_PORT);
            }
            if has_backend {
                info!("• Backend API: http://localhost:{port}", port = ports::DEFAULT_BACKEND_PORT);
            }
            let failed_count = report
                .containers
                .iter()
                .filter(|c| !c.status.is_healthy() && !c.status.is_transitioning())
                .count();

            if failed_count == 0 {
                info!("• All services are running normally!");
            } else {
                warn!("• Some services failed, but available services remain usable");
            }
        }
    }

    /// 打印详细的错误分析
    async fn print_detailed_error_analysis(&self, report: &HealthReport, original_error: &str) {
        error!("=== Startup Failure Analysis ===");

        // 检查是否有具体的容器失败
        let failed_containers: Vec<_> = report
            .containers
            .iter()
            .filter(|c| !c.status.is_healthy())
            .collect();

        if failed_containers.is_empty() {
            error!("❌ Failed to get container status details");
            error!("❌ Original error: {error}", error = original_error);
            return;
        }

        error!("❌ Failed containers: {failed}/{total}", failed = failed_containers.len(), total = report.get_total_count());

        for container in failed_containers {
            error!("━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━");
            error!("Container: {name}", name = container.name);
            error!("Image: {image}", image = container.image);
            error!("Current status: {status}", status = container.status.display_name());

            // 提供针对性的故障排除建议
            self.print_container_troubleshooting(&container.name, &container.image)
                .await;
        }

        // 分析原始错误中的关键信息
        self.analyze_docker_error(original_error).await;
    }

    /// 打印容器故障排除建议
    async fn print_container_troubleshooting(&self, container_name: &str, image_name: &str) {
        if container_name.contains("video-analysis-worker") {
            warn!("💡 Analysis:");
            warn!("  - This container requires NVIDIA GPU support, which may be unavailable on this system");
            warn!("  - Architecture mismatch detected (amd64 vs arm64)");
            warn!("💡 Suggested fix:");
            warn!("  - On Mac ARM64, disable this service or use an ARM64 image");
            warn!("  - Comment out this service in docker-compose.yml");
            warn!("  - Or update image version in .env to an ARM64 variant");
        } else if image_name.contains("amd64") {
            warn!("💡 Analysis:");
            warn!("  - Architecture mismatch: image is amd64 but system is arm64");
            warn!("💡 Suggested fix:");
            warn!("  - Use an arm64 image");
            warn!("  - Or add --platform linux/amd64 when running container");
        } else if container_name.contains("mysql") || container_name.contains("redis") {
            warn!("💡 Analysis:");
            warn!("  - Database startup failed, likely due to port conflict or data directory permissions");
            warn!("💡 Suggested fix:");
            warn!("  - Check whether port 3306(MySQL) or 6379(Redis) is occupied");
            warn!("  - Check directory permissions: ./data/mysql or ./data/redis");
        } else if container_name.contains("backend") || container_name.contains("entrypoint") {
            warn!("💡 Analysis:");
            warn!("  - Container startup script may be missing execute permission");
            warn!("💡 Suggested fix:");
            warn!("  - Check permissions for scripts like docker-entrypoint.sh");
            warn!("  - Run: chmod +x config/docker-entrypoint.sh");
            warn!("  - View logs: docker-compose logs {name}", name = container_name);
        } else {
            warn!("💡 Suggestion:");
            warn!("  - View logs: docker-compose logs {name}", name = container_name);
            warn!("  - Verify images were pulled successfully");
            warn!("  - Verify environment variables");
        }
    }

    /// 分析 Docker 错误信息
    async fn analyze_docker_error(&self, error_message: &str) {
        error!("━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━");
        error!("🔍 Error analysis:");

        let mut has_issues = false;

        if error_message.contains("nvidia") {
            error!("  ❌ NVIDIA GPU driver issue");
            error!("  💡 NVIDIA GPU may be unsupported or driver not installed");
            error!("  💡 Consider disabling services that require GPU");
            has_issues = true;
        }

        if error_message.contains("platform")
            && error_message.contains("amd64")
            && error_message.contains("arm64")
        {
            error!("  ❌ Container architecture mismatch");
            error!("  💡 amd64 image cannot run natively on arm64 system");
            error!("  💡 Use image that matches your architecture");
            has_issues = true;
        }

        if error_message.contains("Permission denied") && error_message.contains("entrypoint") {
            error!("  ❌ Script permission issue");
            error!("  💡 Startup script lacks execute permission");
            error!("  💡 Add execute permission with chmod +x");
            has_issues = true;
        }

        if error_message.contains("port") || error_message.contains("bind") {
            error!("  ❌ Port bind failed");
            error!("  💡 There may be a port conflict");
            error!("  💡 Check current port occupancy");
            has_issues = true;
        }

        if !has_issues {
            error!("  ❓ Unrecognized error type, key lines:");
            // 提取关键的错误行
            let key_lines: Vec<&str> = error_message
                .lines()
                .filter(|line| {
                    line.contains("Error")
                        || line.contains("failed")
                        || line.contains("denied")
                        || line.contains("not found")
                        || line.contains("connection")
                        || line.trim().starts_with("Container")
                })
                .take(5)
                .collect();

            if !key_lines.is_empty() {
                for line in key_lines {
error!("     {line}", line = line.trim());
                }
            } else {
                // 显示前几行作为备选
                for line in error_message.lines().take(3) {
                    if !line.trim().is_empty() {
error!("     {line}", line = line.trim());
                    }
                }
            }
        }

        error!("━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━");
    }

    /// 检查端口冲突
    async fn check_port_conflicts(&mut self) -> DockerServiceResult<()> {
        let compose_file = self.docker_manager.get_compose_file();
        let env_file = self.docker_manager.get_env_file();
        if !compose_file.exists() {
            warn!("docker-compose.yml not found, skipping port conflict check");
            return Ok(());
        }

        info!("🔍 Starting smart port-conflict check...");

        match self
            .port_manager
            .smart_check_compose_port_conflicts(&compose_file, &env_file)
            .await
        {
            Ok(report) => {
                if report.has_conflicts {
                    warn!("⚠️ Port conflict detected, proceeding with smart handling");
                    self.port_manager.print_smart_conflict_report(&report);

                    // 对于Docker容器启动，我们采用更宽松的策略
                    // Docker会在实际绑定时处理端口冲突，这里只是警告
                    warn!("💡 Note: Docker may handle port binding automatically");
                    warn!("   - If occupied by related service, container may reuse existing binding");
                    warn!("   - If occupied by unrelated service, startup may fail");
                    warn!("   - Check startup result and resolve conflicts manually if needed");
                } else {
                    info!("✅ Port check passed, no conflict found");
                    if report.total_checked > 0 {
                        info!("Checked {total} port mappings in total", total = report.total_checked);
                    }
                }
            }
            Err(e) => {
warn!("Port check failed: {error}, continuing startup", error = e.to_string());
                // 端口检查失败不应该阻止服务启动，只是警告
            }
        }

        Ok(())
    }

    /// 检查并修复MySQL容器启动失败的权限问题
    async fn check_and_fix_mysql_if_failed(
        &self,
        report: &HealthReport,
    ) -> DockerServiceResult<()> {
        // 检查是否有MySQL相关的容器启动失败
        let mysql_containers: Vec<_> = report
            .containers
            .iter()
            .filter(|container| {
                // 检查容器名是否包含mysql相关关键词
                let name = container.name.to_lowercase();
                name.contains("mysql")
                    || name.contains("db")
                    || (container.image.to_lowercase().contains("mysql"))
            })
            .collect();

        if mysql_containers.is_empty() {
            return Ok(()); // 没有MySQL容器，无需处理
        }

        info!("🔍 Found {count} MySQL-related containers", count = mysql_containers.len());
        for container in &mysql_containers {
            info!(
                "   - {name} (status: {status}, image: {image})",
                name = container.name,
                status = container.status.display_name(),
                image = container.image
            );
        }

        // 检查MySQL容器是否有启动失败的或处于重启状态
        let problematic_mysql = mysql_containers
            .iter()
            .filter(|container| {
                // 不健康的容器或者处于转换状态(如重启)的容器都需要修复
                !container.status.is_healthy() || container.status.is_transitioning()
            })
            .collect::<Vec<_>>();

        if !problematic_mysql.is_empty() {
            warn!("🔧 MySQL container issue detected, attempting permission fix...");

            for container in &problematic_mysql {
                warn!(
                    "   Problem container: {name} (status: {status})",
                    name = container.name,
                    status = container.status.display_name()
                );
            }

            // 调用权限修复
            if let Err(e) = self
                .directory_permission_manager
                .fix_mysql_permissions_on_failure()
            {
error!("MySQL permission fix failed: {error}", error = e.to_string());
                return Err(e);
            }

            info!("✅ MySQL permission fix completed");
            info!("💡 Fix actions:");
            info!("   - Clean potentially corrupted MySQL data files");
            info!("   - Set MySQL directory permission to 777 (recursive)");
            info!("   - Recreate required directory structure");
            info!("🔄 Wait for auto-restart or restart manually: nuwax-cli docker-service restart mysql");

            Ok(())
        } else {
            Ok(()) // MySQL容器正常，无需修复
        }
    }
}
