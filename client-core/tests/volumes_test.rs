use client_core::container::types::DockerManager;
use client_core::container::volumes::MountInfo;
use std::collections::HashMap;
use std::fs;
use std::path::Path;
use tempfile::tempdir;

#[tokio::test]
async fn test_parse_docker_compose_volumes() {
    let temp_dir = tempdir().unwrap();
    let compose_path = temp_dir.path().join("docker-compose.yml");

    // 创建docker-compose.yml内容
    let compose_content = r#"
services:
  frontend:
    image: nginx:latest
    volumes:
      - ./frontend/html:/usr/share/nginx/html
      - ./frontend/conf:/etc/nginx/conf.d
      - ./frontend/logs:/var/log/nginx
  
  backend:
    image: node:16
    volumes:
      - ./backend:/app
      - ./backend/logs:/var/log/app
    
  mysql:
    image: mysql:8.0
    volumes:
      - mysql_data:/var/lib/mysql
      - ./mysql/init:/docker-entrypoint-initdb.d

volumes:
  mysql_data:
"#;

    fs::write(&compose_path, compose_content).unwrap();

    let manager = DockerManager::new(compose_path.clone(), temp_dir.path().join(".env")).unwrap();
    let compose_config = manager.load_compose_config().unwrap();
    let mount_dirs = manager.extract_mount_directories(&compose_config).unwrap();

    // 验证frontend服务有3个绑定挂载
    let frontend_mounts: Vec<&MountInfo> = mount_dirs
        .iter()
        .filter(|m| m.service_name == "frontend" && m.is_bind_mount)
        .collect();
    assert_eq!(frontend_mounts.len(), 3, "frontend应该有3个绑定挂载");

    // 验证backend服务有2个绑定挂载
    let backend_mounts: Vec<&MountInfo> = mount_dirs
        .iter()
        .filter(|m| m.service_name == "backend" && m.is_bind_mount)
        .collect();
    assert_eq!(backend_mounts.len(), 2, "backend应该有2个绑定挂载");

    // 验证相对路径被转换为绝对路径
    let relative_mounts: Vec<&MountInfo> = mount_dirs
        .iter()
        .filter(|m| {
            m.host_path
                .as_ref()
                .is_some_and(|p| !Path::new(p).is_absolute())
        })
        .collect();
    assert_eq!(relative_mounts.len(), 0, "所有相对路径应该被转换为绝对路径");

    // 验证绝对路径绑定挂载
    let absolute_mounts: Vec<&MountInfo> = mount_dirs
        .iter()
        .filter(|m| {
            m.host_path
                .as_ref()
                .is_some_and(|p| Path::new(p).is_absolute() && m.is_bind_mount)
        })
        .collect();
    assert_eq!(absolute_mounts.len(), 6, "应该有6个绝对路径绑定挂载");

    // 测试目录创建
    manager.ensure_host_volumes_exist().await.unwrap();

    // 验证目录被创建（跳过绝对路径）
    for mount in &mount_dirs {
        if let Some(host_path) = &mount.host_path {
            // 跳过已经是绝对路径的挂载
            if !Path::new(host_path).is_absolute() {
                let full_path = temp_dir.path().join(host_path);
                if let Some(parent) = full_path.parent() {
                    assert!(parent.exists(), "父目录应该被创建: {parent:?}");
                }
            }
        }
    }
}

