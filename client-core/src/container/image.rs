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
            if is_missing_image_error(&stderr) {
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
    /// 只能当诊断用，不能当部署闸门：`docker manifest inspect` 不走 daemon 的
    /// registry-mirrors，实测（Docker 29.9.0 + 配置了镜像加速的主机）对 `docker.io` 引用
    /// 必然报 "error pinging v2 registry"，而同一台机器 `docker pull` 是成功的。
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

    /// 兜底内容判定：解析该引用现网 index 中 `os=linux` 且架构为 `arch` 的子 manifest
    /// 的 `config.digest`。
    ///
    /// 背景（2026-10-26 双机实测）：`:latest` 的 index 被 `docker manifest create/push`
    /// 重建后 digest 必然变化，而镜像内容不变；containerd 存储（Docker 28+ 默认）的
    /// `image inspect .Id` 恰好是 index digest，导致仅按 `.Id` 比对会把内容一致的镜像
    /// 误判成"被覆盖"。index 重序列化不改变子 manifest 与其 config.digest，所以内容
    /// 判定对这类漂移免疫。
    ///
    /// 返回 `Ok(None)` 表示解析不出（引用不是多平台 index、平台子 manifest 不唯一、
    /// 或 `buildx imagetools` 不可用/查询失败），调用方应维持原判定；任何失败都不
    /// 放行镜像。
    pub async fn index_config_digest(&self, reference: &str, arch: &str) -> Result<Option<String>> {
        let index_raw = self
            .run_docker_command_with_timeout(
                &["buildx", "imagetools", "inspect", "--raw", reference],
                std::time::Duration::from_secs(crate::constants::timeout::REGISTRY_PROBE_TIMEOUT),
            )
            .await
            .with_context(|| format!("Failed to inspect registry index for {reference}"))?;
        if !index_raw.status.success() {
            debug!(
                "Registry index query for {reference} failed: {}",
                command_error_summary(&index_raw.stderr)
            );
            return Ok(None);
        }
        let index: serde_json::Value = serde_json::from_slice(&index_raw.stdout)
            .with_context(|| format!("Registry returned malformed index JSON for {reference}"))?;
        let children = match index.get("manifests").and_then(|m| m.as_array()) {
            Some(children) => children,
            // 单平台 manifest：没有 index 漂移问题，原判定本来就够用。
            None => return Ok(None),
        };
        let child_digests: Vec<&str> = children
            .iter()
            .filter_map(|entry| {
                let platform = entry.get("platform")?;
                if platform.get("architecture")?.as_str()? != arch
                    || platform.get("os")?.as_str()? != "linux"
                {
                    return None;
                }
                entry.get("digest")?.as_str()
            })
            .collect();
        // 同架构多个 variant（如 arm64 v7/v8）时无法确定该信哪一个，保守放弃。
        let child_digest = match child_digests.as_slice() {
            [only] => *only,
            _ => return Ok(None),
        };
        let repo = manifest_repo_base(reference);
        let output = self
            .run_docker_command_with_timeout(
                &[
                    "buildx",
                    "imagetools",
                    "inspect",
                    "--raw",
                    &format!("{repo}@{child_digest}"),
                ],
                std::time::Duration::from_secs(crate::constants::timeout::REGISTRY_PROBE_TIMEOUT),
            )
            .await
            .with_context(|| format!("Failed to inspect manifest {repo}@{child_digest}"))?;
        if !output.status.success() {
            debug!(
                "Manifest query for {repo}@{child_digest} failed: {}",
                command_error_summary(&output.stderr)
            );
            return Ok(None);
        }
        let manifest: serde_json::Value =
            serde_json::from_slice(&output.stdout).with_context(|| {
                format!("Registry returned malformed manifest JSON for {child_digest}")
            })?;
        Ok(manifest
            .get("config")
            .and_then(|config| config.get("digest"))
            .and_then(|digest| digest.as_str())
            .map(str::to_string))
    }
}

/// 从镜像引用中取出仓库名（去掉 tag 或 digest 部分），供第二跳 `{repo}@{child}` 查询。
///
/// `registry:5000/img`（无 tag、宿主带端口）会在最右冒号处切错，此时 tag 段含 `/`，
/// 直接把整个引用当 repo（无 tag 引用拼 `@digest` 同样合法）。
fn manifest_repo_base(reference: &str) -> &str {
    if let Some((repo, _)) = reference.split_once('@') {
        return repo;
    }
    match reference.rsplit_once(':') {
        Some((repo, tag)) if !tag.contains('/') => repo,
        _ => reference,
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

/// 这条 inspect 失败是不是"本地没有这个引用"。
///
/// 措辞在实现之间不统一：Docker（实测 29.9.0）输出 `No such image`，podman 的 docker
/// 兼容层输出小写 `no such image`，另有 `not found` 变体。判定不中会把"本地缺镜像"
/// 升级成硬错误，而首次瘦部署每台机器都会走这条分支，所以按关键词大小写不敏感匹配。
/// 反向误判（把 daemon 故障读成缺镜像）才会多跑一次 pull 并在那里报出真正的错。
fn is_missing_image_error(stderr: &str) -> bool {
    let folded = stderr.to_ascii_lowercase();
    folded.contains("no such image") || folded.contains("not found")
}

#[cfg(test)]
mod tests {
    use super::is_missing_image_error;

    #[test]
    fn recognizes_each_missing_image_wording() {
        for stderr in [
            "Error response from daemon: No such image: nuwax-im:0.0.120",
            "Error: no such image: registry.example.cn/nuwax/nuwax-im:0.0.120",
            "Error response from daemon: image not found",
        ] {
            assert!(is_missing_image_error(stderr), "{stderr}");
        }
    }

    #[test]
    fn does_not_read_a_broken_daemon_as_a_missing_image() {
        for stderr in [
            "Cannot connect to the Docker daemon at unix:///var/run/docker.sock. Is the docker daemon running?",
            "permission denied while trying to connect to the Docker daemon socket",
        ] {
            assert!(
                !is_missing_image_error(stderr),
                "{stderr} 不能当成缺镜像，否则 daemon 故障会被报成拉取失败"
            );
        }
    }
}
