use super::types::DockerManager;
use anyhow::Result;
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
}
