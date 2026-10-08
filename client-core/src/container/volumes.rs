use super::types::DockerManager;
use crate::DuckError;
use crate::container::path_utils::{PathProcessor, PathUtilsError};
use anyhow::Result;
use docker_compose_types as dct;
use std::path::Path;
use tracing::{debug, info, warn};

/// 挂载信息结构体
#[derive(Debug, Clone)]
pub struct MountInfo {
    pub service_name: String,
    pub container_path: String,
    pub host_path: Option<String>,
    pub is_bind_mount: bool,
}

// Drive-letter colons belong to a path, while later colons separate target/options.
fn short_volume_parts(spec: &str) -> Option<(&str, &str, Option<&str>)> {
    fn delimiter(path: &str) -> Option<usize> {
        let bytes = path.as_bytes();
        let drive_prefix = bytes.len() >= 3
            && bytes[0].is_ascii_alphabetic()
            && bytes[1] == b':'
            && matches!(bytes[2], b'/' | b'\\');
        let offset = if drive_prefix { 2 } else { 0 };
        path[offset..].find(':').map(|index| index + offset)
    }
    let source_end = delimiter(spec)?;
    let source = &spec[..source_end];
    let rest = &spec[source_end + 1..];
    let (target, mode) = match delimiter(rest) {
        Some(index) => {
            let mode = &rest[index + 1..];
            if mode.contains(':') {
                return None;
            }
            (&rest[..index], Some(mode))
        }
        None => (rest, None),
    };
    Some((source, target, mode))
}

impl DockerManager {
    /// 确保所有宿主机挂载目录存在
    pub async fn ensure_host_volumes_exist(&self) -> Result<()> {
        info!("🔍 Checking and creating host mount directories...");

        let compose_config = self.load_compose_config()?;
        let mount_directories = self.extract_mount_directories(&compose_config)?;

        if mount_directories.is_empty() {
            info!("✅ No host mount directories need to be created");
            return Ok(());
        }

        info!(
            "📁 Found {} mount directories to verify",
            mount_directories.len()
        );

        for mount_info in mount_directories {
            if let Some(host_path) = &mount_info.host_path
                && mount_info.is_bind_mount
            {
                self.create_host_directory_if_not_exists(host_path)?;
            }
        }

        info!("✅ Host mount directory verification completed");
        Ok(())
    }

    /// 从compose配置中提取挂载目录信息
    pub fn extract_mount_directories(&self, compose: &dct::Compose) -> Result<Vec<MountInfo>> {
        let mut mount_infos = Vec::new();

        for (service_name, service_opt) in &compose.services.0 {
            if let Some(service) = service_opt {
                let volumes = &service.volumes;
                for volume in volumes {
                    if let Some(mount_info) = self.parse_volume_spec(service_name, volume) {
                        mount_infos.push(mount_info);
                    }
                }
            }
        }

        Ok(mount_infos)
    }

    /// 解析单个volume规范
    fn parse_volume_spec(&self, service_name: &str, volume: &dct::Volumes) -> Option<MountInfo> {
        match volume {
            dct::Volumes::Simple(volume_str) => {
                let (host_path, container_path, _mode) = short_volume_parts(volume_str)?;

                let is_bind = self.is_bind_mount_path(host_path);

                if is_bind {
                    // 规范化路径（返回 Result）
                    let normalized_host_path = match self.normalize_path(host_path) {
                        Ok(path) => path,
                        Err(e) => {
                            warn!("Path normalization failed: {}", e);
                            return None;
                        }
                    };

                    // 将相对路径转换为相对于compose文件所在目录的绝对路径
                    let host_path_buf = std::path::PathBuf::from(&normalized_host_path);
                    let absolute_host_path = if host_path_buf.is_absolute() {
                        normalized_host_path
                    } else {
                        match self.get_working_directory() {
                            Some(compose_dir) => compose_dir
                                .join(&normalized_host_path)
                                .to_string_lossy()
                                .to_string(),
                            None => {
                                return None;
                            }
                        }
                    };
                    Some(MountInfo {
                        service_name: service_name.to_string(),
                        container_path: container_path.to_string(),
                        host_path: Some(absolute_host_path),
                        is_bind_mount: true,
                    })
                } else {
                    None
                }
            }
            dct::Volumes::Advanced(volume_def) => {
                // 处理高级volume定义
                if let Some(source) = &volume_def.source {
                    let is_bind = self.is_bind_mount_path(source);

                    if is_bind {
                        let container_path = &volume_def.target;
                        // 规范化路径（返回 Result）
                        let normalized_source = match self.normalize_path(source) {
                            Ok(path) => path,
                            Err(e) => {
                                warn!("Path normalization failed: {}", e);
                                return None;
                            }
                        };

                        // 将相对路径转换为相对于compose文件所在目录的绝对路径
                        let source_path_buf = std::path::PathBuf::from(&normalized_source);
                        let absolute_host_path = if source_path_buf.is_absolute() {
                            normalized_source
                        } else {
                            match self.get_working_directory() {
                                Some(compose_dir) => compose_dir
                                    .join(&normalized_source)
                                    .to_string_lossy()
                                    .to_string(),
                                None => {
                                    return None;
                                }
                            }
                        };
                        return Some(MountInfo {
                            service_name: service_name.to_string(),
                            container_path: container_path.to_string(),
                            host_path: Some(absolute_host_path),
                            is_bind_mount: true,
                        });
                    }
                }
                None
            }
        }
    }

