use serde::{Deserialize, Serialize};
use std::process::{Command as StdCommand, Stdio};
use tauri::{AppHandle, Emitter, command};
use tauri_plugin_shell::{ShellExt, process::CommandEvent};

/// 调试环境变量和命令可用性
#[tauri::command]
pub async fn debug_environment() -> Result<String, String> {
    use std::env;

    let mut debug_info = String::new();

    // 检查PATH环境变量
    if let Ok(path) = env::var("PATH") {
        debug_info.push_str(&format!("PATH: {path}\n\n"));
    } else {
        debug_info.push_str("PATH: 未找到\n\n");
    }

    // 检查docker命令
    let docker_check = std::process::Command::new("docker")
        .args(["--version"])
        .output();

    match docker_check {
        Ok(output) if output.status.success() => {
            let version = String::from_utf8_lossy(&output.stdout);
            debug_info.push_str(&format!("Docker: ✅ {version}"));
        }
        Ok(output) => {
            let error = String::from_utf8_lossy(&output.stderr);
            debug_info.push_str(&format!("Docker: ❌ 错误: {error}"));
        }
        Err(e) => {
            debug_info.push_str(&format!("Docker: ❌ 未找到: {e}"));
        }
    }

    // 检查docker-compose命令
    let compose_check = std::process::Command::new("docker-compose")
        .args(["--version"])
        .output();

    match compose_check {
        Ok(output) if output.status.success() => {
            let version = String::from_utf8_lossy(&output.stdout);
            debug_info.push_str(&format!("docker-compose: ✅ {version}"));
        }
        Ok(output) => {
            let error = String::from_utf8_lossy(&output.stderr);
            debug_info.push_str(&format!("docker-compose: ❌ 错误: {error}"));
        }
        Err(e) => {
            debug_info.push_str(&format!("docker-compose: ❌ 未找到: {e}"));
        }
    }

    Ok(debug_info)
}

/// Windows 系统环境变量获取（从注册表和 PowerShell）
#[cfg(target_os = "windows")]
fn get_windows_environment() -> std::collections::HashMap<String, String> {
    let mut env = std::collections::HashMap::new();

    // 尝试通过 PowerShell 获取完整的用户环境变量
    if let Ok(output) = std::process::Command::new("powershell")
        .args([
            "-Command",
            "[Environment]::GetEnvironmentVariables('User') + [Environment]::GetEnvironmentVariables('Machine') | ConvertTo-Json"
        ])
        .output()
    {
        if output.status.success() {
            let stdout = String::from_utf8_lossy(&output.stdout);
            if let Ok(json_env) = serde_json::from_str::<std::collections::HashMap<String, String>>(&stdout) {
                for (key, value) in json_env {
                    env.insert(key, value);
                }
            }
        }
    }

    // 如果 PowerShell 方法失败，回退到基本的环境变量获取
    if env.is_empty() {
        // 尝试通过 reg query 获取用户环境变量
        if let Ok(output) = std::process::Command::new("reg")
            .args(["query", "HKCU\\Environment", "/v", "PATH"])
            .output()
        {
            if output.status.success() {
                let stdout = String::from_utf8_lossy(&output.stdout);
                if let Some(path_line) = stdout.lines().find(|line| line.contains("PATH")) {
                    if let Some(path_value) = path_line.split_whitespace().nth(2) {
                        env.insert("PATH".to_string(), path_value.to_string());
                    }
                }
            }
        }

        // 获取系统环境变量
        if let Ok(output) = std::process::Command::new("reg")
            .args([
                "query",
                "HKLM\\SYSTEM\\CurrentControlSet\\Control\\Session Manager\\Environment",
                "/v",
                "PATH",
            ])
            .output()
        {
            if output.status.success() {
                let stdout = String::from_utf8_lossy(&output.stdout);
                if let Some(path_line) = stdout.lines().find(|line| line.contains("PATH")) {
                    if let Some(system_path) = path_line.split_whitespace().nth(2) {
                        // 合并用户和系统 PATH
                        let user_path = env.get("PATH").cloned().unwrap_or_default();
                        let combined_path = if user_path.is_empty() {
                            system_path.to_string()
                        } else {
                            format!("{};{}", user_path, system_path)
                        };
                        env.insert("PATH".to_string(), combined_path);
                    }
                }
            }
        }
    }

    env
}

