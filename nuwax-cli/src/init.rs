use anyhow::Result;
use client_core::{
    ClientRegisterRequest, api::ApiClient, config::AppConfig, constants::config, database::Database,
};
use tracing::{info, warn};

/// 运行独立的初始化流程
pub async fn run_init(force: bool) -> Result<()> {
    info!("🦆 Nuwax Cli ent 初始化");
    info!("======================");

    // 检查是否已经初始化过
    if !force
        && (client_core::constants::config::get_config_file_path().exists()
            || config::get_database_path().exists())
    {
        warn!("⚠️  检测到已存在的配置文件或数据库文件");
        info!("如果您要重新初始化，请使用 --force 参数");
        info!("示例: nuwax-cli init --force");
        return Ok(());
    }

    info!("📋 步骤 1: 创建配置文件和目录结构");

    // 创建默认配置
    let config = AppConfig::default();
    config.save_to_file("config.toml")?;
    info!("   ✅ 创建配置文件: config.toml");

    // 创建必要的目录结构
    std::fs::create_dir_all("docker")?;
    std::fs::create_dir_all(&config.backup.storage_dir)?;
    config.ensure_cache_dirs()?;
    info!("   ✅ 创建目录结构:");
    info!("      - docker/                (Docker服务文件目录)");
    info!(
        "      - {}         (备份存储目录)",
        config.backup.storage_dir
    );
    info!("      - {}    (缓存目录)", config.cache.cache_dir);
    info!("      - {} (下载缓存目录)", config.cache.download_dir);

    info!("📋 步骤 2: 初始化数据库");

    // 初始化数据库
    let db_path = config::get_database_path();
    let database = Database::connect(&db_path).await?;

    // 显式初始化数据库表结构（只在 init 时执行）
    database.init_database().await?;

    info!("   ✅ 创建DuckDB数据库: {}", db_path.display());

    // 生成新的客户端UUID
    let client_uuid = database.get_or_create_client_uuid().await?;
    info!("   ✅ 生成客户端UUID: {}", client_uuid);

    info!("📋 步骤 3: 向服务器注册客户端");

    // 收集系统信息并注册客户端
    let request = ClientRegisterRequest {
        os: std::env::consts::OS.to_string(),
        arch: std::env::consts::ARCH.to_string(),
    };

    // 创建API客户端（注册时不需要client_id）
    let api_client = ApiClient::new(None, None);
    match api_client.register_client(request).await {
        Ok(server_client_id) => {
            info!("   ✅ 客户端注册成功，获得客户端ID: {}", server_client_id);

            // 保存服务端返回的client_id到数据库，覆盖本地生成的UUID
            database.update_client_id(&server_client_id).await?;
            info!("   ✅ 客户端ID已保存到数据库");
        }
        Err(e) => {
            warn!("   ⚠️  客户端注册失败: {} (可稍后重试)", e);
            info!("   💡 这不会影响本地功能的使用");
        }
    }

    info!("🎉 初始化完成！");
    info!("");
    info!("📝 接下来的步骤:");
    info!("   1️⃣  运行 'nuwax-cli upgrade' 下载Docker服务全量包");
    info!("       - 或者运行 'nuwax-cli upgrade --full --force' 强制下载完整服务包");
    info!("   2️⃣  运行 'nuwax-cli docker-service deploy' 部署Docker服务");
    info!("   3️⃣  运行 'nuwax-cli docker-service start' 启动Docker服务");
    info!("");
    info!("🚀 快捷方式 - 自动升级部署:");
    info!("   • 运行 'nuwax-cli auto-upgrade-deploy run' 自动执行完整的升级部署流程");
    info!(
        "   • 运行 'nuwax-cli auto-upgrade-deploy delay-time-deploy 2 --unit hours' 延时2小时后自动部署"
    );
    info!("");
    info!("💡 提示:");
    info!("   - 配置文件: config.toml (可手动编辑修改配置)");
    info!(
        "   - 数据库文件: {} (存储操作历史和备份记录)",
        db_path.display()
    );
    info!("   - 使用 'nuwax-cli --help' 查看所有可用命令");
    info!("   - 使用 'nuwax-cli status' 查看当前系统状态");

    Ok(())
}
