//! `.env` 合并助手：保留用户已有值、仅补包内新增键。
//!
//! 在线 full/patch 解压器与部署命令共用（C01/P1#2）：
//! - `merge_env_contents`：纯文本合并（用户值优先，包内只补缺失键）；
//! - `merge_preserved_env_file`：原子落盘（临时文件 + fsync + 保留权限 + 失败不污染）。

use anyhow::{Context, Result};
use client_core::atomic_file::{PermissionsPolicy, write_atomic};
use std::fs;
use std::path::Path;

/// 合并 `.env` 内容：`preserved`（用户现值）完全保留，`package`（包内默认值）
/// 只补充用户文件中不存在的键。用户声明由共享 Compose record parser 识别；
/// 包内默认仍限制为单行，不重放用户多行值中的伪声明。
pub fn merge_env_contents(preserved: &str, package: &str) -> Result<String> {
    validate_package_defaults(package)?;
    let mut keys = client_core::container::preflight::compose_env_declared_keys(preserved)?;
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

    Ok(merged)
}

fn validate_package_defaults(package: &str) -> Result<()> {
    for line in package.lines() {
        if env_assignment_key(line).is_none() {
            continue;
        }
        let line = line.trim().strip_prefix("export ").unwrap_or(line.trim());
        let Some(separator) = line.find(['=', ':']) else {
            continue;
        };
        let rhs = line[separator + 1..].trim_start();
        let Some(quote) = rhs
            .chars()
            .next()
            .filter(|quote| matches!(quote, '\'' | '"'))
        else {
            continue;
        };
        let mut escaped = false;
        let mut closed = false;
        for character in rhs.chars().skip(1) {
            if character == quote && !escaped {
                closed = true;
                break;
            }
            escaped = character == '\\' && !escaped;
        }
        if !closed {
            anyhow::bail!(
                "Automatic environment merging does not support multiline quoted package defaults; use single-line escaped defaults"
            );
        }
    }
    Ok(())
}

/// 提取 `KEY=VALUE` 行的键名（跳过注释/空行；`export ` 前缀容忍；键为安全标识符）
pub fn env_assignment_key(line: &str) -> Option<&str> {
    let line = line.strip_prefix('\u{feff}').unwrap_or(line).trim();
    if line.is_empty() || line.starts_with('#') {
        return None;
    }
    let assignment = line.strip_prefix("export ").unwrap_or(line);
    let key = assignment
        .split_once(['=', ':'])
        .map_or(assignment, |(key, _)| key)
        .trim();
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
    let merged = merge_env_contents(&preserved, &package)?;
    let permissions = fs::metadata(preserved_path)
        .with_context(|| {
            format!(
                "Failed to inspect existing environment file: {}",
                preserved_path.display()
            )
        })?
        .permissions();
    write_env_contents(package_path, &merged, Some(permissions))?;
    fs::remove_file(preserved_path).with_context(|| {
        format!(
            "Failed to remove backed-up environment file: {}",
            preserved_path.display()
        )
    })?;
    Ok(())
}

/// Write complete environment contents without unlinking the live file or its alias.
/// The shared writer performs replace-existing on Unix and Windows and retains
/// symlinks. A failed replacement never triggers a destructive delete/retry.
pub fn write_env_contents(
    destination: &Path,
    contents: &str,
    permissions: Option<fs::Permissions>,
) -> Result<()> {
    if let Some(permissions) = permissions.as_ref()
        && destination.exists()
    {
        fs::set_permissions(destination, permissions.clone())
            .context("Failed to preserve environment file permissions")?;
    }
    write_atomic(
        destination,
        contents.as_bytes(),
        PermissionsPolicy::Preserve,
    )
    .context("Failed to replace environment file atomically")?;
    if let Some(permissions) = permissions {
        fs::set_permissions(destination, permissions)
            .context("Failed to restore environment file permissions")?;
    }
    Ok(())
}

