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
    if !env_path.exists() {
        anyhow::bail!(
            "{}",
            t!(
                "device_info_cmd.env_not_found",
                path = env_path.display().to_string()
            )
        );
    }

    let freeze_path = consts::get_fingerprint_file_path();
    let resolved =
        store::resolve(&freeze_path, refresh).context("Failed to resolve device fingerprint")?;

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
    let compose_file = client_core::constants::docker::get_compose_file_path();
    if compose_file.exists()
        && let Ok(content) = std::fs::read_to_string(&compose_file)
        && !content.contains(consts::ENV_KEY_DEVICE_ID)
    {
        warn!(
            compose = %compose_file.display(),
            "docker-compose.yml does not reference DEVICE_ID; backend will not receive the fingerprint"
        );
    }

    Ok(DeviceEnvOutcome {
        resolved,
        env_path: env_path.to_path_buf(),
    })
}

/// 写入单个身份字段哈希；字段未采集（不在 field_hashes 中）则跳过
fn upsert_field_hash(
    env_manager: &mut EnvManager,
    env_key: &str,
    field_name: &str,
    resolved: &ResolvedFingerprint,
) -> Result<()> {
    if let Some(hash) = resolved.fingerprint.field_hashes.get(field_name) {
        env_manager.upsert_variable(env_key, hash, QuoteType::None)?;
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
}
