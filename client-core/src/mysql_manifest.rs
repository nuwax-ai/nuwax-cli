//! 部署包 MySQL schema 清单（`mysql-schema-manifest-v1`）：
//! 解析、校验与存量迁移编排（bootstrap 建库 → 应用账号授权 → 按 schemas 逐库 Live Diff）。
//!
//! 契约来源：build-agent-docker `docs/mysql-schema-contract.md` 与
//! `docker/config/mysql-schema-manifest.json`。要点：
//! - 不扫描 `*.sql` 全执行；所有文件由清单显式引用；
//! - bootstrap（幂等建库）与 permissions（授权）独立执行，不受表差异控制；
//! - `first_install_seeds` 只在首次安装由 MySQL initdb 执行，CLI 存量迁移不重放；
//! - `bootstrap_only` 数据库（业务运行时动态建表）不参与固定表结构同步；
//! - 无 manifest 的旧包走 legacy 固定清单路径；manifest 存在但损坏/不支持时 Fail Fast。

use crate::mysql_executor::MySqlExecutor;
use crate::sql_diff::MultiDbDiffResult;
use crate::sql_diff::generate_live_schema_diff_multi;
use crate::sql_diff::parse_schema_template;
use anyhow::Result;
use anyhow::{Context, bail};
use regex::Regex;
use std::collections::HashSet;
use std::path::Path;
use tracing::info;

/// 本 CLI 实现的清单能力标识（manifest.requires.cli_capability 必须精确匹配）
pub const CLI_CAPABILITY: &str = "mysql-schema-manifest-v1";

/// 数据库/表名安全字符集：字母开头，仅字母、数字、下划线
fn is_safe_identifier(name: &str) -> bool {
    let mut characters = name.chars();
    matches!(characters.next(), Some(first) if first.is_ascii_alphabetic())
        && characters.all(|c| c.is_ascii_alphanumeric() || c == '_')
}

/// 清单引用的文件路径必须位于 `config/` 下、相对、无穿越
fn is_safe_config_path(path: &str) -> bool {
    let path = Path::new(path);
    path.is_relative()
        && path
            .components()
            .all(|component| matches!(component.as_os_str().to_str(), Some(part) if !part.is_empty() && part != "." && part != ".."))
        && path.components().count() >= 2
        && path.components().next().is_some_and(|component| component.as_os_str() == "config")
}

#[derive(Debug, Clone, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SchemaManifest {
    pub contract_version: u32,
    pub requires: RequiresDecl,
    pub mysql_target: MysqlTargetDecl,
    #[serde(default)]
    pub application_connections: Vec<ApplicationConnection>,
    pub databases: Vec<DatabaseDecl>,
    pub bootstrap: BootstrapDecl,
    pub permissions: PermissionsDecl,
    pub schemas: Vec<SchemaDecl>,
    #[serde(default)]
    pub first_install_seeds: Vec<SeedDecl>,
}

#[derive(Debug, Clone, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RequiresDecl {
    pub cli_capability: String,
}

#[derive(Debug, Clone, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MysqlTargetDecl {
    pub service: String,
    pub internal_port: u16,
}

#[derive(Debug, Clone, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ApplicationConnection {
    pub service: String,
    pub role: String,
    pub database: String,
    pub host: String,
    pub port: u16,
    pub environment: ConnectionSelectors,
}

#[derive(Debug, Clone, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ConnectionSelectors {
    /// `null` 表示应用配置模板固定使用声明的 MySQL 服务，无环境变量可覆盖
    pub host: Option<String>,
    pub port: String,
    pub database: String,
}

#[derive(Debug, Clone, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DatabaseDecl {
    pub name: String,
    pub bootstrap_only: bool,
}

#[derive(Debug, Clone, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BootstrapDecl {
    pub path: String,
    pub idempotent: bool,
    pub initdb_target: String,
}

#[derive(Debug, Clone, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PermissionsDecl {
    pub path: String,
    pub user_env: String,
    pub databases: String,
    pub initdb_target: String,
}

#[derive(Debug, Clone, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SchemaDecl {
    pub database: String,
    pub path: String,
    pub initdb_target: String,
}

#[derive(Debug, Clone, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SeedDecl {
    pub database: String,
    pub path: String,
    pub first_install_only: bool,
    pub initdb_target: String,
}

