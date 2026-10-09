//! 部署预检：候选 Compose 与合并后 `.env` 的配置一致性校验。
//!
//! 供在线完整包、在线增量包、离线完整包三个入口在**停止旧服务与修改数据库之前**
//! 调用（`preflight_deploy_config`）。全部为纯函数/纯文件系统检查：
//! - 必填键：按 Compose `?` / `:?` 和最终环境语义检查，只报告键名；
//! - `*_DB_HOST/_DB_PORT/_DB_NAME` 约定键：schema 迁移仅支持同一本地 Compose
//!   mysql 服务，任何指向外部库/未迁移库名的覆盖提前显式拒绝；
//! - 新增宿主产物（im-app 双 jar、repo-collab-app/dist）：半更新状态在启动前拒绝。
//!
//! 所有错误信息只包含键名/库名/路径，绝不回显任何凭据值。

use crate::container::interpolate_env;
use crate::container::load_env_values;
use crate::mysql_manifest::SchemaManifest;
use anyhow::Context as _;
use anyhow::{Result, anyhow, bail};
use sha2::Digest;
use sha2::Sha256;
use std::collections::{HashMap, HashSet};
use std::io::Read;
use std::path::{Component, Path, PathBuf};
use tracing::info;

/// Return required references evaluated when no variables have been supplied.
/// The shared Compose interpreter handles escaped dollars and selected words.
pub fn required_env_keys(compose_text: &str) -> Result<Vec<String>> {
    missing_required_env_keys_with_lookup(compose_text, &HashMap::new(), |_| None)
}

/// Check required references using the same shell-before-env-file precedence as
/// native Compose. `${VAR?}` accepts an explicitly empty value; `${VAR:?}` does not.
/// Errors and returned keys never include credential values or custom messages.
pub fn missing_required_env_keys(
    compose_text: &str,
    values: &HashMap<String, String>,
) -> Result<Vec<String>> {
    missing_required_env_keys_with_lookup(compose_text, values, |key| std::env::var(key).ok())
}

fn missing_required_env_keys_with_lookup(
    compose_text: &str,
    values: &HashMap<String, String>,
    host_value: impl Fn(&str) -> Option<String>,
) -> Result<Vec<String>> {
    let document: serde_yaml::Value = serde_yaml::from_str(compose_text)
        .map_err(|_| anyhow!("preflight cannot parse candidate Compose YAML"))?;
    let lookup = |key: &str| {
        if crate::constants::device_info::ENV_MANAGED_KEYS.contains(&key) {
            values.get(key).cloned()
        } else {
            host_value(key).or_else(|| values.get(key).cloned())
        }
    };
    let mut keys = Vec::new();
    collect_missing_required_values(&document, &lookup, &mut keys)?;
    keys.sort();
    keys.dedup();
    Ok(keys)
}

fn collect_missing_required_values(
    value: &serde_yaml::Value,
    lookup: &impl Fn(&str) -> Option<String>,
    keys: &mut Vec<String>,
) -> Result<()> {
    match value {
        serde_yaml::Value::String(raw) => {
            keys.extend(
                crate::container::interpolation::missing_required_variables(raw, lookup)
                    .map_err(|_| anyhow!("preflight cannot parse Compose variable expression"))?,
            );
        }
        serde_yaml::Value::Sequence(entries) => {
            for entry in entries {
                collect_missing_required_values(entry, lookup, keys)?;
            }
        }
        serde_yaml::Value::Mapping(entries) => {
            // Compose interpolates mapping values, never mapping keys.
            for entry in entries.values() {
                collect_missing_required_values(entry, lookup, keys)?;
            }
        }
        serde_yaml::Value::Tagged(tagged) => {
            collect_missing_required_values(&tagged.value, lookup, keys)?;
        }
        _ => {}
    }
    Ok(())
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
/// - `repo-collab-app/` 存在时，`dist/index.js` 必须是非空普通文件。
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
        let entrypoint = collab_dir.join("dist/index.js");
        if !is_non_empty_file(&entrypoint) {
            return Err(anyhow!(
                "incomplete repo-collab-app artifacts; {} is not a non-empty regular file",
                entrypoint.display()
            ));
        }
    }
    Ok(())
}

fn is_non_empty_file(path: &Path) -> bool {
    std::fs::symlink_metadata(path).is_ok_and(|meta| meta.is_file() && meta.len() > 0)
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
    manifest: Option<&SchemaManifest>,
    delivery: Option<&DeliveryManifest>,
) -> Result<()> {
    let package_root = compose_path.parent().unwrap_or(Path::new("."));
    preflight_deploy_config_at(
        compose_path,
        env_path,
        package_root,
        template_databases,
        manifest,
        delivery,
    )
}

/// Explicit package context: the env file may live outside the package root.
pub fn preflight_deploy_config_at(
    compose_path: &Path,
    env_path: &Path,
    package_root: &Path,
    template_databases: &[String],
    manifest: Option<&SchemaManifest>,
    delivery: Option<&DeliveryManifest>,
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

    let missing = missing_required_env_keys(&compose_text, &values)?;
    if !missing.is_empty() {
        return Err(anyhow!(
            "required environment keys are missing or empty: {missing:?}; \
             define them in {} before deploying",
            env_path.display()
        ));
    }

    match manifest {
        // manifest 路径：逐条 (service, role) 校验候选 Compose 最终连接值
        Some(manifest) => validate_application_connections(&compose_text, &values, manifest)?,
        // legacy 路径：约定键扫描
        None => validate_local_db_targets(&values, template_databases)?,
    }

    validate_component_artifacts(package_root, &compose_text, delivery)?;
    Ok(())
}

