use crate::docker_service::error::{DockerServiceError, DockerServiceResult};
use std::fs;
use std::path::{Path, PathBuf};
use tracing::{debug, info, warn};

/// 目录权限管理器 - 配合 Docker init 容器处理权限
///
/// # 设计原则
///
/// 1. **Rust 代码负责**：
///    - 创建目录结构
///    - 设置配置文件权限为 644（MySQL 安全要求）
///
/// 2. **Docker init 容器负责**：
///    - 修改数据目录所有权为 999:999（chown）
///    - 设置数据目录权限为 755（chmod）
///
/// 3. **权限设置策略**：
///    - chmod 失败只记录警告（WSL2/网络文件系统可能失败，但不影响容器运行）
///    - 配置文件必须是 644，否则 MySQL 会忽略
///
/// # Docker 卷挂载权限机制
///
/// Docker 通过 UID/GID 数值映射权限，不是用户名：
/// - 宿主机 UID 1000 → 容器看到 UID 1000
/// - MySQL 进程是 UID 999 → 无法写入 UID 1000 的目录
/// - 解决：init 容器以 root 运行 `chown 999:999`
#[derive(Debug, Clone)]
pub struct DirectoryPermissionManager {
    work_dir: PathBuf,
}

impl DirectoryPermissionManager {
    /// 创建权限管理器
    pub fn new(work_dir: PathBuf) -> Self {
        Self { work_dir }
    }

    /// 确保 MySQL 配置文件权限正确
    ///
    /// 注意：目录创建由 DockerManager::ensure_host_volumes_exist() 自动处理
    /// 这里只负责设置 MySQL 配置文件权限为 644
    pub fn ensure_mysql_config_safe(&self) -> DockerServiceResult<()> {
        info!("🔒 Checking MySQL config file permissions...");
        self.ensure_mysql_config_permissions()?;
        info!("✅ MySQL config file permission check completed");
        Ok(())
    }

    /// 确保 MySQL 配置文件权限为 644
    ///
    /// MySQL 安全检查：拒绝 group-writable 或 world-writable 的配置文件
    /// - 拒绝：777, 775, 664
    /// - 接受：644, 640, 444
    fn ensure_mysql_config_permissions(&self) -> DockerServiceResult<()> {
        let mysql_cnf = self.work_dir.join("config/mysql.cnf");
        if !mysql_cnf.exists() {
            debug!("MySQL config file not found, skipping permission setup");
            return Ok(());
        }

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;

            // 检查当前权限
            if let Ok(metadata) = fs::metadata(&mysql_cnf) {
                let current_mode = metadata.permissions().mode() & 0o777;
                let is_unsafe = (current_mode & 0o022) != 0; // group-writable 或 world-writable

                if is_unsafe {
                    warn!(
                        "⚠️  MySQL config file permissions are unsafe: {:o} (MySQL may ignore it)",
                        current_mode
                    );
                }
            }

            // 设置为 644
            let metadata = fs::metadata(&mysql_cnf).map_err(|e| {
                DockerServiceError::FileSystem(format!("Failed to read config file metadata: {}", e))
            })?;
            let mut permissions = metadata.permissions();
            permissions.set_mode(0o644);

            fs::set_permissions(&mysql_cnf, permissions).map_err(|e| {
                DockerServiceError::FileSystem(format!("Failed to set config file permissions: {}", e))
            })?;

            info!("🔒 MySQL config file permission: 644 (safe)");
        }

        #[cfg(windows)]
        {
            // Windows 上确保文件可读
            if let Ok(metadata) = fs::metadata(&mysql_cnf) {
                let mut permissions = metadata.permissions();
                permissions.set_readonly(false);
                fs::set_permissions(&mysql_cnf, permissions).ok();
            }
            info!("🔒 Windows: MySQL config file permission has been set");
        }

        Ok(())
    }

    /// MySQL 容器启动失败时的权限修复
    ///
    /// 只在 MySQL 启动失败时调用，尝试修复权限问题
    /// 注意：目录应该已经由 ensure_host_volumes_exist() 创建
    pub fn fix_mysql_permissions_on_failure(&self) -> DockerServiceResult<()> {
        warn!("🔧 MySQL startup failed, attempting permission repair...");

        let mysql_data_dir = self.work_dir.join("data/mysql");
        let mysql_logs_dir = self.work_dir.join("logs/mysql");

        // 检查目录是否存在
        if !mysql_data_dir.exists() {
            warn!("⚠️  MySQL data directory does not exist: {}", mysql_data_dir.display());
            warn!(
                "   This should not happen; ensure_host_volumes_exist() should have created it"
            );
            return Err(DockerServiceError::FileSystem(
                "MySQL data directory does not exist, please check docker-compose.yml configuration"
                    .to_string(),
            ));
        }

        if !mysql_logs_dir.exists() {
            warn!("⚠️  MySQL log directory does not exist: {}", mysql_logs_dir.display());
            warn!(
                "   This should not happen; ensure_host_volumes_exist() should have created it"
            );
            return Err(DockerServiceError::FileSystem(
                "MySQL log directory does not exist, please check docker-compose.yml configuration"
                    .to_string(),
            ));
        }

        // 尝试设置宽松权限（尽力而为）
        #[cfg(unix)]
        {
            self.try_set_permissions(&mysql_data_dir, 0o777)?;
            self.try_set_permissions(&mysql_logs_dir, 0o777)?;
            info!("🔑 MySQL directory permissions set to 777 (most permissive)");
        }

        // 确保配置文件权限正确
        self.ensure_mysql_config_permissions()?;

        info!("✅ Permission repair completed");
        info!("💡 If it still fails, please check:");
        info!("   1. Whether the Docker init container is running correctly");
        info!("   2. Whether there is enough disk space");
        info!("   3. Whether SELinux/AppArmor is blocking access");

        Ok(())
    }

    /// 尝试设置权限（失败只记录警告）
    #[cfg(unix)]
    fn try_set_permissions(&self, path: &Path, mode: u32) -> DockerServiceResult<()> {
        use std::os::unix::fs::PermissionsExt;

        if let Ok(metadata) = fs::metadata(path) {
            let mut permissions = metadata.permissions();
            permissions.set_mode(mode);

            match fs::set_permissions(path, permissions) {
                Ok(_) => debug!("Permission set successfully: {} → {:o}", path.display(), mode),
                Err(e) => warn!(
                    "Failed to set permissions: {} → {:o}, error: {} (usually non-fatal)",
                    path.display(),
                    mode,
                    e
                ),
            }
        }

        Ok(())
    }
}