impl SchemaManifest {
    /// 清单引用的全部文件相对路径（相对 docker/）
    pub fn referenced_paths(&self) -> Vec<String> {
        let mut paths = vec![self.bootstrap.path.clone(), self.permissions.path.clone()];
        paths.extend(self.schemas.iter().map(|schema| schema.path.clone()));
        paths.extend(
            self.first_install_seeds
                .iter()
                .map(|seed| seed.path.clone()),
        );
        paths
    }

    /// 迁移涉及的库名（含 bootstrap_only），用于连接目标校验的允许集合
    pub fn database_names(&self) -> Vec<String> {
        self.databases.iter().map(|db| db.name.clone()).collect()
    }
}

/// 解析并做纯结构校验（无文件系统访问，便于单测）
pub fn parse_schema_manifest(text: &str) -> Result<SchemaManifest> {
    let manifest: SchemaManifest = serde_json::from_str(text)
        .context("mysql-schema-manifest.json is not a valid mysql-schema-manifest-v1 document")?;
    if manifest.contract_version != 1 {
        bail!(
            "unsupported mysql-schema-manifest contract_version {} (this CLI implements version 1)",
            manifest.contract_version
        );
    }
    if manifest.requires.cli_capability != CLI_CAPABILITY {
        bail!(
            "manifest requires CLI capability '{}'; this CLI only implements '{CLI_CAPABILITY}'",
            manifest.requires.cli_capability
        );
    }
    validate_manifest_structure(&manifest)?;
    Ok(manifest)
}

/// 纯结构校验：命名、路径形态、清单内部一致性
pub fn validate_manifest_structure(manifest: &SchemaManifest) -> Result<()> {
    if manifest.mysql_target.service.is_empty() {
        bail!("mysql_target.service must not be empty");
    }

    // 数据库声明：安全标识符、唯一、按建库顺序
    let mut declared: HashSet<&str> = HashSet::new();
    for database in &manifest.databases {
        if !is_safe_identifier(&database.name) {
            bail!(
                "manifest database name '{}' is not a safe identifier (letter followed by [A-Za-z0-9_])",
                database.name
            );
        }
        if !declared.insert(database.name.as_str()) {
            bail!(
                "manifest declares database '{}' more than once",
                database.name
            );
        }
    }
    if declared.is_empty() {
        bail!("manifest declares no databases");
    }

    // 文件引用：config/ 下、相对、无穿越
    for path in manifest.referenced_paths() {
        if !is_safe_config_path(&path) {
            bail!("manifest path '{path}' must be a relative path under config/ without traversal");
        }
    }

    // bootstrap 必须幂等（v1 契约）
    if !manifest.bootstrap.idempotent {
        bail!("manifest bootstrap.idempotent must be true");
    }

    // permissions v1：授权范围为 bootstrap 声明的库
    if manifest.permissions.databases != "bootstrap" {
        bail!(
            "manifest permissions.databases must be 'bootstrap' in contract v1 (got '{}')",
            manifest.permissions.databases
        );
    }
    if manifest.permissions.user_env.is_empty() {
        bail!("manifest permissions.user_env must not be empty");
    }

    // initdb 挂载名：两位数字前缀、全局唯一
    let mut targets: HashSet<&str> = HashSet::new();
    let target_pattern =
        Regex::new(r"^[0-9]{2}_[A-Za-z0-9_.-]+$").expect("initdb target pattern must compile");
    let all_targets = std::iter::once(manifest.bootstrap.initdb_target.as_str())
        .chain(std::iter::once(manifest.permissions.initdb_target.as_str()))
        .chain(
            manifest
                .schemas
                .iter()
                .map(|schema| schema.initdb_target.as_str()),
        )
        .chain(
            manifest
                .first_install_seeds
                .iter()
                .map(|seed| seed.initdb_target.as_str()),
        );
    for target in all_targets {
        if !target_pattern.is_match(target) {
            bail!("manifest initdb_target '{target}' must carry a two-digit ordering prefix");
        }
        if !targets.insert(target) {
            bail!("manifest initdb_target '{target}' is declared more than once");
        }
    }

    // schemas：每个非 bootstrap_only 库恰一份；bootstrap_only 库不得有固定 schema
    for schema in &manifest.schemas {
        let Some(database) = manifest
            .databases
            .iter()
            .find(|database| database.name == schema.database)
        else {
            bail!(
                "manifest schema references undeclared database '{}'",
                schema.database
            );
        };
        if database.bootstrap_only {
            bail!(
                "manifest schema for database '{}' conflicts with bootstrap_only: true",
                schema.database
            );
        }
    }
    for database in &manifest.databases {
        let count = manifest
            .schemas
            .iter()
            .filter(|schema| schema.database == database.name)
            .count();
        if database.bootstrap_only {
            if count != 0 {
                bail!(
                    "bootstrap_only database '{}' must not declare a fixed schema",
                    database.name
                );
            }
        } else if count != 1 {
            bail!(
                "database '{}' must declare exactly one schema (got {count})",
                database.name
            );
        }
    }

    // seeds：只允许首次安装
    for seed in &manifest.first_install_seeds {
        if !declared.contains(seed.database.as_str()) {
            bail!(
                "manifest first_install_seed references undeclared database '{}'",
                seed.database
            );
        }
        if !seed.first_install_only {
            bail!(
                "manifest first_install_seed '{}' must set first_install_only: true",
                seed.path
            );
        }
    }

    // 连接映射：(service, role) 唯一；host/port 与 mysql_target 一致；selector 命名安全
    let mut connection_keys: HashSet<(String, String)> = HashSet::new();
    for connection in &manifest.application_connections {
        let key = (connection.service.clone(), connection.role.clone());
        if !connection_keys.insert(key) {
            bail!(
                "manifest application_connections declares ({}, {}) more than once",
                connection.service,
                connection.role
            );
        }
        if !declared.contains(connection.database.as_str()) {
            bail!(
                "manifest connection ({}, {}) targets undeclared database '{}'",
                connection.service,
                connection.role,
                connection.database
            );
        }
        if connection.host != manifest.mysql_target.service {
            bail!(
                "manifest connection ({}, {}) host '{}' must match mysql_target.service '{}'",
                connection.service,
                connection.role,
                connection.host,
                manifest.mysql_target.service
            );
        }
        if connection.port != manifest.mysql_target.internal_port {
            bail!(
                "manifest connection ({}, {}) port {} must match mysql_target.internal_port {}",
                connection.service,
                connection.role,
                connection.port,
                manifest.mysql_target.internal_port
            );
        }
        if connection.environment.port.is_empty() || connection.environment.database.is_empty() {
            bail!(
                "manifest connection ({}, {}) must name non-empty port/database selectors",
                connection.service,
                connection.role
            );
        }
    }

    Ok(())
}

