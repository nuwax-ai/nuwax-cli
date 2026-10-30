//! 指纹算法：规范化 → 逐字段 SHA-256 → canonical 拼接 → 组合哈希。
//!
//! 全部为纯函数，便于跨平台单测。**算法一旦发布即冻结**：
//! 任何改动都会使存量设备的 device_id 变化，需升版本号（v2 并行）。
//! 黄金向量测试（golden vectors）用于防止算法被意外改动。

use crate::constants::device_info as consts;
use crate::device_info::{Fingerprint, IdentityFields};
use anyhow::Result;
use sha2::{Digest, Sha256};

/// SHA-256 → 小写 hex
pub fn sha256_hex(input: &[u8]) -> String {
    use std::fmt::Write as _;
    let digest = Sha256::digest(input);
    let mut out = String::with_capacity(digest.len() * 2);
    for byte in digest {
        let _ = write!(out, "{byte:02x}");
    }
    out
}

/// 通用规范化：去包裹引号、折叠连续空白、trim
fn normalize_base(raw: &str) -> String {
    let trimmed = raw.trim();
    let unquoted = trimmed
        .strip_prefix('"')
        .and_then(|s| s.strip_suffix('"'))
        .unwrap_or(trimmed);
    unquoted.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// machine_id：小写 hex（systemd machine-id / Windows MachineGuid / macOS IOPlatformUUID）
pub fn normalize_machine_id(raw: &str) -> String {
    normalize_base(raw).to_lowercase()
}

/// DMI UUID / 序列号类：统一小写（与 machine_id 同类处理）
pub fn normalize_uuid(raw: &str) -> String {
    normalize_base(raw).to_lowercase()
}

/// 磁盘序列号：统一大写（厂商大小写混杂）
pub fn normalize_disk_serial(raw: &str) -> String {
    normalize_base(raw).to_uppercase()
}

/// MAC：去 `:` / `-` 分隔符，小写
pub fn normalize_mac(raw: &str) -> String {
    normalize_base(raw).replace([':', '-'], "").to_lowercase()
}

/// canonical 字段优先级顺序（与 compute 的 candidates 数组一致）
pub(crate) fn field_order(name: &str) -> u8 {
    match name {
        consts::FIELD_MACHINE_ID => 0,
        consts::FIELD_DMI_UUID => 1,
        consts::FIELD_DISK_SERIAL => 2,
        consts::FIELD_PRIMARY_MAC => 3,
        _ => 4,
    }
}

/// 由"已按 canonical 顺序排列"的 (字段名, 哈希) 序列组装 device_id。
/// compute（采集路径）与 store（冻结文件完整性自检）**共用此实现**，
/// 保证两处的组装规则永远一致——完整性自检正是依赖这一点。
pub(crate) fn device_id_from_field_hashes<'a>(
    fields: impl IntoIterator<Item = (&'a str, &'a str)>,
) -> String {
    let mut canonical = String::from(consts::CANONICAL_DOMAIN);
    for (name, hash) in fields {
        canonical.push('|');
        canonical.push_str(name);
        canonical.push('=');
        canonical.push_str(hash);
    }
    format!(
        "{}:{}",
        consts::DEVICE_ID_PREFIX,
        sha256_hex(canonical.as_bytes())
    )
}

