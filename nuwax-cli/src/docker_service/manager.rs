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
        info!("开始 Docker 服务部署流程");

        // 1. 环境检查
        self.check_environment().await?;

        // 2. 设置必要目录
        self.docker_manager
            .ensure_host_volumes_exist()
            .await
            .map_err(|err| DockerServiceError::DirectorySetup(err.to_string()))?;

        // // 3. 渐进式权限管理（容器以原生用户运行，通过权限设置解决问题）
        // info!("🔧 应用渐进式权限管理...");
        // info!("   策略: 容器以原生用户运行，通过目录权限设置确保访问权限");
        // if let Err(e) = self
        //     .directory_permission_manager
        //     .progressive_permission_management()
        // {
        //     warn!("渐进式权限管理失败，回退到基础权限: {}", e);
        //     if let Err(fallback_err) = self.directory_permission_manager.basic_permission_fix() {
        //         error!("权限设置完全失败: {}", fallback_err);
        //         return Err(fallback_err);
        //     }
        // }

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

        info!("Docker 服务部署完成");
        Ok(())
    }

    /// 环境检查
    pub async fn check_environment(&self) -> DockerServiceResult<()> {
        info!("检查 Docker 环境...");

        // 检查 Docker 是否安装和运行
        self.docker_manager
            .check_docker_status()
            .await
            .map_err(|e| DockerServiceError::EnvironmentCheck(e.to_string()))?;

        // 检查工作目录
        if !self.work_dir.exists() {
            return Err(DockerServiceError::EnvironmentCheck(format!(
                "工作目录不存在: {}",
                self.work_dir.display()
            )));
        }

        // 检查镜像目录
        let images_dir = self
            .work_dir
            .join(client_core::constants::docker::IMAGES_DIR_NAME);
        if !images_dir.exists() {
            return Err(DockerServiceError::EnvironmentCheck(format!(
                "镜像目录不存在: {}",
                images_dir.display()
            )));
        }

        // 检查 docker-compose.yml
        let compose_file = self
            .work_dir
            .join(client_core::constants::docker::COMPOSE_FILE_NAME);
        if !compose_file.exists() {
            return Err(DockerServiceError::EnvironmentCheck(format!(
                "Docker Compose 配置文件不存在: {}",
                compose_file.display()
            )));
        }

        info!("环境检查通过");
        Ok(())
    }

    /// 检查并创建 docker-compose.yml 中所有挂载的目录
    pub async fn ensure_compose_mount_directories(&self) -> DockerServiceResult<()> {
        info!("🔍 检查并创建docker-compose.yml中的挂载目录...");

        // 设置必要目录
        self.docker_manager
            .ensure_host_volumes_exist()
            .await
            .map_err(|err| DockerServiceError::DirectorySetup(err.to_string()))?;

        info!("✅ 所有挂载目录检查完成");
        Ok(())
    }

    /// 加载 Docker 镜像
    pub async fn load_images(&self) -> DockerServiceResult<LoadResult> {
        info!("开始加载 Docker 镜像...");
        let result = self.image_loader.load_all_images().await?;

        if !result.is_all_successful() {
            warn!(
                "部分镜像加载失败: 成功 {}, 失败 {}",
                result.success_count(),
                result.failure_count()
            );
        }

        Ok(result)
    }

    /// 基于实际镜像映射设置标签
    pub async fn setup_image_tags_with_mappings(
        &self,
        image_mappings: &[(String, String)],
    ) -> DockerServiceResult<TagResult> {
        info!("开始设置镜像标签...");
        let result = self
            .image_loader
            .setup_image_tags_with_mappings(image_mappings)
            .await?;

        if !result.is_all_successful() {
            warn!(
                "部分标签设置失败: 成功 {}, 失败 {}",
                result.success_count(),
                result.failure_count()
            );
        }

        Ok(result)
    }

    /// 基于 ducker 验证镜像后再设置标签（推荐使用）
    pub async fn setup_image_tags_with_ducker_validation(
        &self,
        image_mappings: &[(String, String)],
    ) -> DockerServiceResult<TagResult> {
        info!("开始验证并设置镜像标签...");
        let result = self
            .image_loader
            .setup_image_tags_with_validation(image_mappings)
            .await?;

        if !result.is_all_successful() {
            warn!(
                "部分标签设置失败: 成功 {}, 失败 {}",
                result.success_count(),
                result.failure_count()
            );
        }

        Ok(result)
    }

    /// 使用 ducker 列出当前系统中的所有镜像
    pub async fn list_docker_images_with_ducker(&self) -> DockerServiceResult<Vec<String>> {
        info!("使用 ducker 获取镜像列表...");
        self.image_loader.list_images_with_ducker().await
    }

    /// 启动所有服务
    pub async fn start_services(&mut self) -> DockerServiceResult<()> {
        info!("启动 Docker Compose 服务...");

        // 1. 检查和修复脚本权限
        self.script_permission_manager
            .check_and_fix_script_permissions()
            .await?;

        // // 2. 应用渐进式权限管理（不修改docker-compose.yml）
        // if let Err(e) = self
        //     .directory_permission_manager
        //     .progressive_permission_management()
        // {
        //     warn!("渐进式权限管理失败: {}", e);
        //     // 回退到基础权限修复
        //     if let Err(e2) = self.directory_permission_manager.basic_permission_fix() {
        //         warn!("基础权限修复也失败: {}", e2);
        //     }
        // }

        // 3. 检查端口冲突
        self.check_port_conflicts().await?;

        // 直接使用已配置的 DockerManager，无需切换目录
        let result = self.docker_manager.start_services().await;

        match result {
            Ok(_) => {
                // 等待服务就绪
                info!("等待服务启动完成...");
                let check_interval = Duration::from_secs(timeout::HEALTH_CHECK_INTERVAL);

                // 提前检查MySQL状态，如果发现问题立即修复
                // tokio::time::sleep(Duration::from_secs(10)).await; // 等待10秒让容器启动

                // // 检查并修复MySQL配置文件权限
                // if let Err(e) = self
                //     .directory_permission_manager
                //     .check_and_fix_mysql_config_permissions()
                // {
                //     warn!("MySQL配置文件权限检查失败: {}", e);
                // }

                // if let Ok(initial_report) = self.health_checker.health_check().await {
                //     if (self.check_and_fix_mysql_if_failed(&initial_report).await).is_err() {
                //         warn!("MySQL初始权限修复失败，继续监控");
                //     }
                // }

                match self
                    .health_checker
                    .wait_for_services_ready(check_interval)
                    .await
                {
                    Ok(report) => {
                        info!("所有服务已成功启动!");

                        // // 执行容器启动后权限维护
                        // if let Err(e) = self
                        //     .directory_permission_manager
                        //     .post_container_start_maintenance()
                        //     .await
                        // {
                        //     warn!("容器启动后权限维护失败: {}", e);
                        //     // 不终止整个流程，只是记录警告
                        // }

                        self.print_service_status(&report).await;
                    }
                    Err(e) => {
                        warn!("等待服务启动超时或失败: {}", e);

                        // // 即使超时也执行权限维护，可能有助于解决问题
                        // if let Err(e) = self
                        //     .directory_permission_manager
                        //     .post_container_start_maintenance()
                        //     .await
                        // {
                        //     warn!("容器启动后权限维护失败: {}", e);
                        // }

                        // 即使超时也显示当前状态
                        if let Ok(report) = self.health_checker.health_check().await {
                            self.print_service_status_with_failures(&report).await;
                        }
                    }
                }

                Ok(())
            }
            Err(e) => {
                error!("Docker Compose 启动命令失败，正在检查容器状态...");
                error!("具体错误信息: {e:?}");

                // 基于 ducker 思路：即使 compose 失败，也要检查是否有部分容器成功启动
                match self.health_checker.health_check().await {
                    Ok(report) => {
                        if report.get_running_count() > 0 {
                            info!(
                                "🔍 发现 {}/{} 个容器正在运行，进入健康检查阶段",
                                report.get_running_count(),
                                report.get_total_count()
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
                                    info!("🎉 部分服务最终启动成功!");

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
                                    warn!("⏰ 健康检查超时，但有部分服务正在运行");

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
                                    info!(
                                        "你可以查看日志排查问题: nuwax-cli docker-service logs [服务名]"
                                    );
                                    return Ok(()); // 部分成功，返回 Ok
                                }
                            }
                        } else {
                            error!("没有发现运行中的容器");

                            // // 如果没有容器运行，特别检查MySQL是否因为权限问题失败
                            // if (self.check_and_fix_mysql_if_failed(&report).await).is_err() {
                            //     warn!("MySQL权限修复失败");
                            // }

                            self.print_detailed_error_analysis(&report, &e.to_string())
                                .await;
                        }
                    }
                    Err(e) => {
                        error!("❌ 无法获取容器状态信息");
                        error!("原始错误: {e:?}");
                    }
                }

                Err(DockerServiceError::ServiceManagement(e.to_string()))
            }
        }
    }

    /// 停止所有服务
    pub async fn stop_services(&self) -> DockerServiceResult<()> {
        info!("停止 Docker Compose 服务...");

        // 直接使用已配置的 DockerManager，无需切换目录
        let result = self.docker_manager.stop_services().await;

        match result {
            Ok(_) => {
                info!("服务已成功停止");
                Ok(())
            }
            Err(e) => {
                error!("服务停止失败: {}", e);
                Err(DockerServiceError::ServiceManagement(e.to_string()))
            }
        }
    }

    /// 重启所有服务
    pub async fn restart_services(&mut self) -> DockerServiceResult<()> {
        info!("重启 Docker Compose 服务...");

        // 先停止服务
        self.stop_services().await?;

        // 等待一下确保服务完全停止
        tokio::time::sleep(Duration::from_secs(timeout::RESTART_INTERVAL)).await;

        // 重新启动服务（不重新部署，只是启动）
        self.start_services().await
    }

    /// 重启单个容器
    pub async fn restart_container(&self, container_name: &str) -> DockerServiceResult<()> {
        info!("重启容器: {}", container_name);

        // 直接使用已配置的 DockerManager，无需切换目录
        let result = self.docker_manager.restart_service(container_name).await;

        match result {
            Ok(_) => {
                info!("容器 {} 重启成功", container_name);
                Ok(())
            }
            Err(e) => {
                error!("容器 {} 重启失败: {}", container_name, e);
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
        info!("=== 服务状态概览 ===");
        info!("整体状态: {}", report.finalize().display_name());
        info!(
            "运行中容器: {}/{}",
            report.get_running_count(),
            report.get_total_count()
        );

        if !report.containers.is_empty() {
            info!("容器详情:");
            for container in &report.containers {
                info!(
                    "  • {} - {} ({})",
                    container.name,
                    container.status.display_name(),
                    container.image
                );
            }
        }

        if !report.errors.is_empty() {
            warn!("错误信息:");
            for error in &report.errors {
                warn!("  • {}", error);
            }
        }

        // 显示访问信息
        if report.finalize().is_healthy() {
            info!("=== 服务访问信息 ===");
            use client_core::constants::docker::ports;
            info!(
                "• 前端页面: http://localhost:{}",
                ports::DEFAULT_FRONTEND_PORT
            );
            info!(
                "• 后端API: http://localhost:{}",
                ports::DEFAULT_BACKEND_PORT
            );
            info!("• 服务管理完成，可以开始使用!");
        }
    }

    /// 打印包含失败信息的服务状态
    async fn print_service_status_with_failures(&self, report: &HealthReport) {
        info!("=== 服务状态详情 ===");
        info!("整体状态: {}", report.finalize().display_name());
        info!(
            "运行状况: {}/{} 容器正常运行",
            report.get_running_count(),
            report.get_total_count()
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
            info!("✅ 正常运行的容器:");
            for container in running_containers {
                info!("  • {} ({})", container.name, container.image);
            }
        }

        if !starting_containers.is_empty() {
            warn!("🔄 正在启动的容器:");
            for container in starting_containers {
                warn!(
                    "  • {} - {}",
                    container.name,
                    container.status.display_name()
                );
            }
        }

        if !failed_containers.is_empty() {
            error!("❌ 启动失败的容器:");
            for container in failed_containers {
                error!(
                    "  • {} - {} ({})",
                    container.name,
                    container.status.display_name(),
                    container.image
                );

                // 提供针对性的建议
                self.print_container_troubleshooting(&container.name, &container.image)
                    .await;
            }
        }

        // 显示部分成功时的访问信息
        if report.get_running_count() > 0 {
            info!("=== 可用服务访问信息 ===");
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
                    "• 前端页面: http://localhost:{}",
                    ports::DEFAULT_FRONTEND_PORT
                );
            }
            if has_backend {
                info!(
                    "• 后端API: http://localhost:{}",
                    ports::DEFAULT_BACKEND_PORT
                );
            }
            let failed_count = report
                .containers
                .iter()
                .filter(|c| !c.status.is_healthy() && !c.status.is_transitioning())
                .count();

            if failed_count == 0 {
                info!("• 所有服务都已正常启动!");
            } else {
                warn!("• 部分服务启动失败，但可用服务仍可正常使用");
            }
        }
    }

    /// 打印详细的错误分析
    async fn print_detailed_error_analysis(&self, report: &HealthReport, original_error: &str) {
        error!("=== 服务启动失败分析 ===");

        // 检查是否有具体的容器失败
        let failed_containers: Vec<_> = report
            .containers
            .iter()
            .filter(|c| !c.status.is_healthy())
            .collect();

        if failed_containers.is_empty() {
            error!("❌ 无法获取具体的容器状态信息");
            error!("❌ 原始错误: {}", original_error);
            return;
        }

        error!(
            "❌ 失败的容器数量: {}/{}",
            failed_containers.len(),
            report.get_total_count()
        );

        for container in failed_containers {
            error!("━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━");
            error!("容器名称: {}", container.name);
            error!("镜像名称: {}", container.image);
            error!("当前状态: {}", container.status.display_name());

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
            warn!("💡 故障分析:");
            warn!("  - 该容器需要 NVIDIA GPU 支持，但当前系统可能不支持");
            warn!("  - 检测到架构不匹配问题 (amd64 vs arm64)");
            warn!("💡 解决建议:");
            warn!("  - 在 Mac ARM64 系统上，建议禁用此容器或使用 ARM64 镜像");
            warn!("  - 可以在 docker-compose.yml 中注释掉此服务");
            warn!("  - 或修改 .env 文件中的镜像版本为 arm64 版本");
        } else if image_name.contains("amd64") {
            warn!("💡 故障分析:");
            warn!("  - 架构不匹配: 镜像为 amd64，但系统为 arm64");
            warn!("💡 解决建议:");
            warn!("  - 使用 arm64 版本的镜像");
            warn!("  - 或在 docker run 时添加 --platform linux/amd64 参数");
        } else if container_name.contains("mysql") || container_name.contains("redis") {
            warn!("💡 故障分析:");
            warn!("  - 数据库服务启动失败，可能是端口冲突或数据目录权限问题");
            warn!("💡 解决建议:");
            warn!("  - 检查端口 3306(MySQL) 或 6379(Redis) 是否被占用");
            warn!("  - 检查数据目录权限: ./data/mysql 或 ./data/redis");
        } else if container_name.contains("backend") || container_name.contains("entrypoint") {
            warn!("💡 故障分析:");
            warn!("  - 容器启动脚本可能缺少执行权限");
            warn!("💡 解决建议:");
            warn!("  - 检查 docker-entrypoint.sh 等脚本的执行权限");
            warn!("  - 运行: chmod +x config/docker-entrypoint.sh");
            warn!("  - 查看容器日志: docker-compose logs {}", container_name);
        } else {
            warn!("💡 建议:");
            warn!("  - 查看容器日志: docker-compose logs {}", container_name);
            warn!("  - 检查镜像是否拉取成功");
            warn!("  - 验证环境变量配置");
        }
    }

    /// 分析 Docker 错误信息
    async fn analyze_docker_error(&self, error_message: &str) {
        error!("━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━");
        error!("🔍 错误信息分析:");

        let mut has_issues = false;

        if error_message.contains("nvidia") {
            error!("  ❌ NVIDIA GPU 驱动问题");
            error!("  💡 当前系统不支持 NVIDIA GPU 或驱动未安装");
            error!("  💡 建议禁用需要 GPU 的容器服务");
            has_issues = true;
        }

        if error_message.contains("platform")
            && error_message.contains("amd64")
            && error_message.contains("arm64")
        {
            error!("  ❌ 容器架构不匹配");
            error!("  💡 amd64 镜像无法在 arm64 系统上运行");
            error!("  💡 建议使用对应架构的镜像版本");
            has_issues = true;
        }

        if error_message.contains("Permission denied") && error_message.contains("entrypoint") {
            error!("  ❌ 脚本权限问题");
            error!("  💡 容器启动脚本没有执行权限");
            error!("  💡 建议为脚本文件添加执行权限: chmod +x");
            has_issues = true;
        }

        if error_message.contains("port") || error_message.contains("bind") {
            error!("  ❌ 端口绑定失败");
            error!("  💡 可能存在端口冲突");
            error!("  💡 建议检查端口占用情况");
            has_issues = true;
        }

        if !has_issues {
            error!("  ❓ 未识别的错误类型，查看关键错误信息:");
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
                    error!("     {}", line.trim());
                }
            } else {
                // 显示前几行作为备选
                for line in error_message.lines().take(3) {
                    if !line.trim().is_empty() {
                        error!("     {}", line.trim());
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
            warn!("docker-compose.yml 文件不存在，跳过端口冲突检查");
            return Ok(());
        }

        info!("🔍 开始智能端口冲突检查...");

        match self
            .port_manager
            .smart_check_compose_port_conflicts(&compose_file,&env_file)
            .await
        {
            Ok(report) => {
                if report.has_conflicts {
                    warn!("⚠️ 发现端口占用，但将智能处理");
                    self.port_manager.print_smart_conflict_report(&report);

                    // 对于Docker容器启动，我们采用更宽松的策略
                    // Docker会在实际绑定时处理端口冲突，这里只是警告
                    warn!("💡 注意: Docker容器启动时会自动处理端口绑定");
                    warn!("   - 如果端口被相关服务占用，容器会复用现有连接");
                    warn!("   - 如果端口被不相关服务占用，容器启动可能失败");
                    warn!("   - 建议检查服务启动结果，必要时手动处理端口冲突");
                } else {
                    info!("✅ 端口检查通过，没有发现冲突");
                    if report.total_checked > 0 {
                        info!("总计检查了 {} 个端口映射", report.total_checked);
                    }
                }
            }
            Err(e) => {
                warn!("端口冲突检查失败: {}，将继续启动服务", e);
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

        info!("🔍 检查到 {} 个MySQL相关容器", mysql_containers.len());
        for container in &mysql_containers {
            info!(
                "   - {} (状态: {}, 镜像: {})",
                container.name,
                container.status.display_name(),
                container.image
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
            warn!("🔧 检测到MySQL容器存在问题，尝试权限修复...");

            for container in &problematic_mysql {
                warn!(
                    "   问题容器: {} (状态: {})",
                    container.name,
                    container.status.display_name()
                );
            }

            // 调用权限修复
            if let Err(e) = self
                .directory_permission_manager
                .fix_mysql_permissions_on_failure()
            {
                error!("MySQL权限修复失败: {}", e);
                return Err(e);
            }

            info!("✅ MySQL权限修复完成");
            info!("💡 修复操作包括:");
            info!("   - 清理可能损坏的MySQL数据文件");
            info!("   - 设置MySQL目录权限为777（递归）");
            info!("   - 重新创建必要的目录结构");
            info!("🔄 建议等待容器自动重启或手动重启: nuwax-cli docker-service restart mysql");

            Ok(())
        } else {
            Ok(()) // MySQL容器正常，无需修复
        }
    }
}