/// 归档侧的交付清单校验（停服务前预检用）：对读取到的条目字节做
/// SHA256 一致性检查（compose / mysql 清单文件 / 组件产物），并复算 release_sha256。
/// 与磁盘版 `verify_delivery_manifest` 同语义，数据源为归档条目。
pub fn verify_delivery_against_entries(
    manifest: &DeliveryManifest,
    entries: &HashMap<String, Vec<u8>>,
    local_architecture: Option<&str>,
) -> Result<()> {
    if let Some(architecture) = local_architecture
        && !architecture.is_empty()
        && manifest.architecture != architecture
    {
        bail!(
            "DELIVERY_MANIFEST architecture '{}' does not match this host ('{architecture}')",
            manifest.architecture
        );
    }
    let compose = entries
        .get(manifest.compose.path.as_str())
        .ok_or_else(|| anyhow!("archive is missing compose {}", manifest.compose.path))?;
    if sha256_hex(compose) != manifest.compose.sha256 {
        bail!(
            "delivery compose {} hash mismatch against archive bytes",
            manifest.compose.path
        );
    }

    for (path, expected) in &manifest.mysql.files {
        let bytes = entries
            .get(path.as_str())
            .ok_or_else(|| anyhow!("archive is missing mysql file {path}"))?;
        if sha256_hex(bytes) != *expected {
            bail!("mysql file {path} hash mismatch against archive bytes");
        }
    }

    for component in manifest.components.values() {
        for (path, expected) in &component.artifacts {
            let bytes = entries
                .get(path.as_str())
                .ok_or_else(|| anyhow!("archive is missing component artifact {path}"))?;
            if sha256_hex(bytes) != *expected {
                bail!("component artifact {path} hash mismatch against archive bytes");
            }
        }
    }

    verify_release_identity(manifest)
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
            required_env_keys(compose).expect("valid Compose"),
            vec![
                "MUST_SET".to_string(),
                "PLAIN".to_string(),
                "WITH_MSG".to_string()
            ]
        );
    }

    #[test]
    fn missing_required_keys_rejects_absent_and_empty_values() {
        let compose = "x: ${A:?}\ny: ${B:?}\n";
        let values = env_map(&[("A", "ok"), ("B", "")]);
        assert_eq!(
            missing_required_env_keys_with_lookup(compose, &values, |_| None)
                .expect("valid Compose"),
            vec!["B".to_string()]
        );

        let empty: HashMap<String, String> = HashMap::new();
        assert_eq!(
            missing_required_env_keys_with_lookup(compose, &empty, |_| None)
                .expect("valid Compose"),
            vec!["A".to_string(), "B".to_string()]
        );
    }

    #[test]
    fn required_values_use_effective_shell_precedence_without_empty_fallback() {
        let compose = "value: ${NUWAX_REVIEW_REQUIRED:?synthetic-private-message}\n";
        let file_values = env_map(&[("NUWAX_REVIEW_REQUIRED", "file-value")]);
        assert_eq!(
            missing_required_env_keys_with_lookup(compose, &file_values, |_| Some(String::new()))
                .expect("valid Compose"),
            vec!["NUWAX_REVIEW_REQUIRED"]
        );
        assert!(
            missing_required_env_keys_with_lookup(compose, &file_values, |_| None)
                .expect("valid Compose")
                .is_empty()
        );
        assert!(
            missing_required_env_keys_with_lookup(compose, &HashMap::new(), |_| {
                Some("shell-only".to_string())
            })
            .expect("valid Compose")
            .is_empty()
        );
    }

    #[test]
    fn required_values_preserve_question_colon_and_dollar_escape_semantics() {
        let values = env_map(&[("EMPTY", ""), ("WHITESPACE", "   ")]);
        let compose = concat!(
            "plain: ${EMPTY?}\n",
            "colon: ${EMPTY:?}\n",
            "spaces: ${WHITESPACE:?}\n",
            "escaped: '$${LITERAL:?} $$$${LITERAL_TOO:?}'\n",
            "real_after_escape: '$$${REAL:?}'\n",
            "message: '${MISSING:?${MESSAGE_ONLY:?synthetic-secret}}'\n",
            "'${MAPPING_KEY:?}': ordinary\n",
        );
        assert_eq!(
            missing_required_env_keys_with_lookup(compose, &values, |_| None)
                .expect("valid Compose"),
            vec!["EMPTY", "MISSING", "REAL"]
        );
    }

    #[test]
    fn required_values_inspect_only_selected_default_and_alternate_words() {
        let compose = concat!(
            "default: '${SET:-${INACTIVE_DEFAULT:?}}'\n",
            "alternate: '${UNSET:+${INACTIVE_ALTERNATE:?}}'\n",
            "needed: '${UNSET:-${SELECTED:?}}'\n",
        );
        let values = env_map(&[("SET", "present")]);
        assert_eq!(
            missing_required_env_keys_with_lookup(compose, &values, |_| None)
                .expect("valid Compose"),
            vec!["SELECTED"]
        );
        for invalid in ["x: '${BROKEN:operator}'", "x: 'unterminated"] {
            let error = missing_required_env_keys_with_lookup(invalid, &values, |_| None)
                .expect_err("invalid Compose must fail before stopping services");
            assert!(!error.to_string().contains("operator"));
        }
    }

    #[test]
    fn initdb_mounts_accept_equivalent_short_and_long_bind_syntax() {
        let manifest =
            crate::mysql_manifest::parse_schema_manifest(MANIFEST_JSON).expect("manifest");
        let pairs = [
            (&manifest.bootstrap.path, &manifest.bootstrap.initdb_target),
            (
                &manifest.permissions.path,
                &manifest.permissions.initdb_target,
            ),
            (
                &manifest.schemas[0].path,
                &manifest.schemas[0].initdb_target,
            ),
            (
                &manifest.schemas[1].path,
                &manifest.schemas[1].initdb_target,
            ),
        ];
        let mut short = String::from("services:\n  mysql:\n    volumes:\n");
        let mut long = short.clone();
        for (source, target) in pairs {
            short.push_str(&format!(
                "      - ./{source}:/docker-entrypoint-initdb.d/{target}:ro\n"
            ));
            long.push_str(&format!(
                "      - type: bind\n        source: ./{source}\n        target: /docker-entrypoint-initdb.d/{target}\n        read_only: true\n"
            ));
        }
        validate_initdb_mount_contract(&short, &manifest).expect("short binds must pass");
        validate_initdb_mount_contract(&long, &manifest).expect("long binds must pass");
        assert_eq!(
            collect_bind_mount_sources(&short).expect("short"),
            collect_bind_mount_sources(&long).expect("long")
        );

        let wrong_source = short.replace("./config/bootstrap.sql", "./docker/config/bootstrap.sql");
        assert!(validate_initdb_mount_contract(&wrong_source, &manifest).is_err());
        let named = long.replace("type: bind", "type: volume");
        assert!(validate_initdb_mount_contract(&named, &manifest).is_err());
    }

    #[test]
    fn bind_mount_parser_preserves_windows_drive_letter_colons() {
        let volumes = serde_yaml::from_str::<serde_yaml::Value>(
            "- 'C:\\deployment\\config\\bootstrap.sql:/docker-entrypoint-initdb.d/00_bootstrap.sql:ro'\n",
        ).expect("YAML");
        assert_eq!(
            bind_mount_pairs(&volumes),
            vec![(
                "C:\\deployment\\config\\bootstrap.sql".to_string(),
                "/docker-entrypoint-initdb.d/00_bootstrap.sql".to_string(),
            )]
        );
    }

    #[test]
    fn package_root_is_independent_of_external_env_location() {
        let directory = tempfile::tempdir().expect("tempdir");
        let package = directory.path().join("package");
        let external = directory.path().join("settings");
        std::fs::create_dir_all(package.join("im-app")).expect("mkdir");
        std::fs::create_dir_all(&external).expect("mkdir");
        for name in [
            "nuwax-im-web-bootstrap.jar",
            "nuwax-im-gateway-bootstrap.jar",
        ] {
            std::fs::write(package.join("im-app").join(name), b"jar").expect("jar");
        }
        let compose = package.join("docker-compose.yml");
        let env = external.join("deploy.env");
        std::fs::write(&compose,
            "services:\n  im:\n    volumes:\n      - ./im-app/nuwax-im-web-bootstrap.jar:/app/web.jar\n"
        ).expect("compose");
        std::fs::write(&env, "").expect("env");
        preflight_deploy_config_at(&compose, &env, &package, &[], None, None)
            .expect("component checks must resolve under package, not env parent");
        std::fs::remove_file(package.join("im-app/nuwax-im-gateway-bootstrap.jar"))
            .expect("remove");
        assert!(preflight_deploy_config_at(&compose, &env, &package, &[], None, None).is_err());
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
        assert!(validate_host_artifacts(docker_dir).is_err());
        std::fs::write(
            docker_dir.join("repo-collab-app/dist/index.js"),
            b"entrypoint",
        )
        .expect("asset");
        validate_host_artifacts(docker_dir).expect("complete dist must pass");
    }

    const MANIFEST_JSON: &str = r#"{
      "contract_version": 1,
      "requires": { "cli_capability": "mysql-schema-manifest-v1" },
      "mysql_target": { "service": "mysql", "internal_port": 3306 },
      "application_connections": [
        { "service": "nuwax-im-business", "role": "primary", "database": "nuwax_im",
          "host": "mysql", "port": 3306,
          "environment": { "host": "IM_DB_HOST", "port": "IM_DB_PORT", "database": "IM_DB_NAME" } }
      ],
      "databases": [
        { "name": "agent_platform", "bootstrap_only": false },
        { "name": "nuwax_im", "bootstrap_only": false }
      ],
      "bootstrap": { "path": "config/bootstrap.sql", "idempotent": true, "initdb_target": "00_bootstrap.sql" },
      "permissions": { "path": "config/permissions.sh", "user_env": "MYSQL_USER", "databases": "bootstrap", "initdb_target": "01_permissions.sh" },
      "schemas": [
        { "database": "agent_platform", "path": "config/platform.sql", "initdb_target": "10_platform.sql" },
        { "database": "nuwax_im", "path": "config/im.sql", "initdb_target": "20_im.sql" }
      ],
      "first_install_seeds": []
    }"#;

    const COMPOSE_YAML: &str = "services:\n  nuwax-im-business:\n    environment:\n      - IM_DB_HOST=${IM_DB_HOST}\n      - IM_DB_PORT=${IM_DB_PORT}\n      - IM_DB_NAME=${IM_DB_NAME}\n";

    #[test]
    fn connections_match_migration_targets_pass() {
        let manifest =
            crate::mysql_manifest::parse_schema_manifest(MANIFEST_JSON).expect("manifest");
        let values = env_map(&[
            ("IM_DB_HOST", "mysql"),
            ("IM_DB_PORT", "3306"),
            ("IM_DB_NAME", "nuwax_im"),
        ]);
        validate_application_connections(COMPOSE_YAML, &values, &manifest)
            .expect("correct mapping must pass");
    }

    #[test]
    fn connections_reject_cross_database_and_missing_values() {
        let manifest =
            crate::mysql_manifest::parse_schema_manifest(MANIFEST_JSON).expect("manifest");

        // IM 连到 agent_platform：库名在 manifest 声明中，但不是该连接的目标 → 必须拒绝
        let wrong_db = env_map(&[
            ("IM_DB_HOST", "mysql"),
            ("IM_DB_PORT", "3306"),
            ("IM_DB_NAME", "agent_platform"),
        ]);
        let error =
            validate_application_connections(COMPOSE_YAML, &wrong_db, &manifest).unwrap_err();
        assert!(error.to_string().contains("agent_platform"), "{error}");

        // 外部主机
        let external = env_map(&[
            ("IM_DB_HOST", "db.example.com"),
            ("IM_DB_PORT", "3306"),
            ("IM_DB_NAME", "nuwax_im"),
        ]);
        assert!(validate_application_connections(COMPOSE_YAML, &external, &manifest).is_err());

        // selector 值缺失（键不在服务 environment，也不在 env 中）
        let empty: std::collections::HashMap<String, String> = std::collections::HashMap::new();
        assert!(validate_application_connections(COMPOSE_YAML, &empty, &manifest).is_err());

        // 非内部端口
        let wrong_port = env_map(&[
            ("IM_DB_HOST", "mysql"),
            ("IM_DB_PORT", "13306"),
            ("IM_DB_NAME", "nuwax_im"),
        ]);
        assert!(validate_application_connections(COMPOSE_YAML, &wrong_port, &manifest).is_err());
    }

    #[test]
    fn service_env_resolution_follows_shell_over_env_file_precedence() {
        // compose 条目 IM_DB_NAME=${NUWAX_CONN_SELECTOR_XYZ}：env-file 与 shell 给出不同值时，
        // 最终值必须按 shell > env-file 取（与 docker compose config 语义一致）
        let compose: serde_yaml::Value = serde_yaml::from_str(
            "services:\n  im:\n    environment:\n      - IM_DB_NAME=${NUWAX_CONN_SELECTOR_XYZ}\n",
        )
        .expect("yaml");
        let service = &compose["services"]["im"];
        let env_file = env_map(&[("NUWAX_CONN_SELECTOR_XYZ", "agent_platform")]);

        // 无 shell 值：取 env-file（错误目标）
        let resolved = resolve_service_env_with(service, "IM_DB_NAME", &env_file, |_: &str| None)
            .expect("env-file value");
        assert_eq!(resolved, "agent_platform");

        // shell 给出正确目标：优先于 env-file
        let resolved = resolve_service_env_with(service, "IM_DB_NAME", &env_file, |name: &str| {
            (name == "NUWAX_CONN_SELECTOR_XYZ").then(|| "nuwax_im".to_string())
        })
        .expect("shell value");
        assert_eq!(resolved, "nuwax_im");
    }

    #[test]
    fn backup_member_keys_remain_posix_on_every_host() {
        for (input, expected) in [
            (
                "im-app/nuwax-im-gateway-bootstrap.jar",
                "im-app/nuwax-im-gateway-bootstrap.jar",
            ),
            (
                "./repo-collab-app/dist/index.js",
                "repo-collab-app/dist/index.js",
            ),
            (
                "repo-collab-app//dist/./index.js",
                "repo-collab-app/dist/index.js",
            ),
        ] {
            let actual = backup_member_path(Path::new(input)).expect("safe archive member");
            assert_eq!(actual, expected);
            assert!(
                !actual.contains('\\'),
                "receipt keys must be portable POSIX strings"
            );
        }
        for unsafe_path in [
            "../escape",
            "dir/../escape",
            "/absolute",
            "C:/absolute",
            "./C:/absolute",
            "C:relative",
            "dir\\file",
        ] {
            assert!(
                backup_member_path(Path::new(unsafe_path)).is_err(),
                "{unsafe_path}"
            );
        }
    }

    #[test]
    fn backup_parent_lookup_uses_canonical_posix_keys() {
        let file = "repo-collab-app/dist/index.js";
        let mut entries = HashMap::from([
            (
                file.to_string(),
                BackupArchiveEntry {
                    regular: true,
                    directory: false,
                    size: 1,
                },
            ),
            (
                "repo-collab-app".to_string(),
                BackupArchiveEntry {
                    regular: false,
                    directory: false,
                    size: 0,
                },
            ),
        ]);
        assert!(
            ensure_backup_regular_file(&entries, file).is_err(),
            "unsafe parent must be found using the receipt's slash-separated key"
        );
        entries.insert(
            "repo-collab-app".to_string(),
            BackupArchiveEntry {
                regular: false,
                directory: true,
                size: 0,
            },
        );
        ensure_backup_regular_file(&entries, file).expect("ordinary parent directory");
        entries.insert(
            "repo-collab-app/dist".to_string(),
            BackupArchiveEntry {
                regular: false,
                directory: false,
                size: 0,
            },
        );
        assert!(ensure_backup_regular_file(&entries, file).is_err());
    }

    #[test]
    fn backup_guard_rejects_component_free_and_half_backups() {
        let directory = tempfile::tempdir().expect("tempdir");
        let docker_dir = directory.path().join("docker");
        std::fs::create_dir_all(&docker_dir).expect("mkdir");
        // compose 同时挂载两个 jar（文件挂载）与 dist（目录挂载）
        std::fs::write(
            docker_dir.join("docker-compose.yml"),
            "services:\n  im:\n    volumes:\n      - ./im-app/nuwax-im-web-bootstrap.jar:/app/web.jar\n      - ./im-app/nuwax-im-gateway-bootstrap.jar:/app/gw.jar\n  collab:\n    volumes:\n      - ./repo-collab-app/dist:/app/dist\n",
        )
        .expect("compose");

        // 旧备份（无组件）：拒绝
        let old_backup = directory.path().join("old.tar.gz");
        write_tar_gz(&old_backup, &[("data/mysql/ibdata1", b"db")]);
        assert!(verify_backup_component_compatibility(&old_backup, &docker_dir).is_err());

        // 半包：只有 web jar（gateway 缺失）：拒绝（文件级校验，F05）
        let half_backup = directory.path().join("half.tar.gz");
        write_tar_gz(
            &half_backup,
            &[
                ("im-app/nuwax-im-web-bootstrap.jar", b"jar"),
                ("repo-collab-app/dist/index.js", b"entry"),
            ],
        );
        let error = verify_backup_component_compatibility(&half_backup, &docker_dir).unwrap_err();
        assert!(error.to_string().contains("gateway"), "{error}");

        // 空目录（dist 无非空文件）：拒绝
        let empty_backup = directory.path().join("empty.tar.gz");
        write_tar_gz(
            &empty_backup,
            &[
                ("im-app/nuwax-im-web-bootstrap.jar", b"jar"),
                ("im-app/nuwax-im-gateway-bootstrap.jar", b"jar"),
                ("repo-collab-app/dist/", b""),
            ],
        );
        assert!(verify_backup_component_compatibility(&empty_backup, &docker_dir).is_err());

        // A README cannot substitute for the actual Node entrypoint.
        let readme_backup = directory.path().join("readme-only.tar.gz");
        write_tar_gz(
            &readme_backup,
            &[
                ("im-app/nuwax-im-web-bootstrap.jar", b"jar"),
                ("im-app/nuwax-im-gateway-bootstrap.jar", b"jar"),
                ("repo-collab-app/dist/README.md", b"documentation"),
            ],
        );
        let error = verify_backup_component_compatibility(&readme_backup, &docker_dir)
            .expect_err("dist/index.js is mandatory");
        assert!(error.to_string().contains("dist/index.js"), "{error}");

        let empty_entrypoint = directory.path().join("empty-index.tar.gz");
        write_tar_gz(
            &empty_entrypoint,
            &[
                ("im-app/nuwax-im-web-bootstrap.jar", b"jar"),
                ("im-app/nuwax-im-gateway-bootstrap.jar", b"jar"),
                ("repo-collab-app/dist/index.js", b""),
            ],
        );
        assert!(verify_backup_component_compatibility(&empty_entrypoint, &docker_dir).is_err());

        // 完整备份（files-only 形式：dist 只有 index.js 文件条目）：通过
        let full_backup = directory.path().join("full.tar.gz");
        write_tar_gz(
            &full_backup,
            &[
                ("im-app/nuwax-im-web-bootstrap.jar", b"jar"),
                ("im-app/nuwax-im-gateway-bootstrap.jar", b"jar"),
                ("repo-collab-app/dist/index.js", b"entry"),
                ("data/mysql/ibdata1", b"db"),
            ],
        );
        verify_backup_component_compatibility(&full_backup, &docker_dir)
            .expect("complete files-only backup must pass");
    }

    fn write_tar_gz(path: &std::path::Path, entries: &[(&str, &[u8])]) {
        let encoder = flate2::write::GzEncoder::new(
            std::fs::File::create(path).expect("create"),
            flate2::Compression::default(),
        );
        let mut archive = tar::Builder::new(encoder);
        for (name, bytes) in entries {
            let mut header = tar::Header::new_gnu();
            header.set_size(bytes.len() as u64);
            header.set_mode(0o644);
            header.set_cksum();
            archive
                .append_data(&mut header, name, *bytes)
                .expect("append");
        }
        archive.finish().expect("finish");
        archive
            .into_inner()
            .expect("inner")
            .finish()
            .expect("flush");
    }

    fn write_delivery_workspace(
        directory: &std::path::Path,
    ) -> (std::path::PathBuf, DeliveryManifest) {
        let docker_dir = directory.join("docker");
        std::fs::create_dir_all(docker_dir.join("im-app")).expect("mkdir");
        std::fs::create_dir_all(docker_dir.join("config")).expect("mkdir config");
        std::fs::write(docker_dir.join("docker-compose.yml"), b"services: {}\n").expect("compose");
        std::fs::write(docker_dir.join("config/mysql-schema-manifest.json"), b"{}")
            .expect("manifest");
        std::fs::write(
            docker_dir.join("im-app/nuwax-im-web-bootstrap.jar"),
            b"jar-web",
        )
        .expect("jar");
        std::fs::write(
            docker_dir.join("im-app/nuwax-im-gateway-bootstrap.jar"),
            b"jar-gw",
        )
        .expect("jar");

        let hash_of = |path: &std::path::Path| sha256_hex(&std::fs::read(path).expect("read"));
        let mut components = std::collections::HashMap::new();
        components.insert(
            "nuwax-im".to_string(),
            DeliveryComponent {
                version: serde_json::json!("1.0.0"),
                image_id: "sha256:0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef"
                    .to_string(),
                config_id:
                    "sha256:fedcba9876543210fedcba9876543210fedcba9876543210fedcba9876543210"
                        .to_string(),
                source: "registry.test/nuwax/im:1.0.0".to_string(),
                target: "registry.test/nuwax/im:1.0.0".to_string(),
                artifacts: {
                    let mut artifacts = std::collections::HashMap::new();
                    artifacts.insert(
                        "im-app/nuwax-im-web-bootstrap.jar".to_string(),
                        hash_of(&docker_dir.join("im-app/nuwax-im-web-bootstrap.jar")),
                    );
                    artifacts.insert(
                        "im-app/nuwax-im-gateway-bootstrap.jar".to_string(),
                        hash_of(&docker_dir.join("im-app/nuwax-im-gateway-bootstrap.jar")),
                    );
                    artifacts
                },
            },
        );
        let manifest = DeliveryManifest {
            contract_version: 1,
            architecture: "amd64".to_string(),
            components,
            mysql: DeliveryMysql {
                manifest: "config/mysql-schema-manifest.json".to_string(),
                files: {
                    let mut files = std::collections::HashMap::new();
                    files.insert(
                        "config/mysql-schema-manifest.json".to_string(),
                        hash_of(&docker_dir.join("config/mysql-schema-manifest.json")),
                    );
                    files
                },
            },
            compose: DeliveryCompose {
                path: "docker-compose.yml".to_string(),
                sha256: hash_of(&docker_dir.join("docker-compose.yml")),
            },
            release_sha256: String::new(),
        };
        let payload = serde_json::to_value(DeliveryPayload {
            contract_version: manifest.contract_version,
            architecture: &manifest.architecture,
            components: &manifest.components,
            mysql: &manifest.mysql,
            compose: &manifest.compose,
        })
        .expect("serialize");
        let mut manifest = manifest;
        manifest.release_sha256 = sha256_hex(canonical_json_string(&payload).as_bytes());
        (docker_dir, manifest)
    }

    fn write_delivery_backup(
        path: &Path,
        package: &Path,
        receipt: &DeliveryManifest,
        omitted: Option<&str>,
        tampered: Option<&str>,
    ) {
        let paths = [
            "docker-compose.yml",
            "config/mysql-schema-manifest.json",
            "im-app/nuwax-im-web-bootstrap.jar",
            "im-app/nuwax-im-gateway-bootstrap.jar",
        ];
        let mut content = Vec::new();
        for name in paths {
            if omitted == Some(name) {
                continue;
            }
            let mut bytes = std::fs::read(package.join(name)).expect("fixture file");
            if tampered == Some(name) {
                bytes.extend_from_slice(b"-mixed-version");
            }
            content.push((name.to_string(), bytes));
        }
        content.push((
            "DELIVERY_MANIFEST.json".to_string(),
            serde_json::to_vec(receipt).expect("receipt"),
        ));
        let entries: Vec<_> = content
            .iter()
            .map(|(name, bytes)| (name.as_str(), bytes.as_slice()))
            .collect();
        write_tar_gz(path, &entries);
    }

    #[test]
    fn backup_receipt_verifies_complete_file_identity_before_restore() {
        let directory = tempfile::tempdir().expect("tempdir");
        let (package, receipt) = write_delivery_workspace(directory.path());
        let valid = directory.path().join("complete.tar.gz");
        write_delivery_backup(&valid, &package, &receipt, None, None);
        verify_backup_component_compatibility(&valid, &package)
            .expect("matching receipt and files");

        for (name, omitted, tampered) in [
            (
                "missing",
                Some("im-app/nuwax-im-gateway-bootstrap.jar"),
                None,
            ),
            ("mixed", None, Some("im-app/nuwax-im-gateway-bootstrap.jar")),
            ("compose", None, Some("docker-compose.yml")),
            ("schema", None, Some("config/mysql-schema-manifest.json")),
        ] {
            let path = directory.path().join(format!("{name}.tar.gz"));
            write_delivery_backup(&path, &package, &receipt, omitted, tampered);
            assert!(
                verify_backup_component_compatibility(&path, &package).is_err(),
                "{name} must fail before cleanup"
            );
        }
        let mut corrupt_receipt = receipt.clone();
        corrupt_receipt.release_sha256 = "0".repeat(64);
        let corrupt = directory.path().join("corrupt-receipt.tar.gz");
        write_delivery_backup(&corrupt, &package, &corrupt_receipt, None, None);
        assert!(verify_backup_component_compatibility(&corrupt, &package).is_err());
    }

    #[test]
    fn backup_rejects_symlink_entrypoint_and_duplicate_receipt_files() {
        let directory = tempfile::tempdir().expect("tempdir");
        let package = directory.path().join("package");
        std::fs::create_dir_all(&package).expect("mkdir");
        std::fs::write(
            package.join("docker-compose.yml"),
            "services:\n  collab:\n    volumes:\n      - ./repo-collab-app/dist:/app/dist\n",
        )
        .expect("compose");
        let linked = directory.path().join("linked.tar.gz");
        let encoder = flate2::write::GzEncoder::new(
            std::fs::File::create(&linked).expect("create"),
            flate2::Compression::default(),
        );
        let mut archive = tar::Builder::new(encoder);
        let mut header = tar::Header::new_gnu();
        header.set_entry_type(tar::EntryType::Symlink);
        header.set_link_name("README.md").expect("link name");
        header.set_mode(0o777);
        header.set_size(0);
        header.set_cksum();
        archive
            .append_data(&mut header, "repo-collab-app/dist/index.js", &[][..])
            .expect("append symlink");
        let mut readme = tar::Header::new_gnu();
        readme.set_mode(0o644);
        readme.set_size(6);
        readme.set_cksum();
        archive
            .append_data(
                &mut readme,
                "repo-collab-app/dist/README.md",
                &b"readme"[..],
            )
            .expect("append README");
        archive
            .into_inner()
            .expect("archive")
            .finish()
            .expect("finish");
        assert!(verify_backup_component_compatibility(&linked, &package).is_err());

        let duplicate = directory.path().join("duplicate.tar.gz");
        write_tar_gz(
            &duplicate,
            &[
                ("repo-collab-app/dist/index.js", b"expected-entry"),
                ("repo-collab-app/dist/index.js", b"overwritten-entry"),
            ],
        );
        let error = verify_backup_component_compatibility(&duplicate, &package)
            .expect_err("archive overwrite must fail before restoring");
        assert!(error.to_string().contains("duplicate file"), "{error}");
    }

    /// F01 回归：构建方（component_delivery.py 真实输出，含 config_id）→ CLI 解析 →
    /// canonical release_sha256 复算必须与 producer 记录一致（不忽略未知字段）
    #[test]
    fn delivery_manifest_roundtrips_real_producer_output() {
        let fixture = std::fs::read_to_string(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/tests/fixtures/delivery-from-producer.json"
        ))
        .expect("producer fixture");
        let manifest = parse_delivery_manifest(&fixture).expect("real producer output must parse");
        assert!(manifest.components.contains_key("nuwax-im"));
        assert!(
            manifest.components["nuwax-im"]
                .config_id
                .starts_with("sha256:"),
            "config_id 必须保留独立语义"
        );
        assert_ne!(
            manifest.components["nuwax-im"].config_id,
            manifest.components["nuwax-im"].image_id
        );

        let payload = serde_json::to_value(DeliveryPayload {
            contract_version: manifest.contract_version,
            architecture: &manifest.architecture,
            components: &manifest.components,
            mysql: &manifest.mysql,
            compose: &manifest.compose,
        })
        .expect("serialize");
        assert_eq!(
            sha256_hex(canonical_json_string(&payload).as_bytes()),
            manifest.release_sha256,
            "canonical 复算必须与 producer 的 release_sha256 一致"
        );
    }

    #[test]
    fn delivery_verification_passes_and_rejects_tampered_artifacts() {
        let directory = tempfile::tempdir().expect("tempdir");
        let (docker_dir, manifest) = write_delivery_workspace(directory.path());
        verify_delivery_manifest(&manifest, &docker_dir, Some("amd64"))
            .expect("consistent delivery must pass");
        // 架构不匹配
        assert!(verify_delivery_manifest(&manifest, &docker_dir, Some("arm64")).is_err());

        // 混版：替换一个 jar（hash 不再匹配交付清单）
        std::fs::write(
            docker_dir.join("im-app/nuwax-im-gateway-bootstrap.jar"),
            b"jar-mixed",
        )
        .expect("jar");
        let error = verify_delivery_manifest(&manifest, &docker_dir, Some("amd64")).unwrap_err();
        assert!(error.to_string().contains("gateway"), "{error}");
    }

    #[test]
    fn component_artifacts_follow_active_mounts() {
        let directory = tempfile::tempdir().expect("tempdir");
        let (docker_dir, _manifest) = write_delivery_workspace(directory.path());
        // compose 挂载 im-app jar + repo-collab dist（缺失）
        std::fs::write(
            docker_dir.join("docker-compose.yml"),
            "services:\n  im:\n    volumes:\n      - ./im-app/nuwax-im-web-bootstrap.jar:/app/app.jar\n  collab:\n    volumes:\n      - ./repo-collab-app/dist:/app/dist\n",
        )
        .expect("compose");
        let compose_text =
            std::fs::read_to_string(docker_dir.join("docker-compose.yml")).expect("read");
        let error = validate_component_artifacts(&docker_dir, &compose_text, None).unwrap_err();
        assert!(error.to_string().contains("repo-collab-app"), "{error}");

        // 补齐 dist/index.js 后通过
        std::fs::create_dir_all(docker_dir.join("repo-collab-app/dist")).expect("mkdir");
        std::fs::write(docker_dir.join("repo-collab-app/dist/index.js"), b"entry").expect("entry");
        validate_component_artifacts(&docker_dir, &compose_text, None).expect("complete artifacts");

        // 启用服务缺整个组件目录（删除 im-app 根但 compose 仍挂载）→ 拒绝
        std::fs::remove_dir_all(docker_dir.join("im-app")).expect("remove");
        assert!(validate_component_artifacts(&docker_dir, &compose_text, None).is_err());
    }
}

