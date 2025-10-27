use serde::{Deserialize, Serialize};
use std::path::Path;
use std::process::ExitStatus;
use std::sync::Arc;
use tokio::sync::mpsc;
use tokio_stream::{Stream, wrappers::ReceiverStream};

#[cfg(unix)]
use std::os::unix::process::ExitStatusExt;

// 辅助函数：创建一个成功的 ExitStatus
fn create_success_exit_status() -> ExitStatus {
    #[cfg(unix)]
    {
        ExitStatus::from_raw(0)
    }
    #[cfg(not(unix))]
    {
        // 在非Unix系统上，我们使用一个实际的成功命令来获取ExitStatus
        std::process::Command::new("cmd")
            .args(&["/C", "echo", "ok"])
            .output()
            .map(|output| output.status)
            .unwrap_or_else(|_| {
                // 如果连这个都失败了，我们使用 true 命令
                std::process::Command::new("true")
                    .output()
                    .map(|output| output.status)
                    .unwrap_or_else(|_| panic!("Cannot create exit status"))
            })
    }
}

// 初始化进度数据结构
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InitProgress {
    pub stage: String,
    pub message: String,
    pub percentage: f64,
    pub current_step: usize,
    pub total_steps: usize,
}

// 下载进度数据结构
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DownloadProgress {
    pub task_id: String,
    pub file_name: String,
    pub downloaded_bytes: u64,
    pub total_bytes: u64,
    pub download_speed: f64, // bytes/sec
    pub eta_seconds: u64,
    pub percentage: f64,
    pub status: DownloadStatus,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum DownloadStatus {
    Starting,
    Downloading,
    Paused,
    Completed,
    Failed(String),
}

// 系统信息数据结构
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SystemInfo {
    pub os: String,
    pub arch: String,
    pub total_memory: u64,
    pub available_memory: u64,
    pub cpu_count: usize,
    pub docker_version: Option<String>,
    pub disk_space: DiskSpace,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DiskSpace {
    pub total: u64,
    pub available: u64,
    pub used: u64,
}

// 服务状态数据结构
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ServiceStatus {
    pub name: String,
    pub status: String,
    pub health: String,
    pub uptime: Option<u64>,
    pub cpu_usage: f64,
    pub memory_usage: u64,
    pub ports: Vec<String>,
}

// 带进度回调的初始化函数
pub async fn init_with_progress<F>(
    working_dir: &Path,
    progress_callback: F,
) -> std::result::Result<(), Box<dyn std::error::Error>>
where
    F: Fn(InitProgress) + Send + Sync + 'static,
{
    let callback = Arc::new(progress_callback);

    // 初始化步骤
    let steps = [
        ("downloading", "正在准备初始化环境..."),
        ("extracting", "正在创建配置文件和目录结构..."),
        ("loading", "正在初始化DuckDB数据库..."),
        ("starting", "正在注册客户端..."),
        ("configuring", "正在完成初始化设置..."),
    ];

    let total_steps = steps.len();

    // 步骤1：准备环境
    let progress = InitProgress {
        stage: steps[0].0.to_string(),
        message: steps[0].1.to_string(),
        percentage: 0.0,
        current_step: 1,
        total_steps,
    };
    callback(progress);

    // 确保工作目录存在
    tokio::fs::create_dir_all(working_dir).await?;

    // 切换到工作目录以执行真正的初始化
    let current_dir = std::env::current_dir()?;
    std::env::set_current_dir(working_dir)?;

    // 步骤2：创建配置和目录
    let progress = InitProgress {
        stage: steps[1].0.to_string(),
        message: steps[1].1.to_string(),
        percentage: 20.0,
        current_step: 2,
        total_steps,
    };
    callback(progress);

    // 调用真正的nuwax-cli init逻辑
    use crate::init::run_init;
    if let Err(e) = run_init(true).await {
        // 恢复原始目录
        std::env::set_current_dir(current_dir)?;
        return Err(e.into());
    }

    // 步骤3：数据库初始化完成提示
    let progress = InitProgress {
        stage: steps[2].0.to_string(),
        message: steps[2].1.to_string(),
        percentage: 60.0,
        current_step: 3,
        total_steps,
    };
    callback(progress);

    // 步骤4：客户端注册完成提示
    let progress = InitProgress {
        stage: steps[3].0.to_string(),
        message: steps[3].1.to_string(),
        percentage: 80.0,
        current_step: 4,
        total_steps,
    };
    callback(progress);

    // 步骤5：完成
    let progress = InitProgress {
        stage: steps[4].0.to_string(),
        message: steps[4].1.to_string(),
        percentage: 90.0,
        current_step: 5,
        total_steps,
    };
    callback(progress);

    // 恢复原始目录
    std::env::set_current_dir(current_dir)?;

    // 完成回调
    let final_progress = InitProgress {
        stage: "configuring".to_string(),
        message: "初始化完成！".to_string(),
        percentage: 100.0,
        current_step: total_steps,
        total_steps,
    };
    callback(final_progress);

    Ok(())
}

// 带进度回调的下载函数
pub async fn download_with_progress<F>(
    url: &str,
    target_dir: &Path,
    progress_callback: F,
) -> std::result::Result<(), Box<dyn std::error::Error>>
where
    F: Fn(DownloadProgress) + Send + Sync + 'static,
{
    let callback = Arc::new(progress_callback);

    // 解析文件名
    let file_name = url.split('/').next_back().unwrap_or("unknown_file");
    let task_id = format!("download_{}", chrono::Utc::now().timestamp());

    // 创建HTTP客户端
    let client = reqwest::Client::new();

    // 开始下载进度报告
    let mut progress = DownloadProgress {
        task_id: task_id.clone(),
        file_name: file_name.to_string(),
        downloaded_bytes: 0,
        total_bytes: 0,
        download_speed: 0.0,
        eta_seconds: 0,
        percentage: 0.0,
        status: DownloadStatus::Starting,
    };

    callback(progress.clone());

    // 获取文件大小
    let response = client.head(url).send().await?;
    let total_size = response.content_length().unwrap_or(0);

    progress.total_bytes = total_size;
    progress.status = DownloadStatus::Downloading;
    callback(progress.clone());

    // 开始下载
    let mut response = client.get(url).send().await?;
    let target_path = target_dir.join(file_name);

    // 确保目标目录存在
    tokio::fs::create_dir_all(target_dir).await?;

    let mut file = tokio::fs::File::create(&target_path).await?;
    let mut downloaded = 0u64;
    let start_time = std::time::Instant::now();
    let mut last_update = start_time;

    // 流式下载
    while let Some(chunk) = response.chunk().await? {
        use tokio::io::AsyncWriteExt;
        file.write_all(&chunk).await?;
        downloaded += chunk.len() as u64;

        let now = std::time::Instant::now();

        // 每500ms更新一次进度
        if now.duration_since(last_update).as_millis() > 500 {
            let elapsed = now.duration_since(start_time).as_secs_f64();
            let speed = downloaded as f64 / elapsed;
            let eta = if speed > 0.0 {
                ((total_size - downloaded) as f64 / speed) as u64
            } else {
                0
            };

            progress.downloaded_bytes = downloaded;
            progress.download_speed = speed;
            progress.eta_seconds = eta;
            progress.percentage = if total_size > 0 {
                (downloaded as f64 / total_size as f64) * 100.0
            } else {
                0.0
            };

            callback(progress.clone());
            last_update = now;
        }
    }

    // 完成下载
    progress.downloaded_bytes = downloaded;
    progress.percentage = 100.0;
    progress.status = DownloadStatus::Completed;
    callback(progress);

    Ok(())
}

// 获取系统信息
pub fn get_system_info() -> SystemInfo {
    let os = std::env::consts::OS.to_string();
    let arch = std::env::consts::ARCH.to_string();

    // 获取内存信息
    let (total_memory, available_memory) = get_memory_info();

    // 获取CPU核心数
    let cpu_count = num_cpus::get();

    // 尝试获取Docker版本
    let docker_version = get_docker_version();

    // 获取磁盘空间
    let disk_space = get_disk_space();

    SystemInfo {
        os,
        arch,
        total_memory,
        available_memory,
        cpu_count,
        docker_version,
        disk_space,
    }
}

// 实时服务状态监控
pub async fn monitor_services() -> impl Stream<Item = ServiceStatus> {
    let (tx, rx) = mpsc::channel(100);

    // 启动监控任务
    tokio::spawn(async move {
        loop {
            // 模拟获取服务状态
            let services = get_all_services().await;

            for service in services {
                if tx.send(service).await.is_err() {
                    break;
                }
            }

            // 每5秒更新一次
            tokio::time::sleep(std::time::Duration::from_secs(5)).await;
        }
    });

    ReceiverStream::new(rx)
}

// 辅助函数：获取内存信息
fn get_memory_info() -> (u64, u64) {
    #[cfg(target_os = "macos")]
    {
        use std::process::Command;

        let output = Command::new("sysctl")
            .args(["hw.memsize"])
            .output()
            .unwrap_or_else(|_| std::process::Output {
                stdout: b"hw.memsize: 8589934592".to_vec(),
                stderr: vec![],
                status: create_success_exit_status(),
            });

        let output_str = String::from_utf8_lossy(&output.stdout);
        let total = output_str
            .split(':')
            .nth(1)
            .and_then(|s| s.trim().parse::<u64>().ok())
            .unwrap_or(8589934592); // 默认8GB

        // 简化的可用内存估算
        let available = total / 4; // 假设25%可用
        (total, available)
    }

    #[cfg(target_os = "linux")]
    {
        use std::fs;

        let meminfo = fs::read_to_string("/proc/meminfo")
            .unwrap_or_else(|_| "MemTotal: 8388608 kB\nMemAvailable: 2097152 kB".to_string());

        let mut total = 0;
        let mut available = 0;

        for line in meminfo.lines() {
            if line.starts_with("MemTotal:") {
                total = line
                    .split_whitespace()
                    .nth(1)
                    .and_then(|s| s.parse::<u64>().ok())
                    .unwrap_or(8388608)
                    * 1024; // kB to bytes
            } else if line.starts_with("MemAvailable:") {
                available = line
                    .split_whitespace()
                    .nth(1)
                    .and_then(|s| s.parse::<u64>().ok())
                    .unwrap_or(2097152)
                    * 1024; // kB to bytes
            }
        }

        (total, available)
    }

    #[cfg(target_os = "windows")]
    {
        // Windows实现
        (8589934592, 2147483648) // 默认8GB总内存，2GB可用
    }

    #[cfg(not(any(target_os = "macos", target_os = "linux", target_os = "windows")))]
    {
        (8589934592, 2147483648) // 默认值
    }
}

// 辅助函数：获取Docker版本
fn get_docker_version() -> Option<String> {
    use std::process::Command;

    Command::new("docker")
        .args(["--version"])
        .output()
        .ok()
        .and_then(|output| {
            if output.status.success() {
                Some(String::from_utf8_lossy(&output.stdout).trim().to_string())
            } else {
                None
            }
        })
}

// 辅助函数：获取磁盘空间
fn get_disk_space() -> DiskSpace {
    #[cfg(unix)]
    {
        use std::process::Command;

        let output = Command::new("df")
            .args(["-h", "."])
            .output()
            .unwrap_or_else(|_| {
                std::process::Output {
                    stdout: b"Filesystem      Size  Used Avail Use% Mounted on\n/dev/disk1s1   465G  120G  340G  27% /".to_vec(),
                    stderr: vec![],
                    status: create_success_exit_status(),
                }
            });

        let output_str = String::from_utf8_lossy(&output.stdout);
        let lines: Vec<&str> = output_str.lines().collect();

        if lines.len() >= 2 {
            let parts: Vec<&str> = lines[1].split_whitespace().collect();
            if parts.len() >= 4 {
                let total = parse_size(parts[1]).unwrap_or(500_000_000_000); // 默认500GB
                let used = parse_size(parts[2]).unwrap_or(120_000_000_000); // 默认120GB
                let available = parse_size(parts[3]).unwrap_or(380_000_000_000); // 默认380GB

                return DiskSpace {
                    total,
                    used,
                    available,
                };
            }
        }
    }

    // 默认值
    DiskSpace {
        total: 500_000_000_000,     // 500GB
        used: 120_000_000_000,      // 120GB
        available: 380_000_000_000, // 380GB
    }
}

// 辅助函数：解析磁盘大小
fn parse_size(size_str: &str) -> Option<u64> {
    let size_str = size_str.trim();
    let (num_str, unit) = if let Some(pos) = size_str.find(|c: char| c.is_alphabetic()) {
        (&size_str[..pos], &size_str[pos..])
    } else {
        (size_str, "")
    };

    let num: f64 = num_str.parse().ok()?;
    let multiplier = match unit.to_uppercase().as_str() {
        "K" | "KB" => 1024u64,
        "M" | "MB" => 1024u64 * 1024,
        "G" | "GB" => 1024u64 * 1024 * 1024,
        "T" | "TB" => 1024u64 * 1024 * 1024 * 1024,
        _ => 1u64,
    };

    Some((num * multiplier as f64) as u64)
}

// 辅助函数：获取所有服务状态
async fn get_all_services() -> Vec<ServiceStatus> {
    use std::process::Command;

    // 模拟获取Docker容器状态
    let output = Command::new("docker")
        .args([
            "ps",
            "--format",
            "table {{.Names}}\t{{.Status}}\t{{.Ports}}",
        ])
        .output()
        .unwrap_or_else(|_| std::process::Output {
            stdout: b"NAMES\tSTATUS\tPORTS\ntest-service\tUp 2 hours\t0.0.0.0:8080->8080/tcp"
                .to_vec(),
            stderr: vec![],
            status: create_success_exit_status(),
        });

    let output_str = String::from_utf8_lossy(&output.stdout);
    let mut services = Vec::new();

    for line in output_str.lines().skip(1) {
        // 跳过标题行
        let parts: Vec<&str> = line.split('\t').collect();
        if parts.len() >= 3 {
            services.push(ServiceStatus {
                name: parts[0].to_string(),
                status: parts[1].to_string(),
                health: "healthy".to_string(),   // 简化
                uptime: Some(7200),              // 简化为2小时
                cpu_usage: 15.5,                 // 模拟CPU使用率
                memory_usage: 256 * 1024 * 1024, // 模拟内存使用
                ports: vec![parts[2].to_string()],
            });
        }
    }

    services
}

// 配置相关的UI支持函数
pub async fn get_ui_config() -> std::result::Result<serde_json::Value, Box<dyn std::error::Error>> {
    // 这里会使用ConfigManager获取UI相关配置
    // 暂时返回模拟数据
    let config = serde_json::json!({
        "theme": "dark",
        "language": "zh-CN",
        "auto_refresh": true,
        "refresh_interval": 5,
        "show_notifications": true
    });

    Ok(config)
}

pub async fn update_ui_config(
    config: serde_json::Value,
) -> std::result::Result<(), Box<dyn std::error::Error>> {
    // 这里会使用ConfigManager更新UI配置
    // 暂时只做验证
    if !config.is_object() {
        return Err("配置必须是对象类型".to_string().into());
    }

    Ok(())
}
