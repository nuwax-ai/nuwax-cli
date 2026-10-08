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
        debug!("Running docker-compose command: {:?}", args);

        // 获取已检测的命令类型
        let compose_type = get_compose_command_type();

        match compose_type {
            ComposeCommandType::DockerComposeSubcommand => {
                // 已确认支持 docker compose 子命令，直接使用
                debug!("Using detected docker compose subcommand");
                self.run_docker_compose_subcommand_direct(args).await
            }
            ComposeCommandType::DockerComposeStandalone => {
                // 已确认只有 docker-compose 独立命令，直接使用
                debug!("Using detected standalone docker-compose command");
                self.run_docker_compose_standalone(args).await
            }
            ComposeCommandType::Unknown => {
                // 未检测，先检测再执行
                debug!("Compose command type not detected; probing availability first");
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
            debug!(
                "Compose command type was already initialized by another call: {:?}",
                compose_type
            );
        }

        match compose_type {
            ComposeCommandType::DockerComposeSubcommand => {
                debug!("Executing via docker compose subcommand");
                self.run_docker_compose_subcommand_direct(args).await
            }
            ComposeCommandType::DockerComposeStandalone => {
                debug!("Executing via standalone docker-compose command");
                self.run_docker_compose_standalone(args).await
            }
            ComposeCommandType::Unknown => {
                // 两种命令都不可用
                Err(anyhow::anyhow!(
                    "No available Docker Compose command found; make sure Docker Compose is installed"
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

        let env_path = self.env_file.to_string_lossy().to_string();
        if self.env_file.exists() {
            cmd_args.extend(&["--env-file", &env_path]);
        }

        // 如果指定了项目名称，添加 -p 参数
        if let Some(ref project_name) = self.project_name {
            cmd_args.extend(&["-p", project_name]);
        }

        cmd_args.extend(&["-f", &compose_path]);
        cmd_args.extend(args);

        debug!("Executing docker compose subcommand: {:?}", cmd_args);
        self.run_compose_process("docker", &cmd_args).await
    }

    /// 使用独立的 docker-compose 命令
    async fn run_docker_compose_standalone(&self, args: &[&str]) -> Result<std::process::Output> {
        let compose_path = self.compose_file.to_string_lossy().to_string();
        let mut cmd_args: Vec<&str> = vec![];

        let env_path = self.env_file.to_string_lossy().to_string();
        if self.env_file.exists() {
            cmd_args.extend(&["--env-file", &env_path]);
        }

        // 如果指定了项目名称，添加 -p 参数
        if let Some(ref project_name) = self.project_name {
            cmd_args.extend(&["-p", project_name]);
        }

        cmd_args.extend(&["-f", &compose_path]);
        cmd_args.extend(args);

        debug!(
            "Executing standalone docker-compose command: {:?}",
            cmd_args
        );
        self.run_compose_process("docker-compose", &cmd_args).await
    }

    /// The file is authoritative for CLI-managed device information, even if
    /// this process inherited an older value before the file was repaired.
    fn apply_managed_compose_environment(&self, command: &mut Command) -> Result<()> {
        let values = super::config::load_env_values(&self.env_file)?;
        for key in crate::constants::device_info::ENV_MANAGED_KEYS {
            if let Some(value) = values.get(key) {
                command.env(key, value);
            } else {
                command.env_remove(key);
            }
        }
        Ok(())
    }

    async fn run_compose_process(
        &self,
        program: &str,
        cmd_args: &[&str],
    ) -> Result<std::process::Output> {
        let mut command = Command::new(program);
        self.apply_managed_compose_environment(&mut command)?;
        let output = command
            .args(cmd_args)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .output()
            .await?;

        Ok(output)
    }

    /// 执行 docker 命令
    pub(crate) async fn run_docker_command(&self, args: &[&str]) -> Result<std::process::Output> {
        debug!("Executing docker command: {:?}", args);
        let output = Command::new("docker")
            .args(args)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .output()
            .await?;

        Ok(output)
    }
}

#[cfg(test)]
mod managed_compose_environment_tests {
    use super::*;
    use std::collections::HashMap;
    use std::ffi::OsString;

    #[test]
    fn current_file_replaces_inherited_device_values_without_changing_other_overrides() -> Result<()>
    {
        let directory = tempfile::tempdir()?;
        let env_path = directory.path().join("custom.env");
        std::fs::write(
            &env_path,
            "DEVICE_ID=v1:current\nDEVICE_INFO_CPU_MODEL='Current CPU'\nUNRELATED=from-file\n",
        )?;
        let manager = DockerManager::with_project(
            directory.path().join("compose.yml"),
            env_path.clone(),
            None,
        )?;
        for program in ["docker", "docker-compose"] {
            let mut command = Command::new(program);
            command
                .env("DEVICE_ID", "v1:inherited-old")
                .env("DEVICE_FIELDS_DISK_SERIAL", "old-disk")
                .env("UNRELATED", "from-host");
            manager.apply_managed_compose_environment(&mut command)?;
            let overrides: HashMap<OsString, Option<OsString>> = command
                .as_std()
                .get_envs()
                .map(|(key, value)| (key.to_owned(), value.map(ToOwned::to_owned)))
                .collect();
            assert_eq!(
                overrides.get(&OsString::from("DEVICE_ID")),
                Some(&Some(OsString::from("v1:current")))
            );
            assert_eq!(
                overrides.get(&OsString::from("DEVICE_INFO_CPU_MODEL")),
                Some(&Some(OsString::from("Current CPU")))
            );
            assert_eq!(
                overrides.get(&OsString::from("DEVICE_FIELDS_DISK_SERIAL")),
                Some(&None)
            );
            assert_eq!(
                overrides.get(&OsString::from("UNRELATED")),
                Some(&Some(OsString::from("from-host")))
            );
        }
        std::fs::write(&env_path, "DEVICE_ID=v1:updated\n")?;
        let mut next = Command::new("docker");
        manager.apply_managed_compose_environment(&mut next)?;
        assert!(
            next.as_std()
                .get_envs()
                .any(|(key, value)| key == "DEVICE_ID"
                    && value == Some(std::ffi::OsStr::new("v1:updated")))
        );
        Ok(())
    }
}
