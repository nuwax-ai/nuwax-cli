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
            if let Some(serial) = normalize_serial_like(value) {
                return Some(serial);
            }
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
/// 取首个格式合法且具有 TCP/IP transport 的地址。
/// 断开的网卡仍会有合法 MAC，必须检查第二列；传输标识不依赖系统语言。
pub fn parse_getmac_csv(output: &str) -> Option<String> {
    for line in output.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        let Some((addr, transport)) = two_csv_fields(trimmed) else {
            continue;
        };
        const TRANSPORT_PREFIX: &str = "\\Device\\Tcpip_";
        if !transport
            .get(..TRANSPORT_PREFIX.len())
            .is_some_and(|prefix| prefix.eq_ignore_ascii_case(TRANSPORT_PREFIX))
            || transport.len() == TRANSPORT_PREFIX.len()
        {
            continue;
        }
        // getmac 地址用 '-' 分隔；零地址或广播地址不能作为设备身份。
        let cleaned = addr.replace(['-', ':'], "");
        if cleaned.len() == 12
            && cleaned.chars().all(|c| c.is_ascii_hexdigit())
            && cleaned != "000000000000"
            && !cleaned.eq_ignore_ascii_case("ffffffffffff")
        {
            return Some(addr);
        }
    }
    None
}

/// getmac 无 /v 时输出两个带引号的 CSV 字段，支持 CSV 的双引号转义。
fn two_csv_fields(line: &str) -> Option<(String, String)> {
    fn field(input: &str) -> Option<(String, &str)> {
        let input = input.strip_prefix('"')?;
        let mut value = String::new();
        let mut characters = input.char_indices().peekable();
        while let Some((index, character)) = characters.next() {
            if character == '"' {
                if characters.peek().is_some_and(|(_, next)| *next == '"') {
                    characters.next();
                    value.push('"');
                } else {
                    return Some((value, &input[index + 1..]));
                }
            } else {
                value.push(character);
            }
        }
        None
    }
    let (address, rest) = field(line)?;
    let (transport, trailing) = field(rest.strip_prefix(',')?.trim_start())?;
    if !trailing.trim().is_empty() {
        return None;
    }
    Some((address, transport))
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
    fn wmic_skips_empty_and_placeholder_devices() {
        assert_eq!(
            parse_wmic_value(
                "SerialNumber=\r\nSerialNumber=Default string\r\nSerialNumber=DISK-42\r\n",
                "SerialNumber"
            )
            .as_deref(),
            Some("DISK-42")
        );
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
    fn getmac_valid_mac_with_disconnected_transport_is_skipped() {
        let output = concat!(
            "\"AA-BB-CC-11-22-33\",\"Media disconnected\"\r\n",
            "\"11-22-33-44-55-66\",\"媒体已断开连接\"\r\n",
            "\"00-00-00-00-00-00\",\"\\Device\\Tcpip_{ZERO}\"\r\n",
            "\"22-33-44-55-66-77\",\"\\Device\\Tcpip_{CONNECTED}\"\r\n"
        );
        assert_eq!(
            parse_getmac_csv(output).as_deref(),
            Some("22-33-44-55-66-77")
        );
    }

    #[test]
    fn getmac_requires_exactly_two_csv_fields() {
        assert_eq!(
            parse_getmac_csv("\"AA-BB-CC-11-22-33\",\"\\Device\\Tcpip_{X}\",\"extra\""),
            None
        );
        assert_eq!(
            parse_getmac_csv("\"AA-BB-CC-11-22-33\",\"\\Device\\Tcpip_{X}"),
            None
        );
    }

    #[test]
    fn getmac_invalid_address_rejected() {
        assert_eq!(parse_getmac_csv("\"not-an-address\",\"x\"\r\n"), None);
        assert_eq!(parse_getmac_csv("\"\r\n"), None);
    }
}
