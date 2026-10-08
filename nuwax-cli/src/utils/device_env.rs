//! 设备指纹注入 `.env`：冻结加载/生成 + EnvManager upsert。
//! 被 `prepare_docker_services`、`start_docker_services` 与 `device-info --apply`
//! 三处共用（设计文档 §6.3 / §7）。

use anyhow::{Context, Result};
use client_core::constants::device_info as consts;
use client_core::device_info::store::{self, ResolvedFingerprint};
use rust_i18n::t;
use std::path::{Path, PathBuf};
use tracing::{info, warn};

use crate::utils::env_manager::{EnvManager, QuoteType};

/// 注入结果
#[derive(Debug)]
pub struct DeviceEnvOutcome {
    pub resolved: ResolvedFingerprint,
    pub env_path: PathBuf,
}

/// 使用默认部署路径（docker/.env）注入；`.env` 不存在时 Fail Fast。
/// 错误统一包装 i18n 文案，调用方（部署/启动/命令）无需各自包装。
pub fn ensure_device_env(refresh: bool) -> Result<DeviceEnvOutcome> {
    let env_path = client_core::constants::docker::get_env_file_path();
    ensure_device_env_at(&env_path, refresh).map_err(|e| {
        anyhow::anyhow!(
            "{}",
            t!("device_info_cmd.inject_failed", error = e.to_string())
        )
    })
}

/// 指定 `.env` 路径注入（幂等：每次写入冻结值，可自愈用户手改）。
pub fn ensure_device_env_at(env_path: &Path, refresh: bool) -> Result<DeviceEnvOutcome> {
    ensure_device_env_with_paths(
        env_path,
        &client_core::constants::docker::get_compose_file_path(),
        &consts::get_fingerprint_file_path(),
        refresh,
    )
}

/// Deployment entry point: the selected manager owns these paths. The frozen
/// record keeps its existing deployment-root location for compatibility.
pub fn ensure_device_env_with_paths(
    env_path: &Path,
    compose_path: &Path,
    freeze_path: &Path,
    refresh: bool,
) -> Result<DeviceEnvOutcome> {
    if !env_path.exists() {
        anyhow::bail!(
            "{}",
            t!(
                "device_info_cmd.env_not_found",
                path = env_path.display().to_string()
            )
        );
    }

    let resolved =
        store::resolve(freeze_path, refresh).context("Failed to resolve device fingerprint")?;

    write_device_env(env_path, compose_path, resolved)
}

