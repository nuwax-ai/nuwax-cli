use anyhow::Result;
use client_core::{
    ClientRegisterRequest, api::ApiClient, config::AppConfig, constants::config, database::Database,
};
use rust_i18n::t;
use tracing::{info, warn};

/// 运行独立的初始化流程
pub async fn run_init(force: bool) -> Result<()> {
    info!("{}", t!("init_cmd.title"));
    info!("{}", t!("init_cmd.separator"));

    // 检查是否已经初始化过
    if !force
        && (client_core::constants::config::get_config_file_path().exists()
            || config::get_database_path().exists())
    {
        warn!("{}", t!("init_cmd.existing_files"));
        info!("{}", t!("init_cmd.use_force"));
        info!("{}", t!("init_cmd.force_example"));
        return Ok(());
    }

    info!("{}", t!("init_cmd.step1_title"));

    // 创建默认配置
    let config = AppConfig::default();
    config.save_to_file("config.toml")?;
    info!("{}", t!("init_cmd.config_created"));

    // 创建必要的目录结构
    std::fs::create_dir_all("docker")?;
    std::fs::create_dir_all(&config.backup.storage_dir)?;
    config.ensure_cache_dirs()?;
    info!("{}", t!("init_cmd.dirs_created"));
    info!("{}", t!("init_cmd.dir_docker"));
    info!("{}", t!("init_cmd.dir_backup", dir = config.backup.storage_dir));
    info!("{}", t!("init_cmd.dir_cache", dir = config.cache.cache_dir));
    info!("{}", t!("init_cmd.dir_download", dir = config.cache.download_dir));

    info!("{}", t!("init_cmd.step2_title"));

    // 初始化数据库
    let db_path = config::get_database_path();
    let database = Database::connect(&db_path).await?;

    // 显式初始化数据库表结构（只在 init 时执行）
    database.init_database().await?;

    info!("{}", t!("init_cmd.db_created", path = db_path.display()));

    // 生成新的客户端UUID
    let client_uuid = database.get_or_create_client_uuid().await?;
    info!("{}", t!("init_cmd.uuid_generated", uuid = client_uuid));

    info!("{}", t!("init_cmd.step3_title"));

    // 收集系统信息并注册客户端
    let request = ClientRegisterRequest {
        os: std::env::consts::OS.to_string(),
        arch: std::env::consts::ARCH.to_string(),
    };

    // 创建API客户端（注册时不需要client_id）
    let api_client = ApiClient::new(None, None);
    match api_client.register_client(request).await {
        Ok(server_client_id) => {
            info!("{}", t!("init_cmd.register_success", id = server_client_id));

            // 保存服务端返回的client_id到数据库，覆盖本地生成的UUID
            database.update_client_id(&server_client_id).await?;
            info!("{}", t!("init_cmd.id_saved"));
        }
        Err(e) => {
            warn!("{}", t!("init_cmd.register_failed", error = e.to_string()));
            info!("{}", t!("init_cmd.register_hint"));
        }
    }

    info!("{}", t!("init_cmd.complete"));
    info!("");
    info!("{}", t!("init_cmd.next_steps"));
    info!("{}", t!("init_cmd.step1_upgrade"));
    info!("{}", t!("init_cmd.step1_upgrade_force"));
    info!("{}", t!("init_cmd.step2_deploy"));
    info!("{}", t!("init_cmd.step3_start"));
    info!("");
    info!("{}", t!("init_cmd.shortcut_title"));
    info!("{}", t!("init_cmd.shortcut_auto"));
    info!("{}", t!("init_cmd.shortcut_delay"));
    info!("");
    info!("{}", t!("init_cmd.tips_title"));
    info!("{}", t!("init_cmd.tip_config"));
    info!("{}", t!("init_cmd.tip_db", path = db_path.display()));
    info!("{}", t!("init_cmd.tip_help"));
    info!("{}", t!("init_cmd.tip_status"));

    Ok(())
}
