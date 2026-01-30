use super::environment::{
    ComposeCommandType, detect_compose_command_type, get_compose_command_type,
    set_compose_command_type,
};
use super::types::DockerManager;
use anyhow::Result;
use std::process::Stdio;
use tokio::process::Command;
use tracing::debug;

impl DockerManager {
    /// 执行 docker-compose 命令
    ///
    /// 根据全局检测结果选择使用 `docker compose`（新语法）或 `docker-compose`（旧语法）。
    /// 如果未提前检测，则先检测命令可用性，再执行实际操作。
    pub(crate) async fn run_compose_command(&self, args: &[&str]) -> Result<std::process::Output> {
        debug!("执行docker-compose命令: {:?}", args);

        // 获取已检测的命令类型
        let compose_type = get_compose_command_type();

        match compose_type {
            ComposeCommandType::DockerComposeSubcommand => {
                // 已确认支持 docker compose 子命令，直接使用
                debug!("使用已检测的 docker compose 子命令");
                self.run_docker_compose_subcommand_direct(args).await
            }
            ComposeCommandType::DockerComposeStandalone => {
                // 已确认只有 docker-compose 独立命令，直接使用
                debug!("使用已检测的 docker-compose 独立命令");
                self.run_docker_compose_standalone(args).await
            }
            ComposeCommandType::Unknown => {
                // 未检测，先检测再执行
                debug!("Compose 命令类型未检测，先检测命令可用性");
                self.run_compose_command_with_detection(args).await
            }
        }
    }

    /// 先检测命令可用性，再执行实际操作（避免先执行后回退的问题）
    async fn run_compose_command_with_detection(
        &self,
        args: &[&str],
    ) -> Result<std::process::Output> {
        // 再次检查全局状态（可能已被其他调用设置）
        let mut compose_type = get_compose_command_type();

        if compose_type == ComposeCommandType::Unknown {
            // 确实未检测，执行检测
            compose_type = detect_compose_command_type().await;

            // 只有检测到有效结果时才保存（避免 Unknown 被存储后每次都重新检测）
            if compose_type != ComposeCommandType::Unknown {
                set_compose_command_type(compose_type);
            }
        } else {
            debug!("Compose 命令类型已被其他调用设置: {:?}", compose_type);
        }

        match compose_type {
            ComposeCommandType::DockerComposeSubcommand => {
                debug!("使用 docker compose 子命令执行");
                self.run_docker_compose_subcommand_direct(args).await
            }
            ComposeCommandType::DockerComposeStandalone => {
                debug!("使用 docker-compose 独立命令执行");
                self.run_docker_compose_standalone(args).await
            }
            ComposeCommandType::Unknown => {
                // 两种命令都不可用
                Err(anyhow::anyhow!(
                    "未找到可用的 Docker Compose 命令，请确保已安装 Docker Compose"
                ))
            }
        }
    }

    /// 使用 docker compose 子命令（直接执行，不检查是否支持）
    async fn run_docker_compose_subcommand_direct(
        &self,
        args: &[&str],
    ) -> Result<std::process::Output> {
        let compose_path = self.compose_file.to_string_lossy().to_string();
        let mut cmd_args = vec!["compose"];

        // 如果指定了项目名称，添加 -p 参数
        if let Some(ref project_name) = self.project_name {
            cmd_args.extend(&["-p", project_name]);
        }

        cmd_args.extend(&["-f", &compose_path]);
        cmd_args.extend(args);

        debug!("执行 docker compose 子命令: {:?}", cmd_args);
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

        debug!("执行 docker-compose 独立命令: {:?}", cmd_args);
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