// ───────────────────────── 连接映射校验（manifest.application_connections） ─────────────────────────

/// 按清单逐条校验候选 Compose 最终插值后的服务连接值。
///
/// 每条 `(service, role)` 独立匹配自己的目标：selector 环境变量在该服务
/// environment 中的最终值（`docker compose config` 语义：条目值经 shell > env-file
/// 插值）必须与声明的 host/port/database 精确一致；值缺失同样失败。
/// 不做"库名属于已知集合并集"式的放行——IM 配成 agent_platform 必须被拒绝。
pub fn validate_application_connections(
    compose_text: &str,
    env_values: &HashMap<String, String>,
    manifest: &SchemaManifest,
) -> Result<()> {
    let compose: serde_yaml::Value = serde_yaml::from_str(compose_text)
        .with_context(|| "preflight cannot parse candidate compose YAML")?;

    for connection in &manifest.application_connections {
        let service = &compose["services"][&connection.service];
        if service.is_null() {
            return Err(anyhow!(
                "candidate compose has no service '{}' required by connection ({}, {})",
                connection.service,
                connection.service,
                connection.role
            ));
        }

        if let Some(host_selector) = &connection.environment.host {
            let value = resolve_service_env(service, host_selector, env_values)
                .with_context(|| {
                    format!(
                        "connection ({}, {}): host selector '{host_selector}' is not set on service '{}'",
                        connection.service, connection.role, connection.service
                    )
                })?;
            if value != connection.host {
                return Err(anyhow!(
                    "connection ({}, {}): host selector '{host_selector}' resolves to '{value}', \
                     must stay on the local Compose service '{}'",
                    connection.service,
                    connection.role,
                    connection.host
                ));
            }
        }

        let port = resolve_service_env(service, &connection.environment.port, env_values)
            .with_context(|| {
                format!(
                    "connection ({}, {}): port selector '{}' is not set on service '{}'",
                    connection.service,
                    connection.role,
                    connection.environment.port,
                    connection.service
                )
            })?;
        let port: u16 = port.trim().parse().with_context(|| {
            format!(
                "connection ({}, {}): port selector '{}' resolves to non-integer '{port}'",
                connection.service, connection.role, connection.environment.port
            )
        })?;
        if port != connection.port {
            return Err(anyhow!(
                "connection ({}, {}): port selector resolves to {port}, must be the mysql internal port {}",
                connection.service,
                connection.role,
                connection.port
            ));
        }

        let database = resolve_service_env(service, &connection.environment.database, env_values)
            .with_context(|| {
            format!(
                "connection ({}, {}): database selector '{}' is not set on service '{}'",
                connection.service,
                connection.role,
                connection.environment.database,
                connection.service
            )
        })?;
        if database != connection.database {
            return Err(anyhow!(
                "connection ({}, {}): database selector resolves to '{database}', \
                 which is not this connection's migration target '{}'",
                connection.service,
                connection.role,
                connection.database
            ));
        }
    }
    Ok(())
}