    /// 规范化路径（使用新的路径处理器）
    fn normalize_path(&self, path: &str) -> Result<String, PathUtilsError> {
        let path_processor = PathProcessor::new(
            self.runtime_env.host_os.clone(),
            self.runtime_env.path_format.clone(),
        );
        path_processor.normalize_path(path)
    }

    /// 判断是否为bind mount路径（使用新的路径处理器）
    fn is_bind_mount_path(&self, path: &str) -> bool {
        let path_processor = PathProcessor::new(
            self.runtime_env.host_os.clone(),
            self.runtime_env.path_format.clone(),
        );
        path_processor.is_bind_mount_path(path)
    }

    /// 创建宿主机目录（如果不存在）
    fn create_host_directory_if_not_exists(&self, path_str: &str) -> Result<()> {
        let path = Path::new(path_str);

        if path.exists() {
            if path.is_dir() {
                debug!("✅ Directory already exists: {}", path.display());
                return Ok(());
            } else {
                debug!("✅ File already exists: {}", path.display());
                return Ok(());
            }
        }

        // 检查路径是否有扩展名，可能是文件路径
        let is_likely_file = path.extension().is_some();

        let dir_to_create = if is_likely_file {
            // 如果是文件路径，创建父目录
            path.parent()
        } else {
            // 如果是目录路径，创建该目录
            Some(path)
        };

        if let Some(dir_path) = dir_to_create {
            match std::fs::create_dir_all(dir_path) {
                Ok(_) => {
                    info!("📂 Creating directory: {}", dir_path.display());
                    Ok(())
                }
                Err(e) => {
                    let error_msg =
                        format!("Failed to create directory {}: {}", dir_path.display(), e);
                    warn!("❌ {}", error_msg);
                    Err(DuckError::Docker(error_msg).into())
                }
            }
        } else {
            Ok(())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::container::environment::{HostOs, PathFormat, RuntimeEnvironment};

    #[test]
    fn short_mounts_preserve_drive_colons_and_options() {
        for (spec, expected) in [
            (r"C:\data:/container", (r"C:\data", "/container", None)),
            (
                r"D:\data:/container:ro",
                (r"D:\data", "/container", Some("ro")),
            ),
            (
                "c:/data:/container:rw,z",
                ("c:/data", "/container", Some("rw,z")),
            ),
            (
                r"C:\data:D:\target:ro",
                (r"C:\data", r"D:\target", Some("ro")),
            ),
            (
                r"\\server\share:/container",
                (r"\\server\share", "/container", None),
            ),
            ("/data:/container:z", ("/data", "/container", Some("z"))),
            ("./data:/container", ("./data", "/container", None)),
            (
                "../data:/container:ro",
                ("../data", "/container", Some("ro")),
            ),
            (
                "named_volume:/container",
                ("named_volume", "/container", None),
            ),
        ] {
            assert_eq!(short_volume_parts(spec), Some(expected), "{spec}");
        }
        assert_eq!(short_volume_parts("/container"), None);
        assert_eq!(short_volume_parts("./data:/container:ro:extra"), None);
    }

    #[test]
    fn short_and_long_bind_sources_resolve_to_the_same_host_paths() {
        let directory = tempfile::tempdir().unwrap();
        let runtime_env = if cfg!(windows) {
            RuntimeEnvironment {
                host_os: HostOs::WindowsNative,
                path_format: PathFormat::Windows,
            }
        } else {
            RuntimeEnvironment {
                host_os: HostOs::LinuxNative,
                path_format: PathFormat::Posix,
            }
        };
        let manager = DockerManager {
            compose_file: directory.path().join("compose.yml"),
            env_file: directory.path().join(".env"),
            project_name: None,
            runtime_env,
        };
        for source in [r"C:\data", "C:/data", "./data", "../data", "/data"] {
            let short = dct::Volumes::Simple(format!("{source}:/container:ro"));
            let long: dct::Volumes = serde_json::from_value(serde_json::json!({
                "type": "bind", "source": source, "target": "/container", "read_only": true,
            }))
            .unwrap();
            let short = manager.parse_volume_spec("service", &short).unwrap();
            let long = manager.parse_volume_spec("service", &long).unwrap();
            assert_eq!(short.host_path, long.host_path, "{source}");
            assert_eq!(short.container_path, "/container");
            assert_eq!(long.container_path, "/container");
            assert!(short.is_bind_mount && long.is_bind_mount);
        }
        assert!(
            manager
                .parse_volume_spec(
                    "service",
                    &dct::Volumes::Simple("named_volume:/container".into())
                )
                .is_none()
        );
        assert!(
            manager
                .parse_volume_spec("service", &dct::Volumes::Simple("/container".into()))
                .is_none()
        );
    }
}
