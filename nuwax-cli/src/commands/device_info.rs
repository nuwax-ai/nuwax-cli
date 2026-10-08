//! `device-info` 命令：展示宿主机设备指纹，`--apply` 注入 `.env`
//! （设计文档 §7）。不依赖 CliApp/数据库，在 main.rs 中特判分发。

use anyhow::Result;
use client_core::constants::device_info as consts;
use client_core::device_info::store::{self, ResolvedFingerprint};
use rust_i18n::t;
use std::path::Path;
use tracing::warn;

/// 运行 device-info 命令
pub async fn run_device_info(json: bool, apply: bool, refresh: bool) -> Result<()> {
    let (resolved, applied_env) = if apply {
        let outcome = crate::utils::device_env::ensure_device_env(refresh)?;
        (outcome.resolved, Some(outcome.env_path))
    } else {
        let freeze_path = consts::get_fingerprint_file_path();
        (store::resolve(&freeze_path, refresh)?, None)
    };
    if json {
        print_json(&resolved);
    } else {
        print_report(&resolved, applied_env.as_deref());
    }
    Ok(())
}

fn print_json(resolved: &ResolvedFingerprint) {
    let value = serde_json::json!({
        "fp_version": consts::FP_VERSION,
        "device_id": resolved.device_id(),
        "field_hashes": resolved.fingerprint.field_hashes,
        "environment": resolved.environment,
        "collected_at": resolved.collected_at,
        "source": resolved.source.description(),
        "drift": resolved.drift,
    });
    println!(
        "{}",
        serde_json::to_string_pretty(&value).unwrap_or_else(|_| "{}".to_string())
    );
}

fn print_report(resolved: &ResolvedFingerprint, applied_env: Option<&Path>) {
    let env = &resolved.environment;
    println!("{}", t!("device_info_cmd.report_title"));
    println!("{}", "─".repeat(64));
    println!(
        "  {}: {}",
        t!("device_info_cmd.device_id"),
        resolved.device_id()
    );
    println!(
        "  {}: {}",
        t!("device_info_cmd.source"),
        resolved.source.description()
    );
    println!(
        "  {}: {}",
        t!("device_info_cmd.collected_at"),
        resolved.collected_at
    );
    println!("{}", "─".repeat(64));

    println!("  {}:", t!("device_info_cmd.field_hashes"));
    for (name, hash) in &resolved.fingerprint.field_hashes {
        println!("    {name}: {hash}");
    }

    println!("{}", "─".repeat(64));
    println!("  {}:", t!("device_info_cmd.environment"));
    println!("    hostname   : {}", env.hostname);
    println!("    os/arch    : {} / {}", env.os, env.arch);
    if let Some(cpu) = &env.cpu_model {
        println!("    cpu        : {cpu}");
    }
    println!("    cores/mem  : {}c / {}GB", env.cpu_cores, env.memory_gb);
    if env.wsl {
        println!("    note       : {}", t!("device_info_cmd.wsl_note"));
    }
    if env.containerized {
        println!(
            "    note       : {}",
            t!("device_info_cmd.containerized_note")
        );
    }

    if let Some(drift) = &resolved.drift {
        println!("{}", "─".repeat(64));
        warn!(
            changes = ?drift.changes,
            "Device fingerprint drift detected; frozen device_id kept"
        );
        println!("  ⚠️  {}", t!("device_info_cmd.drift_warning"));
        for (name, change) in &drift.changes {
            println!("    {name}: {change}");
        }
        println!("  {}", t!("device_info_cmd.drift_hint"));
    }

    if let Some(env_path) = applied_env {
        println!("{}", "─".repeat(64));
        println!(
            "  ✅ {}",
            t!(
                "device_info_cmd.applied",
                path = env_path.display().to_string()
            )
        );
    }
}