/// 解析服务环境条目的最终值（与 `docker compose config` 语义一致）：
/// - `KEY=raw` / `KEY: raw`：raw 经插值（shell > env-file，未定义 → 空）后的最终值；
///   插值为空就是空，**不回退** env-file（显式空值语义，F03）；
/// - `KEY`（裸键，无分隔符）/ `KEY:`（null）：值取 shell，其次 env-file（继承语义）。
fn resolve_service_env(
    service: &serde_yaml::Value,
    selector: &str,
    env_values: &HashMap<String, String>,
) -> Option<String> {
    resolve_service_env_with(service, selector, env_values, |name| {
        std::env::var(name).ok()
    })
}

/// `shell_lookup` 注入 shell 环境读取（生产为 `std::env::var`；测试注入受控值，
/// 保持 shell > env-file 的插值优先级可测）
fn resolve_service_env_with(
    service: &serde_yaml::Value,
    selector: &str,
    env_values: &HashMap<String, String>,
    shell_lookup: impl Fn(&str) -> Option<String>,
) -> Option<String> {
    let environment = service.get("environment")?;
    let (raw, bare): (String, bool) = match environment {
        serde_yaml::Value::Sequence(entries) => entries.iter().find_map(|entry| {
            let entry = entry.as_str()?;
            match entry.split_once('=') {
                Some((key, value)) if key.trim() == selector => Some((value.to_string(), false)),
                None if entry.trim() == selector => Some((String::new(), true)),
                _ => None,
            }
        }),
        serde_yaml::Value::Mapping(entries) => entries.iter().find_map(|(key, value)| {
            (key.as_str()? == selector).then(|| match value {
                serde_yaml::Value::String(raw) => (raw.clone(), false),
                serde_yaml::Value::Null => (String::new(), true),
                _ => (String::new(), false),
            })
        }),
        _ => None,
    }?;

    if bare {
        return shell_lookup(selector).or_else(|| env_values.get(selector).cloned());
    }
    interpolate_env(
        &raw,
        &|name: &str| shell_lookup(name).or_else(|| env_values.get(name).cloned()),
        crate::container::interpolation::MissingVariables::Empty,
    )
    .ok()
}

