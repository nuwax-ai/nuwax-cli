//! Compatibility for packages predating mysql-schema-manifest.json.
//! The platform schema is mandatory; additional schemas are selected by their
//! presence or by the candidate Compose mounts, rather than by CLI version.

use anyhow::{Context, Result};
use client_core::constants::sql;
use client_core::sql_diff::{SchemaTemplate, parse_schema_template};
use std::path::{Component, Path, PathBuf};

pub(crate) fn schema_paths(
    compose: Option<&str>,
    contains: impl Fn(&str) -> bool,
) -> Result<Vec<String>> {
    let mut paths: Vec<String> = sql::CRITICAL_UPGRADE_FILES
        .iter()
        .map(|path| (*path).to_string())
        .collect();
    paths.extend(
        sql::OPTIONAL_SCHEMA_SQL_FILES
            .iter()
            .filter(|path| contains(path))
            .map(|path| (*path).to_string()),
    );
    if let Some(compose) = compose {
        for source in client_core::container::preflight::collect_bind_mount_sources(compose)? {
            let path = Path::new(&source);
            let Some(name) = path.file_name().and_then(|name| name.to_str()) else {
                continue;
            };
            // Seeds are first-install data, not a schema migration target.
            if !name.starts_with("init_mysql")
                || !name.ends_with(".sql")
                || name == "init_mysql_data.sql"
            {
                continue;
            }
            if path.components().any(|component| {
                matches!(
                    component,
                    Component::ParentDir | Component::RootDir | Component::Prefix(_)
                )
            }) {
                anyhow::bail!("Legacy schema mount must be inside the package: {source}");
            }
            let normalized = path
                .components()
                .filter_map(|part| match part {
                    Component::Normal(name) => Some(name.to_string_lossy().into_owned()),
                    _ => None,
                })
                .collect::<Vec<_>>()
                .join("/");
            paths.push(normalized);
        }
    }
    paths.sort();
    paths.dedup();
    Ok(paths)
}

/// Operation names are portable package-relative paths, never host paths.
pub(crate) fn validate_changed_paths(changed_paths: &[String]) -> Result<()> {
    for changed in changed_paths {
        let path = Path::new(changed);
        let has_name = path
            .components()
            .any(|part| matches!(part, Component::Normal(_)));
        if !has_name
            || changed.contains(['\\', ':'])
            || path.components().any(|part| {
                matches!(
                    part,
                    Component::ParentDir | Component::RootDir | Component::Prefix(_)
                )
            })
        {
            anyhow::bail!("Patch operation must use a safe package-relative path: {changed}");
        }
    }
    Ok(())
}

pub(crate) fn component_entrypoints(compose: &str) -> Result<Vec<&'static str>> {
    let sources = client_core::container::preflight::collect_bind_mount_sources(compose)?;
    let uses = |root: &str| {
        sources
            .iter()
            .any(|source| source == root || source.starts_with(&format!("{root}/")))
    };
    let mut required = Vec::new();
    if uses("im-app") {
        required.extend([
            "im-app/nuwax-im-web-bootstrap.jar",
            "im-app/nuwax-im-gateway-bootstrap.jar",
        ]);
    }
    if uses("repo-collab-app") {
        required.push("repo-collab-app/dist/index.js");
    }
    Ok(required)
}

/// Patch replace/delete operations remove their target before extraction. A
/// retained path is safe only when no operation targets it or an ancestor.
pub(crate) fn can_retain(path: &str, changed_paths: &[String]) -> bool {
    if validate_changed_paths(changed_paths).is_err() {
        return false;
    }
    let target = Path::new(path);
    !changed_paths.iter().any(|changed| {
        let normalized: PathBuf = Path::new(changed)
            .components()
            .filter_map(|part| match part {
                Component::Normal(name) => Some(name),
                _ => None,
            })
            .collect();
        target.starts_with(&normalized) || normalized.starts_with(target)
    })
}

pub(crate) fn parse_templates(
    paths: &[String],
    mut read: impl FnMut(&str) -> Result<String>,
) -> Result<Vec<SchemaTemplate>> {
    let mut templates = Vec::with_capacity(paths.len());
    for path in paths {
        let content =
            read(path).with_context(|| format!("Missing legacy schema template {path}"))?;
        let template = parse_schema_template(&content)
            .with_context(|| format!("Invalid legacy schema template {path}"))?;
        if templates
            .iter()
            .any(|existing: &SchemaTemplate| existing.database == template.database)
        {
            anyhow::bail!(
                "Duplicate legacy schema template for database `{}`",
                template.database
            );
        }
        templates.push(template);
    }
    Ok(templates)
}

pub(crate) fn disk_templates(
    docker_root: &Path,
    compose_path: &Path,
) -> Result<Vec<SchemaTemplate>> {
    let compose = std::fs::read_to_string(compose_path)
        .with_context(|| format!("Failed to read {}", compose_path.display()))?;
    let paths = schema_paths(Some(&compose), |path| docker_root.join(path).exists())?;
    parse_templates(&paths, |path| {
        std::fs::read_to_string(docker_root.join(path)).map_err(Into::into)
    })
}
