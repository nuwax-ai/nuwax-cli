use super::types::DockerManager;
use anyhow::{Context as _, Result};
use std::path::Path;
use tracing::{debug, info, warn};

impl DockerManager {
    /// 加载 Docker 镜像，返回加载的镜像名称
    pub async fn load_image<P: AsRef<Path>>(&self, image_path: P) -> Result<String> {
        let image_path = image_path.as_ref();
        if !image_path.exists() {
            return Err(anyhow::anyhow!(
                "Image file does not exist: {}",
                image_path.display()
            ));
        }

        info!(
            "Running docker load command: docker load -i {}",
            image_path.display()
        );

        let output = self
            .run_docker_command(&["load", "-i", &image_path.to_string_lossy()])
            .await?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            let stdout = String::from_utf8_lossy(&output.stdout);
            warn!("docker load command failed:");
            warn!("  Exit status: {}", output.status);
            warn!("  stdout: {}", stdout);
            warn!("  stderr: {}", stderr);
            return Err(anyhow::anyhow!("Failed to load image: {stderr}"));
        }

        // 解析输出来获取实际加载的镜像名称
        let stdout = String::from_utf8_lossy(&output.stdout);
        debug!("docker load command completed successfully");
        debug!("Full stdout output:");
        for (i, line) in stdout.lines().enumerate() {
            debug!("  Line {}: {}", i + 1, line);
        }

        for line in stdout.lines() {
            if line.starts_with("Loaded image:")
                && let Some(image_name) = line.strip_prefix("Loaded image:").map(|s| s.trim())
            {
                info!("Parsed loaded image name successfully: {}", image_name);
                return Ok(image_name.to_string());
            }
        }

        // 如果没有找到"Loaded image:"，但命令成功了，返回一个默认值
        warn!("docker load succeeded but image name could not be parsed");
        warn!("Full output: {}", stdout);
        Err(anyhow::anyhow!(
            "Unable to parse docker load output: {stdout}"
        ))
    }

    /// 拉取最新镜像
    pub async fn pull_images(&self) -> Result<()> {
        // 跳过环境先决条件检查，避免 Docker 命令导致的高磁盘 IO
        // self.check_prerequisites().await?;

        let output = self.run_compose_command(&["pull"]).await?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(anyhow::anyhow!("Failed to pull images: {stderr}"));
        }

        Ok(())
    }

    /// 按完整引用从仓库拉取一个镜像。
    ///
    /// 与 `pull_images` 的区别是刻意的：`docker compose pull` 读磁盘上的 Compose，
    /// 而瘦通道必须在停服**之前**把候选包的镜像拉完，那时磁盘上还是旧 Compose。
    pub async fn pull_image(&self, reference: &str) -> Result<()> {
        let output = self
            .run_docker_command_with_timeout(
                &["pull", reference],
                std::time::Duration::from_secs(crate::constants::timeout::REGISTRY_PULL_TIMEOUT),
            )
            .await
            .with_context(|| format!("Failed to run docker pull for {reference}"))?;

        if !output.status.success() {
            return Err(anyhow::anyhow!(
                "docker pull {reference} failed: {}",
                command_error_summary(&output.stderr)
            ));
        }
        Ok(())
    }

    /// 本地镜像的身份（`docker image inspect` 的 `.Id`）；本地没有该引用时返回 None。
    ///
    /// 经典 image store 暴露的是 config digest，containerd store 可能暴露 OCI index
    /// digest，因此调用方要同时接受交付清单里的 `config_id` 与 `image_id`
    /// （与 `backup_release::validate_runtime_delivery` 同一套语义）。
    pub async fn image_identity(&self, reference: &str) -> Result<Option<String>> {
        let output = self
            .run_docker_command(&["image", "inspect", "--format", "{{.Id}}", reference])
            .await
            .with_context(|| format!("Failed to inspect local image {reference}"))?;

        if !output.status.success() {
            // 引用不存在与 docker daemon 不可用都走这里；把 stderr 带上，
            // 否则"daemon 没起来"会被上层误报成"本地缺镜像"。
            let stderr = command_error_summary(&output.stderr);
            if stderr.contains("No such image") || stderr.contains("not found") {
                return Ok(None);
            }
            return Err(anyhow::anyhow!(
                "docker image inspect {reference} failed: {stderr}"
            ));
        }
        let identity = String::from_utf8_lossy(&output.stdout).trim().to_string();
        if identity.is_empty() {
            return Err(anyhow::anyhow!(
                "docker image inspect {reference} returned an empty image id"
            ));
        }
        Ok(Some(identity))
    }

    /// 停服前的仓库可达性预检：客户机能不能解析并访问该引用所在的仓库。
    ///
    /// 失败要发生在停服之前，否则预检本身就把部署变成了停机故障。
    pub async fn registry_reachable(&self, reference: &str) -> Result<bool> {
        let output = self
            .run_docker_command_with_timeout(
                &["manifest", "inspect", reference],
                std::time::Duration::from_secs(crate::constants::timeout::REGISTRY_PROBE_TIMEOUT),
            )
            .await
            .with_context(|| format!("Failed to probe registry for {reference}"))?;

        if output.status.success() {
            return Ok(true);
        }
        let stderr = command_error_summary(&output.stderr);
        // 某些打包（老版本 docker / 部分 Podman CLI）没有启用 manifest 子命令，
        // 这是预检能力缺失，不是客户机连不上仓库；不能拿它挡部署。
        if stderr.contains("experimental") || stderr.contains("unknown shorthand") {
            warn!("Registry reachability pre-check is unavailable on this Docker CLI; skipping it");
            return Ok(true);
        }
        warn!("Registry pre-check failed for {reference}: {stderr}");
        Ok(false)
    }
}

/// 命令失败原因：压平空白、截断，避免把整段层进度或带凭据的输出灌进日志。
fn command_error_summary(stderr: &[u8]) -> String {
    let text = String::from_utf8_lossy(stderr);
    let collapsed = text
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .collect::<Vec<_>>()
        .join(" ");
    const LIMIT: usize = 400;
    if collapsed.len() <= LIMIT {
        return collapsed;
    }
    let mut end = LIMIT;
    while !collapsed.is_char_boundary(end) {
        end -= 1;
    }
    format!("{}…", &collapsed[..end])
}
