use anyhow::Result;
use client_core::sql_diff::generate_schema_diff;
use std::fs;
use std::path::PathBuf;
use tracing::info;

/// 对比两个SQL文件并生成差异SQL
pub async fn run_diff_sql(
    old_sql_path: PathBuf,
    new_sql_path: PathBuf,
    old_version: Option<String>,
    new_version: Option<String>,
    output_file: String,
) -> Result<()> {
    info!("🔄 开始SQL文件差异对比...");
    info!("📄 旧版本SQL: {}", old_sql_path.display());
    info!("📄 新版本SQL: {}", new_sql_path.display());

    // 检查输入文件是否存在
    if !old_sql_path.exists() {
        return Err(anyhow::anyhow!(format!(
            "旧版本SQL文件不存在: {}",
            old_sql_path.display()
        )));
    }

    if !new_sql_path.exists() {
        return Err(anyhow::anyhow!(format!(
            "新版本SQL文件不存在: {}",
            new_sql_path.display()
        )));
    }

    // 读取文件内容
    info!("📖 正在读取SQL文件...");
    let old_sql_content = fs::read_to_string(&old_sql_path).map_err(|e| {
        client_core::error::DuckError::custom(format!("读取旧版本SQL文件失败: {e}"))
    })?;

    let new_sql_content = fs::read_to_string(&new_sql_path).map_err(|e| {
        client_core::error::DuckError::custom(format!("读取新版本SQL文件失败: {e}"))
    })?;

    // 设置默认版本号
    let from_version = old_version.as_deref().unwrap_or("unknown");
    let to_version = new_version.as_deref().unwrap_or("latest");

    // 生成差异SQL
    info!("🔍 正在分析SQL差异...");
    let (diff_sql, description) = generate_schema_diff(
        Some(&old_sql_content),
        &new_sql_content,
        Some(from_version),
        to_version,
    )
    .map_err(|e| client_core::error::DuckError::custom(format!("生成SQL差异失败: {e}")))?;

    info!("📊 SQL差异分析结果: {}", description);

    // 检查是否有实际的SQL语句需要执行
    let meaningful_lines: Vec<&str> = diff_sql
        .lines()
        .filter(|line| !line.trim().is_empty() && !line.trim().starts_with("--"))
        .collect();

    if meaningful_lines.is_empty() {
        info!("✅ 数据库架构无变化，无需升级");
        info!("📄 生成空的差异文件: {}", output_file);

        // 创建包含说明的空差异文件
        let empty_diff_content = format!(
            "-- SQL差异分析结果\n-- {description}\n-- 无需执行任何SQL语句，数据库架构无变化\n"
        );
        fs::write(&output_file, empty_diff_content)
            .map_err(|e| client_core::error::DuckError::custom(format!("写入差异文件失败: {e}")))?;
    } else {
        // 保存差异SQL文件
        fs::write(&output_file, &diff_sql)
            .map_err(|e| client_core::error::DuckError::custom(format!("写入差异文件失败: {e}")))?;

        info!("📄 已保存SQL差异文件: {}", output_file);
        info!("📋 发现 {} 行可执行的SQL语句", meaningful_lines.len());

        // 显示差异SQL内容（截取前10行）
        let diff_lines: Vec<&str> = diff_sql.lines().take(10).collect();
        info!("📋 差异SQL预览（前10行）:");
        for line in diff_lines {
            if !line.trim().is_empty() {
                info!("    {}", line);
            }
        }

        if diff_sql.lines().count() > 10 {
            info!("    ... 更多内容请查看文件: {}", output_file);
        }
    }

    // 显示执行建议
    info!("💡 使用建议:");
    info!("   1. 请先备份您的数据库");
    info!("   2. 在测试环境中验证差异SQL");
    info!("   3. 确认无误后在生产环境执行");

    if !meaningful_lines.is_empty() {
        info!(
            "   4. 执行示例: mysql -u username -p database_name < {}",
            output_file
        );
    }

    info!("✅ SQL差异对比完成");
    Ok(())
}
