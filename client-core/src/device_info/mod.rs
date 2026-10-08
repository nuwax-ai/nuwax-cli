//! 设备指纹模块：采集宿主机设备信息，生成稳定指纹供授权管控使用。
//!
//! 设计文档：docs/DEVICE_FINGERPRINT_DESIGN.md
//!
//! 数据流：各平台采集原始字段 → 规范化/哈希（fingerprint.rs）→
//! 冻结持久化（store.rs）→ 由 nuwax-cli 注入 docker/.env。
//!
//! 平台分层约定：所有平台差异（文件、命令、注册表）都收敛在
//! `collector_{linux,macos,windows}.rs` 中，本模块只做类型定义与统一分发。

pub mod fingerprint;
pub mod parsers;
pub mod store;

#[cfg(target_os = "linux")]
pub mod collector_linux;
#[cfg(target_os = "macos")]
pub mod collector_macos;
#[cfg(target_os = "windows")]
pub mod collector_windows;

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::Path;

/// 身份字段：参与 device_id 计算的原始值（规范化前）。
/// 每个字段都可能因平台差异或权限问题缺失，缺失即跳过。
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct IdentityFields {
    pub machine_id: Option<String>,
    pub dmi_uuid: Option<String>,
    pub disk_serial: Option<String>,
    pub primary_mac: Option<String>,
}

impl IdentityFields {
    /// 已采集到的身份数量
    pub fn present_count(&self) -> usize {
        [
            &self.machine_id,
            &self.dmi_uuid,
            &self.disk_serial,
            &self.primary_mac,
        ]
        .iter()
        .filter(|v| v.is_some())
        .count()
    }
}

/// 展示信息：不参与身份判定，供授权限额与排障使用。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DeviceEnvironment {
    pub hostname: String,
    /// linux / macos / windows
    pub os: String,
    /// x86_64 / aarch64
    pub arch: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cpu_model: Option<String>,
    pub cpu_cores: usize,
    pub memory_gb: u64,
    pub wsl: bool,
    pub containerized: bool,
}

/// 各平台采集器补充的展示类信息（hostname 等平台差异项）
#[derive(Debug, Clone, Default)]
pub struct DisplayInfo {
    pub hostname: Option<String>,
    pub cpu_model: Option<String>,
    pub memory_gb: u64,
}

/// 一次完整采集的结果
#[derive(Debug, Clone)]
pub struct CollectedDevice {
    pub raw: IdentityFields,
    pub environment: DeviceEnvironment,
}

/// 计算后的指纹
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Fingerprint {
    /// 形如 "v1:<64 hex>"
    pub device_id: String,
    /// 字段名 → 规范化值的 SHA-256 hex（BTreeMap 保证 JSON 键序稳定）
    pub field_hashes: BTreeMap<String, String>,
}

/// 注入 DEVICE_INFO 的载荷：环境信息扁平化 + 指纹版本与冻结时间
#[derive(Debug, Clone, Serialize)]
pub struct DeviceInfoPayload {
    #[serde(flatten)]
    pub environment: DeviceEnvironment,
    pub fingerprint_version: usize,
    pub collected_at: String,
}

/// 在当前宿主机上采集设备信息。
///
/// Fail Fast 边界：四个身份字段全部缺失时返回错误——说明宿主机环境异常，
/// 宁可失败也不给出不稳定的 device_id（设计文档 §4.2）。
pub fn collect() -> Result<CollectedDevice> {
    let raw = platform_collect_identity().context("Failed to collect device identity fields")?;
    let environment = collect_environment();

    if raw.present_count() == 0 {
        anyhow::bail!(
            "No device identity field could be collected on this host \
             (machine_id / dmi_uuid / disk_serial / primary_mac all unavailable); \
             refusing to generate an unstable device id"
        );
    }

    Ok(CollectedDevice { raw, environment })
}

/// 环境展示信息：平台差异项由采集器提供，通用项在此汇总
fn collect_environment() -> DeviceEnvironment {
    let display = platform_display_info();
    DeviceEnvironment {
        hostname: display.hostname.unwrap_or_else(|| "unknown".to_string()),
        os: std::env::consts::OS.to_string(),
        arch: std::env::consts::ARCH.to_string(),
        cpu_model: display.cpu_model,
        cpu_cores: num_cpus::get(),
        memory_gb: display.memory_gb,
        wsl: is_wsl(),
        containerized: is_containerized(),
    }
}

fn is_wsl() -> bool {
    std::env::var_os("WSL_DISTRO_NAME").is_some() || std::env::var_os("WSLENV").is_some()
}

fn is_containerized() -> bool {
    // /.dockerenv 由 Docker 在容器内创建；其他运行时（podman 等）遵循同一惯例
    Path::new("/.dockerenv").exists()
}

// ---------- 平台分发（差异实现全部在 collector_* 中） ----------

#[cfg(target_os = "linux")]
fn platform_collect_identity() -> Result<IdentityFields> {
    collector_linux::collect_identity(Path::new("/"))
}

#[cfg(target_os = "macos")]
fn platform_collect_identity() -> Result<IdentityFields> {
    collector_macos::collect_identity()
}

#[cfg(target_os = "windows")]
fn platform_collect_identity() -> Result<IdentityFields> {
    collector_windows::collect_identity()
}

#[cfg(not(any(target_os = "linux", target_os = "macos", target_os = "windows")))]
fn platform_collect_identity() -> Result<IdentityFields> {
    anyhow::bail!(
        "Device fingerprint collection is not supported on this platform: {}",
        std::env::consts::OS
    )
}

#[cfg(target_os = "linux")]
fn platform_display_info() -> DisplayInfo {
    collector_linux::read_display_info(Path::new("/"))
}

#[cfg(target_os = "macos")]
fn platform_display_info() -> DisplayInfo {
    collector_macos::read_display_info()
}

#[cfg(target_os = "windows")]
fn platform_display_info() -> DisplayInfo {
    collector_windows::read_display_info()
}

#[cfg(not(any(target_os = "linux", target_os = "macos", target_os = "windows")))]
fn platform_display_info() -> DisplayInfo {
    DisplayInfo::default()
}