/// 解析 bootstrap SQL 中按序声明的库名（与权限脚本同一行级语法：
/// `CREATE DATABASE IF NOT EXISTS `db` CHARACTER SET x COLLATE y;`）
pub fn bootstrap_declared_databases(bootstrap_sql: &str) -> Result<Vec<String>> {
    let pattern = Regex::new(
        r"^CREATE DATABASE IF NOT EXISTS `([A-Za-z][A-Za-z0-9_]*)` CHARACTER SET [A-Za-z0-9_]+ COLLATE [A-Za-z0-9_]+;$",
    )
    .expect("bootstrap declaration pattern must compile");

    let mut databases = Vec::new();
    for line in bootstrap_sql.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with("--") {
            continue;
        }
        let Some(captures) = pattern.captures(line) else {
            bail!(
                "bootstrap SQL contains a statement outside the declared CREATE DATABASE form: {line}"
            );
        };
        let name = captures[1].to_string();
        if databases.contains(&name) {
            bail!("bootstrap SQL declares database '{name}' more than once");
        }
        databases.push(name);
    }
    if databases.is_empty() {
        bail!("bootstrap SQL declares no databases");
    }
    Ok(databases)
}

/// 文件系统校验：引用文件存在且非空、非符号链接；bootstrap 内容与 databases 一致；
/// schema 文件职责纯净（不含建库/授权/INSERT）、唯一 USE 与声明库一致。
/// `docker_root` 为部署包 docker/ 目录（清单路径相对它解析）。
pub fn validate_manifest_files(manifest: &SchemaManifest, docker_root: &Path) -> Result<()> {
    for path in manifest.referenced_paths() {
        let full = docker_root.join(&path);
        let metadata = std::fs::symlink_metadata(&full)
            .with_context(|| format!("manifest file is missing: {}", full.display()))?;
        if metadata.is_symlink() || !metadata.is_file() || metadata.len() == 0 {
            bail!(
                "manifest file must be a non-empty regular file (no symlinks): {}",
                full.display()
            );
        }
    }

    // bootstrap 内容 ↔ databases 声明（名称与顺序完全一致）
    let bootstrap_sql = std::fs::read_to_string(docker_root.join(&manifest.bootstrap.path))
        .with_context(|| format!("failed to read bootstrap {}", manifest.bootstrap.path))?;
    let declared = bootstrap_declared_databases(&bootstrap_sql)?;
    let expected: Vec<&str> = manifest
        .databases
        .iter()
        .map(|db| db.name.as_str())
        .collect();
    if declared != expected {
        bail!(
            "bootstrap SQL databases {declared:?} must match manifest databases {expected:?} (same order)"
        );
    }

    // schema 文件职责纯净：不含建库/授权/FLUSH/INSERT（这些属于 bootstrap/permissions/seeds）
    for schema in &manifest.schemas {
        let sql = std::fs::read_to_string(docker_root.join(&schema.path))
            .with_context(|| format!("failed to read schema {}", schema.path))?;
        for line in sql.lines() {
            let trimmed = line.trim();
            if trimmed.is_empty() || trimmed.starts_with("--") {
                continue;
            }
            let upper = trimmed.to_ascii_uppercase();
            if upper.starts_with("CREATE DATABASE")
                || upper.starts_with("GRANT")
                || upper.starts_with("FLUSH")
                || upper.starts_with("INSERT")
            {
                bail!(
                    "schema {} must not contain CREATE DATABASE/GRANT/FLUSH/INSERT statements (found: {trimmed})",
                    schema.path
                );
            }
        }
        let template = parse_schema_template(&sql)
            .with_context(|| format!("schema {} failed to parse", schema.path))?;
        if template.database != schema.database {
            bail!(
                "schema {} USE `{}` does not match its declared database '{}'",
                schema.path,
                template.database,
                schema.database
            );
        }
    }

    Ok(())
}

