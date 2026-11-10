use anyhow::Result;
use client_core::{config::AppConfig, constants::config, database::Database};
use tracing::{info, warn};

/// 运行独立的初始化流程
pub async fn run_init(force: bool) -> Result<()> {
    info!("🦆 Nuwax Cli ent Initialization");
    info!("======================");

    // 检查是否已经初始化过
    if !force
        && (client_core::constants::config::get_config_file_path().exists()
            || config::get_database_path().exists())
    {
        warn!("⚠️  Detected existing configuration or database files");
        info!("Use --force flag to reinitialize");
        info!("Example: nuwax-cli init --force");
        return Ok(());
    }

    info!("📋 Step 1: Create configuration and directory structure");

    // 创建默认配置
    let config = AppConfig::default();
    config.save_to_file("config.toml")?;
    info!("   ✅ Created configuration file: config.toml");

    // 创建必要的目录结构
    std::fs::create_dir_all("docker")?;
    std::fs::create_dir_all(&config.backup.storage_dir)?;
    config.ensure_cache_dirs()?;
    info!("   ✅ Created directory structure:");
    info!("      - docker/                (Docker service files directory)");
    info!(
        "      - {dir}         (Backup storage directory)",
        dir = config.backup.storage_dir
    );
    info!(
        "      - {dir}    (Cache directory)",
        dir = config.cache.cache_dir
    );
    info!(
        "      - {dir} (Download cache directory)",
        dir = config.cache.download_dir
    );

    info!("📋 Step 2: Initialize database");

    // 初始化数据库
    let db_path = config::get_database_path();
    let database = Database::connect(&db_path).await?;

    // 显式初始化数据库表结构（只在 init 时执行）
    database.init_database().await?;

    info!(
        "   ✅ Created DuckDB database: {path}",
        path = db_path.display()
    );

    // 生成新的客户端UUID
    let client_uuid = database.get_or_create_client_uuid().await?;
    info!("   ✅ Generated client UUID: {uuid}", uuid = client_uuid);

    info!("🎉 Initialization complete!");
    info!("");
    info!("📝 Next steps:");
    info!("   1️⃣  Run 'nuwax-cli upgrade' to download Docker service package");
    info!("       - Or run 'nuwax-cli upgrade --force' to force full download");
    info!("       - Or run 'nuwax-cli upgrade download' to prepare an offline deployment package");
    info!("   2️⃣  Run 'nuwax-cli docker-service deploy' to deploy Docker services");
    info!("   3️⃣  Run 'nuwax-cli docker-service start' to start Docker services");
    info!("");
    info!("🚀 Shortcut - Auto upgrade deploy:");
    info!("   • Run 'nuwax-cli auto-upgrade-deploy run' for complete upgrade deployment");
    info!("");
    info!("💡 Tips:");
    info!("   - Configuration file: config.toml (can be manually edited)");
    info!(
        "   - Database file: {path} (stores operation history and backup records)",
        path = db_path.display()
    );
    info!("   - Use 'nuwax-cli --help' to see all available commands");
    info!("   - Use 'nuwax-cli status' to check current system status");

    Ok(())
}
