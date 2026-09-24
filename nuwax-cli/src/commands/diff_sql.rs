use anyhow::Result;
use client_core::sql_diff::generate_schema_diff;
use rust_i18n::t;
use std::fs;
use std::path::PathBuf;
use tracing::{info, warn};

/// 对比两个SQL文件并生成差异SQL
pub async fn run_diff_sql(
    old_sql_path: PathBuf,
    new_sql_path: PathBuf,
    old_version: Option<String>,
    new_version: Option<String>,
    output_file: String,
) -> Result<()> {
    info!("🔄 Starting SQL file diff comparison...");
    info!("📄 Old version SQL: {path}", path = old_sql_path.display());
    info!("📄 New version SQL: {path}", path = new_sql_path.display());

    // 检查输入文件是否存在
    if !old_sql_path.exists() {
        return Err(anyhow::anyhow!(
            t!("diff_sql.old_sql_not_found", path = old_sql_path.display()).to_string()
        ));
    }

    if !new_sql_path.exists() {
        return Err(anyhow::anyhow!(
            t!("diff_sql.new_sql_not_found", path = new_sql_path.display()).to_string()
        ));
    }

    // 读取文件内容
    info!("📖 Reading SQL files...");
    let old_sql_content = fs::read_to_string(&old_sql_path).map_err(|e| {
        client_core::error::DuckError::custom(
            t!("diff_sql.read_old_failed", error = e.to_string()).to_string(),
        )
    })?;

    let new_sql_content = fs::read_to_string(&new_sql_path).map_err(|e| {
        client_core::error::DuckError::custom(
            t!("diff_sql.read_new_failed", error = e.to_string()).to_string(),
        )
    })?;

    // 设置默认版本号
    let from_version = old_version.as_deref().unwrap_or("unknown");
    let to_version = new_version.as_deref().unwrap_or("latest");

    // 生成差异SQL
    info!("🔍 Analyzing SQL differences...");
    let (diff_sql, description) = generate_schema_diff(
        Some(&old_sql_content),
        &new_sql_content,
        Some(from_version),
        to_version,
    )
    .map_err(|e| {
        client_core::error::DuckError::custom(
            t!("diff_sql.generate_failed", error = e.to_string()).to_string(),
        )
    })?;

    info!(
        "📊 SQL diff analysis result: {description}",
        description = description
    );

    // 检查是否有实际的SQL语句需要执行
    let meaningful_lines: Vec<&str> = diff_sql
        .lines()
        .filter(|line| !line.trim().is_empty() && !line.trim().starts_with("--"))
        .collect();

    if diff_sql.trim().is_empty() {
        info!("✅ No database schema changes, upgrade not needed");
        info!("📄 Generated empty diff file: {file}", file = output_file);

        // 创建包含说明的空差异文件
        let empty_diff_content = format!(
            "-- SQL diff analysis result\n-- {description}\n-- No SQL execution needed; database schema unchanged\n"
        );
        fs::write(&output_file, empty_diff_content).map_err(|e| {
            client_core::error::DuckError::custom(
                t!("diff_sql.write_failed", error = e.to_string()).to_string(),
            )
        })?;
    } else {
        // 保存差异SQL文件
        fs::write(&output_file, &diff_sql).map_err(|e| {
            client_core::error::DuckError::custom(
                t!("diff_sql.write_failed", error = e.to_string()).to_string(),
            )
        })?;

        info!("📄 SQL diff file saved: {file}", file = output_file);
        if meaningful_lines.is_empty() {
            warn!("⚠️ Schema differences require manual changes; see the diff file");
        } else {
            info!(
                "📋 Found {count} executable SQL lines",
                count = meaningful_lines.len()
            );
        }

        // 显示差异SQL内容（截取前10行）
        let diff_lines: Vec<&str> = diff_sql.lines().take(10).collect();
        info!("📋 SQL diff preview (first 10 lines):");
        for line in diff_lines {
            if !line.trim().is_empty() {
                info!("    {}", line);
            }
        }

        if diff_sql.lines().count() > 10 {
            info!("    ... See more in file: {file}", file = output_file);
        }
    }

    // 显示执行建议
    info!("💡 Usage tips:");
    info!("   1. Backup your database first");
    info!("   2. Verify the diff SQL in test environment");
    info!("   3. Execute in production after confirmation");

    if !meaningful_lines.is_empty() {
        info!(
            "   4. Execute example: mysql -u username -p database_name < {file}",
            file = output_file
        );
    }

    info!("✅ SQL diff comparison completed");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    async fn compare_files(old: &str, new: &str) -> Result<String> {
        let directory = tempfile::tempdir()?;
        let old_path = directory.path().join("old.sql");
        let new_path = directory.path().join("new.sql");
        let output = directory.path().join("diff.sql");
        fs::write(&old_path, old)?;
        fs::write(&new_path, new)?;
        run_diff_sql(
            old_path,
            new_path,
            None,
            None,
            output.to_string_lossy().into_owned(),
        )
        .await?;
        Ok(fs::read_to_string(output)?)
    }

    #[tokio::test]
    async fn warning_only_diff_preserves_manual_instructions() -> Result<()> {
        for (old, new, warning) in [
            (
                "CREATE TABLE users (id INT, name VARCHAR(20));",
                "CREATE TABLE users (id INT);",
                "DROP COLUMN `name`",
            ),
            (
                "CREATE TABLE users (id INT, name VARCHAR(20), KEY idx (id));",
                "CREATE TABLE users (id INT, name VARCHAR(20), KEY idx (name));",
                "manually drop old index",
            ),
            (
                "CREATE TABLE users (id INT) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4 COLLATE=utf8mb4_unicode_ci;",
                "CREATE TABLE users (id INT) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4 COLLATE=utf8mb4_0900_ai_ci;",
                "table option COLLATION differs",
            ),
        ] {
            let output = compare_files(old, new).await?;
            assert!(output.contains(warning), "Missing {warning}: {output}");
            assert!(!output.contains("database schema unchanged"));
            assert!(output.lines().all(|line| {
                let line = line.trim();
                line.is_empty() || line.starts_with("--")
            }));
        }
        Ok(())
    }

    #[tokio::test]
    async fn identical_schema_produces_unchanged_notice() -> Result<()> {
        let sql = "CREATE TABLE users (id INT);";
        let output = compare_files(sql, sql).await?;
        assert!(output.contains("database schema unchanged"));
        Ok(())
    }

    #[tokio::test]
    async fn mixed_diff_keeps_executable_sql_and_manual_instructions() -> Result<()> {
        let output = compare_files(
            "CREATE TABLE users (id INT, old_name VARCHAR(20));",
            "CREATE TABLE users (id INT, new_name VARCHAR(20));",
        )
        .await?;
        assert!(output.contains("ALTER TABLE `users` ADD COLUMN `new_name`"));
        assert!(output.contains("-- To delete it, run manually:"));
        assert!(output.contains("DROP COLUMN `old_name`"));
        Ok(())
    }
}
