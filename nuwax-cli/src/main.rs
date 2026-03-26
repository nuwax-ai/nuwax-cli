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

/// 规范化语言代码，兼容 zh_CN / en_US / zh-HK 等写法
fn normalize_locale(raw: &str) -> String {
    let normalized = raw.split('.').next().unwrap_or(raw).replace('_', "-");
    let lower = normalized.to_ascii_lowercase();

    match lower.as_str() {
        "zh" | "zh-cn" | "zh-hans" => "zh-CN".to_string(),
        "zh-tw" | "zh-hk" | "zh-hant" => "zh-TW".to_string(),
        "en" | "en-us" | "en-gb" => "en".to_string(),
        _ => normalized,
    }
}

/// 确保 locale 在已加载的语言列表中，否则回退到 en
fn sanitize_supported_locale(locale: String) -> String {
    let available = rust_i18n::available_locales!();
    if available.iter().any(|&l| l == locale) {
        return locale;
    }

    let primary = locale.split('-').next().unwrap_or("en");
    match primary {
        "zh" => {
            if available.iter().any(|&l| l == "zh-CN") {
                "zh-CN".to_string()
            } else {
                "en".to_string()
            }
        }
        "en" => "en".to_string(),
        _ => "en".to_string(),
    }
}

/// 检测并设置语言
fn detect_and_set_language(cli: &Cli) {
    // 优先级: CLI 参数 > DEFAULT_LOCALE 环境变量 > NUWAX_LANG > LANG > 系统语言 > 默认英文
    let lang = if let Some(ref lang) = cli.lang {
        normalize_locale(lang)
    } else if let Ok(lang) = std::env::var("DEFAULT_LOCALE") {
        normalize_locale(&lang)
    } else if let Ok(lang) = std::env::var("NUWAX_LANG") {
        normalize_locale(&lang)
    } else if let Ok(lang) = std::env::var("LANG") {
        normalize_locale(&lang)
    } else {
        // 尝试检测系统语言
        #[cfg(target_os = "macos")]
        {
            if let Ok(output) = std::process::Command::new("defaults")
                .args(["read", "-g", "AppleLocale"])
                .output()
            {
                if let Ok(locale) = String::from_utf8(output.stdout) {
                    let lang = normalize_locale(locale.trim());
                    return set_locale(&sanitize_supported_locale(lang));
                }
            }
        }
        "en".to_string()
    };

    set_locale(&sanitize_supported_locale(lang));
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
        warn!("{}", t!("main.testing_mode"));
        warn!("{}", t!("main.testing_env", env = environment.display_name()));
        warn!("{}", t!("main.testing_api", url = client_core::constants::api::get_base_url()));
        warn!("{}", t!("main.testing_config"));
        warn!("{}", t!("main.testing_cancel"));
        warn!("{}", t!("main.testing_wait"));

        // 给用户时间看到警告
        tokio::time::sleep(std::time::Duration::from_secs(2)).await;

        warn!("{}", t!("main.testing_start"));
    }

    // 设置日志记录
    setup_logging(cli.verbose);

    // `init` 命令是特例，它不需要预先加载配置
    if let Commands::Init { force } = cli.command {
        if let Err(e) = run_init(force).await {
            error!("{}", t!("main.init_failed", error = e.to_string()));
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
                    error!("{}", t!("main.detail_status_failed", error = e.to_string()));
                }
            }
            Err(e) => {
                // 应用初始化失败，显示友好提示
                error!("{}", t!("status.error_status", error = e.to_string()));
                info!("");
                info!("{}", t!("status.possible_reasons"));
                info!("{}", t!("status.reason_not_work_dir"));
                info!("{}", t!("status.reason_config_not_found"));
                info!("{}", t!("status.reason_db_locked"));
                info!("");
                info!("{}", t!("status.solutions"));
                info!("{}", t!("status.solution_switch_dir"));
                info!("{}", t!("status.solution_reinit"));
                info!("{}", t!("status.solution_check_process"));
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
            error!("{}", t!("status_cmd.sql_diff_failed", error = e.to_string()));
            std::process::exit(1);
        }
        return;
    }

    // 🚀 特殊处理：AutoUpgradeDeploy 命令需要优先检查CLI版本更新（在任何数据库初始化之前）
    if let Commands::AutoUpgradeDeploy(_) = cli.command {
        info!("{}", t!("main.auto_upgrade_cli_check"));
        if let Err(e) = check_and_install_nuwax_cli_update_early().await {
            error!("{}", t!("main.cli_check_failed", error = e.to_string()));
            std::process::exit(1);
        }
        // 如果有更新，上面的函数会直接退出进程，不会继续执行到这里
        info!("{}", t!("main.cli_check_done"));

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
                error!("{}", t!("main.config_not_found", file = cli.config.display()));
                error!("{}", t!("main.config_not_found_hint"));
            } else {
                error!("{}", t!("main.app_init_failed", error = e.to_string()));
            }
            std::process::exit(1);
        }
    };

    // 运行命令
    if let Err(e) = app.run_command(cli.command).await {
        error!("{}", t!("main.operation_failed", error = e.to_string()));
        std::process::exit(1);
    }
}
