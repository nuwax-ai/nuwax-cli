//! 部署预检：候选 Compose 与合并后 `.env` 的配置一致性校验。
//!
//! 供在线完整包、在线增量包、离线完整包三个入口在**停止旧服务与修改数据库之前**
//! 调用（`preflight_deploy_config`）。全部为纯函数/纯文件系统检查：
//! - 必填键（compose `${VAR:?}` 声明）：缺失或值为空白即拒绝，只报告键名；
//! - `*_DB_HOST/_DB_PORT/_DB_NAME` 约定键：schema 迁移仅支持同一本地 Compose
//!   mysql 服务，任何指向外部库/未迁移库名的覆盖提前显式拒绝；
//! - 新增宿主产物（im-app 双 jar、repo-collab-app/dist）：半更新状态在启动前拒绝。
//!
//! 所有错误信息只包含键名/库名/路径，绝不回显任何凭据值。

use crate::container::load_env_values;
use anyhow::{Result, anyhow};
use regex::Regex;
use std::collections::HashMap;
use std::path::Path;

/// 扫描 compose 文本中的必填变量引用：`${VAR:?}`、`${VAR?}`（含带提示信息形式）。
/// 带 `:-` / `-` 默认值的形式不算必填（有兜底值）。
pub fn required_env_keys(compose_text: &str) -> Vec<String> {
    let pattern = Regex::new(r"\$\{([A-Za-z_][A-Za-z0-9_]*)(:?\?)[^}]*\}")
        .expect("required env key regex must compile");
    let mut keys: Vec<String> = pattern
        .captures_iter(compose_text)
        .map(|captures| captures[1].to_string())
        .collect();
    keys.sort();
    keys.dedup();
    keys
}

/// 校验必填键：缺失或值全空白都视为未配置。返回未配置键列表（仅键名，不含值）。
pub fn missing_required_env_keys(
    compose_text: &str,
    values: &HashMap<String, String>,
) -> Vec<String> {
    required_env_keys(compose_text)
        .into_iter()
        .filter(|key| values.get(key).is_none_or(|value| value.trim().is_empty()))
        .collect()
}

/// 校验 `*_DB_HOST` / `*_DB_PORT` / `*_DB_NAME` 约定键与本地多库迁移目标一致。
///
/// 支持范围（本代 CLI 契约）：同一本地 Compose mysql 服务内的多 database。
/// - HOST 必须是服务名 `mysql`（外部 MySQL 未纳入迁移范围）；
/// - PORT 必须等于 `MYSQL_PORT`（缺省 3306，即容器端口）；
/// - NAME 必须属于 schema 模板库集合或 `.env` 声明的平台库
///   （`MYSQL_DATABASE` / `MYSQL_CUSTOM_DATABASE`）。
///
/// 任何偏离都在迁移前显式拒绝，避免"迁移一个库而应用连接另一个库"。
/// 报错包含键名与库名（非敏感信息），绝不包含任何凭据。
pub fn validate_local_db_targets(
    values: &HashMap<String, String>,
    template_databases: &[String],
) -> Result<()> {
    let mut allowed_databases: Vec<&str> = template_databases.iter().map(String::as_str).collect();
    for declared in ["MYSQL_DATABASE", "MYSQL_CUSTOM_DATABASE"] {
        if let Some(name) = values.get(declared)
            && !name.trim().is_empty()
            && !allowed_databases.contains(&name.trim())
        {
            allowed_databases.push(name.trim());
        }
    }

    for (key, raw_value) in values {
        let value = raw_value.trim();
        if value.is_empty() {
            continue;
        }
        if key.ends_with("_DB_HOST") && key != "_DB_HOST" {
            if value != "mysql" {
                return Err(anyhow!(
                    "env {key} points to host '{value}'; schema migration only supports \
                     the local Compose 'mysql' service"
                ));
            }
        } else if key.ends_with("_DB_PORT") && key != "_DB_PORT" {
            let expected = values
                .get("MYSQL_PORT")
                .map(|port| port.trim().to_string())
                .unwrap_or_else(|| "3306".to_string());
            if value != expected {
                return Err(anyhow!(
                    "env {key} uses port '{value}'; schema migration only supports the \
                     local mysql container port ({expected})"
                ));
            }
        } else if key.ends_with("_DB_NAME")
            && key != "_DB_NAME"
            && !allowed_databases.contains(&value)
        {
            return Err(anyhow!(
                "env {key} names database '{value}'; it is not covered by the local schema \
                 templates {template_databases:?} or the declared platform databases"
            ));
        }
    }
    Ok(())
}

