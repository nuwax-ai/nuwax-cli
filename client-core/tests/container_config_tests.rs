use client_core::container::{DockerManager, ServiceConfig};
use std::path::PathBuf;
use tempfile::tempdir;

/// 测试用的docker-compose.yml内容
const TEST_COMPOSE_CONTENT: &str = r#"
version: '3.8'

services:
  frontend:
    image: nginx:latest
    ports:
      - "80:80"
    restart: always

  backend:
    image: node:16-alpine
    ports:
      - "3000:3000"
    restart: unless-stopped

  database:
    image: mysql:8.0
    environment:
      MYSQL_ROOT_PASSWORD: root
    restart: on-failure

  # 一次性服务 - 数据库初始化
  db-init:
    image: mysql:8.0
    command: ["mysql", "-h", "database", "-u", "root", "-proot", "-e", "CREATE DATABASE IF NOT EXISTS test;"]
    depends_on:
      - database
    restart: "no"

  # 一次性服务 - 权限修复
  permission-fix:
    image: busybox:latest
    command: ["sh", "-c", "chmod 755 /data"]
    volumes:
      - "./data:/data"
    restart: "false"

  # 没有restart配置的服务（默认不重启）
  migration:
    image: alpine:latest
    command: ["echo", "running migration"]

networks:
  default:
    driver: bridge
"#;

/// 测试用的.env文件内容
const TEST_ENV_CONTENT: &str = r#"
# 测试环境变量
DOCKER_REGISTRY=localhost:5000
FRONTEND_VERSION=1.0.0
BACKEND_VERSION=1.0.0
DATABASE_VERSION=1.0.0
"#;

#[cfg(test)]
mod tests {
    use super::*;

    /// 创建临时测试文件
    fn create_test_files() -> (tempfile::TempDir, PathBuf, PathBuf) {
        let temp_dir = tempdir().expect("Failed to create temp dir");
        let compose_path = temp_dir.path().join("docker-compose.yml");
        let env_path = temp_dir.path().join(".env");

        std::fs::write(&compose_path, TEST_COMPOSE_CONTENT).expect("Failed to write compose file");
        std::fs::write(&env_path, TEST_ENV_CONTENT).expect("Failed to write env file");

        (temp_dir, compose_path, env_path)
    }

