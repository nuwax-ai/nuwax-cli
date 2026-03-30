use crate::docker_service::error::{DockerServiceError, DockerServiceResult};
use std::path::{Path, PathBuf};
use std::process::Command;
use tracing::{debug, error, info, warn};

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
        info!("{}", t!("script_permissions.check_start"));

        // 检测运行环境
        let is_windows = cfg!(target_os = "windows");
        if is_windows {
            info!("{}", t!("script_permissions.windows_env_detected"));

            // 执行Windows兼容性检查
            if let Ok(suggestions) = self.windows_compatibility_check().await {
                if !suggestions.is_empty() {
                    warn!("{}", t!("script_permissions.windows_suggestions_title"));
                    for suggestion in suggestions {
                        warn!("{}", t!("script_permissions.suggestion_item", item = suggestion));
                    }
                }
            }
        }

        let script_paths = self.find_docker_scripts()?;

        if script_paths.is_empty() {
            debug!("{}", t!("script_permissions.debug_no_script_files_found"));
            return Ok(());
        }

        info!(
            "{}",
            t!("script_permissions.files_found_for_check", count = script_paths.len())
        );

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
                            info!(
                                "{}",
                                t!("script_permissions.line_endings_converted", path = script_path.display())
                            );
                        }
                    }
                    Err(e) => {
                        warn!(
                            "{}",
                            t!(
                                "script_permissions.line_endings_convert_failed",
                                path = script_path.display(),
                                error = e.to_string()
                            )
                        );
                    }
                }
            }

            // 检查和修复权限
            match self.check_and_fix_file_permission(&script_path).await {
                Ok(was_fixed) => {
                    if was_fixed {
                        fixed_count += 1;
                        info!(
                            "{}",
                            t!("script_permissions.fix_permission_success", path = script_path.display())
                        );
                    } else {
                        debug!(
                            "{}",
                            t!("script_permissions.debug_script_permission_ok", path = script_path.display())
                        );
                    }
                }
                Err(e) => {
                    error_count += 1;
                    error!(
                        "{}",
                        t!(
                            "script_permissions.fix_permission_failed",
                            path = script_path.display(),
                            error = e.to_string()
                        )
                    );

                    // Windows环境提供额外建议
                    if is_windows {
                        warn!("{}", t!("script_permissions.windows_hint_title"));
                        warn!("{}", t!("script_permissions.windows_hint_1"));
                        warn!("{}", t!("script_permissions.windows_hint_2"));
                        warn!("{}", t!("script_permissions.windows_hint_3"));
                    }
                }
            }
        }

        // 汇总结果
        if converted_count > 0 {
            info!(
                "{}",
                t!(
                    "script_permissions.converted_line_endings_count",
                    count = converted_count
                )
            );
        }

        if fixed_count > 0 {
            info!(
                "{}",
                t!("script_permissions.fixed_permissions_count", count = fixed_count)
            );
        }

        if error_count > 0 {
            warn!(
                "{}",
                t!("script_permissions.failed_count_need_manual", count = error_count)
            );
            if is_windows {
                warn!("{}", t!("script_permissions.windows_users_can_try_title"));
                warn!("{}", t!("script_permissions.windows_try_1"));
                warn!("{}", t!("script_permissions.windows_try_2"));
                warn!("{}", t!("script_permissions.windows_try_3"));
            }
        } else {
            info!("{}", t!("script_permissions.check_done"));
        }

        Ok(())
    }

    /// 查找Docker相关的脚本文件
    fn find_docker_scripts(&self) -> DockerServiceResult<Vec<PathBuf>> {
        let mut script_paths = Vec::new();

        // 递归查找工作目录下的所有 .sh 文件
        Self::find_shell_scripts_recursive(&self.work_dir, &mut script_paths)?;

        // 去重
        script_paths.sort();
        script_paths.dedup();

        info!(
            "{}",
            t!("script_permissions.dynamic_scan_found", count = script_paths.len())
        );
        for script in &script_paths {
            debug!("{}", t!("script_permissions.debug_script_found", path = script.display()));
        }

        Ok(script_paths)
    }

    /// 递归查找shell脚本文件
    fn find_shell_scripts_recursive(
        dir: &Path,
        script_paths: &mut Vec<PathBuf>,
    ) -> DockerServiceResult<()> {
        if !dir.is_dir() {
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
                // 递归搜索子目录
                Self::find_shell_scripts_recursive(&path, script_paths)?;
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
            debug!(
                "{}",
                t!("script_permissions.debug_unknown_os_skip_permission_check", path = script_path.display())
            );
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
            debug!(
                "{}",
                t!("script_permissions.debug_script_already_executable", path = script_path.display())
            );
            return Ok(false);
        }

        // 添加执行权限
        info!(
            "{}",
            t!("script_permissions.add_exec_permission", path = script_path.display())
        );
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
        info!(
            "{}",
            t!("script_permissions.windows_check_permission", path = script_path.display())
        );

        // Windows下，我们假设脚本可能需要设置执行权限
        // 因为Windows文件系统挂载到Docker容器时可能丢失执行权限

        // 检查是否已经有执行权限（通过尝试chmod来验证）
        if self.verify_windows_execute_permission(script_path).await? {
            debug!(
                "{}",
                t!("script_permissions.debug_windows_script_should_executable", path = script_path.display())
            );
            return Ok(false);
        }

        // 尝试设置执行权限
        info!(
            "{}",
            t!("script_permissions.add_exec_permission", path = script_path.display())
        );
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
        debug!(
            "{}",
            t!("script_permissions.debug_windows_verify_failed_assume_need_set")
        );
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
            {
                if output.status.success() {
                    debug!("{}", t!("script_permissions.debug_git_bash_verify_executable"));
                    return Ok(true);
                }
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
                    debug!("{}", t!("script_permissions.debug_wsl_verify_executable"));
                    return Ok(true);
                } else {
                    debug!("{}", t!("script_permissions.debug_wsl_verify_not_executable"));
                }
            }
            Err(e) => {
                debug!(
                    "{}",
                    t!("script_permissions.debug_wsl_verify_failed", error = e.to_string())
                );
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
            warn!("{}", t!("script_permissions.unknown_os_skip_permission"));
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

        info!(
            "{}",
            t!("script_permissions.add_exec_permission_done", path = script_path.display())
        );
        Ok(())
    }

    #[cfg(not(unix))]
    async fn add_execute_permission_unix(&self, _script_path: &Path) -> DockerServiceResult<()> {
        Ok(())
    }

    /// Windows系统下添加执行权限
    async fn add_execute_permission_windows(&self, script_path: &Path) -> DockerServiceResult<()> {
        info!(
            "{}",
            t!("script_permissions.windows_set_permission", path = script_path.display())
        );

        // 首先检查文件是否存在
        if !script_path.exists() {
            warn!(
                "{}",
                t!("script_permissions.script_file_not_exists", path = script_path.display())
            );
            return Ok(());
        }

        // 检查文件扩展名
        if let Some(extension) = script_path.extension() {
            if extension != "sh" && extension != "bash" {
                debug!(
                    "{}",
                    t!("script_permissions.debug_skip_non_shell_script", path = script_path.display())
                );
                return Ok(());
            }
        }

        let mut success_methods: Vec<String> = Vec::new();

        // 方法1: 尝试使用Git Bash的chmod
        if let Ok(result) = self.try_git_bash_chmod(script_path).await {
            if result {
                success_methods.push("Git Bash".to_string());
            }
        }

        // 方法2: 尝试使用WSL的chmod
        if let Ok(result) = self.try_wsl_chmod(script_path).await {
            if result {
                success_methods.push("WSL".to_string());
            }
        }

        // 方法3: 尝试直接chmod（如果可用）
        if let Ok(result) = self.try_direct_chmod(script_path).await {
            if result {
                success_methods.push(t!("script_permissions.method_direct_chmod").to_string());
            }
        }

        // 方法4: 尝试修复行尾符
        if let Ok(result) = self.fix_line_endings(script_path).await {
            if result {
                success_methods.push(t!("script_permissions.method_line_endings_fix").to_string());
            }
        }

        if !success_methods.is_empty() {
            info!(
                "{}",
                t!(
                    "script_permissions.set_permission_success_methods",
                    methods = success_methods.join(", ")
                )
            );
            return Ok(());
        }

        // 所有自动方法都失败，提供详细的手动操作指导
        warn!("{}", t!("script_permissions.auto_set_failed_manual_needed"));
        warn!("{}", t!("script_permissions.manual_guide_title"));
        warn!(
            "{}",
            t!(
                "script_permissions.manual_guide_git_bash_cd",
                path = script_path
                    .parent()
                    .unwrap_or_else(|| Path::new("."))
                    .display()
            )
        );
        warn!(
            "{}",
            t!(
                "script_permissions.manual_guide_git_bash_chmod",
                name = script_path.file_name().unwrap().to_string_lossy()
            )
        );
        warn!(
            "{}",
            t!(
                "script_permissions.manual_guide_wsl_chmod",
                path = self
                    .convert_to_wsl_path(script_path)
                    .unwrap_or_else(|_| script_path.display().to_string())
            )
        );
        warn!(
            "{}",
            t!(
                "script_permissions.manual_guide_powershell_chmod",
                path = script_path.display()
            )
        );
        warn!("{}", t!("script_permissions.manual_guide_compose_volume_1"));
        warn!("{}", t!("script_permissions.manual_guide_compose_volume_2"));
        warn!("{}", t!("script_permissions.manual_guide_compose_cmd"));
        warn!("{}", t!("script_permissions.manual_guide_encoding_1"));
        warn!("{}", t!("script_permissions.manual_guide_encoding_2"));
        warn!("{}", t!("script_permissions.manual_guide_encoding_3"));
        warn!("{}", t!("script_permissions.manual_guide_encoding_4"));
        warn!("{}", t!("script_permissions.manual_guide_hint_title"));
        warn!("{}", t!("script_permissions.manual_guide_hint_1"));
        warn!("{}", t!("script_permissions.manual_guide_hint_2"));
        warn!("{}", t!("script_permissions.manual_guide_hint_3"));

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
            {
                if output.status.success() {
                    debug!(
                        "{}",
                        t!("script_permissions.debug_git_bash_chmod_success", path = bash_path)
                    );
                    return Ok(true);
                }
            }
        }

        debug!("{}", t!("script_permissions.debug_git_bash_chmod_unavailable"));
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
                    debug!("{}", t!("script_permissions.debug_wsl_chmod_success"));
                    return Ok(true);
                } else {
                    let stderr = String::from_utf8_lossy(&output.stderr);
                    debug!("{}", t!("script_permissions.debug_wsl_chmod_failed", error = stderr));
                }
            }
            Err(e) => {
                debug!(
                    "{}",
                    t!("script_permissions.debug_wsl_chmod_unavailable", error = e.to_string())
                );
            }
        }

        debug!("{}", t!("script_permissions.debug_wsl_chmod_not_available"));
        Ok(false)
    }

    /// 尝试直接chmod
    async fn try_direct_chmod(&self, script_path: &Path) -> DockerServiceResult<bool> {
        if let Ok(output) = Command::new("chmod").arg("+x").arg(script_path).output() {
            if output.status.success() {
                debug!("{}", t!("script_permissions.debug_direct_chmod_success"));
                return Ok(true);
            }
        }

        debug!("{}", t!("script_permissions.debug_direct_chmod_not_available"));
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
    pub async fn fix_specific_script(&self, script_name: &str) -> DockerServiceResult<()> {
        let script_path = self.work_dir.join("config").join(script_name);

        if !script_path.exists() {
            return Err(DockerServiceError::FileSystem(format!(
                "{}",
                t!("script_permissions.script_file_not_exists", path = script_path.display())
            )));
        }

        info!(
            "{}",
            t!("script_permissions.fix_specific_script", name = script_name)
        );
        self.check_and_fix_file_permission(&script_path).await?;
        Ok(())
    }

    /// 预检查常见问题脚本
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
            debug!(
                "{}",
                t!("script_permissions.debug_already_unix_line_endings", path = script_path.display())
            );
            return Ok(false);
        }

        info!(
            "{}",
            t!(
                "script_permissions.windows_line_endings_found_convert",
                path = script_path.display()
            )
        );

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

        debug!("{}", t!("script_permissions.debug_backup_created", path = backup_path.display()));

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

        info!(
            "{}",
            t!("script_permissions.line_endings_convert_done", path = script_path.display())
        );
        info!(
            "{}",
            t!("script_permissions.line_endings_backup_file", path = backup_path.display())
        );

        Ok(true)
    }

    /// 检查脚本编码问题
    pub async fn check_script_encoding(&self, script_path: &Path) -> DockerServiceResult<bool> {
        if !script_path.exists() {
            return Ok(false);
        }

        // 尝试以UTF-8读取文件
        match std::fs::read_to_string(script_path) {
            Ok(content) => {
                // 检查是否包含BOM
                if content.starts_with('\u{FEFF}') {
                    warn!(
                        "{}",
                        t!("script_permissions.script_has_bom", path = script_path.display())
                    );
                    warn!("{}", t!("script_permissions.script_has_bom_hint"));
                    return Ok(false);
                }

                // 检查是否包含Windows行尾符
                if content.contains("\r\n") {
                    warn!(
                        "{}",
                        t!(
                            "script_permissions.script_has_windows_line_endings",
                            path = script_path.display()
                        )
                    );
                    return Ok(false);
                }

                debug!(
                    "{}",
                    t!("script_permissions.debug_script_encoding_passed", path = script_path.display())
                );
                Ok(true)
            }
            Err(e) => {
                warn!(
                    "{}",
                    t!(
                        "script_permissions.script_encoding_check_failed",
                        path = script_path.display(),
                        error = e.to_string()
                    )
                );
                warn!("{}", t!("script_permissions.script_encoding_check_failed_hint"));
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

        info!("{}", t!("script_permissions.windows_compat_check_start"));

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
                    debug!("{}", t!("script_permissions.debug_wsl_check_failed"));
                }
            }
            Err(e) => {
                debug!(
                    "{}",
                    t!("script_permissions.debug_wsl_unavailable", error = e.to_string())
                );
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
                    debug!("{}", t!("script_permissions.debug_git_config_check_failed"));
                }
            }
            Err(e) => {
                debug!(
                    "{}",
                    t!("script_permissions.debug_git_unavailable", error = e.to_string())
                );
                // 不添加建议，因为Git不是必需的
            }
        }

        // 动态检查所有脚本文件（可选诊断）
        match self.find_docker_scripts() {
            Ok(scripts) => {
                if scripts.is_empty() {
                    debug!("{}", t!("script_permissions.debug_no_scripts_skip_encoding_check"));
                } else {
                    debug!(
                        "{}",
                        t!("script_permissions.debug_start_encoding_check", count = scripts.len())
                    );
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
                        debug!(
                            "{}",
                            t!("script_permissions.debug_encoding_issues_found", count = encoding_issues)
                        );
                    } else {
                        debug!("{}", t!("script_permissions.debug_all_script_encoding_passed"));
                    }
                }
            }
            Err(e) => {
                debug!(
                    "{}",
                    t!("script_permissions.debug_script_scan_failed_non_critical", error = e.to_string())
                );
                // 不添加建议，因为扫描失败不影响核心功能
            }
        }

        if suggestions.is_empty() {
            info!("{}", t!("script_permissions.windows_compat_check_passed"));
        } else {
            warn!(
                "{}",
                t!(
                    "script_permissions.windows_compat_issues_found",
                    count = suggestions.len()
                )
            );
        }

        Ok(suggestions)
    }

    /// 为Windows用户提供一键修复脚本权限的方法
    pub async fn fix_windows_script_permissions(&self) -> DockerServiceResult<()> {
        if !cfg!(target_os = "windows") {
            return Ok(());
        }

        info!("{}", t!("script_permissions.windows_one_click_fix_start"));

        // 查找所有脚本文件
        let scripts = self.find_docker_scripts()?;

        if scripts.is_empty() {
            info!("{}", t!("script_permissions.windows_one_click_fix_no_scripts"));
            return Ok(());
        }

        info!(
            "{}",
            t!(
                "script_permissions.windows_one_click_fix_found",
                count = scripts.len()
            )
        );

        let mut success_count = 0;
        let mut fail_count = 0;

        for script_path in &scripts {
            match self.check_and_fix_file_permission(script_path).await {
                Ok(true) => {
                    info!(
                        "{}",
                        t!(
                            "script_permissions.windows_one_click_fix_success",
                            path = script_path.display()
                        )
                    );
                    success_count += 1;
                }
                Ok(false) => {
                    debug!(
                        "{}",
                        t!("script_permissions.debug_script_permission_already_correct", path = script_path.display())
                    );
                }
                Err(e) => {
                    warn!(
                        "{}",
                        t!(
                            "script_permissions.windows_one_click_fix_failed",
                            path = script_path.display(),
                            error = e.to_string()
                        )
                    );
                    fail_count += 1;
                }
            }
        }

        info!("{}", t!("script_permissions.windows_one_click_fix_done_title"));
        info!(
            "{}",
            t!(
                "script_permissions.windows_one_click_fix_done_success",
                count = success_count
            )
        );
        info!(
            "{}",
            t!(
                "script_permissions.windows_one_click_fix_done_failed",
                count = fail_count
            )
        );
        info!(
            "{}",
            t!(
                "script_permissions.windows_one_click_fix_done_total",
                count = scripts.len()
            )
        );

        if fail_count > 0 {
            warn!("{}", t!("script_permissions.windows_one_click_fix_fail_hint"));
        }

        Ok(())
    }
}