/// 校验新增宿主产物完整性：
/// - `im-app/` 存在时，两个 bootstrap jar 必须都在且非空（同包同版，不允许半更新）；
/// - `repo-collab-app/` 存在时，`dist/` 必须存在且非空。
///
/// 目录整体不存在视为"该组件未部署"，放行（由 compose 按需启用）。
pub fn validate_host_artifacts(docker_dir: &Path) -> Result<()> {
    let im_jars = [
        "im-app/nuwax-im-web-bootstrap.jar",
        "im-app/nuwax-im-gateway-bootstrap.jar",
    ];
    if docker_dir.join("im-app").is_dir() {
        let missing: Vec<&str> = im_jars
            .iter()
            .copied()
            .filter(|jar| !is_non_empty_file(&docker_dir.join(jar)))
            .collect();
        if !missing.is_empty() {
            return Err(anyhow!(
                "incomplete im-app artifacts; missing {missing:?} \
                 (both jars must come from the same package)"
            ));
        }
    }

    let collab_dir = docker_dir.join("repo-collab-app");
    if collab_dir.is_dir() {
        let dist = collab_dir.join("dist");
        let dist_has_files = dist
            .read_dir()
            .map(|entries| entries.flatten().next().is_some())
            .unwrap_or(false);
        if !dist_has_files {
            return Err(anyhow!(
                "incomplete repo-collab-app artifacts; {} is missing or empty",
                dist.display()
            ));
        }
    }
    Ok(())
}

fn is_non_empty_file(path: &Path) -> bool {
    path.is_file()
        && std::fs::metadata(path)
            .map(|meta| meta.len() > 0)
            .unwrap_or(false)
}