/// 存量迁移编排：bootstrap（幂等建库）→ 应用账号授权 → 按 schemas 逐库 Live Diff。
///
/// - bootstrap/permissions 独立执行，不受表差异控制；任何一步失败立即停止；
/// - `first_install_seeds` 不在此执行（首次安装由 MySQL initdb 完成）；
/// - `bootstrap_only` 数据库不出现在 schemas 中，不做表结构同步；
/// - 重试安全：全部语句幂等，Live Diff 每次按数据库实际状态重算。
///
/// `app_user` 为官方镜像创建的应用账号（manifest.permissions.user_env 指向的
/// 环境变量值）；不允许为空或 root。
pub async fn run_manifest_migration(
    executor: &MySqlExecutor,
    manifest: &SchemaManifest,
    docker_root: &Path,
    app_user: &str,
) -> Result<MultiDbDiffResult> {
    let app_user = app_user.trim();
    if app_user.is_empty() || app_user.eq_ignore_ascii_case("root") {
        bail!(
            "application account (from {}) must be a non-empty non-root user for permissions",
            manifest.permissions.user_env
        );
    }

    // 1) 幂等建库（独立于表差异）
    let bootstrap_sql = std::fs::read_to_string(docker_root.join(&manifest.bootstrap.path))
        .with_context(|| format!("failed to read bootstrap {}", manifest.bootstrap.path))?;
    executor
        .execute_diff_sql_once(&bootstrap_sql)
        .await
        .context("manifest bootstrap (CREATE DATABASE IF NOT EXISTS) failed")?;
    info!(
        databases = ?manifest.databases.iter().map(|db| db.name.as_str()).collect::<Vec<_>>(),
        "Manifest bootstrap applied (idempotent CREATE DATABASE)"
    );

    // 2) 应用账号授权（对 bootstrap 声明的全部库）
    let escaped_user = app_user.replace('\'', "''");
    let mut grants = String::new();
    for database in &manifest.databases {
        grants.push_str(&format!(
            "GRANT ALL PRIVILEGES ON `{}`.* TO '{escaped_user}'@'%';\n",
            database.name
        ));
    }
    grants.push_str("FLUSH PRIVILEGES;\n");
    executor
        .execute_diff_sql_once(&grants)
        .await
        .context("manifest permissions (GRANT) failed")?;

    // 3) 按 schemas 逐库 Live Diff（schema 文件只含唯一 USE + 目标表结构）
    let mut templates = Vec::with_capacity(manifest.schemas.len());
    for schema in &manifest.schemas {
        let sql = std::fs::read_to_string(docker_root.join(&schema.path))
            .with_context(|| format!("failed to read schema {}", schema.path))?;
        templates.push(
            parse_schema_template(&sql)
                .with_context(|| format!("schema {} failed to parse", schema.path))?,
        );
    }
    generate_live_schema_diff_multi(executor, &templates, "target version")
        .await
        .map_err(|error| anyhow::anyhow!("{error}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    pub fn valid_manifest_json() -> String {
        r#"{
  "contract_version": 1,
  "requires": { "cli_capability": "mysql-schema-manifest-v1" },
  "mysql_target": { "service": "mysql", "internal_port": 3306 },
  "application_connections": [
    { "service": "backend", "role": "primary", "database": "agent_platform",
      "host": "mysql", "port": 3306,
      "environment": { "host": null, "port": "MYSQL_PORT", "database": "MYSQL_DATABASE" } },
    { "service": "nuwax-im-business", "role": "primary", "database": "nuwax_im",
      "host": "mysql", "port": 3306,
      "environment": { "host": "IM_DB_HOST", "port": "IM_DB_PORT", "database": "IM_DB_NAME" } }
  ],
  "databases": [
    { "name": "agent_platform", "bootstrap_only": false },
    { "name": "agent_custom_table", "bootstrap_only": true },
    { "name": "nuwax_im", "bootstrap_only": false }
  ],
  "bootstrap": { "path": "config/init_mysql_databases.sql", "idempotent": true,
                 "initdb_target": "00_init_mysql_databases.sql" },
  "permissions": { "path": "config/init_mysql_permissions.sh", "user_env": "MYSQL_USER",
                   "databases": "bootstrap", "initdb_target": "01_init_mysql_permissions.sh" },
  "schemas": [
    { "database": "agent_platform", "path": "config/init_mysql.sql", "initdb_target": "10_init_mysql.sql" },
    { "database": "nuwax_im", "path": "config/init_mysql_im.sql", "initdb_target": "20_init_mysql_im.sql" }
  ],
  "first_install_seeds": [
    { "database": "agent_platform", "path": "config/init_mysql_data.sql",
      "first_install_only": true, "initdb_target": "30_init_mysql_data.sql" }
  ]
}"#
        .to_string()
    }

    #[test]
    fn parses_and_structurally_validates_the_real_contract() {
        let manifest = parse_schema_manifest(&valid_manifest_json()).expect("valid manifest");
        assert_eq!(
            manifest.database_names(),
            vec![
                "agent_platform".to_string(),
                "agent_custom_table".to_string(),
                "nuwax_im".to_string()
            ]
        );
        assert_eq!(manifest.schemas.len(), 2);
    }

    #[test]
    fn rejects_unsupported_contract_and_capability() {
        let unsupported =
            valid_manifest_json().replace("\"contract_version\": 1", "\"contract_version\": 2");
        assert!(parse_schema_manifest(&unsupported).is_err());

        let wrong_capability =
            valid_manifest_json().replace("mysql-schema-manifest-v1", "mysql-schema-manifest-v2");
        assert!(parse_schema_manifest(&wrong_capability).is_err());
    }

    #[test]
    fn rejects_schema_for_bootstrap_only_or_missing_database() {
        let mut manifest = parse_schema_manifest(&valid_manifest_json()).expect("valid");
        // bootstrap_only 库出现固定 schema
        manifest.schemas.push(SchemaDecl {
            database: "agent_custom_table".to_string(),
            path: "config/custom.sql".to_string(),
            initdb_target: "40_custom.sql".to_string(),
        });
        assert!(validate_manifest_structure(&manifest).is_err());

        // schema 引用未声明的库
        let mut manifest = parse_schema_manifest(&valid_manifest_json()).expect("valid");
        manifest.schemas.push(SchemaDecl {
            database: "unknown_db".to_string(),
            path: "config/unknown.sql".to_string(),
            initdb_target: "41_unknown.sql".to_string(),
        });
        assert!(validate_manifest_structure(&manifest).is_err());
    }

    #[test]
    fn rejects_duplicate_initdb_targets_and_unsafe_paths() {
        let mut manifest = parse_schema_manifest(&valid_manifest_json()).expect("valid");
        manifest.schemas[1].initdb_target = manifest.schemas[0].initdb_target.clone();
        assert!(validate_manifest_structure(&manifest).is_err());

        let mut manifest = parse_schema_manifest(&valid_manifest_json()).expect("valid");
        manifest.schemas[0].path = "config/../escape.sql".to_string();
        assert!(validate_manifest_structure(&manifest).is_err());

        let mut manifest = parse_schema_manifest(&valid_manifest_json()).expect("valid");
        manifest.databases[0].name = "1bad-name".to_string();
        assert!(validate_manifest_structure(&manifest).is_err());
    }

    #[test]
    fn bootstrap_declared_databases_parses_order_and_rejects_extras() {
        let sql = "-- comment\n\
                   CREATE DATABASE IF NOT EXISTS `agent_platform` CHARACTER SET utf8mb4 COLLATE utf8mb4_unicode_ci;\n\
                   CREATE DATABASE IF NOT EXISTS `nuwax_im` CHARACTER SET utf8mb4 COLLATE utf8mb4_0900_ai_ci;\n";
        assert_eq!(
            bootstrap_declared_databases(sql).expect("parse"),
            vec!["agent_platform".to_string(), "nuwax_im".to_string()]
        );

        assert!(bootstrap_declared_databases("CREATE TABLE t (id INT);").is_err());
        assert!(bootstrap_declared_databases("-- nothing").is_err());
    }

    #[test]
    fn validate_files_checks_bootstrap_consistency_and_schema_purity() {
        let directory = tempfile::tempdir().expect("tempdir");
        let root = directory.path();
        std::fs::create_dir_all(root.join("config")).expect("mkdir");

        let manifest = parse_schema_manifest(&valid_manifest_json()).expect("valid");
        std::fs::write(
            root.join("config/init_mysql_databases.sql"),
            "CREATE DATABASE IF NOT EXISTS `agent_platform` CHARACTER SET utf8mb4 COLLATE utf8mb4_unicode_ci;\n\
             CREATE DATABASE IF NOT EXISTS `agent_custom_table` CHARACTER SET utf8mb4 COLLATE utf8mb4_unicode_ci;\n\
             CREATE DATABASE IF NOT EXISTS `nuwax_im` CHARACTER SET utf8mb4 COLLATE utf8mb4_0900_ai_ci;\n",
        )
        .expect("write");
        std::fs::write(
            root.join("config/init_mysql_permissions.sh"),
            "#!/bin/sh\nexit 0\n",
        )
        .expect("write");
        std::fs::write(
            root.join("config/init_mysql.sql"),
            "USE agent_platform;\nCREATE TABLE `users` (`id` bigint NOT NULL, PRIMARY KEY (`id`));\n",
        )
        .expect("write");
        std::fs::write(
            root.join("config/init_mysql_im.sql"),
            "USE `nuwax_im`;\nCREATE TABLE `im_msg` (`id` bigint NOT NULL, PRIMARY KEY (`id`));\n",
        )
        .expect("write");
        std::fs::write(
            root.join("config/init_mysql_data.sql"),
            "USE agent_platform;\nINSERT INTO `card` (`id`) VALUES (1);\n",
        )
        .expect("write");

        validate_manifest_files(&manifest, root).expect("valid files");

        // USE 与声明库不一致
        std::fs::write(
            root.join("config/init_mysql_im.sql"),
            "USE `agent_platform`;\nCREATE TABLE `im_msg` (`id` bigint NOT NULL, PRIMARY KEY (`id`));\n",
        )
        .expect("write");
        assert!(validate_manifest_files(&manifest, root).is_err());

        // schema 内嵌建库语句（拆分后职责违规）
        std::fs::write(
            root.join("config/init_mysql_im.sql"),
            "CREATE DATABASE IF NOT EXISTS `nuwax_im` CHARACTER SET utf8mb4 COLLATE utf8mb4_0900_ai_ci;\n\
             USE `nuwax_im`;\nCREATE TABLE `im_msg` (`id` bigint NOT NULL, PRIMARY KEY (`id`));\n",
        )
        .expect("write");
        assert!(validate_manifest_files(&manifest, root).is_err());

        // bootstrap 与 manifest 声明不一致
        std::fs::write(
            root.join("config/init_mysql_im.sql"),
            "USE `nuwax_im`;\nCREATE TABLE `im_msg` (`id` bigint NOT NULL, PRIMARY KEY (`id`));\n",
        )
        .expect("write");
        std::fs::write(
            root.join("config/init_mysql_databases.sql"),
            "CREATE DATABASE IF NOT EXISTS `agent_platform` CHARACTER SET utf8mb4 COLLATE utf8mb4_unicode_ci;\n",
        )
        .expect("write");
        assert!(validate_manifest_files(&manifest, root).is_err());
    }
}
