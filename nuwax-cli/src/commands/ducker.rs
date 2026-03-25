use anyhow::Result;
use color_eyre::eyre::Context;
use rust_i18n::t;
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

/// 集成ducker命令 - 提供Docker TUI界面（直接集成,不需要外部安装）
pub async fn run_ducker(args: Vec<String>) -> Result<()> {
    info!("{}", t!("ducker.starting"));

    // 解析ducker参数
    let ducker_args = parse_ducker_args(args)?;

    // 运行ducker的核心逻辑
    run_ducker_tui(ducker_args).await.map_err(|e| {
        error!("{}", t!("ducker.execution_failed", error = e.to_string()));
        anyhow::anyhow!(t!("ducker.execution_failed", error = e.to_string()).to_string())
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
                return Err(anyhow::anyhow!(t!("ducker.help_displayed").to_string()));
            }
            _ => {
                // 忽略未知参数
                warn!("{}", t!("ducker.unknown_arg", arg = args[i]));
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
    info!("{}", t!("ducker.using_nuwax_log"));

    // 安装color_eyre (跳过如果已经安装)
    if color_eyre::install().is_err() {
        warn!("{}", t!("ducker.color_eyre_installed"));
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
🦆 {} - Docker TUI {}

{}: nuwax-cli ducker [options]

{}:
  -e, --export-default-config  {}
  -d, --docker-path <PATH>     {}
      --docker-host <URL>      {}
  -l, --log-path <PATH>        {}
  -h, --help                   {}

{}:
  • {}
  • {}
  • {}
  • {}
  • {}

{}:
  j/↓        {}
  k/↑        {}
  Enter      {}
  d          {}
  l          {}
  q/Esc     {}
  :          {}

{}: {}
"#,
        t!("ducker.help.title"),
        t!("ducker.help.tool"),
        t!("ducker.help.usage"),
        t!("ducker.help.options"),
        t!("ducker.help.export_config"),
        t!("ducker.help.docker_path"),
        t!("ducker.help.docker_host"),
        t!("ducker.help.log_path"),
        t!("ducker.help.help_flag"),
        t!("ducker.help.main_features"),
        t!("ducker.help.feature_containers"),
        t!("ducker.help.feature_images"),
        t!("ducker.help.feature_volumes"),
        t!("ducker.help.feature_logs"),
        t!("ducker.help.feature_tui"),
        t!("ducker.help.shortcuts"),
        t!("ducker.help.key_down"),
        t!("ducker.help.key_up"),
        t!("ducker.help.key_enter"),
        t!("ducker.help.key_delete"),
        t!("ducker.help.key_logs"),
        t!("ducker.help.key_quit"),
        t!("ducker.help.key_command"),
        t!("ducker.help.note_title"),
        t!("ducker.help.note_content")
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
