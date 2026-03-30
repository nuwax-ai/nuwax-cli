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
        info!("{}", t!("docker_service_manager.deploy_start"));

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

        info!("{}", t!("docker_service_manager.deploy_done"));
        Ok(())
    }

    /// 环境检查
    pub async fn check_environment(&self) -> DockerServiceResult<()> {
        info!("{}", t!("docker_service_manager.check_env_start"));

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
            info!(
                "{}",
                t!("docker_service_manager.runtime_env_special", env = runtime_env.summary())
            );
        } else {
            info!(
                "{}",
                t!("docker_service_manager.runtime_env_normal", env = runtime_env.summary())
            );
        }

        info!("{}", t!("docker_service_manager.check_env_done"));
        Ok(())
    }

    /// 检查并创建 docker-compose.yml 中所有挂载的目录
    pub async fn ensure_compose_mount_directories(&self) -> DockerServiceResult<()> {
        info!("{}", t!("docker_service_manager.ensure_mount_dirs_start"));

        // 使用新的环境检测机制
        let runtime_env = self.docker_manager.get_runtime_environment();

        if runtime_env.needs_special_handling() {
            info!("{}", t!("docker_service_manager.windows_podman_detected"));
            info!("{}", t!("docker_service_manager.windows_podman_hint"));
        }

        // 设置必要目录
        self.docker_manager
            .ensure_host_volumes_exist()
            .await
            .map_err(|err| DockerServiceError::DirectorySetup(err.to_string()))?;

        info!("{}", t!("docker_service_manager.ensure_mount_dirs_done"));
        Ok(())
    }

    /// 加载 Docker 镜像
    pub async fn load_images(&self) -> DockerServiceResult<LoadResult> {
        info!("{}", t!("docker_service_manager.load_images_start"));
        let result = self.image_loader.load_all_images().await?;

        if !result.is_all_successful() {
            warn!(
                "{}",
                t!(
                    "docker_service_manager.load_images_partial_failed",
                    success = result.success_count(),
                    failed = result.failure_count()
                )
            );
        }

        Ok(result)
    }

    /// 基于实际镜像映射设置标签
    pub async fn setup_image_tags_with_mappings(
        &self,
        image_mappings: &[(String, String)],
    ) -> DockerServiceResult<TagResult> {
        info!("{}", t!("docker_service_manager.setup_tags_start"));
        let result = self
            .image_loader
            .setup_image_tags_with_mappings(image_mappings)
            .await?;

        if !result.is_all_successful() {
            warn!(
                "{}",
                t!(
                    "docker_service_manager.setup_tags_partial_failed",
                    success = result.success_count(),
                    failed = result.failure_count()
                )
            );
        }

        Ok(result)
    }

    /// 基于 ducker 验证镜像后再设置标签（推荐使用）
    pub async fn setup_image_tags_with_ducker_validation(
        &self,
        image_mappings: &[(String, String)],
    ) -> DockerServiceResult<TagResult> {
        info!("{}", t!("docker_service_manager.setup_tags_with_validation_start"));
        let result = self
            .image_loader
            .setup_image_tags_with_validation(image_mappings)
            .await?;

        if !result.is_all_successful() {
            warn!(
                "{}",
                t!(
                    "docker_service_manager.setup_tags_partial_failed",
                    success = result.success_count(),
                    failed = result.failure_count()
                )
            );
        }

        Ok(result)
    }

    /// 使用 ducker 列出当前系统中的所有镜像
    pub async fn list_docker_images_with_ducker(&self) -> DockerServiceResult<Vec<String>> {
        info!("{}", t!("docker_service_manager.list_images_start"));
        self.image_loader.list_images_with_ducker().await
    }

    /// 启动所有服务
    pub async fn start_services(&mut self) -> DockerServiceResult<()> {
        info!("{}", t!("docker_service_manager.start_services"));

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
                info!("{}", t!("docker_service_manager.wait_services_ready"));
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
                        info!("{}", t!("docker_service_manager.all_services_started"));
                        self.print_service_status(&report).await;
                    }
                    Err(e) => {
                        warn!("{}", t!("docker_service_manager.wait_services_failed", error = e.to_string()));

                        // 即使超时也显示当前状态
                        if let Ok(report) = self.health_checker.health_check().await {
                            self.print_service_status_with_failures(&report).await;
                        }
                    }
                }

                Ok(())
            }
            Err(e) => {
                error!("{}", t!("docker_service_manager.compose_start_failed_checking_status"));
                error!("{}", t!("docker_service_manager.error_detail", error = format!("{e:?}")));

                // 基于 ducker 思路：即使 compose 失败，也要检查是否有部分容器成功启动
                match self.health_checker.health_check().await {
                    Ok(report) => {
                        if report.get_running_count() > 0 {
                            info!(
                                "{}",
                                t!(
                                    "docker_service_manager.partial_running_enter_health_check",
                                    running = report.get_running_count(),
                                    total = report.get_total_count()
                                )
                            );

                            // 有部分容器成功，进入健康检查阶段
                            let check_interval =
                                Duration::from_secs(timeout::HEALTH_CHECK_INTERVAL);

                            match self
                                .health_checker
                                .wait_for_services_ready(check_interval)
                                .await
                            {
                                Ok(final_report) => {
                                    info!("{}", t!("docker_service_manager.partial_final_success"));

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
                                    warn!("{}", t!("docker_service_manager.health_check_timeout_partial_running"));

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
                                    info!("{}", t!("docker_service_manager.logs_hint"));
                                    return Ok(()); // 部分成功，返回 Ok
                                }
                            }
                        } else {
                            error!("{}", t!("docker_service_manager.no_running_container_found"));
                            self.print_detailed_error_analysis(&report, &e.to_string())
                                .await;
                        }
                    }
                    Err(e) => {
                        error!("{}", t!("docker_service_manager.get_container_status_failed"));
                        error!("{}", t!("docker_service_manager.error_detail", error = format!("{e:?}")));
                    }
                }

                Err(DockerServiceError::ServiceManagement(e.to_string()))
            }
        }
    }

    /// 停止所有服务
    pub async fn stop_services(&self) -> DockerServiceResult<()> {
        info!("{}", t!("docker_service_manager.stop_services"));

        // 直接使用已配置的 DockerManager，无需切换目录
        let result = self.docker_manager.stop_services().await;

        match result {
            Ok(_) => {
                info!("{}", t!("docker_service_manager.stop_services_success"));
                Ok(())
            }
            Err(e) => {
                error!("{}", t!("docker_service_manager.stop_services_failed", error = e.to_string()));
                Err(DockerServiceError::ServiceManagement(e.to_string()))
            }
        }
    }

    /// 重启所有服务
    pub async fn restart_services(&mut self) -> DockerServiceResult<()> {
        info!("{}", t!("docker_service_manager.restart_services"));

        // 先停止服务
        self.stop_services().await?;

        // 等待一下确保服务完全停止
        tokio::time::sleep(Duration::from_secs(timeout::RESTART_INTERVAL)).await;

        // 重新启动服务（不重新部署，只是启动）
        self.start_services().await
    }

    /// 重启单个容器
    pub async fn restart_container(&self, container_name: &str) -> DockerServiceResult<()> {
        info!("{}", t!("docker_service_manager.restart_container", name = container_name));

        // 直接使用已配置的 DockerManager，无需切换目录
        let result = self.docker_manager.restart_service(container_name).await;

        match result {
            Ok(_) => {
                info!("{}", t!("docker_service_manager.restart_container_success", name = container_name));
                Ok(())
            }
            Err(e) => {
                error!(
                    "{}",
                    t!(
                        "docker_service_manager.restart_container_failed",
                        name = container_name,
                        error = e.to_string()
                    )
                );
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
        info!("{}", t!("docker_service_manager.status_overview_title"));
        info!(
            "{}",
            t!(
                "docker_service_manager.status_overall",
                status = report.finalize().display_name()
            )
        );
        info!(
            "{}",
            t!(
                "docker_service_manager.status_running_containers",
                running = report.get_running_count(),
                total = report.get_total_count()
            )
        );

        if !report.containers.is_empty() {
            info!("{}", t!("docker_service_manager.container_details_title"));
            for container in &report.containers {
                info!("{}", t!("docker_service_manager.container_detail_item",
                    name = container.name,
                    status = container.status.display_name(),
                    image = container.image
                ));
            }
        }

        if !report.errors.is_empty() {
            warn!("{}", t!("docker_service_manager.error_list_title"));
            for error in &report.errors {
                warn!("{}", t!("docker_service_manager.error_list_item", error = error));
            }
        }

        // 显示访问信息
        if report.finalize().is_healthy() {
            info!("{}", t!("docker_service_manager.access_info_title"));
            use client_core::constants::docker::ports;
            info!(
                "{}",
                t!("docker_service_manager.access_frontend", port = ports::DEFAULT_FRONTEND_PORT)
            );
            info!(
                "{}",
                t!("docker_service_manager.access_backend", port = ports::DEFAULT_BACKEND_PORT)
            );
            info!("{}", t!("docker_service_manager.access_done"));
        }
    }

    /// 打印包含失败信息的服务状态
    async fn print_service_status_with_failures(&self, report: &HealthReport) {
        info!("{}", t!("docker_service_manager.status_detail_title"));
        info!(
            "{}",
            t!(
                "docker_service_manager.status_overall",
                status = report.finalize().display_name()
            )
        );
        info!(
            "{}",
            t!(
                "docker_service_manager.status_health_summary",
                running = report.get_running_count(),
                total = report.get_total_count()
            )
        );

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
            info!("{}", t!("docker_service_manager.running_containers_title"));
            for container in running_containers {
                info!(
                    "{}",
                    t!(
                        "docker_service_manager.running_container_item",
                        name = container.name,
                        image = container.image
                    )
                );
            }
        }

        if !starting_containers.is_empty() {
            warn!("{}", t!("docker_service_manager.starting_containers_title"));
            for container in starting_containers {
                warn!("{}", t!("docker_service_manager.starting_container_item",
                    name = container.name,
                    status = container.status.display_name()
                ));
            }
        }

        if !failed_containers.is_empty() {
            error!("{}", t!("docker_service_manager.failed_containers_title"));
            for container in failed_containers {
                error!("{}", t!("docker_service_manager.failed_container_item",
                    name = container.name,
                    status = container.status.display_name(),
                    image = container.image
                ));

                // 提供针对性的建议
                self.print_container_troubleshooting(&container.name, &container.image)
                    .await;
            }
        }

        // 显示部分成功时的访问信息
        if report.get_running_count() > 0 {
            info!("{}", t!("docker_service_manager.available_access_title"));
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
                info!(
                    "{}",
                    t!("docker_service_manager.access_frontend", port = ports::DEFAULT_FRONTEND_PORT)
                );
            }
            if has_backend {
                info!(
                    "{}",
                    t!("docker_service_manager.access_backend", port = ports::DEFAULT_BACKEND_PORT)
                );
            }
            let failed_count = report
                .containers
                .iter()
                .filter(|c| !c.status.is_healthy() && !c.status.is_transitioning())
                .count();

            if failed_count == 0 {
                info!("{}", t!("docker_service_manager.all_services_started_brief"));
            } else {
                warn!("{}", t!("docker_service_manager.partial_failed_still_available"));
            }
        }
    }

    /// 打印详细的错误分析
    async fn print_detailed_error_analysis(&self, report: &HealthReport, original_error: &str) {
        error!("{}", t!("docker_service_manager.startup_failure_analysis_title"));

        // 检查是否有具体的容器失败
        let failed_containers: Vec<_> = report
            .containers
            .iter()
            .filter(|c| !c.status.is_healthy())
            .collect();

        if failed_containers.is_empty() {
            error!("{}", t!("docker_service_manager.get_container_status_failed"));
            error!("{}", t!("docker_service_manager.original_error", error = original_error));
            return;
        }

        error!(
            "{}",
            t!(
                "docker_service_manager.failed_container_count",
                failed = failed_containers.len(),
                total = report.get_total_count()
            )
        );

        for container in failed_containers {
            error!("{}", t!("docker_service_manager.separator_line"));
            error!("{}", t!("docker_service_manager.container_name", name = container.name));
            error!("{}", t!("docker_service_manager.container_image", image = container.image));
            error!(
                "{}",
                t!(
                    "docker_service_manager.container_current_status",
                    status = container.status.display_name()
                )
            );

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
            warn!("{}", t!("docker_service_manager.troubleshoot_analysis_title"));
            warn!("{}", t!("docker_service_manager.troubleshoot_gpu_issue_1"));
            warn!("{}", t!("docker_service_manager.troubleshoot_gpu_issue_2"));
            warn!("{}", t!("docker_service_manager.troubleshoot_solution_title"));
            warn!("{}", t!("docker_service_manager.troubleshoot_gpu_solution_1"));
            warn!("{}", t!("docker_service_manager.troubleshoot_gpu_solution_2"));
            warn!("{}", t!("docker_service_manager.troubleshoot_gpu_solution_3"));
        } else if image_name.contains("amd64") {
            warn!("{}", t!("docker_service_manager.troubleshoot_analysis_title"));
            warn!("{}", t!("docker_service_manager.troubleshoot_arch_issue"));
            warn!("{}", t!("docker_service_manager.troubleshoot_solution_title"));
            warn!("{}", t!("docker_service_manager.troubleshoot_arch_solution_1"));
            warn!("{}", t!("docker_service_manager.troubleshoot_arch_solution_2"));
        } else if container_name.contains("mysql") || container_name.contains("redis") {
            warn!("{}", t!("docker_service_manager.troubleshoot_analysis_title"));
            warn!("{}", t!("docker_service_manager.troubleshoot_db_issue"));
            warn!("{}", t!("docker_service_manager.troubleshoot_solution_title"));
            warn!("{}", t!("docker_service_manager.troubleshoot_db_solution_1"));
            warn!("{}", t!("docker_service_manager.troubleshoot_db_solution_2"));
        } else if container_name.contains("backend") || container_name.contains("entrypoint") {
            warn!("{}", t!("docker_service_manager.troubleshoot_analysis_title"));
            warn!("{}", t!("docker_service_manager.troubleshoot_script_issue"));
            warn!("{}", t!("docker_service_manager.troubleshoot_solution_title"));
            warn!("{}", t!("docker_service_manager.troubleshoot_script_solution_1"));
            warn!("{}", t!("docker_service_manager.troubleshoot_script_solution_2"));
            warn!("{}", t!("docker_service_manager.troubleshoot_view_logs", name = container_name));
        } else {
            warn!("{}", t!("docker_service_manager.troubleshoot_generic_title"));
            warn!("{}", t!("docker_service_manager.troubleshoot_view_logs", name = container_name));
            warn!("{}", t!("docker_service_manager.troubleshoot_generic_1"));
            warn!("{}", t!("docker_service_manager.troubleshoot_generic_2"));
        }
    }

    /// 分析 Docker 错误信息
    async fn analyze_docker_error(&self, error_message: &str) {
        error!("{}", t!("docker_service_manager.separator_line"));
        error!("{}", t!("docker_service_manager.error_analysis_title"));

        let mut has_issues = false;

        if error_message.contains("nvidia") {
            error!("{}", t!("docker_service_manager.error_nvidia_issue"));
            error!("{}", t!("docker_service_manager.error_nvidia_hint_1"));
            error!("{}", t!("docker_service_manager.error_nvidia_hint_2"));
            has_issues = true;
        }

        if error_message.contains("platform")
            && error_message.contains("amd64")
            && error_message.contains("arm64")
        {
            error!("{}", t!("docker_service_manager.error_arch_issue"));
            error!("{}", t!("docker_service_manager.error_arch_hint_1"));
            error!("{}", t!("docker_service_manager.error_arch_hint_2"));
            has_issues = true;
        }

        if error_message.contains("Permission denied") && error_message.contains("entrypoint") {
            error!("{}", t!("docker_service_manager.error_script_permission_issue"));
            error!("{}", t!("docker_service_manager.error_script_permission_hint_1"));
            error!("{}", t!("docker_service_manager.error_script_permission_hint_2"));
            has_issues = true;
        }

        if error_message.contains("port") || error_message.contains("bind") {
            error!("{}", t!("docker_service_manager.error_port_bind_issue"));
            error!("{}", t!("docker_service_manager.error_port_bind_hint_1"));
            error!("{}", t!("docker_service_manager.error_port_bind_hint_2"));
            has_issues = true;
        }

        if !has_issues {
            error!("{}", t!("docker_service_manager.error_unknown_type"));
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
                    error!("{}", t!("docker_service_manager.error_line_item", line = line.trim()));
                }
            } else {
                // 显示前几行作为备选
                for line in error_message.lines().take(3) {
                    if !line.trim().is_empty() {
                        error!("{}", t!("docker_service_manager.error_line_item", line = line.trim()));
                    }
                }
            }
        }

        error!("{}", t!("docker_service_manager.separator_line"));
    }

    /// 检查端口冲突
    async fn check_port_conflicts(&mut self) -> DockerServiceResult<()> {
        let compose_file = self.docker_manager.get_compose_file();
        let env_file = self.docker_manager.get_env_file();
        if !compose_file.exists() {
            warn!("{}", t!("docker_service_manager.compose_file_missing_skip_port_check"));
            return Ok(());
        }

        info!("{}", t!("docker_service_manager.smart_port_check_start"));

        match self
            .port_manager
            .smart_check_compose_port_conflicts(&compose_file, &env_file)
            .await
        {
            Ok(report) => {
                if report.has_conflicts {
                    warn!("{}", t!("docker_service_manager.port_conflict_detected"));
                    self.port_manager.print_smart_conflict_report(&report);

                    // 对于Docker容器启动，我们采用更宽松的策略
                    // Docker会在实际绑定时处理端口冲突，这里只是警告
                    warn!("{}", t!("docker_service_manager.port_conflict_note_title"));
                    warn!("{}", t!("docker_service_manager.port_conflict_note_1"));
                    warn!("{}", t!("docker_service_manager.port_conflict_note_2"));
                    warn!("{}", t!("docker_service_manager.port_conflict_note_3"));
                } else {
                    info!("{}", t!("docker_service_manager.port_check_passed"));
                    if report.total_checked > 0 {
                        info!("{}", t!("docker_service_manager.port_check_total", total = report.total_checked));
                    }
                }
            }
            Err(e) => {
                warn!("{}", t!("docker_service_manager.port_check_failed_continue", error = e.to_string()));
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

        info!("{}", t!("docker_service_manager.mysql_container_found", count = mysql_containers.len()));
        for container in &mysql_containers {
            info!("{}", t!("docker_service_manager.mysql_container_item",
                name = container.name,
                status = container.status.display_name(),
                image = container.image
            ));
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
            warn!("{}", t!("docker_service_manager.mysql_problem_detected_fixing"));

            for container in &problematic_mysql {
                warn!("{}", t!("docker_service_manager.mysql_problem_container_item",
                    name = container.name,
                    status = container.status.display_name()
                ));
            }

            // 调用权限修复
            if let Err(e) = self
                .directory_permission_manager
                .fix_mysql_permissions_on_failure()
            {
                error!("{}", t!("docker_service_manager.mysql_fix_failed", error = e.to_string()));
                return Err(e);
            }

            info!("{}", t!("docker_service_manager.mysql_fix_done"));
            info!("{}", t!("docker_service_manager.mysql_fix_detail_title"));
            info!("{}", t!("docker_service_manager.mysql_fix_detail_1"));
            info!("{}", t!("docker_service_manager.mysql_fix_detail_2"));
            info!("{}", t!("docker_service_manager.mysql_fix_detail_3"));
            info!("{}", t!("docker_service_manager.mysql_fix_restart_hint"));

            Ok(())
        } else {
            Ok(()) // MySQL容器正常，无需修复
        }
    }
}
