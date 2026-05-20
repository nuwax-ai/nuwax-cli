use crate::docker_service::error::{DockerServiceError, DockerServiceResult};
use std::path::{Path, PathBuf};
use std::process::Command;
use tracing::{debug, error, info, warn};

/// 递归查找脚本文件的最大深度限制
const MAX_SCRIPT_SCAN_DEPTH: usize = 5;

/// 脚本权限管理器
pub struct ScriptPermissionManager {
    work_dir: PathBuf,
}

impl ScriptPermissionManager {
    /// 创建新的脚本权限管理器
    pub fn new(work_dir: PathBuf) -> Self {
        Self { work_dir }
    }

    /// 检查并修复 Docker Compose 相关脚本权限
    pub async fn check_and_fix_script_permissions(&self) -> DockerServiceResult<()> {
        info!("🔍 Checking Docker related script permissions...");

        // 检测运行环境
        let is_windows = cfg!(target_os = "windows");
        if is_windows {
            info!("🪟 Windows environment detected, running cross-platform compatibility checks");

            // 执行Windows兼容性检查
            if let Ok(suggestions) = self.windows_compatibility_check().await
                && !suggestions.is_empty() {
                    warn!("🪟 Windows suggestions:");
                    for suggestion in suggestions {
                        warn!("  • {item}", item = suggestion);
                    }
                }
        }

        let script_paths = self.find_docker_scripts()?;

        if script_paths.is_empty() {
            debug!("No script files found for permission check");
            return Ok(());
        }

        info!("Found {count} script files to check permissions", count = script_paths.len());

        let mut fixed_count = 0;
        let mut converted_count = 0;
        let mut error_count = 0;

        for script_path in script_paths {
            // Windows环境下，先检查并修复行尾符
            if is_windows {
                match self.fix_line_endings(&script_path).await {
                    Ok(was_converted) => {
                        if was_converted {
                            converted_count += 1;
                            info!("🔄 Line endings converted: {path}", path = script_path.display());
                        }
                    }
                    Err(e) => {
                        warn!("⚠️  Failed converting line endings {path}: {error}", path = script_path.display(), error = e.to_string());
                    }
                }
            }

            // 检查和修复权限
            match self.check_and_fix_file_permission(&script_path).await {
                Ok(was_fixed) => {
                    if was_fixed {
                        fixed_count += 1;
                        info!("✅ Script permission fixed: {path}", path = script_path.display());
                    } else {
                        debug!("✓ Script permission is OK: {path}", path = script_path.display());
                    }
                }
                Err(e) => {
                    error_count += 1;
                    error!("❌ Failed to fix script permission {path}: {error}", path = script_path.display(), error = e.to_string());

                    // Windows环境提供额外建议
                    if is_windows {
                        warn!("💡 Windows hints:");
                        warn!("  - Ensure Docker Desktop is running");
                        warn!("  - Try running command as Administrator");
                        warn!("  - Check whether file is occupied by other programs");
                    }
                }
            }
        }

        // 汇总结果
        if converted_count > 0 {
            info!("🔄 Converted line endings for {count} scripts",
                    count = converted_count
                );
        }

        if fixed_count > 0 {
            info!("🛠️  Fixed execute permission for {count} scripts", count = fixed_count);
        }

        if error_count > 0 {
            warn!("⚠️  {count} scripts failed, manual handling may be required", count = error_count);
            if is_windows {
                warn!("🪟 Windows users can try:");
                warn!("  1. Run in Git Bash: chmod +x config/docker-entrypoint.sh");
                warn!("  2. Or run in WSL: chmod +x config/docker-entrypoint.sh");
                warn!("  3. Ensure Docker file sharing is enabled");
            }
        } else {
            info!("✅ Script permission check completed");
        }

        Ok(())
    }

