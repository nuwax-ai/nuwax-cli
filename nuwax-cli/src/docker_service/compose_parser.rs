use serde_yaml::Value;
use std::collections::HashSet;
use std::path::PathBuf;

/// 挂载信息
#[derive(Debug, Clone)]
pub struct MountInfo {
    pub host_path: String,
    pub container_path: String,
    pub options: Option<String>,
    pub is_bind_mount: bool,
}

/// Docker Compose 解析器
pub struct DockerComposeParser {
    compose: Value,
}

impl DockerComposeParser {
    /// 从文件解析Docker Compose配置
    pub fn from_file(path: &PathBuf) -> Result<Self, Box<dyn std::error::Error>> {
        let content = std::fs::read_to_string(path)?;
        let compose: Value = serde_yaml::from_str(&content)?;
        Ok(Self { compose })
    }

    /// 提取所有绑定挂载目录
    pub fn extract_mount_directories(&self) -> Vec<String> {
        let mut mount_dirs = HashSet::new();

        // 获取services部分
        if let Some(services) = self.compose.get("services") {
            if let Some(services_map) = services.as_mapping() {
                for (_service_name, service) in services_map {
                    if let Some(volumes) = service.get("volumes") {
                        if let Some(volumes_array) = volumes.as_sequence() {
                            for volume in volumes_array {
                                if let Some(volume_str) = volume.as_str() {
                                    if let Some(host_path) =
                                        self.extract_host_path_from_volume(volume_str)
                                    {
                                        mount_dirs.insert(host_path);
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }

        mount_dirs.into_iter().collect()
    }

    /// 从volume字符串中提取主机路径
    fn extract_host_path_from_volume(&self, volume: &str) -> Option<String> {
        // 检查是否是Windows路径格式 (如 C:\path 或 C:/path)
        let colon_pos = if volume.len() > 1 && volume.chars().nth(1) == Some(':') {
            // 如果是Windows路径，寻找第二个冒号作为分隔符
            volume.find(':').and_then(|first_colon| {
                volume[first_colon + 1..]
                    .find(':')
                    .map(|second_colon| first_colon + 1 + second_colon)
            })
        } else {
            // 否则寻找第一个冒号
            volume.find(':')
        };

        if let Some(colon_pos) = colon_pos {
            let host_path = &volume[..colon_pos];

            // 检查是否是绑定挂载路径（相对或绝对路径）
            if self.is_bind_mount_path(host_path) {
                // 规范化路径：移除多余的 ./ 和 //
                let normalized_path = self.normalize_path(host_path);
                return Some(normalized_path);
            }
        }
        None
    }

    /// 规范化路径，移除多余的 ./ 和 //
    fn normalize_path(&self, path: &str) -> String {
        use std::path::PathBuf;

        let path_buf = PathBuf::from(path);

        // 对于相对路径，保留原始的相对路径表示，但处理重复路径
        if path.starts_with("./") || path.starts_with("../") {
            // 处理 "docker/./data" -> "docker/data" 这种情况
            let parts: Vec<&str> = path.split('/').collect();
            let mut cleaned_parts = Vec::new();

            for part in parts {
                match part {
                    "" => continue,  // 跳过空部分
                    "." => continue, // 跳过当前目录
                    ".." => {
                        // 处理父目录
                        if !cleaned_parts.is_empty() && cleaned_parts.last() != Some(&"..") {
                            cleaned_parts.pop();
                        } else {
                            cleaned_parts.push("..");
                        }
                    }
                    _ => cleaned_parts.push(part),
                }
            }

            let result = cleaned_parts.join("/");

            // 确保相对路径以 ./ 开头（除非已经是 ../ 或根路径）
            if !result.starts_with("../") && !result.starts_with("/") && !result.is_empty() {
                if !result.starts_with("./") {
                    format!("./{}", result.trim_start_matches("./"))
                } else {
                    result
                }
            } else {
                result
            }
        } else {
            // 对于绝对路径，直接返回原路径，保持格式不变
            path.to_string()
        }
    }

    /// 检查是否是绑定挂载路径
    #[allow(dead_code)]
    fn is_bind_mount_path(&self, path: &str) -> bool {
        let path_buf = std::path::PathBuf::from(path);

        path_buf.is_absolute() ||
        path.starts_with("./") ||
        path.starts_with("../") ||
        // Windows路径检查：C:、D:等盘符
        (path.len() > 1 && path.chars().nth(1) == Some(':'))
    }

    /// 获取所有挂载信息（用于调试）
    #[allow(dead_code)]
    pub fn get_mount_info(&self) -> Vec<MountInfo> {
        let mut mount_info = Vec::new();

        if let Some(services) = self.compose.get("services") {
            if let Some(services_map) = services.as_mapping() {
                for (_service_name, service) in services_map {
                    if let Some(volumes) = service.get("volumes") {
                        if let Some(volumes_array) = volumes.as_sequence() {
                            for volume in volumes_array {
                                if let Some(volume_str) = volume.as_str() {
                                    if let Some(info) = self.parse_volume_to_mount_info(volume_str)
                                    {
                                        mount_info.push(info);
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }

        mount_info
    }

    /// 解析volume字符串为MountInfo
    fn parse_volume_to_mount_info(&self, volume: &str) -> Option<MountInfo> {
        if let Some(colon_pos) = volume.find(':') {
            let host_path = &volume[..colon_pos];
            let remaining = &volume[colon_pos + 1..];

            if self.is_bind_mount_path(host_path) {
                let (container_path, options) = if let Some(second_colon) = remaining.find(':') {
                    (
                        &remaining[..second_colon],
                        Some(remaining[second_colon + 1..].to_string()),
                    )
                } else {
                    (remaining, None)
                };

                return Some(MountInfo {
                    host_path: host_path.to_string(),
                    container_path: container_path.to_string(),
                    options,
                    is_bind_mount: true,
                });
            }
        }
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use tempfile::NamedTempFile;

    #[test]
    fn test_extract_bind_mount_directories() {
        let compose_content = r#"
services:
  app:
    image: nginx
    volumes:
      - "./data:/app/data"
      - "./config:/app/config:ro"
      - "named_volume:/app/named"
      - "/absolute/path:/app/absolute"
      - "C:\\windows\\path:/app/windows"
  db:
    image: postgres
    volumes:
      - "./db_data:/var/lib/postgresql/data"
"#;

        let mut temp_file = NamedTempFile::new().unwrap();
        temp_file.write_all(compose_content.as_bytes()).unwrap();
        temp_file.flush().unwrap();

        let parser = DockerComposeParser::from_file(&temp_file.path().to_path_buf()).unwrap();
        let mount_dirs = parser.extract_mount_directories();

        assert!(mount_dirs.contains(&"./data".to_string()));
        assert!(mount_dirs.contains(&"./config".to_string()));
        assert!(mount_dirs.contains(&"./db_data".to_string()));
        assert!(mount_dirs.contains(&"/absolute/path".to_string()));
        assert!(mount_dirs.contains(&"C:\\windows\\path".to_string()));

        // 确保不包含命名卷
        assert!(!mount_dirs.contains(&"named_volume".to_string()));
    }

    #[test]
    fn test_mount_info_parsing() {
        let compose_content = r#"
services:
  app:
    image: nginx
    volumes:
      - "./data:/app/data:rw"
      - "./config:/app/config:ro"
"#;

        let mut temp_file = NamedTempFile::new().unwrap();
        temp_file.write_all(compose_content.as_bytes()).unwrap();
        temp_file.flush().unwrap();

        let parser = DockerComposeParser::from_file(&temp_file.path().to_path_buf()).unwrap();
        let mount_info = parser.get_mount_info();

        assert_eq!(mount_info.len(), 2);

        let data_mount = mount_info.iter().find(|m| m.host_path == "./data").unwrap();
        assert_eq!(data_mount.container_path, "/app/data");
        assert_eq!(data_mount.options, Some("rw".to_string()));
        assert!(data_mount.is_bind_mount);

        let config_mount = mount_info
            .iter()
            .find(|m| m.host_path == "./config")
            .unwrap();
        assert_eq!(config_mount.container_path, "/app/config");
        assert_eq!(config_mount.options, Some("ro".to_string()));
        assert!(config_mount.is_bind_mount);
    }
}
