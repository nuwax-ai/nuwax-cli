#[macro_use]
extern crate rust_i18n;
i18n!("../locales", fallback = ["en", "zh-CN"]);

use clap::Parser;
use client_core::container::{detect_compose_command_type, set_compose_command_type};
use client_core::{DuckError, environment::Environment};
use nuwax_cli::{
    Cli, CliApp, Commands, check_and_install_nuwax_cli_update_early, run_diff_sql, run_init,
    setup_logging,
};
use rust_i18n::set_locale;
use tracing::{error, info, warn};

/// 检测并设置语言
fn detect_and_set_language(_cli: &Cli) {
    // CLI logs and messages are intentionally fixed to English.
    set_locale("en");
}

#[tokio::main]
async fn main() {
    // 解析命令行参数
    let cli = Cli::parse();

    // 设置语言
    detect_and_set_language(&cli);

    // 检测环境并显示提示
    let environment = Environment::from_env();
    if environment.is_testing() {
        warn!("⚠️  RUNNING IN TESTING MODE");
warn!("   Environment: {env}", env = environment.display_name());
warn!("   API Endpoint: {url}", url = client_core::constants::api::get_base_url());
        warn!("   Configuration: config-test.toml (if exists)");
        warn!("   Use Ctrl+C to cancel if this is not intended");
        warn!("   Waiting 2 seconds...");

        // 给用户时间看到警告
        tokio::time::sleep(std::time::Duration::from_secs(2)).await;

        warn!("🚀 Starting in Testing Environment...");
    }

    // 设置日志记录
    setup_logging(cli.verbose);

    // `init` 命令是特例，它不需要预先加载配置
    if let Commands::Init { force } = cli.command {
        if let Err(e) = run_init(force).await {
error!("❌ Initialization failed: {error}", error = e.to_string());
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
error!("❌ Failed to get detailed status: {error}", error = e.to_string());
                }
            }
            Err(e) => {
                // 应用初始化失败，显示友好提示
error!("⚠️  Unable to get full status info: {error}", error = e.to_string());
                info!("");
                info!("💡 Possible reasons:");
                info!("   - Current directory is not Nuwax Cli ent working directory");
                info!("   - Configuration or database file not in current directory");
                info!("   - Database file locked by another process");
                info!("");
                info!("🔧 Solutions:");
                info!("   1. Switch to Nuwax Cli ent initialized directory (contains config.toml)");
                info!("   2. Or run 'nuwax-cli init' in a new directory to reinitialize");
                info!("   3. Ensure no other nuwax-cli process is running");
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
error!("❌ SQL diff comparison failed: {error}", error = e.to_string());
            std::process::exit(1);
        }
        return;
    }

    // 🚀 特殊处理：AutoUpgradeDeploy 命令需要优先检查CLI版本更新（在任何数据库初始化之前）
    if let Commands::AutoUpgradeDeploy(_) = cli.command {
        info!("🔍 AutoUpgradeDeploy command detected, prioritizing CLI version check...");
        if let Err(e) = check_and_install_nuwax_cli_update_early().await {
error!("❌ CLI version check failed: {error}", error = e.to_string());
            std::process::exit(1);
        }
        // 如果有更新，上面的函数会直接退出进程，不会继续执行到这里
        info!("✅ CLI version check complete, continuing with AutoUpgradeDeploy command");

        // 🔍 检测 Docker Compose 命令类型（仅在此处检测一次，后续直接使用）
        let compose_type = detect_compose_command_type().await;
        set_compose_command_type(compose_type);
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
error!("❌ Configuration file '{file}' not found.", file = cli.config.display());
                error!("👉 Please run 'nuwax-cli init' first to create configuration file.");
            } else {
error!("❌ Application initialization failed: {error}", error = e.to_string());
            }
            std::process::exit(1);
        }
    };

    // 运行命令
    if let Err(e) = app.run_command(cli.command).await {
error!("❌ Operation failed: {error}", error = e.to_string());
        std::process::exit(1);
    }
}
