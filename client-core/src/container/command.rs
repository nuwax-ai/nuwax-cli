use super::types::DockerManager;
use anyhow::Result;
use std::process::Stdio;
use tokio::process::Command;
use tracing::{debug, info, warn};

impl DockerManager {
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