    /// 查找Docker相关的脚本文件
    fn find_docker_scripts(&self) -> DockerServiceResult<Vec<PathBuf>> {
        let mut script_paths = Vec::new();

        // 递归查找工作目录下的所有 .sh 文件（限制最大深度）
        Self::find_shell_scripts_recursive(&self.work_dir, &mut script_paths, MAX_SCRIPT_SCAN_DEPTH)?;

        // 去重
        script_paths.sort();
        script_paths.dedup();

        info!("🔍 Dynamically scanned {count} script files (max depth: {depth})", count = script_paths.len(), depth = MAX_SCRIPT_SCAN_DEPTH);
        for script in &script_paths {
debug!("Script found: {path}", path = script.display());
        }

        Ok(script_paths)
    }

    /// 递归查找shell脚本文件（带深度限制）
    fn find_shell_scripts_recursive(
        dir: &Path,
        script_paths: &mut Vec<PathBuf>,
        max_depth: usize,
    ) -> DockerServiceResult<()> {
        if !dir.is_dir() || max_depth == 0 {
            return Ok(());
        }

        let entries = std::fs::read_dir(dir).map_err(|e| {
            DockerServiceError::FileSystem(format!(
                "{}",
                t!(
                    "script_permissions.read_dir_failed",
                    path = dir.display(),
                    error = e.to_string()
                )
            ))
        })?;

        for entry in entries {
            let entry = entry.map_err(|e| {
                DockerServiceError::FileSystem(format!(
                    "{}",
                    t!("script_permissions.read_dir_entry_failed", error = e.to_string())
                ))
            })?;
            let path = entry.path();

            if path.is_dir() {
                let dir_name = path.file_name().unwrap_or_default().to_string_lossy();

                // 跳过不需要遍历的目录（数据目录、上传目录等）
                if client_core::constants::docker::EXCLUDE_DIRS
                    .contains(&dir_name.as_ref())
                {
                    debug!("Skipping protected directory: {path}", path = path.display());
                    continue;
                }

                // 递归搜索子目录（深度减1）
                Self::find_shell_scripts_recursive(&path, script_paths, max_depth - 1)?;
            } else if path.extension().and_then(|s| s.to_str()) == Some("sh") {
                script_paths.push(path);
            }
        }

        Ok(())
    }

    /// 检查并修复单个文件权限
    async fn check_and_fix_file_permission(&self, script_path: &Path) -> DockerServiceResult<bool> {
        // 检查文件是否存在
        if !script_path.exists() {
            return Err(DockerServiceError::FileSystem(format!(
                "{}",
                t!("script_permissions.script_file_not_exists", path = script_path.display())
            )));
        }

        // 检查当前权限
        let metadata = std::fs::metadata(script_path).map_err(|e| {
            DockerServiceError::FileSystem(format!(
                "{}",
                t!(
                    "script_permissions.get_file_metadata_failed",
                    path = script_path.display(),
                    error = e.to_string()
                )
            ))
        })?;

        if cfg!(unix) {
            // Unix/Linux/macOS 系统权限检查
            self.check_unix_permissions(script_path, &metadata).await
        } else if cfg!(windows) {
            // Windows 系统权限检查
            self.check_windows_permissions(script_path, &metadata).await
        } else {
            debug!("Unknown OS, skip permission check: {path}", path = script_path.display());
            Ok(false)
        }
    }

    /// Unix系统权限检查
    #[cfg(unix)]
    async fn check_unix_permissions(
        &self,
        script_path: &Path,
        metadata: &std::fs::Metadata,
    ) -> DockerServiceResult<bool> {
        use std::os::unix::fs::PermissionsExt;
        let mode = metadata.permissions().mode();
        let is_executable = (mode & 0o111) != 0; // 检查是否有执行权限

        if is_executable {
            debug!("Script already executable: {path}", path = script_path.display());
            return Ok(false);
        }

        // 添加执行权限
        info!("Adding execute permission: {path}", path = script_path.display());
        self.add_execute_permission(script_path).await?;
        Ok(true)
    }

    /// Windows系统权限检查
    #[cfg(not(unix))]
    async fn check_unix_permissions(
        &self,
        _script_path: &Path,
        _metadata: &std::fs::Metadata,
    ) -> DockerServiceResult<bool> {
        Ok(false)
    }