#[derive(Debug, Serialize, Deserialize)]
pub struct CommandResult {
    pub success: bool,
    pub exit_code: i32,
    pub stdout: String,
    pub stderr: String,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct CliVersion {
    pub version: String,
    pub available: bool,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct ProcessInfo {
    pub pid: u32,
    pub command: String,
    pub running: bool,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct ProcessCheckResult {
    pub processes_found: Vec<ProcessInfo>,
    pub processes_killed: Vec<u32>,
    pub success: bool,
    pub message: String,
}

/// 获取用户的完整环境变量（包括shell配置）
fn get_user_environment() -> std::collections::HashMap<String, String> {
    let mut env = std::env::vars().collect::<std::collections::HashMap<String, String>>();

    // 在 macOS 和 Linux 上，尝试加载用户的 shell 环境
    #[cfg(any(target_os = "macos", target_os = "linux"))]
    {
        // 获取用户的家目录
        if let Some(_home) = env.get("HOME") {
            // 常见的 Docker 安装路径
            let docker_paths = vec![
                "/usr/local/bin",
                "/usr/bin",
                "/opt/homebrew/bin", // macOS Homebrew (Apple Silicon)
                "/usr/local/bin",    // macOS Homebrew (Intel)
                "/Applications/Docker.app/Contents/Resources/bin", // Docker Desktop for Mac
                "/snap/bin",         // Ubuntu Snap
            ];

            // 获取当前的 PATH
            let current_path = env.get("PATH").cloned().unwrap_or_default();

            // 添加 Docker 可能的安装路径到 PATH
            let mut path_components: Vec<String> =
                docker_paths.iter().map(|p| p.to_string()).collect();

            // 将原有的 PATH 组件添加到列表中
            if !current_path.is_empty() {
                path_components.extend(current_path.split(':').map(|s| s.to_string()));
            }

            // 去重并重新组合 PATH
            path_components.sort();
            path_components.dedup();
            let enhanced_path = path_components.join(":");

            env.insert("PATH".to_string(), enhanced_path);

            // 设置其他可能需要的环境变量
            env.insert("LANG".to_string(), "en_US.UTF-8".to_string());
            env.insert("LC_ALL".to_string(), "en_US.UTF-8".to_string());
        }
    }

    // 在 Windows 上添加常见的 Docker 和 Podman 路径
    #[cfg(target_os = "windows")]
    {
        // 获取系统和用户环境变量
        let enhanced_env = get_windows_environment();
        for (key, value) in enhanced_env {
            env.insert(key, value);
        }

        // 额外添加容器工具路径
        let user_profile = std::env::var("USERPROFILE").unwrap_or_default();
        let podman_user_path = format!(
            "{}\\AppData\\Local\\Programs\\Podman\\resources\\bin",
            user_profile
        );
        let windows_apps_path = format!("{}\\AppData\\Local\\Microsoft\\WindowsApps", user_profile);

        let docker_paths = vec![
            "C:\\Program Files\\Docker\\Docker\\resources\\bin",
            "C:\\ProgramData\\DockerDesktop\\version-bin",
            // Podman Desktop 路径
            "C:\\Program Files\\RedHat\\Podman\\resources\\bin",
            "C:\\Program Files\\Podman\\resources\\bin",
            // 用户目录下的 Podman 安装
            &podman_user_path,
            // Windows Store 应用路径（包含 docker-compose）
            &windows_apps_path,
            // 系统路径
            "C:\\Windows\\system32",
            "C:\\Windows",
            "C:\\Windows\\System32\\Wbem",
            "C:\\Windows\\System32\\WindowsPowerShell\\v1.0\\",
        ];

        let current_path = env.get("PATH").cloned().unwrap_or_default();
        let mut path_components: Vec<String> = docker_paths.iter().map(|p| p.to_string()).collect();

        if !current_path.is_empty() {
            path_components.extend(current_path.split(';').map(|s| s.to_string()));
        }

        // 去重并重新组合 PATH
        path_components.sort();
        path_components.dedup();
        let enhanced_path = path_components.join(";");

        env.insert("PATH".to_string(), enhanced_path);
    }

    env
}

/// 执行nuwax-cli命令（Sidecar方式）
#[command]
pub async fn execute_duck_cli_sidecar(
    app: AppHandle,
    args: Vec<String>,
    working_dir: Option<String>,
) -> Result<CommandResult, String> {
    let shell = app.shell();

    let mut cmd = shell
        .sidecar("nuwax-cli")
        .map_err(|e| format!("创建sidecar命令失败: {e}"))?;

    if !args.is_empty() {
        cmd = cmd.args(&args);
    }

    if let Some(dir) = working_dir {
        cmd = cmd.current_dir(dir);
    }

    // 设置增强的环境变量
    let enhanced_env = get_user_environment();
    for (key, value) in enhanced_env {
        cmd = cmd.env(key, value);
    }

    let (mut rx, mut _child) = cmd.spawn().map_err(|e| format!("执行命令失败: {e}"))?;

    let mut stdout = String::new();
    let mut stderr = String::new();
    let mut exit_code = 0;

    while let Some(event) = rx.recv().await {
        match event {
            CommandEvent::Stdout(data) => {
                let output = String::from_utf8_lossy(&data);
                stdout.push_str(&output);
                // 实时发送输出到前端
                let _ = app.emit("cli-output", &output);
            }
            CommandEvent::Stderr(data) => {
                let output = String::from_utf8_lossy(&data);
                stderr.push_str(&output);
                // 实时发送错误到前端
                let _ = app.emit("cli-error", &output);
            }
            CommandEvent::Terminated(payload) => {
                exit_code = payload.code.unwrap_or(-1);
                let _ = app.emit("cli-complete", exit_code);
                break;
            }
            _ => {}
        }
    }

    Ok(CommandResult {
        success: exit_code == 0,
        exit_code,
        stdout,
        stderr,
    })
}

/// 执行系统nuwax-cli命令（Shell方式）
#[command]
pub async fn execute_duck_cli_system(
    app: AppHandle,
    args: Vec<String>,
    working_dir: Option<String>,
) -> Result<CommandResult, String> {
    let shell = app.shell();

    let mut cmd = shell.command("nuwax-cli");

    if !args.is_empty() {
        cmd = cmd.args(&args);
    }

    if let Some(dir) = working_dir {
        cmd = cmd.current_dir(dir);
    }

    // 设置增强的环境变量
    let enhanced_env = get_user_environment();
    for (key, value) in enhanced_env {
        cmd = cmd.env(key, value);
    }

    let output = cmd
        .output()
        .await
        .map_err(|e| format!("执行系统命令失败: {e}"))?;

    let stdout = String::from_utf8_lossy(&output.stdout).to_string();
    let stderr = String::from_utf8_lossy(&output.stderr).to_string();
    let exit_code = output.status.code().unwrap_or(-1);

    // 发送输出到前端
    if !stdout.is_empty() {
        let _ = app.emit("cli-output", &stdout);
    }
    if !stderr.is_empty() {
        let _ = app.emit("cli-error", &stderr);
    }
    let _ = app.emit("cli-complete", exit_code);

    Ok(CommandResult {
        success: exit_code == 0,
        exit_code,
        stdout,
        stderr,
    })
}

/// 智能执行nuwax-cli命令（混合策略）
#[command]
pub async fn execute_duck_cli_smart(
    app: AppHandle,
    args: Vec<String>,
    working_dir: Option<String>,
) -> Result<CommandResult, String> {
    // 优先使用Sidecar方式
    match execute_duck_cli_sidecar(app.clone(), args.clone(), working_dir.clone()).await {
        Ok(result) => {
            // Sidecar成功，直接返回结果（已发送事件）
            Ok(result)
        }
        Err(sidecar_error) => {
            // 发送降级通知
            let _ = app.emit("cli-output", "⚠️ Sidecar方式失败，使用系统命令...");

            // 降级到系统命令
            match execute_duck_cli_system(app.clone(), args, working_dir).await {
                Ok(result) => {
                    // System成功，返回结果（已发送事件）
                    Ok(result)
                }
                Err(system_error) => {
                    // 发送失败通知
                    let _ = app.emit("cli-error", "❌ 所有CLI执行方式都失败");
                    let _ = app.emit("cli-complete", -1);

                    Err(format!(
                        "所有CLI执行方式都失败 - Sidecar: {sidecar_error} | System: {system_error}"
                    ))
                }
            }
        }
    }
}

/// 检查CLI工具版本
#[command]
pub async fn get_cli_version(app: AppHandle) -> Result<CliVersion, String> {
    match execute_duck_cli_smart(app, vec!["--version".to_string()], None).await {
        Ok(result) => {
            if result.success {
                // 从输出中提取版本号
                let version = result
                    .stdout
                    .lines()
                    .find(|line| line.contains("nuwax-cli"))
                    .and_then(|line| line.split_whitespace().last())
                    .unwrap_or("unknown")
                    .to_string();

                Ok(CliVersion {
                    version,
                    available: true,
                })
            } else {
                Ok(CliVersion {
                    version: "error".to_string(),
                    available: false,
                })
            }
        }
        Err(error) => {
            println!("获取CLI版本失败: {error}");
            Ok(CliVersion {
                version: "unavailable".to_string(),
                available: false,
            })
        }
    }
}

/// 检查CLI工具是否可用
#[command]
pub async fn check_cli_available(app: AppHandle) -> Result<bool, String> {
    let version_info = get_cli_version(app).await?;
    Ok(version_info.available)
}

/// 检查并清理运行中的nuwax-cli进程
#[command]
pub async fn check_and_cleanup_duck_processes() -> Result<ProcessCheckResult, String> {
    let mut processes_found = Vec::new();
    let mut processes_killed = Vec::new();

    // 检查运行中的nuwax-cli进程
    #[cfg(target_os = "macos")]
    let output = StdCommand::new("ps")
        .args(["aux"])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output();

    #[cfg(target_os = "linux")]
    let output = StdCommand::new("ps")
        .args(&["aux"])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output();

    #[cfg(target_os = "windows")]
    let output = StdCommand::new("tasklist")
        .args(&["/FI", "IMAGENAME eq nuwax-cli.exe", "/FO", "CSV"])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output();

    match output {
        Ok(output) => {
            let stdout = String::from_utf8_lossy(&output.stdout);

            // 解析进程信息
            for line in stdout.lines() {
                if line.contains("nuwax-cli") && !line.contains("grep") {
                    // 提取PID和命令信息
                    let parts: Vec<&str> = line.split_whitespace().collect();

                    #[cfg(any(target_os = "macos", target_os = "linux"))]
                    let (pid_str, command) = if parts.len() >= 11 {
                        (parts[1], parts[10..].join(" "))
                    } else {
                        continue;
                    };

                    #[cfg(target_os = "windows")]
                    let (pid_str, command) = if parts.len() >= 2 {
                        // Windows tasklist CSV格式: "ImageName","PID","SessionName",...
                        let csv_parts: Vec<&str> = line.split(',').collect();
                        if csv_parts.len() >= 2 {
                            let pid = csv_parts[1].trim_matches('"');
                            let cmd = csv_parts[0].trim_matches('"');
                            (pid, cmd.to_string())
                        } else {
                            continue;
                        }
                    } else {
                        continue;
                    };

                    if let Ok(pid) = pid_str.parse::<u32>() {
                        processes_found.push(ProcessInfo {
                            pid,
                            command: command.to_string(),
                            running: true,
                        });

                        // 尝试终止进程
                        #[cfg(any(target_os = "macos", target_os = "linux"))]
                        let kill_result = StdCommand::new("kill")
                            .args(["-TERM", &pid.to_string()])
                            .output();

                        #[cfg(target_os = "windows")]
                        let kill_result = StdCommand::new("taskkill")
                            .args(&["/PID", &pid.to_string(), "/F"])
                            .output();

                        match kill_result {
                            Ok(_) => {
                                processes_killed.push(pid);

                                // 等待一下，然后检查进程是否真的被终止
                                tokio::time::sleep(tokio::time::Duration::from_millis(500)).await;

                                // 验证进程是否被终止
                                #[cfg(any(target_os = "macos", target_os = "linux"))]
                                let check_result = StdCommand::new("kill")
                                    .args(["-0", &pid.to_string()])
                                    .output();

                                #[cfg(target_os = "windows")]
                                let check_result = StdCommand::new("tasklist")
                                    .args(&["/FI", &format!("PID eq {}", pid)])
                                    .output();

                                // 如果进程仍在运行，尝试强制终止
                                match check_result {
                                    Ok(output) if output.status.success() => {
                                        #[cfg(any(target_os = "macos", target_os = "linux"))]
                                        let _ = StdCommand::new("kill")
                                            .args(["-KILL", &pid.to_string()])
                                            .output();

                                        #[cfg(target_os = "windows")]
                                        let _ = StdCommand::new("taskkill")
                                            .args(&["/PID", &pid.to_string(), "/F", "/T"])
                                            .output();
                                    }
                                    _ => {
                                        // 进程已经被终止
                                    }
                                }
                            }
                            Err(e) => {
                                println!("终止进程 {pid} 失败: {e}");
                            }
                        }
                    }
                }
            }
        }
        Err(e) => {
            return Err(format!("检查进程失败: {e}"));
        }
    }

    let success = !processes_found.is_empty();
    let message = if processes_found.is_empty() {
        "没有发现运行中的 nuwax-cli 进程".to_string()
    } else {
        format!(
            "发现 {} 个 nuwax-cli 进程，已终止 {} 个",
            processes_found.len(),
            processes_killed.len()
        )
    };

    Ok(ProcessCheckResult {
        processes_found,
        processes_killed,
        success,
        message,
    })
}

/// 检查数据库文件是否被锁定
#[command]
pub async fn check_database_lock(_app: AppHandle, working_dir: String) -> Result<bool, String> {
    use std::fs::OpenOptions;
    use std::path::PathBuf;

    let db_path = PathBuf::from(&working_dir)
        .join("data")
        .join("duck_client.db");

    if !db_path.exists() {
        return Ok(false); // 文件不存在，没有锁定问题
    }

    // 尝试以独占模式打开文件来检测锁定
    match OpenOptions::new().read(true).write(true).open(&db_path) {
        Ok(_file) => Ok(false), // 能够打开，说明没有被锁定
        Err(e) => {
            let error_msg = e.to_string();
            if error_msg.contains("Resource busy")
                || error_msg.contains("locked")
                || error_msg.contains("being used")
            {
                Ok(true) // 文件被锁定
            } else {
                Err(format!("检查数据库锁定状态失败: {error_msg}"))
            }
        }
    }
}