/// 由原始身份字段计算指纹。缺失或规范化后为空的字段直接跳过（设计文档 §4.2）。
/// canonical 按固定字段优先级顺序单趟拼接：域分隔前缀 + `name=hash` 以 `|` 相连，
/// 编码无歧义且不受容器迭代序影响。
pub fn compute(raw: &IdentityFields) -> Result<Fingerprint> {
    let machine_id = raw.machine_id.as_deref().map(normalize_machine_id);
    let dmi_uuid = raw.dmi_uuid.as_deref().map(normalize_uuid);
    let disk_serial = raw.disk_serial.as_deref().map(normalize_disk_serial);
    let primary_mac = raw.primary_mac.as_deref().map(normalize_mac);

    let candidates = [
        (consts::FIELD_MACHINE_ID, machine_id),
        (consts::FIELD_DMI_UUID, dmi_uuid),
        (consts::FIELD_DISK_SERIAL, disk_serial),
        (consts::FIELD_PRIMARY_MAC, primary_mac),
    ];

    // 按优先级顺序收集 (字段名, 哈希)，缺失或规范化后为空的字段直接跳过
    let ordered: Vec<(&'static str, String)> = candidates
        .into_iter()
        .filter_map(|(name, value)| {
            let value = value?;
            if value.is_empty() {
                return None;
            }
            Some((name, sha256_hex(value.as_bytes())))
        })
        .collect();

    if ordered.is_empty() {
        anyhow::bail!("No identity field available after normalization");
    }

    let device_id = device_id_from_field_hashes(ordered.iter().map(|(n, h)| (*n, h.as_str())));
    let field_hashes = ordered
        .into_iter()
        .map(|(name, hash)| (name.to_string(), hash))
        .collect();

    Ok(Fingerprint {
        device_id,
        field_hashes,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    // ---------- 黄金向量（与设计文档示例一致，锁死算法） ----------
    // 来源：nuwax-cli/docs/DEVICE_FINGERPRINT_DESIGN.md 及会话内 shasum -a 256 实算
    const GOLDEN_MACHINE_ID: &str = "7f05a4e28c9b3d6f1a0e5c8d2b4f7a91";
    const GOLDEN_DMI_UUID: &str = "0E8D5F8E-6C1A-4B2D-9E3F-A1B2C3D4E5F6";
    const GOLDEN_DISK_SERIAL: &str = "wd-wcc4n7xrz2va";
    const GOLDEN_MAC: &str = "AA:BB:CC:11:22:33";

    const GOLDEN_H_MACHINE_ID: &str =
        "3fb59a395a93a73cfdf93528db13642d25fa7ed166c201be8f59ca9232c8aff9";
    const GOLDEN_H_DMI_UUID: &str =
        "c8743baf574a672e72e17e4eb99507b12600982e4eec8acb7cd2930320084ba4";
    const GOLDEN_H_DISK_SERIAL: &str =
        "f28d3bca917672d4d3c52eb6aa9476941451de660f0229ce805e80bb07984cbd";
    const GOLDEN_H_MAC: &str = "035f79cf6426abcb5701c1fea6128a48ca00c6d673dda11fe917eff3065ada37";
    const GOLDEN_DEVICE_ID: &str =
        "v1:aff7c6444dcd6d7bfb98890d0daaa6d004e626157e58c7dbd62ba172e44c345e";

    fn golden_raw() -> IdentityFields {
        IdentityFields {
            machine_id: Some(GOLDEN_MACHINE_ID.to_string()),
            dmi_uuid: Some(GOLDEN_DMI_UUID.to_string()),
            disk_serial: Some(GOLDEN_DISK_SERIAL.to_string()),
            primary_mac: Some(GOLDEN_MAC.to_string()),
        }
    }

    #[test]
    fn golden_vector_full_fingerprint() {
        let fp = compute(&golden_raw()).expect("golden compute");
        assert_eq!(
            fp.field_hashes
                .get(consts::FIELD_MACHINE_ID)
                .map(String::as_str),
            Some(GOLDEN_H_MACHINE_ID)
        );
        assert_eq!(
            fp.field_hashes
                .get(consts::FIELD_DMI_UUID)
                .map(String::as_str),
            Some(GOLDEN_H_DMI_UUID)
        );
        assert_eq!(
            fp.field_hashes
                .get(consts::FIELD_DISK_SERIAL)
                .map(String::as_str),
            Some(GOLDEN_H_DISK_SERIAL)
        );
        assert_eq!(
            fp.field_hashes
                .get(consts::FIELD_PRIMARY_MAC)
                .map(String::as_str),
            Some(GOLDEN_H_MAC)
        );
        assert_eq!(fp.device_id, GOLDEN_DEVICE_ID);
    }

    #[test]
    fn golden_vector_deterministic() {
        // 同输入两次计算结果必须一致（防迭代序/随机性引入）
        let a = compute(&golden_raw()).expect("a");
        let b = compute(&golden_raw()).expect("b");
        assert_eq!(a, b);
    }

    // ---------- 规范化 ----------

    #[test]
    fn normalize_strips_quotes_whitespace_and_newline() {
        assert_eq!(normalize_machine_id("  \"ABC123\"\n "), "abc123");
        assert_eq!(normalize_uuid(" 0E8D-5F8E "), "0e8d-5f8e");
        assert_eq!(normalize_disk_serial(" wd-wcc\n"), "WD-WCC");
        assert_eq!(normalize_mac("AA:BB-CC:11-22-33"), "aabbcc112233");
    }

    #[test]
    fn normalize_collapses_inner_whitespace() {
        assert_eq!(normalize_base("a  b\t\tc"), "a b c");
    }

    // ---------- 缺字段与空值 ----------

    #[test]
    fn missing_fields_are_skipped_deterministically() {
        let partial = IdentityFields {
            machine_id: Some(GOLDEN_MACHINE_ID.to_string()),
            dmi_uuid: None,
            disk_serial: None,
            primary_mac: Some(GOLDEN_MAC.to_string()),
        };
        let fp = compute(&partial).expect("partial compute");
        assert_eq!(fp.field_hashes.len(), 2);
        assert!(!fp.field_hashes.contains_key(consts::FIELD_DMI_UUID));

        // 同样输入 → 同样输出
        let fp2 = compute(&partial).expect("partial compute 2");
        assert_eq!(fp, fp2);
    }

    #[test]
    fn all_fields_missing_fails_fast() {
        let empty = IdentityFields::default();
        assert!(compute(&empty).is_err());
    }

    #[test]
    fn whitespace_only_field_treated_as_missing() {
        let raw = IdentityFields {
            machine_id: Some("   ".to_string()),
            dmi_uuid: None,
            disk_serial: None,
            primary_mac: None,
        };
        assert!(compute(&raw).is_err());
    }

    // ---------- 字段顺序无关性（采集顺序不影响结果） ----------

    // canonical 字段顺序（machine_id → dmi_uuid → disk_serial → primary_mac）由
    // compute 内部硬编码保证，黄金向量测试已覆盖四字段全在时的顺序锁定

    #[test]
    fn sha256_hex_known_vector() {
        assert_eq!(
            sha256_hex(b"abc"),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
    }
}