    /// Windows系统权限检查和修复
    async fn check_windows_permissions(
        &self,
        script_path: &Path,
        _metadata: &std::fs::Metadata,
    ) -> DockerServiceResult<bool> {
        info!("🪟 Checking script permission on Windows: {path}", path = script_path.display());

        // Windows下，我们假设脚本可能需要设置执行权限
        // 因为Windows文件系统挂载到Docker容器时可能丢失执行权限

        // 检查是否已经有执行权限（通过尝试chmod来验证）
        if self.verify_windows_execute_permission(script_path).await? {
            debug!("Script should be executable in container: {path}", path = script_path.display());
            return Ok(false);
        }

        // 尝试设置执行权限
        info!("Adding execute permission: {path}", path = script_path.display());
        self.add_execute_permission(script_path).await?;
        Ok(true)
    }

    /// 验证Windows下的脚本执行权限
    async fn verify_windows_execute_permission(
        &self,
        script_path: &Path,
    ) -> DockerServiceResult<bool> {
        // 在Windows下，我们通过尝试chmod来验证权限
        // 如果chmod成功且没有实际改变，说明权限已经正确

        // 方法1: 尝试Git Bash验证
        if let Ok(result) = self.verify_with_git_bash(script_path).await {
            return Ok(result);
        }

        // 方法2: 尝试WSL验证
        if let Ok(result) = self.verify_with_wsl(script_path).await {
            return Ok(result);
        }

        // 默认假设需要设置权限
        debug!("Cannot verify Windows script permission, assume setting is needed");
        Ok(false)
    }

    /// 使用Git Bash验证权限
    async fn verify_with_git_bash(&self, script_path: &Path) -> DockerServiceResult<bool> {
        let git_bash_paths = vec![
            "C:\\Program Files\\Git\\bin\\bash.exe",
            "C:\\Program Files (x86)\\Git\\bin\\bash.exe",
            "bash",
        ];

        for bash_path in git_bash_paths {
            if let Ok(output) = Command::new(bash_path)
                .arg("-c")
                .arg(format!("test -x \"{}\"", script_path.display()))
                .output()
                && output.status.success() {
                    debug!("Git Bash verify: script is executable");
                    return Ok(true);
                }
        }

        Ok(false)
    }

    /// 使用WSL验证权限
    async fn verify_with_wsl(&self, script_path: &Path) -> DockerServiceResult<bool> {
        let wsl_path = self.convert_to_wsl_path(script_path)?;

        match Command::new("wsl")
            .arg("test")
            .arg("-x")
            .arg(&wsl_path)
            .output()
        {
            Ok(output) => {
                if output.status.success() {
                    debug!("WSL verify: script is executable");
                    return Ok(true);
                } else {
                    debug!("WSL verify: script is not executable");
                }
            }
            Err(e) => {
                debug!("WSL verification failed, WSL may not be installed: {error}", error = e.to_string());
            }
        }

        Ok(false)
    }

    /// 为脚本添加执行权限（跨平台）
    async fn add_execute_permission(&self, script_path: &Path) -> DockerServiceResult<()> {
        if cfg!(unix) {
            // Unix/Linux/macOS系统
            self.add_execute_permission_unix(script_path).await
        } else if cfg!(windows) {
            // Windows系统
            self.add_execute_permission_windows(script_path).await
        } else {
            warn!("Unknown OS, skip permission setup");
            Ok(())
        }
    }

    /// Unix系统下添加执行权限
    #[cfg(unix)]
    async fn add_execute_permission_unix(&self, script_path: &Path) -> DockerServiceResult<()> {
        let output = Command::new("chmod")
            .arg("+x")
            .arg(script_path)
            .output()
            .map_err(|e| {
                DockerServiceError::Permission(format!(
                    "{}",
                    t!("script_permissions.run_chmod_failed", error = e.to_string())
                ))
            })?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(DockerServiceError::Permission(format!(
                "{}",
                t!("script_permissions.chmod_exit_failed", error = stderr)
            )));
        }