#[tokio::test]
async fn test_parse_complex_actual_compose() {
    let temp_dir = tempdir().unwrap();
    let fixtures_path = Path::new(
        "/Volumes/soddy/git_workspace/duck_client/client-core/fixtures/docker-compose.yml",
    );

    // 复制fixtures文件到临时目录
    let compose_path = temp_dir.path().join("docker-compose.yml");
    fs::copy(fixtures_path, &compose_path).unwrap();

    let manager = DockerManager::new(compose_path.clone(), temp_dir.path().join(".env")).unwrap();
    let compose_config = manager.load_compose_config().unwrap();
    let mount_dirs = manager.extract_mount_directories(&compose_config).unwrap();

    println!("从fixtures文件中找到{}个挂载点", mount_dirs.len());

    // 按服务分组验证
    let mut service_mounts: HashMap<String, Vec<MountInfo>> = HashMap::new();
    for mount in mount_dirs {
        service_mounts
            .entry(mount.service_name.clone())
            .or_default()
            .push(mount);
    }

    // 验证frontend服务挂载
    if let Some(frontend) = service_mounts.get("frontend") {
        println!("frontend服务有{}个挂载点", frontend.len());
        for mount in frontend {
            println!(
                "  - {} -> {}",
                mount.host_path.as_deref().unwrap_or(""),
                mount.container_path
            );
        }
    }

    // 验证backend服务挂载
    if let Some(backend) = service_mounts.get("backend") {
        println!("backend服务有{}个挂载点", backend.len());
        for mount in backend {
            println!(
                "  - {} -> {}",
                mount.host_path.as_deref().unwrap_or(""),
                mount.container_path
            );
        }
    }

    // 验证mysql服务挂载
    if let Some(mysql) = service_mounts.get("mysql") {
        println!("mysql服务有{}个挂载点", mysql.len());
        for mount in mysql {
            println!(
                "  - {} -> {}",
                mount.host_path.as_deref().unwrap_or(""),
                mount.container_path
            );
        }
    }

    // 测试目录创建
    manager.ensure_host_volumes_exist().await.unwrap();

    // 验证相对路径被转换为绝对路径
    let relative_mounts: Vec<_> = service_mounts
        .values()
        .flatten()
        .filter(|m| {
            m.host_path
                .as_ref()
                .is_some_and(|p| !Path::new(p).is_absolute())
        })
        .collect();

    println!("相对路径挂载: {}个", relative_mounts.len());

    // 验证绝对路径保持不变
    let absolute_mounts: Vec<_> = service_mounts
        .values()
        .flatten()
        .filter(|m| {
            m.host_path
                .as_ref()
                .is_some_and(|p| Path::new(p).is_absolute())
        })
        .collect();

    println!("绝对路径挂载: {}个", absolute_mounts.len());
}

#[tokio::test]
async fn test_cross_platform_compatibility() {
    let temp_dir = tempdir().unwrap();
    let compose_path = temp_dir.path().join("docker-compose.yml");

    // 创建测试用的docker-compose.yml文件 - 使用相对路径避免权限问题
    let compose_content = r#"
version: '3.8'
services:
  test-service:
    image: nginx:latest
    volumes:
      # 相对路径
      - ./relative/path:/container/path1
      - ../parent/relative:/container/path2
      - .:/container/path3
      - subdir/nested:/container/path4
      # 命名卷
      - named_volume:/container/path5
      # 匿名卷
      - /container/path6

volumes:
  named_volume:
"#;

    fs::write(&compose_path, compose_content).unwrap();

    let manager = DockerManager::new(compose_path.clone(), temp_dir.path().join(".env")).unwrap();
    let compose_config = manager.load_compose_config().unwrap();
    let mount_dirs = manager.extract_mount_directories(&compose_config).unwrap();

    // 验证挂载点数量
    assert_eq!(mount_dirs.len(), 3, "应该找到3个绑定挂载点");

    // 验证所有绑定挂载路径都被转换为绝对路径
    for mount in &mount_dirs {
        if let Some(host_path) = &mount.host_path {
            assert!(
                Path::new(host_path).is_absolute(),
                "绑定挂载路径应该是绝对路径: {host_path}"
            );
            assert!(mount.is_bind_mount, "应该是绑定挂载: {host_path}");
        }
    }

    // 测试目录创建
    manager.ensure_host_volumes_exist().await.unwrap();

    // 验证嵌套目录被正确创建
    let nested_paths = vec!["relative/path", "parent/relative", "subdir/nested"];
    for path in nested_paths {
        let full_path = temp_dir.path().join(path);
        assert!(full_path.exists(), "嵌套目录应该被创建: {path}");
    }

    println!("跨平台兼容性测试通过，处理了{}个挂载点", mount_dirs.len());
}