fn write_device_env(
    env_path: &Path,
    compose_path: &Path,
    resolved: ResolvedFingerprint,
) -> Result<DeviceEnvOutcome> {
    let mut env_manager = EnvManager::new();
    env_manager
        .load(env_path)
        .with_context(|| format!("Failed to load .env: {}", env_path.display()))?;
    env_manager.ensure_marker(consts::ENV_MANAGED_MARKER);

    // 逐字段写入（设计文档 §6.1：Java 侧经 yml 原生 map 占位符 + 宽松绑定读取，
    // 零 JSON 解析）。缺失字段不写键——yml 占位符默认空串即"未采集"
    env_manager.upsert_variable(
        consts::ENV_KEY_DEVICE_ID,
        resolved.device_id(),
        QuoteType::None,
    )?;
    upsert_field_hash(
        &mut env_manager,
        consts::ENV_KEY_FIELD_MACHINE_ID,
        consts::FIELD_MACHINE_ID,
        &resolved,
    )?;
    upsert_field_hash(
        &mut env_manager,
        consts::ENV_KEY_FIELD_DMI_UUID,
        consts::FIELD_DMI_UUID,
        &resolved,
    )?;
    upsert_field_hash(
        &mut env_manager,
        consts::ENV_KEY_FIELD_DISK_SERIAL,
        consts::FIELD_DISK_SERIAL,
        &resolved,
    )?;
    upsert_field_hash(
        &mut env_manager,
        consts::ENV_KEY_FIELD_PRIMARY_MAC,
        consts::FIELD_PRIMARY_MAC,
        &resolved,
    )?;

    let env = &resolved.environment;
    // 自由文本（可能含空格/特殊字符）用单引号包裹并清洗（§6.1）
    env_manager.upsert_variable(
        consts::ENV_KEY_INFO_HOSTNAME,
        &sanitize_display(&env.hostname),
        QuoteType::Single,
    )?;
    env_manager.upsert_variable(consts::ENV_KEY_INFO_OS, &env.os, QuoteType::None)?;
    env_manager.upsert_variable(consts::ENV_KEY_INFO_ARCH, &env.arch, QuoteType::None)?;
    if let Some(cpu_model) = env.cpu_model.as_deref().map(sanitize_display) {
        env_manager.upsert_variable(
            consts::ENV_KEY_INFO_CPU_MODEL,
            &cpu_model,
            QuoteType::Single,
        )?;
    } else {
        env_manager.remove_variable(consts::ENV_KEY_INFO_CPU_MODEL);
    }
    env_manager.upsert_variable(
        consts::ENV_KEY_INFO_CPU_CORES,
        &env.cpu_cores.to_string(),
        QuoteType::None,
    )?;
    env_manager.upsert_variable(
        consts::ENV_KEY_INFO_MEMORY_GB,
        &env.memory_gb.to_string(),
        QuoteType::None,
    )?;
    env_manager.upsert_variable(
        consts::ENV_KEY_INFO_FINGERPRINT_VERSION,
        &consts::FP_VERSION.to_string(),
        QuoteType::None,
    )?;
    env_manager.upsert_variable(
        consts::ENV_KEY_INFO_COLLECTED_AT,
        &sanitize_display(&resolved.collected_at),
        QuoteType::Single,
    )?;
    env_manager.upsert_variable(
        consts::ENV_KEY_INFO_WSL,
        env.wsl.to_string().as_str(),
        QuoteType::None,
    )?;
    env_manager.upsert_variable(
        consts::ENV_KEY_INFO_CONTAINERIZED,
        env.containerized.to_string().as_str(),
        QuoteType::None,
    )?;
    env_manager
        .save()
        .with_context(|| format!("Failed to save .env: {}", env_path.display()))?;

    info!(
        device_id = resolved.device_id(),
        path = %env_path.display(),
        "Device fingerprint injected into .env"
    );

    // 自定义 compose 缺透传时给出提示，避免"写了却没生效"的静默失败（设计文档 §7）
    if compose_path.exists()
        && let Ok(content) = std::fs::read_to_string(compose_path)
        && !content.contains(consts::ENV_KEY_DEVICE_ID)
    {
        warn!(
            compose = %compose_path.display(),
            "docker-compose.yml does not reference DEVICE_ID; backend will not receive the fingerprint"
        );
    }

    Ok(DeviceEnvOutcome {
        resolved,
        env_path: env_path.to_path_buf(),
    })
}

/// Write an available field, or remove every stale definition if it is absent.
fn upsert_field_hash(
    env_manager: &mut EnvManager,
    env_key: &str,
    field_name: &str,
    resolved: &ResolvedFingerprint,
) -> Result<()> {
    if let Some(hash) = resolved.fingerprint.field_hashes.get(field_name) {
        env_manager.upsert_variable(env_key, hash, QuoteType::None)?;
    } else {
        env_manager.remove_variable(env_key);
    }
    Ok(())
}