/// 用与运行时相同的 Compose env-file 解析器（引号/行内注释/插值/优先级）解析
/// 合并后的 `.env` 文本。候选停服前预检与运行预检共享同一语义（F03）。
pub fn compose_env_values_from_text(text: &str) -> Result<HashMap<String, String>> {
    crate::container::config::compose_env::parse_env_values(text, &|key: &str| {
        std::env::var(key).ok()
    })
    .map_err(|error| anyhow!("failed to parse merged .env text: {error}"))
}

/// Top-level env declarations for preservation, without evaluating values or
/// requiring host variables used by a package default that may be skipped.
pub fn compose_env_declared_keys(text: &str) -> Result<HashSet<String>> {
    crate::container::config::compose_env::declared_env_keys(text)
        .context("failed to read environment declarations")
}

// ───────────────────────── 交付清单（DELIVERY_MANIFEST.json v1） ─────────────────────────

#[derive(Debug, Clone, serde::Deserialize, serde::Serialize)]
#[serde(deny_unknown_fields)]
pub struct DeliveryManifest {
    pub contract_version: u32,
    pub architecture: String,
    #[serde(default)]
    pub components: HashMap<String, DeliveryComponent>,
    pub mysql: DeliveryMysql,
    pub compose: DeliveryCompose,
    pub release_sha256: String,
}