/// Merge package defaults directly into the live file in a single atomic write.
pub fn merge_env_file_defaults(destination: &Path, defaults: &str) -> Result<()> {
    let preserved = fs::read_to_string(destination)
        .with_context(|| format!("Failed to read environment file: {}", destination.display()))?;
    write_env_contents(
        destination,
        &merge_env_contents(&preserved, defaults)?,
        None,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn windows_bom_does_not_allow_package_defaults_to_override_user_values() -> Result<()> {
        let merged = merge_env_contents(
            "\u{feff}MYSQL_PASSWORD=operator-value\n",
            "MYSQL_PASSWORD=package-default\nNEW_KEY=added\n",
        )?;
        assert!(merged.starts_with("\u{feff}MYSQL_PASSWORD=operator-value\n"));
        assert!(!merged.contains("package-default"));
        let values = client_core::container::preflight::compose_env_values_from_text(&merged)?;
        assert_eq!(
            values.get("MYSQL_PASSWORD").map(String::as_str),
            Some("operator-value")
        );
        assert_eq!(values.get("NEW_KEY").map(String::as_str), Some("added"));
        Ok(())
    }

    #[test]
    fn merge_keeps_user_values_and_only_adds_new_keys() {
        let preserved = "MYSQL_PASSWORD=user-secret\nPORT=8080\n";
        let package = "MYSQL_PASSWORD=package-default\nNEW_KEY=added\n";
        let merged = merge_env_contents(preserved, package).expect("valid env fixture");
        assert!(merged.contains("MYSQL_PASSWORD=user-secret"));
        assert!(merged.contains("PORT=8080"));
        assert!(merged.contains("NEW_KEY=added"));
        assert!(!merged.contains("package-default"));
    }

    #[test]
    fn merge_preserves_comments_and_missing_trailing_newline() {
        let preserved = "# operator config\nA=1";
        let package = "B=2\n";
        let merged = merge_env_contents(preserved, package).expect("valid env fixture");
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

    #[test]
    fn bare_and_colon_user_keys_are_not_overwritten_by_defaults() {
        let merged = merge_env_contents(
            "KEY\nCOLON: user\n",
            "KEY=package\nCOLON=package\nNEW=added\n",
        )
        .expect("valid env fixture");
        assert_eq!(merged, "KEY\nCOLON: user\nNEW=added\n");
    }

    #[test]
    fn user_multiline_contents_survive_and_package_multiline_fails_fast() {
        let preserved = "TEXT='first\nsecond'\n";
        assert_eq!(
            merge_env_contents(preserved, "NEW=added\n").expect("single-line defaults"),
            "TEXT='first\nsecond'\nNEW=added\n"
        );
        assert!(merge_env_contents(preserved, "NEW='first\nsecond'\n").is_err());
    }

    #[test]
    fn multiline_user_value_does_not_hide_a_real_new_default() {
        let preserved = "PEM='first\nNEW_KEY=inside-pem\nlast'\n";
        let merged = merge_env_contents(preserved, "NEW_KEY=default\n")
            .expect("single-line package default");
        assert_eq!(merged, format!("{preserved}NEW_KEY=default\n"));
        let values = client_core::container::preflight::compose_env_values_from_text(&merged)
            .expect("merged env");
        assert_eq!(
            values.get("PEM").map(String::as_str),
            Some("first\nNEW_KEY=inside-pem\nlast")
        );
        assert_eq!(values.get("NEW_KEY").map(String::as_str), Some("default"));
    }

    #[test]
    fn declaration_discovery_retains_bare_keys_without_evaluating_defaults() {
        let preserved = "NUWAX_BARE_DECLARATION_WITHOUT_HOST\nEXISTING=operator\n";
        let package = concat!(
            "NUWAX_BARE_DECLARATION_WITHOUT_HOST=package-default\n",
            "EXISTING=${NUWAX_UNSET_SKIPPED_DEFAULT:?must-not-be-evaluated}\n",
            "NEW=added\n",
        );
        let merged =
            merge_env_contents(preserved, package).expect("skipped defaults do not evaluate");
        assert_eq!(merged, format!("{preserved}NEW=added\n"));
        let declarations = client_core::container::preflight::compose_env_declared_keys(
            "VALUE=${NUWAX_UNSET_DISCOVERY_VALUE:?must-not-be-evaluated}\nBARE\n",
        )
        .expect("key discovery does not resolve values");
        assert!(declarations.contains("VALUE"));
        assert!(declarations.contains("BARE"));
    }

    #[test]
    fn invalid_defaults_do_not_replace_the_live_env() {
        let directory = tempfile::tempdir().expect("tempdir");
        let destination = directory.path().join(".env");
        fs::write(&destination, "EXISTING=operator\n").expect("fixture");
        assert!(merge_env_file_defaults(&destination, "NEW='unterminated\n").is_err());
        assert_eq!(
            fs::read_to_string(&destination).expect("live file"),
            "EXISTING=operator\n"
        );
        assert_eq!(
            fs::read_dir(directory.path()).expect("directory").count(),
            1
        );
    }

    #[cfg(unix)]
    #[test]
    fn direct_merge_retains_the_live_symlink_and_permissions() {
        use std::os::unix::fs::{PermissionsExt, symlink};
        let directory = tempfile::tempdir().expect("tempdir");
        let target = directory.path().join("operator.env");
        fs::write(&target, "EXISTING=operator\n").expect("fixture");
        fs::set_permissions(&target, fs::Permissions::from_mode(0o600)).expect("permissions");
        let alias = directory.path().join(".env");
        symlink("operator.env", &alias).expect("alias");
        merge_env_file_defaults(&alias, "EXISTING=package\nNEW=added\n").expect("merge");
        assert_eq!(
            fs::read_link(&alias).expect("link"),
            Path::new("operator.env")
        );
        assert_eq!(
            fs::read_to_string(&target).expect("content"),
            "EXISTING=operator\nNEW=added\n"
        );
        assert_eq!(
            fs::metadata(&target)
                .expect("metadata")
                .permissions()
                .mode()
                & 0o777,
            0o600
        );
        assert_eq!(
            fs::read_dir(directory.path()).expect("directory").count(),
            2
        );
    }
}
