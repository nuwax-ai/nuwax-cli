use crate::docker_service::error::{DockerServiceError, DockerServiceResult};
use std::fs;
use std::path::{Path, PathBuf};
use tracing::{debug, info, warn};
use walkdir::WalkDir;

/// 目录权限管理器 - 专注于统一用户ID映射
#[derive(Debug, Clone)]
pub struct DirectoryPermissionManager {
    work_dir: PathBuf,
}

impl DirectoryPermissionManager {
    /// 创建新的目录权限管理器
    pub fn new(work_dir: PathBuf) -> Self {
        info!("🔧 初始化权限管理器");
        Self { work_dir }
    }

    /// 设置基础权限（回退方案）
    fn set_basic_permissions(&self) -> DockerServiceResult<()> {
        info!("🔧 应用基础权限设置（回退方案）");

        let data_dir = self.work_dir.join("data");
        if data_dir.exists() {
            // 设置775权限（稍微宽松一些）
            self.set_directory_permissions_recursive(&data_dir, 0o775)?;
            info!("✅ 数据目录权限设置为775");
        }

        let logs_dir = self.work_dir.join("logs");
        if logs_dir.exists() {
            self.set_directory_permissions_recursive(&logs_dir, 0o775)?;
            info!("✅ 日志目录权限设置为775");
        }

        Ok(())
    }

    /// 设置目录权限（跨平台兼容）
    /// 
    /// 注意：在某些环境下权限设置可能失败但不影响容器运行：
    /// - WSL2 挂载的 Windows 分区（/mnt/c/）
    /// - 某些网络文件系统（NFS、CIFS）
    /// - Docker Desktop 的卷挂载
    /// 
    /// 因此这个函数采用"尽力而为"策略，失败时只记录警告
    fn set_directory_permission(&self, path: &Path, mode: u32) -> DockerServiceResult<()> {
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;

            let metadata = fs::metadata(path)
                .map_err(|e| DockerServiceError::FileSystem(format!("获取文件元数据失败: {e}")))?;

            let mut permissions = metadata.permissions();
            permissions.set_mode(mode);

            match fs::set_permissions(path, permissions) {
                Ok(_) => {
                    debug!("权限设置成功: {} (mode: {:o})", path.display(), mode);
                }
                Err(e) => {
                    // 权限设置失败通常不影响容器运行，因为：
                    // 1. Docker 有自己的用户映射机制
                    // 2. WSL2/网络文件系统可能不支持标准 Unix 权限
                    // 3. 容器内的用户可能与宿主机不同
                    warn!(
                        "权限设置失败: {} (mode: {:o}), 错误: {} - 这通常不影响容器运行",
                        path.display(),
                        mode,
                        e
                    );
                }
            }
        }

        #[cfg(windows)]
        {
            // Windows 上尝试设置权限，失败也不影响容器运行
            if let Err(e) = self.set_windows_permission(path, mode) {
                warn!(
                    "Windows权限设置失败: {} (mode: {:o}), 错误: {} - 这通常不影响容器运行",
                    path.display(),
                    mode,
                    e
                );
            } else {
                debug!("Windows权限设置成功: {} (mode: {:o})", path.display(), mode);
            }
        }

