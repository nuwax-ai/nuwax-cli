//! `.env` 合并助手：保留用户已有值、仅补包内新增键。
//!
//! 在线 full/patch 解压器与部署命令共用（C01/P1#2）：
//! - `merge_env_contents`：纯文本合并（用户值优先，包内只补缺失键）；
//! - `merge_preserved_env_file`：原子落盘（临时文件 + fsync + 保留权限 + 失败不污染）。

use anyhow::{Context, Result};
use std::fs;
use std::io::Write;
use std::path::Path;

/// 合并 `.env` 内容：`preserved`（用户现值）完全保留，`package`（包内默认值）
/// 只补充用户文件中不存在的键。行级语义与 Compose env_file 一致。
pub fn merge_env_contents(preserved: &str, package: &str) -> String {
    let mut keys = preserved
        .lines()
        .filter_map(env_assignment_key)
        .map(str::to_owned)
        .collect::<std::collections::HashSet<_>>();
    let mut merged = preserved.to_owned();

    for line in package.lines() {
        let Some(key) = env_assignment_key(line) else {
            continue;
        };
        if keys.insert(key.to_owned()) {
            if !merged.is_empty() && !merged.ends_with('\n') {
                merged.push('\n');
            }
            merged.push_str(line);
            merged.push('\n');
        }
    }

    merged
}

/// 提取 `KEY=VALUE` 行的键名（跳过注释/空行；`export ` 前缀容忍；键为安全标识符）
pub fn env_assignment_key(line: &str) -> Option<&str> {
    let line = line.trim();
    if line.is_empty() || line.starts_with('#') {
        return None;
    }
    let assignment = line.strip_prefix("export ").unwrap_or(line);
    let (key, _) = assignment.split_once('=')?;
    let key = key.trim();
    let mut characters = key.chars();
    let first = characters.next()?;
    if !(first == '_' || first.is_ascii_alphabetic())
        || !characters.all(|character| character == '_' || character.is_ascii_alphanumeric())
    {
        return None;
    }
    Some(key)
}

/// 把 `preserved`（用户现值）合并进 `package_path`（包内 .env，将被替换为合并结果）。
/// 原子写入：同目录临时文件 → fsync → 保留原权限 → rename；失败不污染目标。
pub fn merge_preserved_env_file(preserved_path: &Path, package_path: &Path) -> Result<()> {
    let preserved = fs::read_to_string(preserved_path).with_context(|| {
        format!(
            "Failed to read existing environment file: {}",
            preserved_path.display()
        )
    })?;
    let package = fs::read_to_string(package_path).with_context(|| {
        format!(
            "Failed to read package environment file: {}",
            package_path.display()
        )
    })?;
    let merged = merge_env_contents(&preserved, &package);
    let permissions = fs::metadata(preserved_path)
        .with_context(|| {
            format!(
                "Failed to inspect existing environment file: {}",
                preserved_path.display()
            )
        })?
        .permissions();
    let mut temp_file = tempfile::NamedTempFile::new_in(
        package_path
            .parent()
            .context("Package environment file has no parent directory")?,
    )
    .context("Failed to create temporary merged environment file")?;
    temp_file
        .write_all(merged.as_bytes())
        .context("Failed to write merged environment file")?;
    temp_file
        .as_file()
        .sync_all()
        .context("Failed to flush merged environment file")?;
    fs::set_permissions(temp_file.path(), permissions)
        .context("Failed to preserve environment file permissions")?;

    fs::remove_file(package_path).with_context(|| {
        format!(
            "Failed to replace package environment file: {}",
            package_path.display()
        )
    })?;
    temp_file
        .persist(package_path)
        .map_err(|error| error.error)
        .with_context(|| {
            format!(
                "Failed to install merged environment file: {}",
                package_path.display()
            )
        })?;
    fs::remove_file(preserved_path).with_context(|| {
        format!(
            "Failed to remove backed-up environment file: {}",
            preserved_path.display()
        )
    })?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn merge_keeps_user_values_and_only_adds_new_keys() {
        let preserved = "MYSQL_PASSWORD=user-secret\nPORT=8080\n";
        let package = "MYSQL_PASSWORD=package-default\nNEW_KEY=added\n";
        let merged = merge_env_contents(preserved, package);
        assert!(merged.contains("MYSQL_PASSWORD=user-secret"));
        assert!(merged.contains("PORT=8080"));
        assert!(merged.contains("NEW_KEY=added"));
        assert!(!merged.contains("package-default"));
    }

    #[test]
    fn merge_preserves_comments_and_missing_trailing_newline() {
        let preserved = "# operator config\nA=1";
        let package = "B=2\n";
        let merged = merge_env_contents(preserved, package);
        assert!(merged.starts_with("# operator config\nA=1\n"));
        assert!(merged.ends_with("B=2\n"));
    }

    #[test]
    fn assignment_key_tolerates_export_and_rejects_garbage() {
        assert_eq!(env_assignment_key("KEY=value"), Some("KEY"));
        assert_eq!(env_assignment_key("export KEY=value"), Some("KEY"));
        assert_eq!(env_assignment_key("# comment"), None);
        assert_eq!(env_assignment_key("no-equals"), None);
        assert_eq!(env_assignment_key("1BAD=x"), None);
        assert_eq!(env_assignment_key("BAD-KEY=x"), None);
    }
}
