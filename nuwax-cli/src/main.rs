use clap::Parser;
use client_core::DuckError;
use nuwax_cli::{Cli, CliApp, Commands, run_diff_sql, run_init, setup_logging};
use tracing::{error, info};

#[tokio::main]
async fn main() {
    // 解析命令行参数
    let cli = Cli::parse();

    // 设置日志记录
    setup_logging(cli.verbose);

    // `init` 命令是特例，它不需要预先加载配置
    if let Commands::Init { force } = cli.command {
        if let Err(e) = run_init(force).await {
            error!("❌ 初始化失败: {}", e);
            std::process::exit(1);
        }
        return;
    }

    // `status` 命令特殊处理：即使应用初始化失败也要显示基本信息
    if let Commands::Status = cli.command {
        // 总是先显示客户端版本信息（内置的，不依赖配置）
        nuwax_cli::show_client_version();

        // 尝试初始化应用显示完整状态
        match CliApp::new_with_auto_config().await {
            Ok(app) => {
                // 应用初始化成功，显示完整状态信息
                if let Err(e) = nuwax_cli::run_status_details(&app).await {
                    error!("❌ 获取详细状态失败: {}", e);
                }
            }
            Err(e) => {
                // 应用初始化失败，显示友好提示
                error!("⚠️  无法获取完整状态信息: {}", e);
                info!("");
                info!("💡 可能的原因:");
                info!("   - 当前目录不是 Nuwax Cli ent 工作目录");
                info!("   - 配置文件或数据库文件不在当前目录");
                info!("   - 数据库文件被其他进程占用");
                info!("");
                info!("🔧 解决方案:");
                info!("   1. 切换到 Nuwax Cli ent 初始化的目录（包含 config.toml 的目录）");
                info!("   2. 或者在新目录运行 'nuwax-cli init' 重新初始化");
                info!("   3. 确保没有其他 nuwax-cli 进程在运行");
            }
        }
        return;
    }

    // `diff-sql` 命令特殊处理：不需要数据库初始化，纯文件操作
    if let Commands::DiffSql {
        old_sql,
        new_sql,
        old_version,
        new_version,
        output,
    } = cli.command
    {
        if let Err(e) = run_diff_sql(old_sql, new_sql, old_version, new_version, output).await {
            error!("❌ SQL差异对比失败: {}", e);
            std::process::exit(1);
        }
        return;
    }

    // 对于其他所有命令，我们需要加载配置并初始化App
    let mut app = match CliApp::new_with_config_path(&cli.config).await {
        Ok(app) => app,
        Err(e) => {
            // 检查错误的根本原因是否是ConfigNotFound
            let mut source = e.source();
            let mut is_config_not_found = false;
            while let Some(err) = source {
                if err.downcast_ref::<DuckError>().is_some() {
                    if let Some(DuckError::ConfigNotFound) = err.downcast_ref::<DuckError>() {
                        is_config_not_found = true;
                        break;
                    }
                }
                source = err.source();
            }

            if is_config_not_found {
                error!("❌ 配置文件 '{}' 未找到。", cli.config.display());
                error!("👉 请先运行 'nuwax-cli init' 命令来创建配置文件。");
            } else {
                error!("❌ 应用初始化失败: {}", e);
            }
            std::process::exit(1);
        }
    };

    // 运行命令
    if let Err(e) = app.run_command(cli.command).await {
        error!("❌ 操作失败: {}", e);
        std::process::exit(1);
    }
}
