use anyhow::Result;
use color_eyre::eyre::Context;
use std::path::PathBuf;
use tracing::{error, info, warn};

/// ducker 命令行参数结构
#[derive(Debug, Default)]
pub struct DuckerArgs {
    pub export_default_config: bool,
    pub docker_path: Option<String>,
    pub docker_host: Option<String>,
    pub log_path: Option<PathBuf>,
}

/// 集成ducker命令 - 提供Docker TUI界面（直接集成，不需要外部安装）
pub async fn run_ducker(args: Vec<String>) -> Result<()> {
    info!("启动ducker Docker TUI工具...");

    // 解析ducker参数
    let ducker_args = parse_ducker_args(args)?;

    // 运行ducker的核心逻辑
    run_ducker_tui(ducker_args).await.map_err(|e| {
        error!("ducker执行失败: {}", e);
        anyhow::anyhow!(format!("ducker执行失败: {e}"))
    })
}

/// 解析ducker命令行参数
fn parse_ducker_args(args: Vec<String>) -> Result<DuckerArgs> {
    let mut ducker_args = DuckerArgs::default();

    // 简单的参数解析 (处理常用的ducker参数)
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "-e" | "--export-default-config" => {
                ducker_args.export_default_config = true;
            }
            "-d" | "--docker-path" => {
                if i + 1 < args.len() {
                    ducker_args.docker_path = Some(args[i + 1].clone());
                    i += 1;
                }
            }
            "--docker-host" => {
                if i + 1 < args.len() {
                    ducker_args.docker_host = Some(args[i + 1].clone());
                    i += 1;
                }
            }
            "-l" | "--log-path" => {
                if i + 1 < args.len() {
                    ducker_args.log_path = Some(PathBuf::from(&args[i + 1]));
                    i += 1;
                }
            }
            "-h" | "--help" => {
                // 显示帮助信息
                show_ducker_help();
                return Err(anyhow::anyhow!("显示帮助信息"));
            }
            _ => {
                // 忽略未知参数
                warn!("未知的ducker参数: {}", args[i]);
            }
        }
        i += 1;
    }

    Ok(ducker_args)
}

/// 运行ducker的TUI界面（集成版本）
async fn run_ducker_tui(args: DuckerArgs) -> color_eyre::Result<()> {
    use ducker::{
        config::Config,
        docker::util::new_local_docker_connection,
        events::{self, EventLoop, Key, Message},
        state, terminal,
        ui::App,
    };

    // 跳过ducker的日志初始化，因为我们已经在nuwax-cli中初始化了
    info!("使用nuwax-cli的日志系统，跳过ducker的日志初始化");

    // 安装color_eyre (跳过如果已经安装)
    if color_eyre::install().is_err() {
        warn!("color_eyre已经安装，跳过");
    }

    // 创建ducker配置
    let config = Config::new(
        &args.export_default_config,
        args.docker_path,
        args.docker_host,
    )?;

    // 创建Docker连接
    let docker = new_local_docker_connection(&config.docker_path, config.docker_host.as_deref())
        .await
        .context("failed to create docker connection, potentially due to misconfiguration")?;

    // 初始化终端
    terminal::init_panic_hook();
    let mut terminal = ratatui::init();
    terminal.clear()?;

    // 创建事件循环和应用
    let mut events = EventLoop::new();
    let events_tx = events.get_tx();
    let mut app = App::new(events_tx, docker, config)
        .await
        .context("failed to create app")?;

    events.start().context("failed to start event loop")?;

    // 主事件循环
    while app.running != state::Running::Done {
        terminal
            .draw(|f| {
                app.draw(f);
            })
            .context("failed to update view")?;

        match events
            .next()
            .await
            .context("unable to receive next event")?
        {
            Message::Input(k) => {
                let res = app.update(k).await;
                if !res.is_consumed() {
                    // 处理系统退出事件
                    if k == Key::Ctrl('c') || k == Key::Ctrl('d') {
                        break;
                    }
                }
            }
            Message::Transition(t) => {
                if t == events::Transition::ToNewTerminal {
                    terminal = ratatui::init();
                    terminal.clear()?;
                } else {
                    let _ = &app.transition(t).await;
                }
            }
            Message::Tick => {
                app.update(Key::Null).await;
            }
            Message::Error(_) => {
                // 错误处理 - 目前忽略
            }
        }
    }

    // 恢复终端
    ratatui::restore();

    Ok(())
}

/// 显示ducker集成帮助
fn show_ducker_help() {
    println!(
        r#"
🦆 Ducker 集成版本 - Docker TUI 工具

用法: nuwax-cli ducker [选项]

选项:
  -e, --export-default-config  导出默认配置到配置目录
  -d, --docker-path <PATH>     Docker socket路径
      --docker-host <URL>      Docker主机URL (例: tcp://1.2.3.4:2375)
  -l, --log-path <PATH>        日志文件路径
  -h, --help                   显示此帮助信息

ducker 主要功能:
  • 容器管理 (启动/停止/删除/进入容器)
  • 镜像管理和清理
  • 卷和网络管理
  • 实时日志查看
  • 类似k9s的优秀TUI体验

键盘快捷键:
  j/↓        向下导航
  k/↑        向上导航
  Enter      选择/查看详情
  d          删除选中项
  l          查看日志
  q/Esc     退出
  :          命令模式

注意: 此版本已集成到nuwax-cli中，无需单独安装ducker。
"#
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_ducker_args() {
        let args = vec![
            "--docker-host".to_string(),
            "tcp://localhost:2375".to_string(),
        ];
        let parsed = parse_ducker_args(args).unwrap();
        assert_eq!(parsed.docker_host, Some("tcp://localhost:2375".to_string()));
    }
}