#[derive(Debug, Clone, serde::Deserialize, serde::Serialize)]
#[serde(deny_unknown_fields)]
pub struct DeliveryComponent {
    pub version: serde_json::Value,
    pub image_id: String,
    /// 镜像配置 blob 的 sha256（Docker 29 的 image_id 可能为 OCI index，二者语义不同，
    /// 由构建方同时输出；canonical release payload 包含该字段）
    pub config_id: String,
    pub source: String,
    pub target: String,
    pub artifacts: HashMap<String, String>,
}

#[derive(Debug, Clone, serde::Deserialize, serde::Serialize)]
#[serde(deny_unknown_fields)]
pub struct DeliveryMysql {
    pub manifest: String,
    pub files: HashMap<String, String>,
}

#[derive(Debug, Clone, serde::Deserialize, serde::Serialize)]
#[serde(deny_unknown_fields)]
pub struct DeliveryCompose {
    pub path: String,
    pub sha256: String,
}

pub fn parse_delivery_manifest(text: &str) -> Result<DeliveryManifest> {
    let manifest: DeliveryManifest = serde_json::from_str(text)
        .with_context(|| "DELIVERY_MANIFEST.json is not a valid v1 document")?;
    if manifest.contract_version != 1 {
        bail!(
            "unsupported DELIVERY_MANIFEST contract_version {} (supported: 1)",
            manifest.contract_version
        );
    }
    if !manifest
        .release_sha256
        .chars()
        .all(|c| c.is_ascii_hexdigit())
        || manifest.release_sha256.len() != 64
    {
        bail!("DELIVERY_MANIFEST release_sha256 must be a 64-char hex digest");
    }
    Ok(manifest)
}

fn file_sha256(path: &Path) -> Result<String> {
    let bytes = std::fs::read(path)
        .with_context(|| format!("cannot read file for hashing: {}", path.display()))?;
    Ok(sha256_hex(&bytes))
}

fn sha256_hex(bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes);
    digest.iter().map(|byte| format!("{byte:02x}")).collect()
}

/// 校验交付清单与磁盘一致（同包同版证明）：Compose/MySQL 清单文件/组件产物的
/// SHA256 必须与实际文件一致；`local_architecture` 给定时须与清单架构一致。
pub fn verify_delivery_manifest(
    manifest: &DeliveryManifest,
    docker_root: &Path,
    local_architecture: Option<&str>,
) -> Result<()> {
    if let Some(architecture) = local_architecture
        && !architecture.is_empty()
        && manifest.architecture != architecture
    {
        bail!(
            "DELIVERY_MANIFEST architecture '{}' does not match this host ('{architecture}')",
            manifest.architecture
        );
    }

    let mut checked = 0usize;
    let compose_path = docker_root.join(&manifest.compose.path);
    let actual = file_sha256(&compose_path)?;
    if actual != manifest.compose.sha256 {
        bail!(
            "compose {} hash mismatch: delivery manifest recorded {}, disk has {actual}",
            compose_path.display(),
            manifest.compose.sha256
        );
    }
    checked += 1;

    for (path, expected) in &manifest.mysql.files {
        let actual = file_sha256(&docker_root.join(path))?;
        if actual != *expected {
            bail!(
                "mysql file {path} hash mismatch: delivery manifest recorded {expected}, disk has {actual}"
            );
        }
        checked += 1;
    }

    for component in manifest.components.values() {
        for (path, expected) in &component.artifacts {
            let full = docker_root.join(path);
            if !full.is_file() || std::fs::metadata(&full).map(|m| m.len()).unwrap_or(0) == 0 {
                bail!(
                    "component artifact {path} recorded in DELIVERY_MANIFEST is missing or empty"
                );
            }
            let actual = file_sha256(&full)?;
            if actual != *expected {
                bail!(
                    "component artifact {path} hash mismatch: delivery manifest recorded {expected}, disk has {actual}"
                );
            }
            checked += 1;
        }
    }

    // release_sha256：按构建侧同样的规范形（键排序、紧凑分隔、UTF-8 直通）重算
    verify_release_identity(manifest)?;

    info!(
        files_checked = checked,
        "Delivery manifest verified against disk"
    );
    Ok(())
}

#[derive(serde::Serialize)]
struct DeliveryPayload<'a> {
    contract_version: u32,
    architecture: &'a str,
    components: &'a HashMap<String, DeliveryComponent>,
    mysql: &'a DeliveryMysql,
    compose: &'a DeliveryCompose,
}

fn verify_release_identity(manifest: &DeliveryManifest) -> Result<()> {
    let payload = serde_json::to_value(DeliveryPayload {
        contract_version: manifest.contract_version,
        architecture: &manifest.architecture,
        components: &manifest.components,
        mysql: &manifest.mysql,
        compose: &manifest.compose,
    })
    .context("cannot re-serialize delivery manifest")?;
    let recomputed = sha256_hex(canonical_json_string(&payload).as_bytes());
    if recomputed != manifest.release_sha256 {
        bail!("DELIVERY_MANIFEST release_sha256 mismatch: files do not form the declared release");
    }
    Ok(())
}

/// Python `json.dumps(payload, sort_keys=True, separators=(",", ":"), ensure_ascii=False)`
/// 的等价规范形（用于 release_sha256 复算；键按码点排序，字符串 UTF-8 直通）
fn canonical_json_string(value: &serde_json::Value) -> String {
    let mut out = String::new();
    canonical_json(value, &mut out);
    out
}

fn canonical_json(value: &serde_json::Value, out: &mut String) {
    match value {
        serde_json::Value::Object(map) => {
            let mut keys: Vec<&String> = map.keys().collect();
            keys.sort();
            out.push('{');
            for (index, key) in keys.iter().enumerate() {
                if index > 0 {
                    out.push(',');
                }
                canonical_json(&serde_json::Value::String((*key).clone()), out);
                out.push(':');
                let value = map.get(key.as_str()).unwrap_or(&serde_json::Value::Null);
                canonical_json(value, out);
            }
            out.push('}');
        }
        serde_json::Value::Array(items) => {
            out.push('[');
            for (index, item) in items.iter().enumerate() {
                if index > 0 {
                    out.push(',');
                }
                canonical_json(item, out);
            }
            out.push(']');
        }
        serde_json::Value::String(text) => {
            let encoded =
                serde_json::to_string(&serde_json::Value::String(text.clone())).unwrap_or_default();
            out.push_str(&encoded);
        }
        other => {
            let encoded = serde_json::to_string(other).unwrap_or_default();
            out.push_str(&encoded);
        }
    }
}

// ───────────────────────── 挂载驱动的组件产物校验 ─────────────────────────

/// 收集 compose 全部 bind 挂载的宿主相对路径（相对 docker/；支持长/短语法）
pub fn collect_bind_mount_sources(compose_text: &str) -> Result<Vec<String>> {
    let compose: serde_yaml::Value = serde_yaml::from_str(compose_text)
        .with_context(|| "cannot parse compose YAML for bind mounts")?;
    let mut sources = Vec::new();
    let services = compose
        .get("services")
        .and_then(|services| services.as_mapping())
        .ok_or_else(|| anyhow!("compose has no services"))?;
    for service in services.values() {
        let Some(volumes) = service.get("volumes") else {
            continue;
        };
        for (source, _) in bind_mount_pairs(volumes) {
            sources.push(normalize_mount_source(&source));
        }
    }
    sources.sort();
    sources.dedup();
    Ok(sources)
}