    #[tokio::test]
    async fn test_is_oneshot_service() {
        let (_temp_dir, compose_path, env_path) = create_test_files();
        let manager =
            DockerManager::new(compose_path, env_path).expect("Failed to create DockerManager");

        // 测试restart: "false"的一次性服务
        let result = manager.is_oneshot_service("permission-fix").await;
        assert!(result.is_ok());
        assert!(result.unwrap());

        // 测试restart: always的服务（非一次性）
        let result = manager.is_oneshot_service("frontend").await;
        assert!(result.is_ok());
        assert!(!result.unwrap());

        // 测试restart: unless-stopped的服务（非一次性）
        let result = manager.is_oneshot_service("backend").await;
        assert!(result.is_ok());
        assert!(!result.unwrap());

        // 测试restart: on-failure的服务（非一次性）
        let result = manager.is_oneshot_service("database").await;
        assert!(result.is_ok());
        assert!(!result.unwrap());


        // 测试不存在的服务
        let result = manager.is_oneshot_service("non-existent").await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_parse_service_config() {
        let (_temp_dir, compose_path, env_path) = create_test_files();
        let manager =
            DockerManager::new(compose_path, env_path).expect("Failed to create DockerManager");

        // 测试restart: always的服务
        let result = manager.parse_service_config("frontend").await;
        assert!(result.is_ok());
        let config = result.unwrap();
        assert_eq!(config.restart, Some("always".to_string()));

        // 测试restart: unless-stopped的服务
        let result = manager.parse_service_config("backend").await;
        assert!(result.is_ok());
        let config = result.unwrap();
        assert_eq!(config.restart, Some("unless-stopped".to_string()));

        // 测试restart: on-failure的服务
        let result = manager.parse_service_config("database").await;
        assert!(result.is_ok());
        let config = result.unwrap();
        assert_eq!(config.restart, Some("on-failure".to_string()));

        // 测试restart: "no"的服务
        let result = manager.parse_service_config("db-init").await;
        assert!(result.is_ok());
        let config = result.unwrap();
        assert_eq!(config.restart, Some("no".to_string()));

        // 测试restart: "false"的服务
        let result = manager.parse_service_config("permission-fix").await;
        assert!(result.is_ok());
        let config = result.unwrap();
        assert_eq!(config.restart, Some("false".to_string()));

        // 测试没有restart配置的服务
        let result = manager.parse_service_config("migration").await;
        assert!(result.is_ok());
        let config = result.unwrap();
        assert_eq!(config.restart, None);

        // 测试不存在的服务
        let result = manager.parse_service_config("non-existent").await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_get_compose_service_names() {
        let (_temp_dir, compose_path, env_path) = create_test_files();
        let manager =
            DockerManager::new(compose_path, env_path).expect("Failed to create DockerManager");

        let result = manager.get_compose_service_names().await;
        assert!(result.is_ok());
        let service_names = result.unwrap();

        // 验证服务数量
        assert_eq!(service_names.len(), 6);

        // 验证所有服务名称都存在
        let expected_services = vec![
            "frontend",
            "backend",
            "database",
            "db-init",
            "permission-fix",
            "migration",
        ];

        for service in expected_services {
            assert!(
                service_names.contains(service),
                "Service {service} not found"
            );
        }
    }

    #[tokio::test]
    async fn test_with_fixtures_file() {
        // 使用项目实际的fixtures文件进行测试
        let compose_path = PathBuf::from("fixtures/docker-compose.yml");
        let env_path = PathBuf::from("fixtures/.env");

        if compose_path.exists() && env_path.exists() {
            let manager =
                DockerManager::new(compose_path, env_path).expect("Failed to create DockerManager");

            // 测试get_compose_service_names
            let result = manager.get_compose_service_names().await;
            assert!(result.is_ok());
            let service_names = result.unwrap();

            // 验证实际的服务数量
            assert!(service_names.len() >= 10); // 实际文件中有多个服务

            // 验证一些关键服务存在
            assert!(service_names.contains("frontend"));
            assert!(service_names.contains("backend"));
            assert!(service_names.contains("mysql"));
            assert!(service_names.contains("mysql-permission-fix"));
            assert!(service_names.contains("redis"));

            // 测试mysql-permission-fix是一次性服务
            let result = manager.is_oneshot_service("mysql-permission-fix").await;
            assert!(result.is_ok());
            assert!(result.unwrap());

            // 测试mysql不是一次性服务
            let result = manager.is_oneshot_service("mysql").await;
            assert!(result.is_ok());
            assert!(!result.unwrap());

            // 测试frontend不是一次性服务
            let result = manager.is_oneshot_service("frontend").await;
            assert!(result.is_ok());
            assert!(!result.unwrap());

            // 测试parse_service_config
            let result = manager.parse_service_config("mysql").await;
            assert!(result.is_ok());
            let config = result.unwrap();
            assert_eq!(config.restart, Some("always".to_string()));

            let result = manager.parse_service_config("mysql-permission-fix").await;
            assert!(result.is_ok());
            let config = result.unwrap();
            assert_eq!(config.restart, Some("no".to_string()));
        } else {
            // 如果fixtures文件不存在，跳过这个测试
            println!("Fixtures files not found, skipping integration test");
        }
    }

    #[tokio::test]
    async fn test_service_config_struct() {
        // 测试ServiceConfig结构体
        let config = ServiceConfig {
            restart: Some("always".to_string()),
        };
        assert_eq!(config.restart, Some("always".to_string()));

        let config = ServiceConfig { restart: None };
        assert_eq!(config.restart, None);
    }
}