        Ok(())
    }

    /// 检测当前路径是否在 WSL2 的 Windows 挂载点上
    /// 
    /// WSL2 挂载的 Windows 分区（如 /mnt/c/）通常不支持标准 Unix 权限
    #[cfg(unix)]
    fn is_wsl2_windows_mount(&self, path: &Path) -> bool {
        // 检查路径是否以 /mnt/ 开头（WSL2 的 Windows 分区挂载点）
        path.starts_with("/mnt/")
    }

    /// Windows系统上的权限设置（通过PowerShell）
    #[cfg(windows)]
    fn set_windows_permission(&self, path: &Path, mode: u32) -> DockerServiceResult<()> {
        use std::process::Command;

        // 将Unix权限模式转换为Windows权限
        let (owner, group, others) = match mode {
            0o644 => ("FullControl", "Read", "Read"),
            0o755 => ("FullControl", "Read", "Read"),
            0o775 => ("FullControl", "FullControl", "Read"),
            0o777 => ("FullControl", "FullControl", "Read"), // 降级为775权限
            _ => ("FullControl", "Read", "Read"),            // 默认安全权限
        };

        let path_str = path.to_string_lossy();

        // 对于MySQL配置文件，使用更严格的权限设置
        if path_str.contains("mysql.cnf") {
            info!("🔒 Windows系统：为MySQL配置文件设置权限");

            // 尝试简单的权限设置（不需要管理员权限）
            let simple_result = Command::new("powershell")
                .args([
                    "-Command",
                    &format!(
                        "try {{ $file = Get-Item '{}'; $file.Attributes = 'Normal'; Write-Output 'SUCCESS' }} catch {{ Write-Error $_.Exception.Message }}",
                        path_str
                    ),
                ])
                .output();

            match simple_result {
                Ok(output) if output.status.success() => {
                    let stdout = String::from_utf8_lossy(&output.stdout);
                    if stdout.contains("SUCCESS") {
                        info!("✅ MySQL配置文件权限设置成功（简单模式）");
                    } else {
                        warn!("MySQL配置文件权限设置部分失败，但继续执行");
                    }
                }
                Ok(output) => {
                    let error = String::from_utf8_lossy(&output.stderr);
                    warn!("MySQL配置文件权限设置失败（权限不足）: {}", error);
                    info!("💡 提示：以管理员身份运行可获得更好的权限控制");
                }
                Err(e) => {
                    warn!("MySQL配置文件权限设置失败: {}", e);
                }
            }
        } else {
            // 对于其他文件，使用标准权限设置
            let output = Command::new("powershell")
                .args([
                    "-Command",
                    &format!(
                        "try {{ $acl = Get-Acl '{}'; $rule = New-Object System.Security.AccessControl.FileSystemAccessRule('Everyone', '{}', 'ContainerInherit,ObjectInherit', 'None', 'Allow'); $acl.SetAccessRule($rule); Set-Acl '{}' $acl }} catch {{ Write-Error $_.Exception.Message }}",
                        path_str, others, path_str
                    ),
                ])
                .output()
                .map_err(|e| DockerServiceError::FileSystem(format!("PowerShell执行失败: {e}")))?;

            if !output.status.success() {
                let error = String::from_utf8_lossy(&output.stderr);
                return Err(DockerServiceError::FileSystem(format!(
                    "PowerShell权限设置失败: {}",
                    error
                )));
            }
        }

        Ok(())
    }

    /// 递归设置目录权限
    fn set_directory_permissions_recursive(
        &self,
        dir: &Path,
        mode: u32,
    ) -> DockerServiceResult<()> {
        for entry in WalkDir::new(dir) {
            let entry =
                entry.map_err(|e| DockerServiceError::FileSystem(format!("访问目录失败: {e}")))?;
            let path = entry.path();

            if path.is_dir() {
                self.set_directory_permission(path, mode)?;
            }
        }

        Ok(())
    }

    /// 基础权限修复（兼容性方法）
    pub fn basic_permission_fix(&self) -> DockerServiceResult<()> {
        info!("🔧 执行基础权限修复...");
        self.set_basic_permissions()
    }

    /// 渐进式权限管理（配合Docker Compose init容器）
    pub fn progressive_permission_management(&self) -> DockerServiceResult<()> {
        info!("🔧 开始渐进式权限管理...");
        info!("💡 策略：依赖mysql-permission-fix容器处理MySQL权限问题");

        // 第一步：设置Docker基础目录权限
        self.set_docker_base_permissions()?;

        // 第二步：创建必要目录（权限由init容器处理）
        self.prepare_mysql_directory()?;

        // 第三步：基础权限设置（mysql.cnf权限由init容器处理）
        self.handle_mysql_config_permissions()?;

        info!("✅ 渐进式权限管理完成");
        info!("🐳 MySQL权限将由mysql-permission-fix容器自动处理");
        Ok(())
    }

    /// 专门处理MySQL配置文件权限问题（配合Docker Compose init容器）
    fn handle_mysql_config_permissions(&self) -> DockerServiceResult<()> {
        info!("🔒 处理MySQL配置文件权限问题...");

        let mysql_cnf = self.work_dir.join("config/mysql.cnf");
        if !mysql_cnf.exists() {
            info!("   MySQL配置文件不存在，跳过处理");
            return Ok(());
        }

        // 创建必要目录
        let mysql_data_dir = self.work_dir.join("data/mysql");
        let mysql_logs_dir = self.work_dir.join("logs/mysql");

        if !mysql_data_dir.exists() {
            std::fs::create_dir_all(&mysql_data_dir).map_err(|e| {
                DockerServiceError::FileSystem(format!("创建MySQL数据目录失败: {e}"))
            })?;
        }

        if !mysql_logs_dir.exists() {
            std::fs::create_dir_all(&mysql_logs_dir).map_err(|e| {
                DockerServiceError::FileSystem(format!("创建MySQL日志目录失败: {e}"))
            })?;
        }

        // 在Unix系统上设置权限（与Docker Compose init容器协调）
        #[cfg(unix)]
        {
            // 设置MySQL数据目录为755权限，owner为当前用户
            self.set_directory_permissions_recursive(&mysql_data_dir, 0o755)?;
            self.set_directory_permissions_recursive(&mysql_logs_dir, 0o755)?;
            info!("✅ MySQL数据和日志目录权限设置为755");

            // 设置MySQL配置文件为644权限（MySQL不会忽略，因为不是world-writable）
            self.set_directory_permission(&mysql_cnf, 0o644)?;
            info!("✅ MySQL配置文件权限设置为644（安全，MySQL不会忽略）");
        }

        #[cfg(windows)]
        {
            // Windows上设置Everyone:Read权限
            self.set_directory_permission(&mysql_data_dir, 0o755)?;
            self.set_directory_permission(&mysql_logs_dir, 0o755)?;
            info!("✅ MySQL数据和日志目录权限设置为标准权限");

            // Windows上设置MySQL配置文件为严格权限
            self.set_windows_mysql_config_strict_permissions(&mysql_cnf)?;
            info!("✅ MySQL配置文件权限设置为严格模式");
        }

        Ok(())
    }

    /// Windows上为MySQL配置文件设置严格权限
    #[cfg(windows)]
    fn set_windows_mysql_config_strict_permissions(
        &self,
        mysql_cnf: &Path,
    ) -> DockerServiceResult<()> {
        use std::process::Command;

        let path_str = mysql_cnf.to_string_lossy();

        // 移除Everyone的完全控制权限
        let output = Command::new("icacls")
            .args([&path_str, "/remove", "Everyone"])
            .output()
            .map_err(|e| DockerServiceError::FileSystem(format!("icacls remove failed: {e}")))?;

        if !output.status.success() {
            warn!("移除Everyone权限失败，但继续设置只读权限");
        }

        // 设置Everyone只读权限
        let output = Command::new("icacls")
            .args([&path_str, "/grant", "Everyone:R"])
            .output()
            .map_err(|e| DockerServiceError::FileSystem(format!("icacls grant failed: {e}")))?;

        if !output.status.success() {
            let error = String::from_utf8_lossy(&output.stderr);
            return Err(DockerServiceError::FileSystem(format!(
                "设置MySQL配置文件只读权限失败: {}",
                error
            )));
        }

        Ok(())
    }

    /// 第一步：设置Docker基础目录为755权限
    fn set_docker_base_permissions(&self) -> DockerServiceResult<()> {
        info!("📁 设置Docker基础目录权限为755...");

        let base_directories = [
            "config", "logs", "app", "upload", "backups",
            "data", // data目录也先设置为755
        ];

        for dir_name in &base_directories {
            let dir_path = self.work_dir.join(dir_name);

            // 确保目录存在
            if !dir_path.exists() {
                fs::create_dir_all(&dir_path).map_err(|e| {
                    DockerServiceError::FileSystem(format!(
                        "创建目录 {} 失败: {}",
                        dir_path.display(),
                        e
                    ))
                })?;
                info!("✅ 已创建目录: {}", dir_path.display());
            }

            // 设置为755权限（不递归，只设置顶级目录）
            self.set_directory_permission(&dir_path, 0o755)?;
            info!("✅ 已设置目录权限 {} → 755", dir_name);
        }

        Ok(())
    }

    /// 第二步：预处理MySQL目录权限（配合Docker Compose init容器）
    fn prepare_mysql_directory(&self) -> DockerServiceResult<()> {
        info!("🔑 预处理MySQL目录权限...");

        let mysql_data_dir = self.work_dir.join("data/mysql");
        let mysql_logs_dir = self.work_dir.join("logs/mysql");

        // 确保MySQL相关目录存在
        if !mysql_data_dir.exists() {
            fs::create_dir_all(&mysql_data_dir).map_err(|e| {
                DockerServiceError::FileSystem(format!("创建MySQL数据目录失败: {e}"))
            })?;
            info!("✅ 已创建MySQL数据目录");
        }

        if !mysql_logs_dir.exists() {
            fs::create_dir_all(&mysql_logs_dir).map_err(|e| {
                DockerServiceError::FileSystem(format!("创建MySQL日志目录失败: {e}"))
            })?;
            info!("✅ 已创建MySQL日志目录");
        }

        // 设置适当的权限（Docker Compose init容器会进一步处理所有权）
        self.set_directory_permissions_recursive(&mysql_data_dir, 0o755)?;
        self.set_directory_permissions_recursive(&mysql_logs_dir, 0o755)?;
        info!("🔑 已设置MySQL目录权限 → 755 (Docker init容器将处理所有权)");

        // 设置MySQL配置文件为644权限（安全且不会被MySQL忽略）
        let mysql_cnf = self.work_dir.join("config/mysql.cnf");
        if mysql_cnf.exists() {
            self.set_directory_permission(&mysql_cnf, 0o644)?;
            info!("🔒 已将config/mysql.cnf权限设置为644（安全权限）");
        }

        Ok(())
    }

    /// MySQL容器启动失败时的权限修复（安全版本 - 不删除用户数据）
    pub fn fix_mysql_permissions_on_failure(&self) -> DockerServiceResult<()> {
        warn!("🔧 MySQL容器启动失败，进行安全权限修复（不删除数据）...");

        let mysql_data_dir = self.work_dir.join("data/mysql");
        let mysql_logs_dir = self.work_dir.join("logs/mysql");

        // 1. 检查MySQL数据目录状态
        if mysql_data_dir.exists() {
            info!("📁 检测到现有MySQL数据目录，保护用户数据...");

            // 安全检查：判断是否为全新目录
            if let Ok(entries) = fs::read_dir(&mysql_data_dir) {
                let entries: Vec<_> = entries.collect();
                let entry_count = entries.len();

                if entry_count > 0 {
                    // 检查是否只包含损坏的初始化文件
                    let safe_to_clean = self.is_safe_to_clean_mysql_dir(&mysql_data_dir)?;

                    if safe_to_clean {
                        warn!(
                            "🔍 检测到损坏的MySQL初始化文件（{}项），安全清理...",
                            entry_count
                        );
                        self.safe_cleanup_mysql_init_files(&mysql_data_dir)?;
                    } else {
                        warn!(
                            "⚠️  检测到可能的用户数据（{}项），仅修复权限，不删除数据",
                            entry_count
                        );
                        info!("🛡️  如果需要重新初始化，请手动备份并清理数据目录");

                        // 仅修复权限，不删除数据
                        self.fix_existing_mysql_permissions(&mysql_data_dir)?;
                        return Ok(());
                    }
                }
            }
        }

        // 2. 确保目录存在并设置正确权限
        self.ensure_mysql_directories(&mysql_data_dir, &mysql_logs_dir)?;

        // 3. 设置最宽松的权限以确保容器访问
        self.set_directory_permissions_recursive(&mysql_data_dir, 0o775)?;
        self.set_directory_permissions_recursive(&mysql_logs_dir, 0o775)?;
        info!("🔑 已设置MySQL目录权限 → 775 (数据+日志)");

        // 设置MySQL配置文件为644权限（安全且不会被MySQL忽略）
        let mysql_cnf = self.work_dir.join("config/mysql.cnf");
        if mysql_cnf.exists() {
            self.set_directory_permission(&mysql_cnf, 0o644)?;
            info!("🔒 已将config/mysql.cnf权限设置为644（安全权限，MySQL不会忽略）");
        }

        // 4. 确保父目录权限正确
        if let Some(data_parent) = mysql_data_dir.parent() {
            self.set_directory_permission(data_parent, 0o755)?;
        }

        info!("✅ MySQL安全权限修复完成");
        Ok(())
    }

    /// 判断MySQL目录是否安全清理（只包含损坏的初始化文件）
    fn is_safe_to_clean_mysql_dir(&self, mysql_dir: &Path) -> DockerServiceResult<bool> {
        let entries = fs::read_dir(mysql_dir)
            .map_err(|e| DockerServiceError::FileSystem(format!("读取MySQL目录失败: {e}")))?;

        let mut has_user_data = false;
        let mut has_init_files = false;

        for entry in entries {
            let entry = entry
                .map_err(|e| DockerServiceError::FileSystem(format!("读取目录项失败: {e}")))?;
            let file_name = entry.file_name().to_string_lossy().to_string();

            // 检查是否有用户数据表明真实使用
            if self.is_likely_user_data(&file_name) {
                has_user_data = true;
                break;
            }

            // 检查是否有初始化相关文件
            if self.is_mysql_init_file(&file_name) {
                has_init_files = true;
            }
        }

        // 只有当没有用户数据且只有初始化文件时才安全清理
        let safe_to_clean = !has_user_data && has_init_files;

        if safe_to_clean {
            info!("🔍 判断为安全清理：无用户数据，仅有损坏的初始化文件");
        } else if has_user_data {
            warn!("🛡️  检测到用户数据，拒绝自动清理");
        }

        Ok(safe_to_clean)
    }

    /// 判断文件名是否为可能的用户数据
    fn is_likely_user_data(&self, file_name: &str) -> bool {
        // 用户数据库文件特征
        let user_data_patterns = [
            // 用户创建的数据库目录
            "agent_platform",
            "agent_custom",
            "custom_",
            "app_",
            "user_",
            // 具有数据的系统表文件（大小检查在调用处）
            "mysql.ibd",
            // 事务日志文件（通常表明有用户操作）
            "undo_001",
            "undo_002",
            // 二进制日志
            "mysql-bin",
            "binlog",
        ];

        for pattern in &user_data_patterns {
            if file_name.contains(pattern) {
                return true;
            }
        }

        false
    }

    /// 判断文件名是否为MySQL初始化文件
    fn is_mysql_init_file(&self, file_name: &str) -> bool {
        let init_patterns = [
            "ib_buffer_pool",
            "#ib_",
            "auto.cnf",
            "mysql.sock",
            "ca-key.pem",
            "ca.pem",
            "client-cert.pem",
            "client-key.pem",
            "private_key.pem",
            "public_key.pem",
            "server-cert.pem",
            "server-key.pem",
            // 空的或很小的系统文件
            "ibdata1",
            "ibtmp1",
        ];

        for pattern in &init_patterns {
            if file_name.contains(pattern) {
                return true;
            }
        }

        false
    }

    /// 安全清理MySQL初始化文件
    fn safe_cleanup_mysql_init_files(&self, mysql_dir: &Path) -> DockerServiceResult<()> {
        info!("🗑️  安全清理损坏的MySQL初始化文件...");

        let entries = fs::read_dir(mysql_dir)
            .map_err(|e| DockerServiceError::FileSystem(format!("读取MySQL目录失败: {e}")))?;

        let mut cleaned_count = 0;

        for entry in entries {
            let entry = entry
                .map_err(|e| DockerServiceError::FileSystem(format!("读取目录项失败: {e}")))?;
            let path = entry.path();
            let file_name = entry.file_name().to_string_lossy().to_string();

            // 只删除确认的初始化文件
            if self.is_mysql_init_file(&file_name) && !self.is_likely_user_data(&file_name) {
                if path.is_file() {
                    if let Err(e) = fs::remove_file(&path) {
                        warn!("删除文件 {} 失败: {}", path.display(), e);
                    } else {
                        cleaned_count += 1;
                        debug!("已删除初始化文件: {}", file_name);
                    }
                } else if path.is_dir() {
                    // 对于目录，更谨慎处理
                    if self.is_safe_init_directory(&file_name) {
                        if let Err(e) = fs::remove_dir_all(&path) {
                            warn!("删除目录 {} 失败: {}", path.display(), e);
                        } else {
                            cleaned_count += 1;
                            debug!("已删除初始化目录: {}", file_name);
                        }
                    }
                }
            }
        }

        info!(
            "✅ 安全清理完成，删除了 {} 个损坏的初始化文件",
            cleaned_count
        );
        Ok(())
    }

    /// 判断目录是否为安全的初始化目录
    fn is_safe_init_directory(&self, dir_name: &str) -> bool {
        let safe_dirs = [
            "#innodb_redo",
            "#innodb_temp",
            "mysql", // 只有在确认为空的系统mysql目录时
            "performance_schema",
            "sys",
        ];

        safe_dirs.contains(&dir_name)
    }

    /// 修复现有MySQL数据的权限（不删除数据）
    fn fix_existing_mysql_permissions(&self, mysql_dir: &Path) -> DockerServiceResult<()> {
        info!("🔧 修复现有MySQL数据权限（保护用户数据）...");

        // 递归修复所有文件和目录的权限
        for entry in WalkDir::new(mysql_dir) {
            let entry =
                entry.map_err(|e| DockerServiceError::FileSystem(format!("访问目录失败: {e}")))?;
            let path = entry.path();

            if path.is_dir() {
                // 目录设置为775（drwxrwxr-x）
                self.set_directory_permission(path, 0o775)?;
            } else {
                // 文件设置为666（-rw-rw-rw-）
                #[cfg(unix)]
                {
                    use std::os::unix::fs::PermissionsExt;

                    let metadata = fs::metadata(path).map_err(|e| {
                        DockerServiceError::FileSystem(format!("获取文件元数据失败: {e}"))
                    })?;
                    let mut permissions = metadata.permissions();
                    permissions.set_mode(0o666);
                    fs::set_permissions(path, permissions).map_err(|e| {
                        DockerServiceError::FileSystem(format!("设置文件权限失败: {e}"))
                    })?;
                }

                #[cfg(windows)]
                {
                    // Windows上跳过文件权限设置
                    tracing::debug!("Windows系统跳过文件权限设置: {}", path.display());
                }
            }
        }

        info!("✅ 现有数据权限修复完成，用户数据已保护");
        Ok(())
    }

    /// 确保MySQL相关目录存在
    fn ensure_mysql_directories(
        &self,
        mysql_data_dir: &Path,
        mysql_logs_dir: &Path,
    ) -> DockerServiceResult<()> {
        if !mysql_data_dir.exists() {
            fs::create_dir_all(mysql_data_dir).map_err(|e| {
                DockerServiceError::FileSystem(format!("创建MySQL数据目录失败: {e}"))
            })?;
            info!("✅ 已创建MySQL数据目录");
        }

        if !mysql_logs_dir.exists() {
            fs::create_dir_all(mysql_logs_dir).map_err(|e| {
                DockerServiceError::FileSystem(format!("创建MySQL日志目录失败: {e}"))
            })?;
            info!("✅ 已创建MySQL日志目录");
        }

        Ok(())
    }

    /// 容器启动后的权限维护（兼容性方法）
    pub async fn post_container_start_maintenance(&self) -> DockerServiceResult<()> {
        info!("🔧 执行容器启动后权限维护...");

        // 简化版本：只做基础的权限修复
        self.set_basic_permissions()?;

        // 专门检查MySQL配置文件权限
        if let Err(e) = self.check_and_fix_mysql_config_permissions() {
            warn!("MySQL配置文件权限检查失败: {}", e);
        }

        info!("✅ 容器启动后权限维护完成");
        Ok(())
    }

    /// 专门检查和修复MySQL配置文件权限
    pub fn check_and_fix_mysql_config_permissions(&self) -> DockerServiceResult<()> {
        info!("🔍 检查MySQL配置文件权限...");

        let mysql_cnf = self.work_dir.join("config/mysql.cnf");

        if !mysql_cnf.exists() {
            warn!("⚠️  MySQL配置文件不存在: {}", mysql_cnf.display());
            return Ok(());
        }

        // 检查当前权限
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;

            if let Ok(metadata) = fs::metadata(&mysql_cnf) {
                let permissions = metadata.permissions();
                let mode = permissions.mode();

                info!("📄 MySQL配置文件当前权限: {:o}", mode);

                // 检查是否为world-writable（777或776等）
                if mode & 0o002 != 0 {
                    warn!(
                        "⚠️  检测到MySQL配置文件为world-writable (权限: {:o})，MySQL将忽略此文件",
                        mode
                    );
                    info!("🔧 正在修复MySQL配置文件权限...");

                    // 设置为775权限（所有者完全控制，组完全控制，其他用户只读）
                    self.set_directory_permission(&mysql_cnf, 0o775)?;

                    // 验证权限设置
                    if let Ok(new_metadata) = fs::metadata(&mysql_cnf) {
                        let new_permissions = new_metadata.permissions();
                        let new_mode = new_permissions.mode();
                        info!("✅ MySQL配置文件权限已修复: {:o}", new_mode);

                        if new_mode & 0o002 == 0 {
                            info!("✅ MySQL配置文件权限修复成功，MySQL现在可以使用此配置文件");
                        } else {
                            warn!(
                                "⚠️  MySQL配置文件权限修复可能不完整，权限仍为: {:o}",
                                new_mode
                            );
                        }
                    }
                } else {
                    info!("✅ MySQL配置文件权限正常: {:o}", mode);
                }
            }
        }

        #[cfg(windows)]
        {
            info!("🪟 Windows系统：检查MySQL配置文件权限...");

            // 在Windows上，我们主要关注文件是否可读
            if let Ok(metadata) = fs::metadata(&mysql_cnf) {
                let permissions = metadata.permissions();

                if permissions.readonly() {
                    warn!("⚠️  MySQL配置文件为只读，可能影响Docker容器访问");
                    // 尝试设置为可写
                    let mut new_permissions = permissions;
                    new_permissions.set_readonly(false);
                    if let Err(e) = fs::set_permissions(&mysql_cnf, new_permissions) {
                        warn!("设置MySQL配置文件权限失败: {}", e);
                    } else {
                        info!("✅ MySQL配置文件权限已设置为可写");
                    }
                } else {
                    info!("✅ MySQL配置文件权限正常（可写）");
                }
            }

            // 在Windows上，我们还需要确保Docker容器可以访问此文件
            // 这通常通过Docker卷挂载的权限设置来处理
            info!("💡 在Windows上，MySQL配置文件权限主要通过Docker卷挂载控制");

            // 额外检查：确保MySQL配置文件不是world-writable
            info!("🔍 检查MySQL配置文件是否为world-writable...");
            if let Err(e) = self.check_windows_mysql_config_permissions(&mysql_cnf) {
                warn!("Windows MySQL配置文件权限检查失败: {}", e);
            }
        }

        Ok(())
    }

    /// Windows系统上的MySQL配置文件权限检查
    #[cfg(windows)]
    fn check_windows_mysql_config_permissions(&self, mysql_cnf: &Path) -> DockerServiceResult<()> {
        use std::process::Command;

        let path_str = mysql_cnf.to_string_lossy();

        // 使用PowerShell检查文件权限
        let output = Command::new("powershell")
            .args([
                "-Command",
                &format!(
                    "try {{ $acl = Get-Acl '{}'; $rules = $acl.Access; $worldWritable = $false; foreach ($rule in $rules) {{ if ($rule.IdentityReference -eq 'Everyone' -and $rule.FileSystemRights -eq 'FullControl') {{ $worldWritable = $true; break }} }}; if ($worldWritable) {{ Write-Output 'WORLD_WRITABLE' }} else {{ Write-Output 'SECURE' }} }} catch {{ Write-Error $_.Exception.Message }}",
                    path_str
                ),
            ])
            .output()
            .map_err(|e| DockerServiceError::FileSystem(format!("PowerShell权限检查失败: {e}")))?;

        if !output.status.success() {
            let error = String::from_utf8_lossy(&output.stderr);
            return Err(DockerServiceError::FileSystem(format!(
                "PowerShell权限检查失败: {}",
                error
            )));
        }

        let binding = String::from_utf8_lossy(&output.stdout);
        let result = binding.trim();

        if result == "WORLD_WRITABLE" {
            warn!("⚠️  检测到MySQL配置文件为world-writable，MySQL将忽略此文件");
            info!("🔧 正在修复MySQL配置文件权限...");

            // 重新设置严格权限
            self.set_directory_permission(mysql_cnf, 0o775)?;
            info!("✅ MySQL配置文件权限已修复（Windows严格模式）");
        } else {
            info!("✅ MySQL配置文件权限正常（非world-writable）");
        }

        Ok(())
    }

    /// 在容器启动后检查MySQL配置文件权限
    pub async fn post_mysql_start_permission_check(&self) -> DockerServiceResult<()> {
        info!("🔍 容器启动后检查MySQL配置文件权限...");

        // 等待一段时间让容器完全启动
        tokio::time::sleep(tokio::time::Duration::from_secs(5)).await;

        // 检查MySQL配置文件权限
        self.check_and_fix_mysql_config_permissions()?;

        // 如果配置文件权限有问题，尝试重新设置
        let mysql_cnf = self.work_dir.join("config/mysql.cnf");
        if mysql_cnf.exists() {
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;

                if let Ok(metadata) = fs::metadata(&mysql_cnf) {
                    let permissions = metadata.permissions();
                    let mode = permissions.mode();

                    if mode & 0o002 != 0 {
                        warn!("⚠️  检测到MySQL配置文件仍为world-writable，尝试强制修复...");

                        // 强制设置为644权限（更严格的权限）
                        self.set_directory_permission(&mysql_cnf, 0o644)?;
                        info!("🔒 已将MySQL配置文件权限设置为644（更严格）");

                        // 建议重启MySQL容器
                        info!("💡 建议重启MySQL容器以应用新的配置文件权限");
                    }
                }
            }

            #[cfg(windows)]
            {
                // Windows系统上的权限检查
                if let Err(e) = self.check_windows_mysql_config_permissions(&mysql_cnf) {
                    warn!("Windows MySQL配置文件权限检查失败: {}", e);
                }
            }
        }

        Ok(())
    }
}
