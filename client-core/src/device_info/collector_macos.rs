//! macOS 采集器：ioreg / sysctl / ifconfig 子进程 + 纯解析函数。
//! 磁盘序列号默认不采集（system_profiler 需 2–5 秒，收益低，设计文档 §3.2）。

use crate::device_info::{DisplayInfo, IdentityFields};
use anyhow::Result;
use std::process::Command;

/// 采集身份字段
pub fn collect_identity() -> Result<IdentityFields> {
    let machine_id = ioreg_value("IOPlatformUUID");
    let dmi_uuid = ioreg_value("IOPlatformSerialNumber");
    Ok(IdentityFields {
        machine_id,
        dmi_uuid,
        disk_serial: None,
        primary_mac: primary_mac(),
    })
}

/// 展示类信息：hostname / CPU 型号 / 内存 GB
pub fn read_display_info() -> DisplayInfo {
    DisplayInfo {
        hostname: run_capture("hostname", &[]).map(|s| s.trim().to_string()),
        cpu_model: run_capture("sysctl", &["-n", "machdep.cpu.brand_string"])
            .map(|s| s.trim().to_string()),
        memory_gb: run_capture("sysctl", &["-n", "hw.memsize"])
            .and_then(|s| s.trim().parse::<u64>().ok())
            .map(|bytes| bytes / (1024 * 1024 * 1024))
            .unwrap_or(0),
    }
}

/// 执行命令取 stdout（失败/非零退出码 → None）
fn run_capture(program: &str, args: &[&str]) -> Option<String> {
    let output = Command::new(program).args(args).output().ok()?;
    if !output.status.success() {
        return None;
    }
    Some(String::from_utf8_lossy(&output.stdout).into_owned())
}

/// `ioreg -rd1 -c IOPlatformExpertDevice` 输出中解析指定键
fn ioreg_value(key: &str) -> Option<String> {
    let output = run_capture("ioreg", &["-rd1", "-c", "IOPlatformExpertDevice"])?;
    parse_ioreg_value(&output, key)
}

/// 主网卡 MAC：en0 优先，回退第一个 en*
fn primary_mac() -> Option<String> {
    for iface in ["en0", "en1", "en2"] {
        if let Some(output) = run_capture("ifconfig", &[iface])
            && let Some(mac) = parse_ifconfig_ether(&output)
        {
            return Some(mac);
        }
    }
    None
}

// ---------- 纯解析函数（全平台可测） ----------

/// ioreg 输出形如：`    "IOPlatformUUID" = "0E8D5F8E-..."`。
/// `<"...">`（二进制数据形式）与无引号形式均跳过，仅接受字符串键值。
pub fn parse_ioreg_value(output: &str, key: &str) -> Option<String> {
    let prefix = format!("\"{key}\"");
    for line in output.lines() {
        let trimmed = line.trim();
        let Some(rest) = trimmed.strip_prefix(&prefix) else {
            continue;
        };
        let Some(rest) = rest.trim_start().strip_prefix('=') else {
            continue;
        };
        let rest = rest.trim();
        // 仅接受 `"value"` 字符串形式；`<...>` 数据形式与本键无关
        if let Some(value) = rest.strip_prefix('"').and_then(|s| s.strip_suffix('"')) {
            return Some(value.to_string());
        }
    }
    None
}

/// ifconfig 输出中的 `ether aa:bb:cc:dd:ee:ff` 行
pub fn parse_ifconfig_ether(output: &str) -> Option<String> {
    output
        .lines()
        .find_map(|line| line.trim().strip_prefix("ether "))
        .map(|v| v.trim().to_string())
        .filter(|v| !v.is_empty())
}

#[cfg(test)]
mod tests {
    use super::*;

    const IOREG_SAMPLE: &str = r#"+
-o IOPlatformExpertDevice  {
    "IOPlatformSerialNumber" = "C02X1234ABCD"
    "IOPlatformUUID" = "0E8D5F8E-6C1A-4B2D-9E3F-A1B2C3D4E5F6"
    "board-id" = <"Mac-7BA5B2D9E42F94AC">
}
"#;

    #[test]
    fn parses_ioreg_uuid_and_serial() {
        assert_eq!(
            parse_ioreg_value(IOREG_SAMPLE, "IOPlatformUUID").as_deref(),
            Some("0E8D5F8E-6C1A-4B2D-9E3F-A1B2C3D4E5F6")
        );
        assert_eq!(
            parse_ioreg_value(IOREG_SAMPLE, "IOPlatformSerialNumber").as_deref(),
            Some("C02X1234ABCD")
        );
        assert_eq!(parse_ioreg_value(IOREG_SAMPLE, "board-id").as_deref(), None);
    }

    #[test]
    fn parses_ifconfig_ether() {
        let sample = "en0: flags=8863<UP,BROADCAST,SMART,RUNNING,SIMPLEX,MULTICAST> mtu 1500\n\toptions=400<CHANNEL_IO>\n\tether aa:bb:cc:11:22:33\n\tinet6 fe80::1%en0 prefixlen 64 scopeid 0x4\n";
        assert_eq!(
            parse_ifconfig_ether(sample).as_deref(),
            Some("aa:bb:cc:11:22:33")
        );
        assert_eq!(parse_ifconfig_ether("lo0: flags=8049\n"), None);
    }

    // 冒烟：真实机器上字段可采集（CI macOS runner 上同样有效）
    #[test]
    fn live_collection_smoke() {
        let fields = collect_identity().expect("macos collect");
        // MacBook/iMac 至少有 IOPlatformUUID 与 en0
        assert!(fields.machine_id.is_some(), "IOPlatformUUID missing");
        assert!(fields.present_count() >= 1);
    }
}