/// 组合预检入口：候选 Compose + 合并后 `.env` 的一次性校验。
///
/// `template_databases` 为 schema 模板声明的库名（可为空——旧部署预检时
/// 模板文件可能尚未就位，此时库目标校验退化为仅校验 HOST/PORT 约定）。
/// 在停止旧服务、删除文件或修改数据库之前调用；任何失败都是部署前置错误。
pub fn preflight_deploy_config(
    compose_path: &Path,
    env_path: &Path,
    template_databases: &[String],
) -> Result<()> {
    let compose_text = std::fs::read_to_string(compose_path).map_err(|error| {
        anyhow!(
            "preflight cannot read compose file {}: {error}",
            compose_path.display()
        )
    })?;
    let values = load_env_values(env_path).map_err(|error| {
        anyhow!(
            "preflight cannot read env file {}: {error}",
            env_path.display()
        )
    })?;

    let missing = missing_required_env_keys(&compose_text, &values);
    if !missing.is_empty() {
        return Err(anyhow!(
            "required environment keys are missing or empty: {missing:?}; \
             define them in {} before deploying",
            env_path.display()
        ));
    }

    validate_local_db_targets(&values, template_databases)?;
    validate_host_artifacts(env_path.parent().unwrap_or(Path::new(".")))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn env_map(pairs: &[(&str, &str)]) -> HashMap<String, String> {
        pairs
            .iter()
            .map(|(key, value)| (key.to_string(), value.to_string()))
            .collect()
    }

    #[test]
    fn required_keys_detects_only_mandatory_references() {
        let compose = concat!(
            "services:\n",
            "  a:\n",
            "    environment:\n",
            "      - A=${MUST_SET:?}\n",
            "      - B=${WITH_MSG:?message here}\n",
            "      - C=${PLAIN?}\n",
            "      - D=${HAS_DEFAULT:-fallback}\n",
            "      - E=${PLAIN_REF}\n",
        );
        assert_eq!(
            required_env_keys(compose),
            vec![
                "MUST_SET".to_string(),
                "PLAIN".to_string(),
                "WITH_MSG".to_string()
            ]
        );
    }

    #[test]
    fn missing_required_keys_rejects_absent_and_blank_values() {
        let compose = "x: ${A:?}\ny: ${B:?}\n";
        let values = env_map(&[("A", "ok"), ("B", "   ")]);
        assert_eq!(
            missing_required_env_keys(compose, &values),
            vec!["B".to_string()]
        );

        let empty: HashMap<String, String> = HashMap::new();
        assert_eq!(
            missing_required_env_keys(compose, &empty),
            vec!["A".to_string(), "B".to_string()]
        );
    }

    #[test]
    fn db_targets_accept_local_mysql_and_template_databases() {
        let values = env_map(&[
            ("MYSQL_PORT", "3306"),
            ("MYSQL_DATABASE", "agent_platform"),
            ("IM_DB_HOST", "mysql"),
            ("IM_DB_PORT", "3306"),
            ("IM_DB_NAME", "nuwax_im"),
        ]);
        let templates = vec!["nuwax_im".to_string()];
        validate_local_db_targets(&values, &templates).expect("local targets must pass");
    }

    #[test]
    fn db_targets_reject_external_host_and_unknown_database() {
        let templates = vec!["nuwax_im".to_string(), "agent_platform".to_string()];

        let external = env_map(&[("IM_DB_HOST", "db.example.com"), ("IM_DB_NAME", "nuwax_im")]);
        let error = validate_local_db_targets(&external, &templates).unwrap_err();
        assert!(error.to_string().contains("IM_DB_HOST"));
        assert!(error.to_string().contains("db.example.com"));

        let wrong_port = env_map(&[("MYSQL_PORT", "3306"), ("IM_DB_PORT", "13306")]);
        assert!(validate_local_db_targets(&wrong_port, &templates).is_err());

        let unknown_db = env_map(&[
            ("MYSQL_DATABASE", "agent_platform"),
            ("IM_DB_HOST", "mysql"),
            ("IM_DB_NAME", "someone_else_db"),
        ]);
        let error = validate_local_db_targets(&unknown_db, &templates).unwrap_err();
        assert!(error.to_string().contains("IM_DB_NAME"));
        assert!(error.to_string().contains("someone_else_db"));
    }

    #[test]
    fn host_artifacts_reject_half_updated_im_jars() {
        let directory = tempfile::tempdir().expect("tempdir");
        let docker_dir = directory.path();

        // 未部署组件：放行
        validate_host_artifacts(docker_dir).expect("absent artifacts must pass");

        // 只有一个 jar：拒绝（半更新）
        std::fs::create_dir_all(docker_dir.join("im-app")).expect("mkdir");
        std::fs::write(docker_dir.join("im-app/nuwax-im-web-bootstrap.jar"), b"jar").expect("jar");
        let error = validate_host_artifacts(docker_dir).unwrap_err();
        assert!(error.to_string().contains("nuwax-im-gateway-bootstrap.jar"));

        // 双 jar 齐且非空：通过
        std::fs::write(
            docker_dir.join("im-app/nuwax-im-gateway-bootstrap.jar"),
            b"jar",
        )
        .expect("jar");
        validate_host_artifacts(docker_dir).expect("complete im-app must pass");

        // repo-collab-app 存在但 dist 缺失：拒绝
        std::fs::create_dir_all(docker_dir.join("repo-collab-app")).expect("mkdir");
        assert!(validate_host_artifacts(docker_dir).is_err());

        std::fs::create_dir_all(docker_dir.join("repo-collab-app/dist")).expect("mkdir");
        std::fs::write(docker_dir.join("repo-collab-app/dist/index.js"), b"").expect("asset");
        validate_host_artifacts(docker_dir).expect("complete dist must pass");
    }
}
