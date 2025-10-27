use super::types::DockerManager;
use anyhow::Result;
use std::path::Path;
use tracing::{debug, info, warn};

impl DockerManager {
    /// 加载 Docker 镜像，返回加载的镜像名称
    pub async fn load_image<P: AsRef<Path>>(&self, image_path: P) -> Result<String> {

        let image_path = image_path.as_ref();
        if !image_path.exists() {
            return Err(anyhow::anyhow!("镜像文件不存在: {}", image_path.display()));
        }

        info!(
            "执行docker load命令: docker load -i {}",
            image_path.display()
        );

        let output = self
            .run_docker_command(&["load", "-i", &image_path.to_string_lossy()])
            .await?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            let stdout = String::from_utf8_lossy(&output.stdout);
            warn!("docker load命令执行失败:");
            warn!("  状态码: {}", output.status);
            warn!("  stdout: {}", stdout);
            warn!("  stderr: {}", stderr);
            return Err(anyhow::anyhow!("加载镜像失败: {stderr}"));
        }

        // 解析输出来获取实际加载的镜像名称
        let stdout = String::from_utf8_lossy(&output.stdout);
        debug!("docker load命令成功执行");
        debug!("完整stdout输出:");
        for (i, line) in stdout.lines().enumerate() {
            debug!("  第{}行: {}", i + 1, line);
        }

        for line in stdout.lines() {
            if line.starts_with("Loaded image:") {
                if let Some(image_name) = line.strip_prefix("Loaded image:").map(|s| s.trim()) {
                    info!("成功解析加载的镜像名称: {}", image_name);
                    return Ok(image_name.to_string());
                }
            }
        }

        // 如果没有找到"Loaded image:"，但命令成功了，返回一个默认值
        warn!("docker load命令成功但无法解析镜像名称");
        warn!("完整输出: {}", stdout);
        Err(anyhow::anyhow!("无法解析docker load输出: {stdout}"))
    }

    /// 拉取最新镜像
    pub async fn pull_images(&self) -> Result<()> {
        self.check_prerequisites().await?;

        let output = self.run_compose_command(&["pull"]).await?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(anyhow::anyhow!("拉取镜像失败: {stderr}"));
        }

        Ok(())
    }
}
