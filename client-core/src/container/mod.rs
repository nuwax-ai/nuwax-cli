// 模块声明
mod command;
mod config;
mod image;
mod service;
pub mod types;
pub mod volumes;

#[cfg(test)]
mod config_test;
mod modern_docker;

// 重新导出公共API
pub use types::{DockerManager, ServiceConfig, ServiceInfo, ServiceStatus};

// 导入测试模块
#[cfg(test)]
mod tests {
    use super::*;
    use std::path::{Path, PathBuf};
    use tempfile::tempdir;

    fn create_dummy_compose_file(dir: &Path) -> PathBuf {
        let compose_file = dir.join(crate::constants::docker::COMPOSE_FILE_NAME);
        std::fs::write(
            &compose_file,
            r#"
version: '3.8'
services:
  test-service:
    image: nginx:alpine
    ports:
      - "8080:80"
"#,
        )
        .unwrap();
        compose_file
    }

    #[test]
    fn test_docker_manager_creation() {
        let dir = tempdir().unwrap();
        let compose_file = create_dummy_compose_file(dir.path());
        let env_file = dir.path().join(".env");

        let manager = DockerManager::new(&compose_file, &env_file).unwrap();
        assert_eq!(manager.get_compose_file(), compose_file);
    }

    #[test]
    fn test_docker_manager_with_nonexistent_file() {
        // 允许创建DockerManager实例，即使文件不存在
        let result = DockerManager::new("/nonexistent/docker-compose.yml", "/nonexistent/.env");
        assert!(result.is_ok());

        // 但是compose_file_exists应该返回false
        let manager = result.unwrap();
        assert!(!manager.compose_file_exists());
    }
}
