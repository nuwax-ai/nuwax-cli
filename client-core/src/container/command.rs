use super::types::DockerManager;
use anyhow::Result;
use std::process::Stdio;
use tokio::process::Command;
use tracing::{debug, info, warn};

impl DockerManager {
    /// 检查 Docker 状态
    pub async fn check_docker_status(&self) -> Result<()> {
        info!("🔍 检查Docker环境...");

        // 直接执行命令检查 Docker 是否可用
        debug!("检查Docker版本...");
        match Command::new("docker").args(["--version"]).output().await {
            Ok(output) if output.status.success() => {
                let version_output = String::from_utf8_lossy(&output.stdout);
                info!("✅ Docker版本: {}", version_output.trim());
            }
            Ok(output) => {
                let stderr = String::from_utf8_lossy(&output.stderr);
                warn!("❌ Docker版本检查失败: {}", stderr);
                return Err(anyhow::anyhow!("Docker 未安装或不在 PATH 中"));
            }
            Err(e) => {
                warn!("❌ Docker命令执行失败: {}", e);
                return Err(anyhow::anyhow!("Docker 未安装或不在 PATH 中"));
            }
        }

        // 检查 Docker 服务是否运行
        debug!("检查Docker服务状态...");
        info!("🔍 检查Docker服务运行状态...");
        let output = self.run_docker_command(&["info"]).await?;
        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            warn!("❌ Docker服务状态检查失败: {}", stderr);
            return Err(anyhow::anyhow!("Docker 服务未运行: {stderr}"));
        }

        info!("✅ Docker服务运行正常");
        Ok(())
    }

    /// 检查 Docker 和 Docker Compose 是否可用
    pub async fn check_prerequisites(&self) -> Result<()> {
        self.check_prerequisites_with_path(None).await
    }

    /// 检查 Docker 和 Docker Compose 是否可用（支持自定义路径）
    pub async fn check_prerequisites_with_path(&self, custom_compose_file: Option<&std::path::PathBuf>) -> Result<()> {
        info!("🔍 开始检查Docker环境先决条件...");

        // 首先检查 Docker Compose 文件是否存在
        let compose_file = custom_compose_file.unwrap_or(&self.compose_file);
        debug!("检查Docker Compose文件: {}", compose_file.display());
        if !compose_file.exists() {
            let error_msg = format!("Docker Compose 文件不存在: {}", compose_file.display());
            warn!("❌ {}", error_msg);
            return Err(anyhow::anyhow!(error_msg));
        }
        info!("✅ Docker Compose文件存在: {}", compose_file.display());

        // 检查 Docker 状态
        self.check_docker_status().await?;

        // 检查 docker-compose 或 docker compose 命令
        info!("🔍 检查Docker Compose命令可用性...");
        debug!("尝试检查docker-compose命令...");

        let standalone_available = Command::new("docker-compose")
            .args(["--version"])
            .output()
            .await
            .is_ok();
        let subcommand_available = self
            .run_docker_command(&["compose", "--version"])
            .await
            .is_ok();

        if standalone_available {
            info!("✅ 找到docker-compose独立命令");
        } else if subcommand_available {
            info!("✅ 找到docker compose子命令");
        } else {
            warn!("❌ Docker Compose命令不可用");
            return Err(anyhow::anyhow!("Docker Compose 未安装或不可用"));
        }

        info!("✅ Docker环境检查完成，所有先决条件满足");
        Ok(())
    }

    /// 执行 docker-compose 命令
    pub(crate) async fn run_compose_command(&self, args: &[&str]) -> Result<std::process::Output> {
        debug!("执行docker-compose命令: {:?}", args);

        // 尝试使用 docker compose（新语法）
        if let Ok(output) = self.run_docker_compose_subcommand(args).await {
            return Ok(output);
        }

        // 回退到 docker-compose（旧语法）
        self.run_docker_compose_standalone(args).await
    }

    /// 使用 docker compose 子命令
    async fn run_docker_compose_subcommand(&self, args: &[&str]) -> Result<std::process::Output> {
        let compose_path = self.compose_file.to_string_lossy().to_string();
        let mut cmd_args = vec!["compose"];

        // 如果指定了项目名称，添加 -p 参数
        if let Some(ref project_name) = self.project_name {
            cmd_args.extend(&["-p", project_name]);
        }

        cmd_args.extend(&["-f", &compose_path]);
        cmd_args.extend(args);

        debug!("尝试使用docker compose子命令: {:?}", cmd_args);
        self.run_docker_command(&cmd_args).await
    }

    /// 使用独立的 docker-compose 命令
    async fn run_docker_compose_standalone(&self, args: &[&str]) -> Result<std::process::Output> {
        let compose_path = self.compose_file.to_string_lossy().to_string();
        let mut cmd_args: Vec<&str> = vec![];

        // 如果指定了项目名称，添加 -p 参数
        if let Some(ref project_name) = self.project_name {
            cmd_args.extend(&["-p", project_name]);
        }

        cmd_args.extend(&["-f", &compose_path]);
        cmd_args.extend(args);

        debug!("尝试使用docker-compose独立命令: {:?}", cmd_args);
        let output = Command::new("docker-compose")
            .args(&cmd_args)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .output()
            .await?;

        Ok(output)
    }

    /// 执行 docker 命令
    pub(crate) async fn run_docker_command(&self, args: &[&str]) -> Result<std::process::Output> {
        debug!("执行docker命令: {:?}", args);
        let output = Command::new("docker")
            .args(args)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .output()
            .await?;

        Ok(output)
    }
}
