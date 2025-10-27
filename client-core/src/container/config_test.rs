use super::types::DockerManager;
use anyhow::Result;
use docker_compose_types as dct;
use std::path::PathBuf;

#[test]
fn test_load_compose_config_with_env_variables() -> Result<()> {
    // 1. 设置相对于 Cargo manifest 的路径
    let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let compose_path = manifest_dir.join("fixtures/docker-compose.yml");
    let env_path = manifest_dir.join("fixtures/.env");

    // 2. 创建 DockerManager 实例
    // 构造函数需要一个有效的 compose 文件路径，即使我们测试的函数会使用自己的路径参数
    let manager = DockerManager::new(compose_path.clone(), env_path.clone())?;

    // 3. 调用待测试的函数
    let compose_config = manager.load_compose_config()?;

    // 检查服务是否被正确解析
    let services = compose_config.services; // No .expect() needed
    assert_eq!(services.0.len(), 11, "应该有 11 个服务");

    // 检查环境变量是否被正确替换
    let frontend_service = services
        .0
        .get("frontend")
        .expect("应该找到 frontend 服务")
        .as_ref()
        .expect("frontend service 应该是 Some");
    assert_eq!(
        frontend_service.image.as_deref(),
        Some("registry.yichamao.com/agent-platform-front:latest"),
        "frontend 服务的 image 环境变量未被正确替换"
    );

    let mysql_service = services
        .0
        .get("mysql")
        .expect("应该找到 mysql 服务")
        .as_ref()
        .expect("mysql service 应该是 Some");
    // 检查 .env 文件中的密码是否被正确加载
    if let dct::Environment::List(env_list) = &mysql_service.environment {
        let password_entry = env_list
            .iter()
            .find(|s| s.starts_with("MYSQL_ROOT_PASSWORD="));
        assert!(password_entry.is_some(), "未找到 MYSQL_ROOT_PASSWORD 条目");
        assert_eq!(
            password_entry.unwrap(),
            "MYSQL_ROOT_PASSWORD=root",
            "MYSQL_ROOT_PASSWORD 环境变量未被正确加载或替换"
        );
    } else {
        panic!("mysql 服务的 environment 应该是 List 类型");
    }

    Ok(())
}
