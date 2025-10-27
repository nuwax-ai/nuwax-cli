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
        info!("🔍 检查Docker相关脚本权限...");

        // 检测运行环境
        let is_windows = cfg!(target_os = "windows");
        if is_windows {
            info!("🪟 检测到Windows环境，将进行跨平台兼容性检查");

            // 执行Windows兼容性检查
            if let Ok(suggestions) = self.windows_compatibility_check().await {
                if !suggestions.is_empty() {
                    warn!("🪟 Windows环境建议:");
                    for suggestion in suggestions {
                        warn!("  • {}", suggestion);
                    }
                }
            }
        }

        let script_paths = self.find_docker_scripts()?;

        if script_paths.is_empty() {
            debug!("未找到需要检查权限的脚本文件");
            return Ok(());
        }

        info!("找到 {} 个脚本文件需要检查权限", script_paths.len());

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
                            info!("🔄 已转换行尾符: {}", script_path.display());
                        }
                    }
                    Err(e) => {
                        warn!("⚠️  行尾符转换失败 {}: {}", script_path.display(), e);
                    }
                }
            }

            // 检查和修复权限
            match self.check_and_fix_file_permission(&script_path).await {
                Ok(was_fixed) => {
                    if was_fixed {
                        fixed_count += 1;
                        info!("✅ 已修复脚本权限: {}", script_path.display());
                    } else {
                        debug!("✓ 脚本权限正常: {}", script_path.display());
                    }
                }
                Err(e) => {
                    error_count += 1;
                    error!("❌ 修复脚本权限失败 {}: {}", script_path.display(), e);

                    // Windows环境提供额外建议
                    if is_windows {
                        warn!("💡 Windows环境建议:");
                        warn!("  - 确保Docker Desktop正在运行");
                        warn!("  - 尝试以管理员身份运行命令");
                        warn!("  - 检查文件是否被其他程序占用");
                    }
                }
            }
        }

        // 汇总结果
        if converted_count > 0 {
            info!("🔄 已转换 {} 个脚本的行尾符格式", converted_count);
        }

        if fixed_count > 0 {
            info!("🛠️  已修复 {} 个脚本的执行权限", fixed_count);
        }

        if error_count > 0 {
            warn!("⚠️  {} 个脚本处理失败，可能需要手动处理", error_count);
            if is_windows {
                warn!("🪟 Windows用户可以尝试:");
                warn!("  1. 在Git Bash中运行: chmod +x config/docker-entrypoint.sh");
                warn!("  2. 或在WSL中运行: chmod +x config/docker-entrypoint.sh");
                warn!("  3. 确保Docker设置中启用了文件共享");
            }
        } else {
            info!("✅ 脚本权限检查完成");
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

        info!("🔍 动态扫描到 {} 个脚本文件", script_paths.len());
        for script in &script_paths {
            debug!("发现脚本: {}", script.display());
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
            DockerServiceError::FileSystem(format!("读取目录失败 {}: {}", dir.display(), e))
        })?;

        for entry in entries {
            let entry = entry
                .map_err(|e| DockerServiceError::FileSystem(format!("读取目录项失败: {e}")))?;
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
                "脚本文件不存在: {}",
                script_path.display()
            )));
        }

        // 检查当前权限
        let metadata = std::fs::metadata(script_path).map_err(|e| {
            DockerServiceError::FileSystem(format!(
                "获取文件元数据失败 {}: {}",
                script_path.display(),
                e
            ))
        })?;

        if cfg!(unix) {
            // Unix/Linux/macOS 系统权限检查
            self.check_unix_permissions(script_path, &metadata).await
        } else if cfg!(windows) {
            // Windows 系统权限检查
            self.check_windows_permissions(script_path, &metadata).await
        } else {
            debug!("未知操作系统，跳过权限检查: {}", script_path.display());
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
            debug!("脚本已有执行权限: {}", script_path.display());
            return Ok(false);
        }

        // 添加执行权限
        info!("正在为脚本添加执行权限: {}", script_path.display());
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
        info!("🪟 Windows环境下检查脚本权限: {}", script_path.display());

        // Windows下，我们假设脚本可能需要设置执行权限
        // 因为Windows文件系统挂载到Docker容器时可能丢失执行权限

        // 检查是否已经有执行权限（通过尝试chmod来验证）
        if self.verify_windows_execute_permission(script_path).await? {
            debug!("脚本在容器中应该有执行权限: {}", script_path.display());
            return Ok(false);
        }

        // 尝试设置执行权限
        info!("正在为脚本添加执行权限: {}", script_path.display());
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
        debug!("无法验证Windows脚本权限，假设需要设置");
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
                    debug!("Git Bash 验证: 脚本有执行权限");
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
                    debug!("WSL 验证: 脚本有执行权限");
                    return Ok(true);
                } else {
                    debug!("WSL 验证: 脚本无执行权限");
                }
            }
            Err(e) => {
                debug!("WSL验证失败，WSL可能未安装: {}", e);
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
            warn!("未知操作系统，跳过权限设置");
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
            .map_err(|e| DockerServiceError::Permission(format!("执行chmod命令失败: {e}")))?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(DockerServiceError::Permission(format!(
                "chmod命令执行失败: {stderr}"
            )));
        }

        info!("✅ 已添加执行权限: {}", script_path.display());
        Ok(())
    }

    #[cfg(not(unix))]
    async fn add_execute_permission_unix(&self, _script_path: &Path) -> DockerServiceResult<()> {
        Ok(())
    }

    /// Windows系统下添加执行权限
    async fn add_execute_permission_windows(&self, script_path: &Path) -> DockerServiceResult<()> {
        info!("🪟 Windows环境下设置脚本权限: {}", script_path.display());

        // 首先检查文件是否存在
        if !script_path.exists() {
            warn!("⚠️ 脚本文件不存在: {}", script_path.display());
            return Ok(());
        }

        // 检查文件扩展名
        if let Some(extension) = script_path.extension() {
            if extension != "sh" && extension != "bash" {
                debug!("跳过非shell脚本: {}", script_path.display());
                return Ok(());
            }
        }

        let mut success_methods = Vec::new();

        // 方法1: 尝试使用Git Bash的chmod
        if let Ok(result) = self.try_git_bash_chmod(script_path).await {
            if result {
                success_methods.push("Git Bash");
            }
        }

        // 方法2: 尝试使用WSL的chmod
        if let Ok(result) = self.try_wsl_chmod(script_path).await {
            if result {
                success_methods.push("WSL");
            }
        }

        // 方法3: 尝试直接chmod（如果可用）
        if let Ok(result) = self.try_direct_chmod(script_path).await {
            if result {
                success_methods.push("直接chmod");
            }
        }

        // 方法4: 尝试修复行尾符
        if let Ok(result) = self.fix_line_endings(script_path).await {
            if result {
                success_methods.push("行尾符修复");
            }
        }

        if !success_methods.is_empty() {
            info!(
                "✅ 脚本权限设置成功，使用的方法: {}",
                success_methods.join(", ")
            );
            return Ok(());
        }

        // 所有自动方法都失败，提供详细的手动操作指导
        warn!("⚠️ 自动设置权限失败，请手动操作:");
        warn!("🪟 Windows宿主机脚本权限设置指南:");
        warn!("");
        warn!("方法1: 使用Git Bash (推荐)");
        warn!("  1. 打开Git Bash");
        warn!("  2. 导航到脚本目录:");
        warn!(
            "     cd \"{}\"",
            script_path
                .parent()
                .unwrap_or_else(|| Path::new("."))
                .display()
        );
        warn!("  3. 运行命令:");
        warn!(
            "     chmod +x \"{}\"",
            script_path.file_name().unwrap().to_string_lossy()
        );
        warn!("");
        warn!("方法2: 使用WSL");
        warn!("  1. 打开WSL终端");
        warn!("  2. 转换路径并设置权限:");
        warn!(
            "     chmod +x \"{}\"",
            self.convert_to_wsl_path(script_path)
                .unwrap_or_else(|_| script_path.display().to_string())
        );
        warn!("");
        warn!("方法3: 使用PowerShell");
        warn!("  1. 打开PowerShell");
        warn!("  2. 运行命令:");
        warn!("     bash -c \"chmod +x '{}'\"", script_path.display());
        warn!("");
        warn!("方法4: 在docker-compose.yml中添加权限设置");
        warn!("  在相关服务的volumes中添加:");
        warn!("    volumes:");
        warn!("      - ./config:/app/config:ro");
        warn!("      - ./script:/app/script:ro");
        warn!("  并在entrypoint中添加:");
        warn!("    command: sh -c \"chmod +x /app/script/*.sh && your-original-command\"");
        warn!("");
        warn!("方法5: 检查文件编码和行尾符");
        warn!("  1. 确保文件使用UTF-8编码（无BOM）");
        warn!("  2. 确保行尾符是LF而不是CRLF");
        warn!("  3. 在文件开头添加: #!/bin/bash");
        warn!("  4. 使用文本编辑器（如VS Code）设置行尾符为LF");
        warn!("");
        warn!("💡 提示: 如果脚本在Docker容器内执行失败，可以:");
        warn!("  1. 在docker-compose.yml中添加环境变量: CHMOD_SCRIPTS=true");
        warn!("  2. 或者在容器启动时手动执行: chmod +x /path/to/script.sh");
        warn!("  3. 使用Dockerfile中的COPY命令时添加权限: COPY --chmod=+x script.sh /app/");

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
                    debug!("Git Bash chmod 成功: {}", bash_path);
                    return Ok(true);
                }
            }
        }

        debug!("Git Bash chmod 不可用");
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
                    debug!("WSL chmod 成功");
                    return Ok(true);
                } else {
                    let stderr = String::from_utf8_lossy(&output.stderr);
                    debug!("WSL chmod 失败: {}", stderr);
                }
            }
            Err(e) => {
                debug!("WSL chmod 不可用，WSL可能未安装: {}", e);
            }
        }

        debug!("WSL chmod 不可用");
        Ok(false)
    }

    /// 尝试直接chmod
    async fn try_direct_chmod(&self, script_path: &Path) -> DockerServiceResult<bool> {
        if let Ok(output) = Command::new("chmod").arg("+x").arg(script_path).output() {
            if output.status.success() {
                debug!("直接 chmod 成功");
                return Ok(true);
            }
        }

        debug!("直接 chmod 不可用");
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
                "脚本文件不存在: {}",
                script_path.display()
            )));
        }

        info!("🛠️  修复特定脚本权限: {}", script_name);
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
                        issues.push(format!("脚本缺少执行权限: {}", entrypoint_script.display()));
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
                            issues.push(format!("脚本缺少执行权限: {}", script_path.display()));
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
                "读取脚本文件失败 {}: {}",
                script_path.display(),
                e
            ))
        })?;

        // 检查是否包含Windows行尾符
        if !content.contains("\r\n") {
            debug!("脚本已是Unix行尾符格式: {}", script_path.display());
            return Ok(false);
        }

        info!("发现Windows行尾符，正在转换: {}", script_path.display());

        // 转换行尾符: CRLF -> LF
        let unix_content = content.replace("\r\n", "\n");

        // 创建备份文件
        let backup_path = script_path.with_extension("sh.bak");
        std::fs::copy(script_path, &backup_path).map_err(|e| {
            DockerServiceError::FileSystem(format!(
                "创建备份文件失败 {}: {}",
                backup_path.display(),
                e
            ))
        })?;

        debug!("已创建备份文件: {}", backup_path.display());

        // 写入转换后的内容
        std::fs::write(script_path, unix_content).map_err(|e| {
            DockerServiceError::FileSystem(format!(
                "写入转换后的脚本失败 {}: {}",
                script_path.display(),
                e
            ))
        })?;

        info!("✅ 行尾符转换完成: {}", script_path.display());
        info!("💾 备份文件: {}", backup_path.display());

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
                    warn!("脚本包含BOM标记: {}", script_path.display());
                    warn!("建议: 使用文本编辑器去除BOM标记");
                    return Ok(false);
                }

                // 检查是否包含Windows行尾符
                if content.contains("\r\n") {
                    warn!("脚本使用Windows行尾符: {}", script_path.display());
                    return Ok(false);
                }

                debug!("脚本编码检查通过: {}", script_path.display());
                Ok(true)
            }
            Err(e) => {
                warn!("脚本编码检查失败 {}: {}", script_path.display(), e);
                warn!("可能不是有效的UTF-8编码");
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

        info!("🪟 执行Windows兼容性检查...");

        // 检查Docker是否运行
        if Command::new("docker").arg("version").output().is_err() {
            suggestions.push("Docker Desktop可能未运行，请启动Docker Desktop".to_string());
        }

        // 检查是否有WSL2（如果WSL已安装）
        match Command::new("wsl").arg("--list").arg("--verbose").output() {
            Ok(output) => {
                if output.status.success() {
                    let wsl_output = String::from_utf8_lossy(&output.stdout);
                    if wsl_output.contains("Version 2") {
                        suggestions.push(
                            "建议在WSL2环境中运行Docker相关操作以获得更好的兼容性".to_string(),
                        );
                    }
                } else {
                    debug!("WSL检查失败，可能WSL未安装或配置不正确");
                }
            }
            Err(e) => {
                debug!("WSL未安装或不可用: {}", e);
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
                            "Git配置 core.autocrlf=true 可能导致脚本行尾符问题，建议设置为false"
                                .to_string(),
                        );
                    }
                } else {
                    debug!("Git配置检查失败，可能Git未安装或配置不存在");
                }
            }
            Err(e) => {
                debug!("Git未安装或不可用: {}", e);
                // 不添加建议，因为Git不是必需的
            }
        }

        // 动态检查所有脚本文件（可选诊断）
        match self.find_docker_scripts() {
            Ok(scripts) => {
                if scripts.is_empty() {
                    debug!("未发现脚本文件，跳过编码检查");
                } else {
                    debug!("开始检查 {} 个脚本文件的编码问题", scripts.len());
                    let mut encoding_issues = 0;

                    for script_path in scripts {
                        // 检查文件编码和行尾符
                        if let Ok(content) = std::fs::read_to_string(&script_path) {
                            let mut has_issues = false;

                            if content.contains("\r\n") {
                                suggestions.push(format!(
                                    "脚本 {} 使用Windows行尾符(CRLF)，建议转换为Unix行尾符(LF)",
                                    script_path
                                        .file_name()
                                        .unwrap_or_else(|| std::ffi::OsStr::new("unknown"))
                                        .to_string_lossy()
                                ));
                                has_issues = true;
                            }

                            if content.starts_with('\u{FEFF}') {
                                suggestions.push(format!(
                                    "脚本 {} 包含BOM标记，建议去除BOM",
                                    script_path
                                        .file_name()
                                        .unwrap_or_else(|| std::ffi::OsStr::new("unknown"))
                                        .to_string_lossy()
                                ));
                                has_issues = true;
                            }

                            if has_issues {
                                encoding_issues += 1;
                            }
                        }
                    }

                    if encoding_issues > 0 {
                        debug!("发现 {} 个脚本存在编码问题", encoding_issues);
                    } else {
                        debug!("所有脚本编码检查通过");
                    }
                }
            }
            Err(e) => {
                debug!("脚本扫描失败（非关键错误）: {}", e);
                // 不添加建议，因为扫描失败不影响核心功能
            }
        }

        if suggestions.is_empty() {
            info!("✅ Windows兼容性检查通过");
        } else {
            warn!("⚠️  发现 {} 个Windows兼容性问题", suggestions.len());
        }

        Ok(suggestions)
    }

    /// 为Windows用户提供一键修复脚本权限的方法
    pub async fn fix_windows_script_permissions(&self) -> DockerServiceResult<()> {
        if !cfg!(target_os = "windows") {
            return Ok(());
        }

        info!("🪟 开始Windows脚本权限一键修复...");

        // 查找所有脚本文件
        let scripts = self.find_docker_scripts()?;

        if scripts.is_empty() {
            info!("📭 未找到需要修复的脚本文件");
            return Ok(());
        }

        info!("🔍 找到 {} 个脚本文件，开始修复权限...", scripts.len());

        let mut success_count = 0;
        let mut fail_count = 0;

        for script_path in &scripts {
            match self.check_and_fix_file_permission(script_path).await {
                Ok(true) => {
                    info!("✅ 成功修复脚本权限: {}", script_path.display());
                    success_count += 1;
                }
                Ok(false) => {
                    debug!("脚本权限已正确: {}", script_path.display());
                }
                Err(e) => {
                    warn!("❌ 修复脚本权限失败: {} - {}", script_path.display(), e);
                    fail_count += 1;
                }
            }
        }

        info!("📊 脚本权限修复完成:");
        info!("  ✅ 成功修复: {} 个", success_count);
        info!("  ❌ 修复失败: {} 个", fail_count);
        info!("  📝 总计处理: {} 个", scripts.len());

        if fail_count > 0 {
            warn!("💡 对于修复失败的脚本，请参考上面的手动操作指南");
        }

        Ok(())
    }
}