fn normalize_mount_source(source: &str) -> String {
    source.strip_prefix("./").unwrap_or(source).to_string()
}

/// Shared short-volume splitting preserves drive-letter colons on Windows.
/// Named volumes are excluded: this contract requires actual host bind mounts.
fn bind_mount_pairs(volumes: &serde_yaml::Value) -> Vec<(String, String)> {
    let Some(entries) = volumes.as_sequence() else {
        return Vec::new();
    };
    entries
        .iter()
        .filter_map(|entry| match entry {
            serde_yaml::Value::String(short) => {
                let (source, target, _) = crate::container::volumes::short_volume_parts(short)?;
                let bytes = source.as_bytes();
                let drive = bytes.len() >= 3
                    && bytes[0].is_ascii_alphabetic()
                    && bytes[1] == b':'
                    && matches!(bytes[2], b'/' | b'\\');
                let bind = source.starts_with('.')
                    || source.starts_with('/')
                    || source.starts_with('\\')
                    || drive;
                bind.then(|| (source.to_string(), target.to_string()))
            }
            serde_yaml::Value::Mapping(long) => {
                if long.get("type").and_then(|value| value.as_str()) != Some("bind") {
                    return None;
                }
                Some((
                    long.get("source")?.as_str()?.to_string(),
                    long.get("target")?.as_str()?.to_string(),
                ))
            }
            _ => None,
        })
        .collect()
}

fn component_entrypoints(mounts: &[String]) -> Vec<&'static str> {
    let uses_root = |root: &str| {
        mounts
            .iter()
            .any(|source| source == root || source.starts_with(&format!("{root}/")))
    };
    let mut required = Vec::new();
    if uses_root("im-app") {
        required.extend([
            "im-app/nuwax-im-web-bootstrap.jar",
            "im-app/nuwax-im-gateway-bootstrap.jar",
        ]);
    }
    if uses_root("repo-collab-app") {
        required.push("repo-collab-app/dist/index.js");
    }
    required
}

/// 挂载驱动的组件产物完整性：
/// - 引用组件目录（im-app/、repo-collab-app/）的每个挂载源必须实际存在且非空
///   （启用服务缺整个目录也会在此失败）；
/// - 有交付清单时叠加 hash 校验（verify_delivery_manifest）；
/// - 无交付清单（legacy 包）退回启发式：目录存在 → 双 jar / dist 非空。
pub fn validate_component_artifacts(
    docker_dir: &Path,
    compose_text: &str,
    delivery: Option<&DeliveryManifest>,
) -> Result<()> {
    let component_roots: Vec<String> = delivery
        .map(|manifest| {
            let mut roots: Vec<String> = manifest
                .components
                .values()
                .flat_map(|component| {
                    component
                        .artifacts
                        .keys()
                        .map(|path| path.split('/').next().unwrap_or(path).to_string())
                })
                .collect();
            roots.sort();
            roots.dedup();
            roots
        })
        .unwrap_or_else(|| vec!["im-app".to_string(), "repo-collab-app".to_string()]);

    let mounts = collect_bind_mount_sources(compose_text)?;
    for source in &mounts {
        let in_component = component_roots
            .iter()
            .any(|root| source == root || source.starts_with(&format!("{root}/")));
        if !in_component {
            continue;
        }
        let full = docker_dir.join(source);
        let present = if full.is_dir() {
            full.read_dir()
                .map(|entries| entries.flatten().next().is_some())
                .unwrap_or(false)
        } else {
            full.is_file()
                && std::fs::metadata(&full)
                    .map(|meta| meta.len() > 0)
                    .unwrap_or(false)
        };
        if !present {
            bail!(
                "compose mounts '{source}' but the path is missing or empty under {} \
                 (component artifacts must be complete before services start)",
                docker_dir.display()
            );
        }
    }

    let entrypoints = component_entrypoints(&mounts);
    for entrypoint in &entrypoints {
        if !is_non_empty_file(&docker_dir.join(entrypoint)) {
            bail!(
                "required component entrypoint {entrypoint} is missing or not a non-empty regular file"
            );
        }
    }

    if let Some(delivery) = delivery {
        ensure_delivery_covers_entrypoints(delivery, &entrypoints)?;
        verify_delivery_manifest(delivery, docker_dir, None)?;
        return Ok(());
    }

    // legacy 启发式
    validate_host_artifacts(docker_dir)
}

fn ensure_delivery_covers_entrypoints(
    delivery: &DeliveryManifest,
    entrypoints: &[&str],
) -> Result<()> {
    for entrypoint in entrypoints {
        if !delivery
            .components
            .values()
            .any(|component| component.artifacts.contains_key(*entrypoint))
        {
            bail!("DELIVERY_MANIFEST does not identify required component entrypoint {entrypoint}");
        }
    }
    Ok(())
}

// ───────────────────────── 备份/回滚兼容预检 ─────────────────────────

/// 恢复旧备份前的组件兼容检查：当前 Compose 引用的组件产物目录（im-app、
/// repo-collab-app 等 bind 挂载）必须存在于备份归档中。旧备份早于组件引入时，
/// 恢复流程会清理这些目录却保留引用它们的新 Compose，留下不完整部署——
/// 必须在任何破坏性清理之前失败，而不是假称旧备份包含旧 Compose。
pub fn verify_backup_component_compatibility(
    backup_archive: &Path,
    docker_dir: &Path,
) -> Result<()> {
    let compose_path = docker_dir.join("docker-compose.yml");
    let compose_text = if compose_path.exists() {
        std::fs::read_to_string(&compose_path).context("cannot read backup release Compose")?
    } else {
        "services: {}".to_string()
    };
    let mounts = collect_bind_mount_sources(&compose_text)?;
    let component_roots = ["im-app", "repo-collab-app"];
    let required_files: Vec<String> = mounts
        .iter()
        .filter(|source| {
            component_roots
                .iter()
                .any(|root| source == root || source.starts_with(&format!("{root}/")))
        })
        .cloned()
        .collect();
    // The receipt is small; artifact and database contents remain streaming.
    let file = std::fs::File::open(backup_archive)
        .with_context(|| format!("cannot open backup archive: {}", backup_archive.display()))?;
    let decoder = flate2::read::GzDecoder::new(file);
    let mut archive = tar::Archive::new(decoder);
    let mut entries: HashMap<String, BackupArchiveEntry> = HashMap::new();
    let mut receipt = None;
    for entry in archive.entries()? {
        let entry = entry
            .with_context(|| format!("cannot read backup archive: {}", backup_archive.display()))?;
        let mut entry = entry;
        let name = backup_member_path(&entry.path()?)?;
        let kind = entry.header().entry_type();
        let metadata = BackupArchiveEntry {
            regular: crate::backup_release::is_archive_regular_file(kind),
            directory: kind.is_dir(),
            // `Entry::size` includes GNU sparse holes; stored extent bytes can
            // be zero even for a non-empty logical file.
            size: entry.size(),
        };
        if let Some(previous) = entries.insert(name.clone(), metadata)
            && (previous.regular || metadata.regular)
        {
            bail!("backup contains duplicate file entry {name}");
        }
        if name == "DELIVERY_MANIFEST.json" {
            if !kind.is_file() || metadata.size == 0 || metadata.size > 4 * 1024 * 1024 {
                bail!("backup DELIVERY_MANIFEST.json must be a non-empty regular file below 4 MiB");
            }
            let mut bytes = Vec::new();
            entry.read_to_end(&mut bytes)?;
            let text = String::from_utf8(bytes).context("backup delivery manifest is not UTF-8")?;
            receipt = Some(parse_delivery_manifest(&text)?);
        }
    }

    for source in &required_files {
        let source_path = source.as_str();
        let looks_like_file = source_path
            .rsplit('/')
            .next()
            .is_some_and(|leaf| leaf.contains('.'));
        if looks_like_file {
            // 文件挂载：备份必须含该路径的普通文件条目且非空（半包拒绝）
            ensure_backup_regular_file(&entries, source_path)?;
        } else {
            // 目录挂载：备份必须含该目录下至少一个非空普通文件（files-only 归档合法）
            let prefix = format!("{source_path}/");
            let present = entries.iter().any(|(name, metadata)| {
                name.starts_with(&prefix) && metadata.regular && metadata.size > 0
            });
            if !present {
                bail!(
                    "current compose mounts '{source}' but the backup archive contains no \
                     non-empty files under it; refusing a component-empty restore of {}",
                    backup_archive.display()
                );
            }
        }
    }
    let entrypoints = component_entrypoints(&mounts);
    for entrypoint in &entrypoints {
        ensure_backup_regular_file(&entries, entrypoint)?;
    }
    if let Some(receipt) = receipt {
        ensure_delivery_covers_entrypoints(&receipt, &entrypoints)?;
        verify_release_identity(&receipt)?;
        let expected = expected_delivery_files(&receipt)?;
        for path in expected.keys() {
            ensure_backup_regular_file(&entries, path)?;
        }
        verify_backup_delivery_hashes(backup_archive, &expected)?;
    }
    Ok(())
}

