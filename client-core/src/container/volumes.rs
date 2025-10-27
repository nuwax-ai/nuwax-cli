use super::types::DockerManager;
use crate::DuckError;
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

impl DockerManager {
    /// 确保所有宿主机挂载目录存在
    pub async fn ensure_host_volumes_exist(&self) -> Result<()> {
        info!("🔍 检查并创建宿主机挂载目录...");

        let compose_config = self.load_compose_config()?;
        let mount_directories = self.extract_mount_directories(&compose_config)?;

        if mount_directories.is_empty() {
            info!("✅ 未发现需要创建的宿主机挂载目录");
            return Ok(());
        }

        info!("📁 发现 {} 个需要检查的挂载目录", mount_directories.len());

        for mount_info in mount_directories {
            if let Some(host_path) = &mount_info.host_path {
                if mount_info.is_bind_mount {
                    self.create_host_directory_if_not_exists(host_path)?;
                }
            }
        }

        info!("✅ 宿主机挂载目录检查完成");
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
                let parts: Vec<&str> = volume_str.split(':').collect();

                match parts.len() {
                    2 | 3 => {
                        // 格式: host_path:container_path 或 host_path:container_path:mode
                        let host_path = parts[0];
                        let container_path = parts[1];

                        let is_bind = self.is_bind_mount_path(host_path);

                        if is_bind {
                            // 规范化路径：移除多余的 ./ 和 //
                            let normalized_host_path = self.normalize_path(host_path);

                            // 将相对路径转换为相对于compose文件所在目录的绝对路径
                            let host_path_buf = std::path::PathBuf::from(&normalized_host_path);
                            let absolute_host_path = if host_path_buf.is_absolute() {
                                normalized_host_path
                            } else {
                                match self.get_working_directory() {
                                    Some(compose_dir) => compose_dir
                                        .join(normalized_host_path)
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
                    _ => None,
                }
            }
            dct::Volumes::Advanced(volume_def) => {
                // 处理高级volume定义
                if let Some(source) = &volume_def.source {
                    let is_bind = self.is_bind_mount_path(source);

                    if is_bind {
                        let container_path = &volume_def.target;
                        // 规范化路径：移除多余的 ./ 和 //
                        let normalized_source = self.normalize_path(source);

                        // 将相对路径转换为相对于compose文件所在目录的绝对路径
                        let source_path_buf = std::path::PathBuf::from(&normalized_source);
                        let absolute_host_path = if source_path_buf.is_absolute() {
                            normalized_source
                        } else {
                            match self.get_working_directory() {
                                Some(compose_dir) => compose_dir
                                    .join(normalized_source)
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

    /// 规范化路径，移除多余的 ./ 和 //
    fn normalize_path(&self, path: &str) -> String {
        use std::path::PathBuf;

        let path_buf = PathBuf::from(path);
        let mut components = Vec::new();

        for component in path_buf.components() {
            match component {
                std::path::Component::CurDir => {
                    // 跳过当前目录 .
                    continue;
                }
                std::path::Component::ParentDir => {
                    // 处理父目录 ..
                    if let Some(last) = components.last() {
                        if last != &std::path::Component::RootDir {
                            components.pop();
                        }
                    }
                }
                _ => {
                    components.push(component);
                }
            }
        }

        let normalized = components
            .iter()
            .map(|c| c.as_os_str().to_string_lossy())
            .collect::<Vec<_>>()
            .join(std::path::MAIN_SEPARATOR_STR);

        // 确保空路径返回 "."
        if normalized.is_empty() {
            ".".to_string()
        } else {
            normalized
        }
    }

    /// 判断是否为bind mount路径
    fn is_bind_mount_path(&self, path: &str) -> bool {
        // 判断是否为bind mount路径（宿主机绝对路径或相对路径）
        // 排除命名卷（不包含路径分隔符且不是绝对路径）
        !path.is_empty()
            && (std::path::PathBuf::from(path).is_absolute()
                || path.starts_with("./")
                || path.starts_with("../")
                || (path.contains(std::path::MAIN_SEPARATOR)
                    && !std::path::PathBuf::from(path).is_absolute())) // 相对文件路径，排除命名卷
    }

    /// 创建宿主机目录（如果不存在）
    fn create_host_directory_if_not_exists(&self, path_str: &str) -> Result<()> {
        let path = Path::new(path_str);

        if path.exists() {
            if path.is_dir() {
                debug!("✅ 目录已存在: {}", path.display());
                return Ok(());
            } else {
                debug!("✅ 文件已存在: {}", path.display());
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
                    info!("📂 创建目录: {}", dir_path.display());
                    Ok(())
                }
                Err(e) => {
                    let error_msg = format!("创建目录失败 {}: {}", dir_path.display(), e);
                    warn!("❌ {}", error_msg);
                    Err(DuckError::Docker(error_msg).into())
                }
            }
        } else {
            Ok(())
        }
    }
}
