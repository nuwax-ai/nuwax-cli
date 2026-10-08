//! Windows 采集器：注册表（winreg）+ PowerShell/wmic/getmac 子进程（设计文档 §3.3）。
//! 解析逻辑在 `parsers.rs`（全平台可测）；本文件仅命令与注册表访问。

use crate::device_info::DisplayInfo;
use crate::device_info::IdentityFields;
use crate::device_info::parsers;
use anyhow::Result;
use std::process::Command;
use winreg::RegKey;
use winreg::enums::HKEY_LOCAL_MACHINE;

/// 采集身份字段
pub fn collect_identity() -> Result<IdentityFields> {
    Ok(IdentityFields {
        machine_id: read_machine_guid(),
        dmi_uuid: read_dmi_serial(),
        disk_serial: read_disk_serial(),
        primary_mac: read_primary_mac(),
    })
}

/// 展示类信息：hostname / CPU 型号 / 内存 GB
pub fn read_display_info() -> DisplayInfo {
    DisplayInfo {
        hostname: std::env::var_os("COMPUTERNAME")
            .map(|n| n.to_string_lossy().into_owned())
            .or_else(|| run_capture("hostname", &[]).map(|s| s.trim().to_string())),
        cpu_model: std::env::var("PROCESSOR_IDENTIFIER").ok(),
        memory_gb: run_capture(
            "powershell",
            &[
                "-NoProfile",
                "-Command",
                "(Get-CimInstance Win32_ComputerSystem).TotalPhysicalMemory",
            ],
        )
        .and_then(|s| s.trim().parse::<u64>().ok())
        .map(|bytes| bytes / (1024 * 1024 * 1024))
        .unwrap_or(0),
    }
}

/// 注册表 HKLM\SOFTWARE\Microsoft\Cryptography\MachineGuid
/// （Users 可读无需管理员；winreg 的 predef 以 KEY_WOW64_64KEY 打开，规避 32 位重定向）
fn read_machine_guid() -> Option<String> {
    let hklm = RegKey::predef(HKEY_LOCAL_MACHINE);
    let crypto = hklm.open_subkey("SOFTWARE\\Microsoft\\Cryptography").ok()?;
    let guid: String = crypto.get_value("MachineGuid").ok()?;
    let trimmed = guid.trim();
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed.to_string())
    }
}

/// BIOS/主板序列号：PowerShell Get-CimInstance 为主（Win11 24H2 起移除 wmic），wmic 兜底
fn read_dmi_serial() -> Option<String> {
    if let Some(output) = run_capture(
        "powershell",
        &[
            "-NoProfile",
            "-Command",
            "(Get-CimInstance Win32_BIOS).SerialNumber",
        ],
    ) {
        if let Some(v) = parsers::parse_powershell_single(&output) {
            return Some(v);
        }
    }
    if let Some(output) = run_capture("wmic", &["bios", "get", "serialnumber", "/value"]) {
        if let Some(v) = parsers::parse_wmic_value(&output, "SerialNumber") {
            return Some(v);
        }
    }
    // 主板序列号兜底（部分 OEM BIOS 为空但主板有序列号）
    run_capture(
        "powershell",
        &[
            "-NoProfile",
            "-Command",
            "(Get-CimInstance Win32_BaseBoard).SerialNumber",
        ],
    )
    .and_then(|output| parsers::parse_powershell_single(&output))
}

/// 第一块物理磁盘序列号：Get-PhysicalDisk 为主，wmic diskdrive 兜底
fn read_disk_serial() -> Option<String> {
    if let Some(output) = run_capture(
        "powershell",
        &[
            "-NoProfile",
            "-Command",
            "Get-PhysicalDisk | Select-Object -First 1 -ExpandProperty SerialNumber",
        ],
    ) {
        if let Some(v) = parsers::parse_powershell_single(&output) {
            return Some(v);
        }
    }
    run_capture("wmic", &["diskdrive", "get", "SerialNumber", "/value"])
        .and_then(|output| parsers::parse_wmic_value(&output, "SerialNumber"))
}

/// 主网卡 MAC：getmac CSV 输出的首个已连接物理地址
fn read_primary_mac() -> Option<String> {
    let output = run_capture("getmac", &["/fo", "csv", "/nh"])?;
    parsers::parse_getmac_csv(&output)
}

/// 执行命令取 stdout（失败/非零退出码 → None）
fn run_capture(program: &str, args: &[&str]) -> Option<String> {
    let output = Command::new(program).args(args).output().ok()?;
    if !output.status.success() {
        return None;
    }
    Some(String::from_utf8_lossy(&output.stdout).into_owned())
}

#[cfg(all(test, target_os = "windows"))]
mod tests {
    use super::*;

    // 冒烟：真实 Windows 上字段可采集
    #[test]
    fn live_collection_smoke() {
        let fields = collect_identity().expect("windows collect");
        assert!(fields.machine_id.is_some(), "MachineGuid missing");
        assert!(fields.present_count() >= 1);
    }
}