/// 展示类自由文本清洗：值被单引号包裹写入 .env，内嵌 `'` 或控制字符
/// 会破坏 dotenvy / compose 解析（设计文档 §6.1）
fn sanitize_display(raw: &str) -> String {
    raw.chars()
        .filter(|c| !c.is_control() && *c != '\'')
        .collect::<String>()
        .trim()
        .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sanitize_display_removes_quotes_and_controls() {
        assert_eq!(sanitize_display("prod's\t01"), "prods01");
        assert_eq!(sanitize_display("Intel(R) Xeon(R)"), "Intel(R) Xeon(R)");
        assert_eq!(sanitize_display("  padded  "), "padded");
    }

    #[test]
    fn ensure_env_fails_fast_when_env_missing() {
        let dir = tempfile::tempdir().expect("tempdir");
        let env_path = dir.path().join(".env");
        let err = ensure_device_env_at(&env_path, false).expect_err("must fail");
        assert!(format!("{err:#}").contains(".env"));
    }

    fn collected_without_optional_fields() -> ResolvedFingerprint {
        use client_core::device_info::{DeviceEnvironment, IdentityFields, fingerprint};
        ResolvedFingerprint {
            fingerprint: fingerprint::compute(&IdentityFields {
                machine_id: Some("synthetic-machine".to_string()),
                primary_mac: Some("22:33:44:55:66:77".to_string()),
                ..Default::default()
            })
            .unwrap(),
            environment: DeviceEnvironment {
                hostname: "synthetic-host".to_string(),
                os: "linux".to_string(),
                arch: "x86_64".to_string(),
                cpu_model: None,
                cpu_cores: 16,
                memory_gb: 128,
                wsl: false,
                containerized: false,
            },
            collected_at: "2026-10-08T10:00:00+08:00".to_string(),
            source: store::ResolvedSource::Refreshed,
            drift: None,
        }
    }

    #[test]
    fn injection_preserves_credentials_removes_stale_fields_and_is_idempotent() -> Result<()> {
        let directory = tempfile::tempdir()?;
        let env_path = directory.path().join("prod.env");
        let compose_path = directory.path().join("prod.yml");
        let unchanged = concat!(
            "export MYSQL_PASSWORD=\"synthetic\\\"quote\\$value\" # preserved\r\n",
            "EXTRA='one # literal\r\ntwo'\r\n",
        );
        std::fs::write(
            &env_path,
            format!(
                "{unchanged}DEVICE_FIELDS_DISK_SERIAL=old\r\nexport DEVICE_FIELDS_DISK_SERIAL=older\r\nDEVICE_FIELDS_DMI_UUID=stale\r\nDEVICE_INFO_CPU_MODEL='stale CPU'\r\n"
            ),
        )?;
        std::fs::write(
            &compose_path,
            "services:\n  backend:\n    image: alpine\n    environment:\n      - DEVICE_ID=${DEVICE_ID}\n",
        )?;
        let resolved = collected_without_optional_fields();
        let outcome = write_device_env(&env_path, &compose_path, resolved.clone())?;
        assert_eq!(outcome.env_path, env_path);
        let first = std::fs::read_to_string(&env_path)?;
        assert!(first.starts_with(unchanged));
        let values = dotenvy::from_path_iter(&env_path)?
            .collect::<std::result::Result<std::collections::HashMap<_, _>, _>>()?;
        assert_eq!(values["MYSQL_PASSWORD"], "synthetic\"quote$value");
        assert_eq!(values["DEVICE_ID"], resolved.device_id());
        assert_eq!(values["DEVICE_INFO_CPU_CORES"], "16");
        assert_eq!(values["DEVICE_INFO_MEMORY_GB"], "128");
        for absent in [
            consts::ENV_KEY_FIELD_DISK_SERIAL,
            consts::ENV_KEY_FIELD_DMI_UUID,
            consts::ENV_KEY_INFO_CPU_MODEL,
        ] {
            assert!(!values.contains_key(absent));
            assert!(!first.contains(&format!("{absent}=")));
        }
        write_device_env(&env_path, &compose_path, resolved)?;
        assert_eq!(std::fs::read_to_string(&env_path)?, first);
        assert_eq!(first.matches(consts::ENV_MANAGED_MARKER).count(), 1);
        Ok(())
    }

    #[test]
    fn missing_custom_env_does_not_create_or_refresh_state() {
        let directory = tempfile::tempdir().unwrap();
        let freeze_path = directory.path().join("state/fingerprint.json");
        assert!(
            ensure_device_env_with_paths(
                &directory.path().join("custom.env"),
                &directory.path().join("custom.yml"),
                &freeze_path,
                true,
            )
            .is_err()
        );
        assert!(!freeze_path.exists());
    }
}