        info!("✅ Execute permission added: {path}", path = script_path.display());
        Ok(())
    }

    #[cfg(not(unix))]
    async fn add_execute_permission_unix(&self, _script_path: &Path) -> DockerServiceResult<()> {
        Ok(())
    }

    /// Windows系统下添加执行权限
    async fn add_execute_permission_windows(&self, script_path: &Path) -> DockerServiceResult<()> {
        info!("🪟 Setting script permission on Windows: {path}", path = script_path.display());

        // 首先检查文件是否存在
        if !script_path.exists() {
            warn!("⚠️ Script file does not exist: {path}", path = script_path.display());
            return Ok(());
        }

        // 检查文件扩展名
        if let Some(extension) = script_path.extension()
            && extension != "sh" && extension != "bash" {
                debug!("Skip non-shell script: {path}", path = script_path.display());
                return Ok(());
            }

        let mut success_methods: Vec<String> = Vec::new();

        // 方法1: 尝试使用Git Bash的chmod
        if let Ok(result) = self.try_git_bash_chmod(script_path).await
            && result {
                success_methods.push("Git Bash".to_string());
            }

        // 方法2: 尝试使用WSL的chmod
        if let Ok(result) = self.try_wsl_chmod(script_path).await
            && result {
                success_methods.push("WSL".to_string());
            }

        // 方法3: 尝试直接chmod（如果可用）
        if let Ok(result) = self.try_direct_chmod(script_path).await
            && result {
                success_methods.push(t!("script_permissions.method_direct_chmod").to_string());
            }

        // 方法4: 尝试修复行尾符
        if let Ok(result) = self.fix_line_endings(script_path).await
            && result {
                success_methods.push(t!("script_permissions.method_line_endings_fix").to_string());
            }

        if !success_methods.is_empty() {
            info!("✅ Script permission set successfully, methods: {methods}", methods = success_methods.join(", "));
            return Ok(());
        }

        // 所有自动方法都失败，提供详细的手动操作指导
        warn!("⚠️ Automatic permission setup failed, manual action required:");
        warn!("🪟 Windows host script permission guide:");
        warn!("Git Bash: cd \"{path}\"", path = script_path
                    .parent()
                    .unwrap_or_else(|| Path::new("."))
                    .display());
        warn!("Git Bash: chmod +x \"{name}\"", name = script_path.file_name().unwrap().to_string_lossy());
        warn!("WSL: chmod +x \"{path}\"", path = self
                    .convert_to_wsl_path(script_path)
                    .unwrap_or_else(|_| script_path.display().to_string()));
        warn!("PowerShell: bash -c \"chmod +x '{path}'\"", path = script_path.display());
        warn!("docker-compose volumes: - ./config:/app/config:ro");
        warn!("docker-compose volumes: - ./script:/app/script:ro");
        warn!("entrypoint command: sh -c \"chmod +x /app/script/*.sh && your-original-command\"");
        warn!("Encoding check: ensure UTF-8 (without BOM)");
        warn!("Encoding check: ensure LF rather than CRLF");
        warn!("Encoding check: add #!/bin/bash at file start");
        warn!("Encoding check: set line endings to LF in editor");
        warn!("💡 Hint: if script fails inside container, you can:");
        warn!("  1. Add env var in docker-compose.yml: CHMOD_SCRIPTS=true");
        warn!("  2. Or manually run at startup: chmod +x /path/to/script.sh");
        warn!("  3. Use Dockerfile COPY with permission: COPY --chmod=+x script.sh /app/");

        // 不返回错误，让程序继续运行，用户可以手动修复
        Ok(())
    }

    /// 尝试使用Git Bash的chmod
    async fn try_git_bash_chmod(&self, script_path: &Path) -> DockerServiceResult<bool> {
        // 查找Git Bash路径
        let git_bash_paths = vec![
            "C:\\Program Files\\Git\\bin\\bash.exe",
            "C:\\Program Files (x86)\\Git\\bin\\bash.exe",
            "bash", // 如果在PATH中
        ];

        for bash_path in git_bash_paths {
            if let Ok(output) = Command::new(bash_path)
                .arg("-c")
                .arg(format!("chmod +x \"{}\"", script_path.display()))
                .output()
                && output.status.success() {
                    debug!("Git Bash chmod succeeded: {path}", path = bash_path);
                    return Ok(true);
                }
        }

        debug!("Git Bash chmod is unavailable");
        Ok(false)
    }

    /// 尝试使用WSL的chmod
    async fn try_wsl_chmod(&self, script_path: &Path) -> DockerServiceResult<bool> {
        // 转换Windows路径为WSL路径
        let wsl_path = self.convert_to_wsl_path(script_path)?;

        match Command::new("wsl")
            .arg("chmod")
            .arg("+x")
            .arg(&wsl_path)
            .output()
        {
            Ok(output) => {
                if output.status.success() {
                    debug!("WSL chmod succeeded");
                    return Ok(true);
                } else {
                    let stderr = String::from_utf8_lossy(&output.stderr);
                    debug!("WSL chmod failed: {error}", error = stderr);
                }
            }
            Err(e) => {
                debug!("WSL chmod unavailable, WSL may not be installed: {error}", error = e.to_string());
            }
        }

        debug!("WSL chmod is unavailable");
        Ok(false)
    }

    /// 尝试直接chmod
    async fn try_direct_chmod(&self, script_path: &Path) -> DockerServiceResult<bool> {
        if let Ok(output) = Command::new("chmod").arg("+x").arg(script_path).output()
            && output.status.success() {
                debug!("Direct chmod succeeded");
                return Ok(true);
            }

        debug!("Direct chmod is unavailable");
        Ok(false)
    }

    /// 转换Windows路径为WSL路径
    fn convert_to_wsl_path(&self, windows_path: &Path) -> DockerServiceResult<String> {
        let path_str = windows_path.to_string_lossy();

        // 简单的路径转换逻辑
        if path_str.starts_with("C:") {
            let wsl_path = path_str.replace("C:", "/mnt/c").replace("\\", "/");
            Ok(wsl_path)
        } else if path_str.starts_with("D:") {
            let wsl_path = path_str.replace("D:", "/mnt/d").replace("\\", "/");
            Ok(wsl_path)
        } else {
            // 相对路径，直接使用
            Ok(path_str.replace("\\", "/"))
        }
    }

    /// 手动修复特定脚本权限
    #[allow(dead_code)]
    pub async fn fix_specific_script(&self, script_name: &str) -> DockerServiceResult<()> {
        let script_path = self.work_dir.join("config").join(script_name);

        if !script_path.exists() {
            return Err(DockerServiceError::FileSystem(format!(
                "{}",
                t!("script_permissions.script_file_not_exists", path = script_path.display())
            )));
        }

        info!("🛠️  Fix specific script permission: {name}", name = script_name);
        self.check_and_fix_file_permission(&script_path).await?;
        Ok(())
    }

    /// 预检查常见问题脚本
    #[allow(dead_code)]
    pub async fn precheck_common_script_issues(&self) -> DockerServiceResult<Vec<String>> {
        let mut issues = Vec::new();

        // 检查docker-entrypoint.sh权限
        let entrypoint_script = self.work_dir.join("config/docker-entrypoint.sh");
        if entrypoint_script.exists() {
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                if let Ok(metadata) = std::fs::metadata(&entrypoint_script) {
                    let mode = metadata.permissions().mode();
                        if (mode & 0o111) == 0 {
                            issues.push(
                                t!(
                                    "script_permissions.script_missing_exec_permission",
                                    path = entrypoint_script.display()
                                )
                                .to_string(),
                            );
                        }
                }
            }
        }

        // 检查其他常见脚本
        let common_scripts = vec![
            "config/video_analysis/entrypoint-master.sh",
            "config/video_analysis/entrypoint-worker.sh",
            "script/init-minio.sh",
        ];

        for script_name in common_scripts {
            let script_path = self.work_dir.join(script_name);
            if script_path.exists() {
                #[cfg(unix)]
                {
                    use std::os::unix::fs::PermissionsExt;
                    if let Ok(metadata) = std::fs::metadata(&script_path) {
                        let mode = metadata.permissions().mode();
                        if (mode & 0o111) == 0 {
                            issues.push(
                                t!(
                                    "script_permissions.script_missing_exec_permission",
                                    path = script_path.display()
                                )
                                .to_string(),
                            );
                        }
                    }
                }
            }
        }

        Ok(issues)
    }

    /// 修复Windows行尾符问题（CRLF -> LF）
    async fn fix_line_endings(&self, script_path: &Path) -> DockerServiceResult<bool> {
        if !script_path.exists() {
            return Ok(false);
        }

        // 读取文件内容
        let content = std::fs::read_to_string(script_path).map_err(|e| {
            DockerServiceError::FileSystem(format!(
                "{}",
                t!(
                    "script_permissions.read_script_file_failed",
                    path = script_path.display(),
                    error = e.to_string()
                )
            ))
        })?;

        // 检查是否包含Windows行尾符
        if !content.contains("\r\n") {
            debug!("Script already uses Unix line endings: {path}", path = script_path.display());
            return Ok(false);
        }

        info!("Windows line endings found, converting: {path}", path = script_path.display());

        // 转换行尾符: CRLF -> LF
        let unix_content = content.replace("\r\n", "\n");

        // 创建备份文件
        let backup_path = script_path.with_extension("sh.bak");
        std::fs::copy(script_path, &backup_path).map_err(|e| {
            DockerServiceError::FileSystem(format!(
                "{}",
                t!(
                    "script_permissions.create_backup_failed",
                    path = backup_path.display(),
                    error = e.to_string()
                )
            ))
        })?;

debug!("Backup file created: {path}", path = backup_path.display());

        // 写入转换后的内容
        std::fs::write(script_path, unix_content).map_err(|e| {
            DockerServiceError::FileSystem(format!(
                "{}",
                t!(
                    "script_permissions.write_converted_script_failed",
                    path = script_path.display(),
                    error = e.to_string()
                )
            ))
        })?;

        info!("✅ Line endings conversion done: {path}", path = script_path.display());
        info!("💾 Backup file: {path}", path = backup_path.display());

        Ok(true)
    }

    /// 检查脚本编码问题
    #[allow(dead_code)]
    pub async fn check_script_encoding(&self, script_path: &Path) -> DockerServiceResult<bool> {
        if !script_path.exists() {
            return Ok(false);
        }

        // 尝试以UTF-8读取文件
        match std::fs::read_to_string(script_path) {
            Ok(content) => {
                // 检查是否包含BOM
                if content.starts_with('\u{FEFF}') {
                    warn!("Script contains BOM: {path}", path = script_path.display());
                    warn!("Suggestion: remove BOM in text editor");
                    return Ok(false);
                }

                // 检查是否包含Windows行尾符
                if content.contains("\r\n") {
                    warn!("Script uses Windows line endings: {path}", path = script_path.display());
                    return Ok(false);
                }

                debug!("Script encoding check passed: {path}", path = script_path.display());
                Ok(true)
            }
            Err(e) => {
                warn!("Script encoding check failed {path}: {error}", path = script_path.display(), error = e.to_string());
                warn!("File may not be valid UTF-8");
                Ok(false)
            }
        }
    }

    /// Windows环境下的额外检查和建议
    pub async fn windows_compatibility_check(&self) -> DockerServiceResult<Vec<String>> {
        let mut suggestions = Vec::new();

        if !cfg!(target_os = "windows") {
            return Ok(suggestions);
        }

        info!("🪟 Running Windows compatibility checks...");

        // 检查Docker是否运行
        if Command::new("docker").arg("version").output().is_err() {
            suggestions.push(t!("script_permissions.suggest_docker_desktop_running").to_string());
        }

        // 检查是否有WSL2（如果WSL已安装）
        match Command::new("wsl").arg("--list").arg("--verbose").output() {
            Ok(output) => {
                if output.status.success() {
                    let wsl_output = String::from_utf8_lossy(&output.stdout);
                    if wsl_output.contains("Version 2") {
                        suggestions.push(
                            t!("script_permissions.suggest_use_wsl2").to_string(),
                        );
                    }
                } else {
                    debug!("WSL check failed, WSL may be missing or misconfigured");
                }
            }
            Err(e) => {
                debug!("WSL is unavailable: {error}", error = e.to_string());
                // 不添加建议，因为WSL不是必需的
            }
        }

        // 检查Git配置（如果Git已安装）
        match Command::new("git")
            .arg("config")
            .arg("core.autocrlf")
            .output()
        {
            Ok(output) => {
                if output.status.success() {
                    let git_config = String::from_utf8_lossy(&output.stdout).trim().to_string();
                    if git_config == "true" {
                        suggestions.push(
                            t!("script_permissions.suggest_git_autocrlf_false").to_string(),
                        );
                    }
                } else {
                    debug!("Git config check failed, Git may be missing or not configured");
                }
            }
            Err(e) => {
                debug!("Git is unavailable: {error}", error = e.to_string());
                // 不添加建议，因为Git不是必需的
            }
        }

        // 动态检查所有脚本文件（可选诊断）
        match self.find_docker_scripts() {
            Ok(scripts) => {
                if scripts.is_empty() {
                    debug!("No script files found, skip encoding check");
                } else {
                    debug!("Start checking encoding issues for {count} script files", count = scripts.len());
                    let mut encoding_issues = 0;

                    for script_path in scripts {
                        // 检查文件编码和行尾符
                        if let Ok(content) = std::fs::read_to_string(&script_path) {
                            let mut has_issues = false;

                            if content.contains("\r\n") {
                                suggestions.push(format!(
                                    "{}",
                                    t!(
                                        "script_permissions.suggest_script_crlf_to_lf",
                                        name = script_path
                                            .file_name()
                                            .unwrap_or_else(|| std::ffi::OsStr::new("unknown"))
                                            .to_string_lossy()
                                    )
                                ));
                                has_issues = true;
                            }

                            if content.starts_with('\u{FEFF}') {
                                suggestions.push(format!(
                                    "{}",
                                    t!(
                                        "script_permissions.suggest_script_remove_bom",
                                        name = script_path
                                            .file_name()
                                            .unwrap_or_else(|| std::ffi::OsStr::new("unknown"))
                                            .to_string_lossy()
                                    )
                                ));
                                has_issues = true;
                            }

                            if has_issues {
                                encoding_issues += 1;
                            }
                        }
                    }

                    if encoding_issues > 0 {
                        debug!("Found encoding issues in {count} scripts", count = encoding_issues);
                    } else {
                        debug!("All script encoding checks passed");
                    }
                }
            }
            Err(e) => {
                debug!("Script scan failed (non-critical): {error}", error = e.to_string());
                // 不添加建议，因为扫描失败不影响核心功能
            }
        }

        if suggestions.is_empty() {
            info!("✅ Windows compatibility check passed");
        } else {
            warn!("⚠️  Found {count} Windows compatibility issues", count = suggestions.len());
        }

        Ok(suggestions)
    }

    /// 为Windows用户提供一键修复脚本权限的方法
    #[allow(dead_code)]
    pub async fn fix_windows_script_permissions(&self) -> DockerServiceResult<()> {
        if !cfg!(target_os = "windows") {
            return Ok(());
        }

        info!("🪟 Start one-click Windows script permission fix...");

        // 查找所有脚本文件
        let scripts = self.find_docker_scripts()?;

        if scripts.is_empty() {
            info!("📭 No script files found to fix");
            return Ok(());
        }

        info!("🔍 Found {count} script files, starting permission fix...", count = scripts.len());

        let mut success_count = 0;
        let mut fail_count = 0;

        for script_path in &scripts {
            match self.check_and_fix_file_permission(script_path).await {
                Ok(true) => {
                    info!("✅ Script permission fixed successfully: {path}", path = script_path.display());
                    success_count += 1;
                }
                Ok(false) => {
                    debug!("Script permission already correct: {path}", path = script_path.display());
                }
                Err(e) => {
                    warn!("❌ Failed fixing script permission: {path} - {error}", path = script_path.display(), error = e.to_string());
                    fail_count += 1;
                }
            }
        }

        info!("📊 Script permission fix completed:");
        info!("  ✅ Fixed successfully: {count}",
                count = success_count
            );
        info!("  ❌ Failed fixes: {count}",
                count = fail_count
            );
        info!("  📝 Total processed: {count}", count = scripts.len());

        if fail_count > 0 {
            warn!("💡 For failed scripts, refer to the manual guide above");
        }

        Ok(())
    }
}