#[derive(Clone, Copy)]
struct BackupArchiveEntry {
    regular: bool,
    directory: bool,
    size: u64,
}

fn backup_member_path(path: &Path) -> Result<String> {
    if path.to_string_lossy().contains('\\') {
        bail!("backup member paths must use portable forward slashes");
    }
    let mut normalized = Vec::new();
    for part in path.components() {
        match part {
            Component::CurDir => {}
            Component::Normal(name) => {
                let name = name.to_string_lossy().into_owned();
                let bytes = name.as_bytes();
                if normalized.is_empty()
                    && bytes.len() >= 2
                    && bytes[0].is_ascii_alphabetic()
                    && bytes[1] == b':'
                {
                    bail!("backup contains an unsafe drive-qualified member path");
                }
                normalized.push(name);
            }
            _ => bail!("backup contains an unsafe member path"),
        }
    }
    // Archive/receipt keys are POSIX strings, independent of the host's native
    // separators. Rebuilding a PathBuf here would introduce backslashes on Windows.
    Ok(normalized.join("/"))
}

fn ensure_backup_regular_file(
    entries: &HashMap<String, BackupArchiveEntry>,
    path: &str,
) -> Result<()> {
    let canonical = backup_member_path(Path::new(path))?;
    if !entries
        .get(&canonical)
        .is_some_and(|entry| entry.regular && entry.size > 0)
    {
        bail!("backup is missing required non-empty regular file {path}");
    }
    let mut parent = canonical.as_str();
    while let Some((prefix, _)) = parent.rsplit_once('/') {
        if let Some(entry) = entries.get(prefix)
            && !entry.directory
        {
            bail!("backup parent of {path} is not a regular directory");
        }
        parent = prefix;
    }
    Ok(())
}

fn expected_delivery_files(delivery: &DeliveryManifest) -> Result<HashMap<String, String>> {
    let mut files = HashMap::new();
    let mut add = |path: &str, hash: &str| -> Result<()> {
        if path.contains('\\') || backup_member_path(Path::new(path))? != path || path.is_empty() {
            bail!("DELIVERY_MANIFEST contains an unsafe or noncanonical file path");
        }
        if let Some(previous) = files.insert(path.to_string(), hash.to_string())
            && previous != hash
        {
            bail!("DELIVERY_MANIFEST has conflicting identities for {path}");
        }
        Ok(())
    };
    add(&delivery.compose.path, &delivery.compose.sha256)?;
    for (path, hash) in &delivery.mysql.files {
        add(path, hash)?;
    }
    for component in delivery.components.values() {
        for (path, hash) in &component.artifacts {
            add(path, hash)?;
        }
    }
    if !files.contains_key(&delivery.mysql.manifest) {
        bail!("DELIVERY_MANIFEST does not identify its MySQL schema manifest");
    }
    Ok(files)
}

fn verify_backup_delivery_hashes(
    archive_path: &Path,
    expected: &HashMap<String, String>,
) -> Result<()> {
    let file = std::fs::File::open(archive_path)?;
    let mut archive = tar::Archive::new(flate2::read::GzDecoder::new(file));
    let mut verified = HashSet::new();
    for entry in archive.entries()? {
        let mut entry = entry?;
        let name = backup_member_path(&entry.path()?)?;
        let Some(expected_hash) = expected.get(&name) else {
            continue;
        };
        let mut hasher = Sha256::new();
        let mut buffer = [0u8; 64 * 1024];
        loop {
            let count = entry.read(&mut buffer)?;
            if count == 0 {
                break;
            }
            hasher.update(&buffer[..count]);
        }
        let actual: String = hasher
            .finalize()
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect();
        if &actual != expected_hash {
            bail!("backup file {name} does not match its DELIVERY_MANIFEST identity");
        }
        verified.insert(name);
    }
    if verified.len() != expected.len() {
        bail!("backup is missing files declared by DELIVERY_MANIFEST");
    }
    Ok(())
}

/// initdb 挂载契约：mysql 服务必须把清单声明的每个文件按其 `initdb_target`
/// 挂到 `/docker-entrypoint-initdb.d/`（source 指向声明路径，target 文件名一致），
/// 否则新装初始化顺序与清单声明脱节。
pub fn validate_initdb_mount_contract(compose_text: &str, manifest: &SchemaManifest) -> Result<()> {
    validate_initdb_mount_text_at(compose_text, Path::new("."), Path::new("."), manifest)
}

/// Resolve host sources relative to the selected Compose file, independently of
/// where the env file lives, and compare them with declared package files.
pub fn validate_initdb_mount_contract_at(
    compose_path: &Path,
    package_root: &Path,
    manifest: &SchemaManifest,
) -> Result<()> {
    let text = std::fs::read_to_string(compose_path)
        .with_context(|| format!("cannot read initdb Compose file {}", compose_path.display()))?;
    validate_initdb_mount_text_at(
        &text,
        compose_path.parent().unwrap_or(Path::new(".")),
        package_root,
        manifest,
    )
}

fn validate_initdb_mount_text_at(
    compose_text: &str,
    compose_directory: &Path,
    package_root: &Path,
    manifest: &SchemaManifest,
) -> Result<()> {
    let compose: serde_yaml::Value = serde_yaml::from_str(compose_text)
        .with_context(|| "cannot parse compose YAML for initdb mounts")?;
    let mysql_service = compose
        .get("services")
        .and_then(|services| services.get(&manifest.mysql_target.service))
        .ok_or_else(|| {
            anyhow!(
                "compose has no '{}' service for initdb mounts",
                manifest.mysql_target.service
            )
        })?;
    let Some(volumes) = mysql_service.get("volumes").and_then(|v| v.as_sequence()) else {
        bail!(
            "compose service '{}' declares no volumes; initdb mounts are missing",
            manifest.mysql_target.service
        );
    };

    let mounts: Vec<(PathBuf, String)> =
        bind_mount_pairs(&serde_yaml::Value::Sequence(volumes.clone()))
            .into_iter()
            .map(|(source, target)| {
                Ok((context_path(compose_directory, Path::new(&source))?, target))
            })
            .collect::<Result<_>>()?;

    let mut expected: Vec<(&str, &str)> = vec![
        (&manifest.bootstrap.path, &manifest.bootstrap.initdb_target),
        (
            &manifest.permissions.path,
            &manifest.permissions.initdb_target,
        ),
    ];
    for schema in &manifest.schemas {
        expected.push((&schema.path, &schema.initdb_target));
    }
    for seed in &manifest.first_install_seeds {
        expected.push((&seed.path, &seed.initdb_target));
    }

    for (source, filename) in &expected {
        let target = format!("/docker-entrypoint-initdb.d/{filename}");
        let expected_source = context_path(package_root, Path::new(source))?;
        let targets: Vec<_> = mounts
            .iter()
            .filter(|(_, mount_target)| mount_target == &target)
            .collect();
        if targets.len() != 1 || targets[0].0 != expected_source {
            bail!(
                "initdb mount contract violated: expected {source} -> {target} on service '{}'; \
                 new-install initialization order would diverge from the manifest",
                manifest.mysql_target.service
            );
        }
    }
    Ok(())
}

fn context_path(base: &Path, path: &Path) -> Result<PathBuf> {
    let full = if path.is_absolute() {
        path.to_path_buf()
    } else {
        let base = if base.is_absolute() {
            base.to_path_buf()
        } else {
            std::env::current_dir()?.join(base)
        };
        base.join(path)
    };
    let mut normalized = PathBuf::new();
    for component in full.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                if !normalized.pop() {
                    bail!("mount source cannot escape the filesystem root");
                }
            }
            part => normalized.push(part.as_os_str()),
        }
    }
    Ok(normalized)
}
