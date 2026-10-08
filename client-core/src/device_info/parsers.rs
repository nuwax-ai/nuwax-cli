//! Windows 采集输出的纯解析函数。
//! 单独成模块（不 cfg windows）以便解析逻辑在全平台做快照测试，
//! 命令本体仅在 Windows 上执行（设计文档 §9）。

/// wmic `/value` 输出解析：`SerialNumber=ABC123` → Some("ABC123")
/// 忽略空值与占位行
pub fn parse_wmic_value(output: &str, key: &str) -> Option<String> {
    let prefix = format!("{key}=");
    for line in output.lines() {
        let trimmed = line.trim();
        if let Some(rest) = trimmed.strip_prefix(&prefix) {
            let value = rest.trim();
            return normalize_serial_like(value);
        }
    }
    None
}

/// PowerShell 单值输出解析：取首个非空行并做占位符过滤
pub fn parse_powershell_single(output: &str) -> Option<String> {
    let first = output.lines().map(str::trim).find(|l| !l.is_empty())?;
    normalize_serial_like(first)
}

/// `getmac /fo csv /nh` 输出解析：
/// ```text
/// "AA-BB-CC-DD-EE-FF","\Device\Tcpip_{...}"
/// "Media Disconnected","\Device\Tcpip_{...}"
/// ```
/// 取首个格式合法且已连接的物理地址
pub fn parse_getmac_csv(output: &str) -> Option<String> {
    for line in output.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        let Some(addr) = first_csv_field(trimmed) else {
            continue;
        };
        if addr.eq_ignore_ascii_case("Media Disconnected") || addr.eq_ignore_ascii_case("N/A") {
            continue;
        }
        // 蓝牙等非以太网传输通常可由地址格式过滤；getmac 地址用 '-' 分隔
        let cleaned = addr.replace(['-', ':'], "");
        if cleaned.len() == 12 && cleaned.chars().all(|c| c.is_ascii_hexdigit()) {
            return Some(addr.to_string());
        }
    }
    None
}

/// 取 CSV 双字段行（值带双引号）的首个字段；
/// 完整的配对切分本身校验了行形状（两个引号包裹字段）
fn first_csv_field(line: &str) -> Option<&str> {
    let line = line.strip_prefix('"')?;
    let (first, rest) = line.split_once("\",\"")?;
    rest.strip_suffix('"')?;
    Some(first)
}

/// OEM 占位序列号视为缺失：空 / None / Default string / To be filled by O.E.M. 等
pub fn normalize_serial_like(value: &str) -> Option<String> {
    let trimmed = value.trim();
    const PLACEHOLDERS: [&str; 6] = [
        "",
        "none",
        "null",
        "default string",
        "to be filled by o.e.m.",
        "system serial number",
    ];
    if PLACEHOLDERS.contains(&trimmed.to_lowercase().as_str()) {
        return None;
    }
    Some(trimmed.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    const WMIC_BIOS: &str = "\r\n\r\nSerialNumber=PF3XYZ99\r\n\r\n";
    const WMIC_DISK: &str = "\r\n\r\nSerialNumber=      WD-WCC4N7XRZ2VA    \r\n\r\n";

    #[test]
    fn parses_wmic_serial() {
        assert_eq!(
            parse_wmic_value(WMIC_BIOS, "SerialNumber").as_deref(),
            Some("PF3XYZ99")
        );
        assert_eq!(
            parse_wmic_value(WMIC_DISK, "SerialNumber").as_deref(),
            Some("WD-WCC4N7XRZ2VA")
        );
        assert_eq!(parse_wmic_value("Name=value", "Missing"), None);
    }

    #[test]
    fn wmic_placeholder_treated_as_missing() {
        assert_eq!(
            parse_wmic_value("SerialNumber=To Be Filled By O.E.M.", "SerialNumber"),
            None
        );
        assert_eq!(parse_wmic_value("SerialNumber=None", "SerialNumber"), None);
        assert_eq!(
            parse_wmic_value("SerialNumber=Default string", "SerialNumber"),
            None
        );
        assert_eq!(parse_wmic_value("SerialNumber=", "SerialNumber"), None);
    }

    #[test]
    fn parses_powershell_single() {
        assert_eq!(
            parse_powershell_single("\r\nPF3XYZ99\r\n").as_deref(),
            Some("PF3XYZ99")
        );
        assert_eq!(parse_powershell_single(""), None);
        assert_eq!(
            parse_powershell_single("  \r\n  0  \r\n").as_deref(),
            Some("0")
        );
    }

    const GETMAC_CSV: &str = "\"AA-BB-CC-11-22-33\",\"\\Device\\Tcpip_{A1B2C3}\"\r\n\"Media Disconnected\",\"\\Device\\Tcpip_{D4E5F6}\"\r\n\"11-22-33-44-55-66\",\"\\Device\\Tcpip_{G7H8I9}\"\r\n";

    #[test]
    fn parses_getmac_first_connected() {
        assert_eq!(
            parse_getmac_csv(GETMAC_CSV).as_deref(),
            Some("AA-BB-CC-11-22-33")
        );
    }

    #[test]
    fn getmac_skips_disconnected() {
        let only_disconnected = "\"Media Disconnected\",\"\\Device\\Tcpip_{D4}\"\r\n";
        assert_eq!(parse_getmac_csv(only_disconnected), None);
    }

    #[test]
    fn getmac_invalid_address_rejected() {
        assert_eq!(parse_getmac_csv("\"not-an-address\",\"x\"\r\n"), None);
        assert_eq!(parse_getmac_csv("\"\r\n"), None);
    }
}
