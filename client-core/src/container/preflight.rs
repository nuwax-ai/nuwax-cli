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

use crate::container::interpolate_env;
use crate::container::load_env_values;
use crate::mysql_manifest::SchemaManifest;
use anyhow::Context as _;
use anyhow::{Result, anyhow, bail};
use regex::Regex;
use sha2::Digest;
use sha2::Sha256;
use std::collections::HashMap;
use std::path::Path;
use tracing::info;

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

    let missing = missing_required_env_keys(&compose_text, &values);
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

    let docker_dir = env_path.parent().unwrap_or(Path::new("."));
    validate_component_artifacts(docker_dir, &compose_text, delivery)?;
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

    let payload = serde_json::to_value(DeliveryPayload {
        contract_version: manifest.contract_version,
        architecture: &manifest.architecture,
        components: &manifest.components,
        mysql: &manifest.mysql,
        compose: &manifest.compose,
    })
    .with_context(|| "cannot re-serialize delivery manifest")?;
    let canonical = canonical_json_string(&payload);
    if sha256_hex(canonical.as_bytes()) != manifest.release_sha256 {
        bail!("DELIVERY_MANIFEST release_sha256 mismatch against archive contents");
    }
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
    let payload = serde_json::to_value(DeliveryPayload {
        contract_version: manifest.contract_version,
        architecture: &manifest.architecture,
        components: &manifest.components,
        mysql: &manifest.mysql,
        compose: &manifest.compose,
    })
    .with_context(|| "cannot re-serialize delivery manifest")?;
    let canonical = canonical_json_string(&payload);
    let recomputed = sha256_hex(canonical.as_bytes());
    if recomputed != manifest.release_sha256 {
        bail!(
            "DELIVERY_MANIFEST release_sha256 mismatch: recorded {}, recomputed {recomputed} \
             (staged files do not form the declared release)",
            manifest.release_sha256
        );
    }

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
        if let Some(entries) = volumes.as_sequence() {
            for entry in entries {
                match entry {
                    serde_yaml::Value::String(short) => {
                        // 短语法 `source:target[:mode]`，只接受显式相对 bind 源（./ 或 ../）
                        let source = short.split(':').next().unwrap_or(short);
                        if source.starts_with("./") || source.starts_with("../") {
                            sources.push(normalize_mount_source(source));
                        }
                    }
                    serde_yaml::Value::Mapping(long) => {
                        let bind = long
                            .get(serde_yaml::Value::String("type".to_string()))
                            .and_then(|value| value.as_str())
                            .is_some_and(|kind| kind == "bind");
                        if !bind {
                            continue;
                        }
                        if let Some(source) = long
                            .get(serde_yaml::Value::String("source".to_string()))
                            .and_then(|value| value.as_str())
                        {
                            sources.push(normalize_mount_source(source));
                        }
                    }
                    _ => {}
                }
            }
        }
    }
    sources.sort();
    sources.dedup();
    Ok(sources)
}

fn normalize_mount_source(source: &str) -> String {
    let normalized = source.strip_prefix("./").unwrap_or(source);
    normalized
        .strip_prefix("docker/")
        .unwrap_or(normalized)
        .to_string()
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

    if let Some(delivery) = delivery {
        verify_delivery_manifest(delivery, docker_dir, None)?;
        return Ok(());
    }

    // legacy 启发式
    validate_host_artifacts(docker_dir)
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
    let Ok(compose_text) = std::fs::read_to_string(docker_dir.join("docker-compose.yml")) else {
        return Ok(());
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
    if required_files.is_empty() {
        return Ok(());
    }

    // 备份归档（tar.gz）条目清单：文件名 + 是否普通文件 + 大小
    let file = std::fs::File::open(backup_archive)
        .with_context(|| format!("cannot open backup archive: {}", backup_archive.display()))?;
    let decoder = flate2::read::GzDecoder::new(file);
    let mut archive = tar::Archive::new(decoder);
    let mut entries: Vec<(String, bool, u64)> = Vec::new();
    for entry in archive.entries()? {
        let entry = entry
            .with_context(|| format!("cannot read backup archive: {}", backup_archive.display()))?;
        let header = entry.header();
        let name = entry.path()?.to_string_lossy().into_owned();
        let is_file = header.entry_type().is_file();
        let size = header.size().unwrap_or(0);
        entries.push((name, is_file, size));
    }

    for source in &required_files {
        let source_path = source.as_str();
        let looks_like_file = source_path
            .rsplit('/')
            .next()
            .is_some_and(|leaf| leaf.contains('.'));
        if looks_like_file {
            // 文件挂载：备份必须含该路径的普通文件条目且非空（半包拒绝）
            let present = entries
                .iter()
                .any(|(name, is_file, size)| name == source_path && *is_file && *size > 0);
            if !present {
                bail!(
                    "current compose mounts '{source}' but the backup archive does not contain \
                     it as a non-empty regular file; refusing a half restore of {}",
                    backup_archive.display()
                );
            }
        } else {
            // 目录挂载：备份必须含该目录下至少一个非空普通文件（files-only 归档合法）
            let prefix = format!("{source_path}/");
            let present = entries
                .iter()
                .any(|(name, is_file, size)| name.starts_with(&prefix) && *is_file && *size > 0);
            if !present {
                bail!(
                    "current compose mounts '{source}' but the backup archive contains no \
                     non-empty files under it; refusing a component-empty restore of {}",
                    backup_archive.display()
                );
            }
        }
    }
    Ok(())
}

/// initdb 挂载契约：mysql 服务必须把清单声明的每个文件按其 `initdb_target`
/// 挂到 `/docker-entrypoint-initdb.d/`（source 指向声明路径，target 文件名一致），
/// 否则新装初始化顺序与清单声明脱节。
pub fn validate_initdb_mount_contract(compose_text: &str, manifest: &SchemaManifest) -> Result<()> {
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

    let mut mounts: Vec<(String, String)> = Vec::new();
    for volume in volumes {
        if let Some(mapping) = volume.as_mapping() {
            let source = mapping
                .get(serde_yaml::Value::String("source".to_string()))
                .and_then(|value| value.as_str())
                .map(normalize_mount_source);
            let target = mapping
                .get(serde_yaml::Value::String("target".to_string()))
                .and_then(|value| value.as_str());
            if let (Some(source), Some(target)) = (source, target) {
                mounts.push((source, target.to_string()));
            }
        }
    }

    let mut expected: Vec<(String, String)> = Vec::new();
    let push = |expected: &mut Vec<(String, String)>, path: &str, target: &str| {
        let normalized = normalize_mount_source(path);
        expected.push((normalized, format!("/docker-entrypoint-initdb.d/{target}")));
    };
    push(
        &mut expected,
        &manifest.bootstrap.path,
        &manifest.bootstrap.initdb_target,
    );
    push(
        &mut expected,
        &manifest.permissions.path,
        &manifest.permissions.initdb_target,
    );
    for schema in &manifest.schemas {
        push(&mut expected, &schema.path, &schema.initdb_target);
    }
    for seed in &manifest.first_install_seeds {
        push(&mut expected, &seed.path, &seed.initdb_target);
    }

    for (source, target) in &expected {
        let mounted = mounts
            .iter()
            .any(|(mount_source, mount_target)| mount_source == source && mount_target == target);
        if !mounted {
            bail!(
                "initdb mount contract violated: expected {source} -> {target} on service '{}'; \
                 new-install initialization order would diverge from the manifest",
                manifest.mysql_target.service
            );
        }
    }
    Ok(())
}
