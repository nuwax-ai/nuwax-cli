//! 冻结持久化：首次采集结果写入 `./data/device_fingerprint.json`，
//! 后续重采与之比对——漂移时维持冻结值（授权连续性优先），
//! 仅显式 `--refresh` 才重新绑定（设计文档 §5）。

use crate::atomic_file::{PermissionsPolicy, write_atomic};
use crate::device_info::{CollectedDevice, Fingerprint, collect, fingerprint};
use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::Path;
use tracing::warn;

/// 冻结文件记录
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FrozenRecord {
    pub fp_version: usize,
    /// 冻结时间（RFC3339）
    pub collected_at: String,
    /// 规范化后的原始身份值（仅存在于本文件与本地展示，不进入 .env）
    pub raw: BTreeMap<String, String>,
    pub device_id: String,
    pub field_hashes: BTreeMap<String, String>,
    pub environment: crate::device_info::DeviceEnvironment,
    /// 最近一次检测到的漂移（字段级差异）
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub drift: Option<DriftInfo>,
}

/// 漂移详情
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DriftInfo {
    pub detected_at: String,
    /// 字段 → 变化说明（changed / unreadable_now / newly_readable）
    pub changes: BTreeMap<String, String>,
}

/// `resolve` 的产出：实际生效的指纹与其来源
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ResolvedSource {
    /// 本次新采集并冻结
    NewlyFrozen,
    /// 与冻结值一致
    FrozenConfirmed,
    /// 检测到漂移但维持冻结值（授权连续性优先）
    FrozenDrifted,
    /// --refresh 强制重新绑定
    Refreshed,
}

impl ResolvedSource {
    /// 供日志/展示使用的来源描述
    pub fn description(&self) -> &'static str {
        match self {
            ResolvedSource::NewlyFrozen => "newly frozen",
            ResolvedSource::FrozenConfirmed => "frozen (confirmed)",
            ResolvedSource::FrozenDrifted => "frozen (drift detected, keeping frozen id)",
            ResolvedSource::Refreshed => "refreshed",
        }
    }
}

#[derive(Debug, Clone)]
pub struct ResolvedFingerprint {
    pub fingerprint: Fingerprint,
    pub environment: crate::device_info::DeviceEnvironment,
    pub collected_at: String,
    pub source: ResolvedSource,
    pub drift: Option<DriftInfo>,
}

impl ResolvedFingerprint {
    /// device_id（即冻结值）
    pub fn device_id(&self) -> &str {
        &self.fingerprint.device_id
    }
}

/// 解析当前生效的指纹（采集当前机器信息后走统一逻辑）。
pub fn resolve(freeze_path: &Path, refresh: bool) -> Result<ResolvedFingerprint> {
    let current = collect().context("Failed to collect current device info")?;
    resolve_with_collected(freeze_path, &current, refresh)
}

/// 核心解析逻辑（采集结果可注入，供测试使用真实路径）：
/// 1. `refresh == true` → 忽略冻结，重新绑定
/// 2. 无冻结文件 → 冻结当前值
/// 3. 有冻结文件 → 与本次采集比对：一致 → 沿用；不一致 → 维持冻结值并记录 drift
///
/// 冻结文件损坏 / 内部不一致（device_id 与 field_hashes 对不上，疑似篡改）→
/// Fail Fast 报错并提示 `device-info --refresh` 修复。
pub(crate) fn resolve_with_collected(
    freeze_path: &Path,
    current: &CollectedDevice,
    refresh: bool,
) -> Result<ResolvedFingerprint> {
    let current_fp =
        fingerprint::compute(&current.raw).context("Failed to compute device fingerprint")?;
    let now = chrono::Local::now().to_rfc3339();

    let existing: Option<FrozenRecord> = if refresh {
        None
    } else {
        load_optional(freeze_path)?
    };

    let Some(frozen) = existing else {
        let record = FrozenRecord {
            fp_version: crate::constants::device_info::FP_VERSION,
            collected_at: now.clone(),
            raw: normalized_raw(&current.raw),
            device_id: current_fp.device_id.clone(),
            field_hashes: current_fp.field_hashes.clone(),
            environment: current.environment.clone(),
            drift: None,
        };
        save(freeze_path, &record)?;
        return Ok(ResolvedFingerprint {
            fingerprint: current_fp,
            environment: current.environment.clone(),
            collected_at: now,
            source: if refresh {
                ResolvedSource::Refreshed
            } else {
                ResolvedSource::NewlyFrozen
            },
            drift: None,
        });
    };

    // 与冻结值比对：一致 → 沿用；漂移 → 维持冻结值（授权连续性优先）并记录 drift
    let drift =
        diff_fields(&frozen.field_hashes, &current_fp.field_hashes).map(|changes| DriftInfo {
            detected_at: now.clone(),
            changes,
        });
    if let Some(info) = &drift {
        warn!(
            fields = ?info.changes.keys().collect::<Vec<_>>(),
            "Device fingerprint drift detected; keeping frozen device_id for license continuity \
             (run 'nuwax-cli device-info --refresh' to rebind)"
        );
        let mut drifted_record = frozen.clone();
        drifted_record.drift = Some(info.clone());
        save(freeze_path, &drifted_record)?;
    }

    Ok(ResolvedFingerprint {
        fingerprint: Fingerprint {
            device_id: frozen.device_id.clone(),
            field_hashes: frozen.field_hashes.clone(),
        },
        // Only identity and its binding time are frozen. Resource limits and
        // display information must describe the host at the current collection.
        environment: current.environment.clone(),
        collected_at: frozen.collected_at.clone(),
        source: if drift.is_some() {
            ResolvedSource::FrozenDrifted
        } else {
            ResolvedSource::FrozenConfirmed
        },
        drift,
    })
}

/// 读取冻结文件；文件不存在返回 None。
/// 文件存在但损坏 / 内部不一致 → 错误（Fail Fast，附修复指引）。
pub fn load_optional(path: &Path) -> Result<Option<FrozenRecord>> {
    if !path.exists() {
        return Ok(None);
    }
    let content = std::fs::read_to_string(path)
        .with_context(|| format!("Failed to read device fingerprint file: {}", path.display()))?;
    let record: FrozenRecord = serde_json::from_str(&content).with_context(|| {
        format!(
            "Device fingerprint file is corrupted: {}. \
             Fix: run 'nuwax-cli device-info --refresh' to regenerate it",
            path.display()
        )
    })?;

    if record.fp_version != crate::constants::device_info::FP_VERSION {
        anyhow::bail!(
            "Device fingerprint file has unsupported version {}: {}. \
             Fix: run 'nuwax-cli device-info --refresh'",
            record.fp_version,
            path.display()
        );
    }

    // 完整性自检：冻结的字段哈希重新组合应得到冻结的 device_id，
    // 不一致说明文件被篡改（改了哈希没改 id，或反之）
    if recompute_from_frozen(&record) != record.device_id {
        anyhow::bail!(
            "Device fingerprint file is internally inconsistent (stored device_id does not \
             match its own field hashes): {}. Fix: run 'nuwax-cli device-info --refresh'",
            path.display()
        );
    }
    Ok(Some(record))
}

/// 由冻结记录的 field_hashes 重算 device_id（组装规则与采集路径共用
/// fingerprint::device_id_from_field_hashes，见其文档）
fn recompute_from_frozen(record: &FrozenRecord) -> String {
    let mut entries: Vec<(&String, &String)> = record.field_hashes.iter().collect();
    entries.sort_by_key(|(name, _)| fingerprint::field_order(name));
    fingerprint::device_id_from_field_hashes(
        entries
            .into_iter()
            .map(|(name, hash)| (name.as_str(), hash.as_str())),
    )
}

/// 保存冻结文件（Unix 下权限 0600，仅当前用户可读写）
pub fn save(path: &Path, record: &FrozenRecord) -> Result<()> {
    let content = serde_json::to_string_pretty(record).context("Failed to serialize record")?;
    write_atomic(path, content.as_bytes(), PermissionsPolicy::Private)
        .with_context(|| format!("Failed to save fingerprint file: {}", path.display()))
}

/// 字段级差异：changed / unreadable_now / newly_readable
fn diff_fields(
    frozen: &BTreeMap<String, String>,
    current: &BTreeMap<String, String>,
) -> Option<BTreeMap<String, String>> {
    let mut changes = BTreeMap::new();
    for (name, frozen_hash) in frozen {
        match current.get(name) {
            Some(cur) if cur == frozen_hash => {}
            Some(_) => {
                changes.insert(name.clone(), "changed".to_string());
            }
            None => {
                changes.insert(name.clone(), "unreadable_now".to_string());
            }
        }
    }
    for name in current.keys() {
        if !frozen.contains_key(name) {
            changes.insert(name.clone(), "newly_readable".to_string());
        }
    }
    if changes.is_empty() {
        None
    } else {
        Some(changes)
    }
}

/// 规范化后的原始值（冻结文件保存规范化形式，重放计算无需再次规范化）
fn normalized_raw(raw: &crate::device_info::IdentityFields) -> BTreeMap<String, String> {
    use crate::constants::device_info as consts;
    let mut out = BTreeMap::new();
    if let Some(v) = &raw.machine_id {
        let n = fingerprint::normalize_machine_id(v);
        if !n.is_empty() {
            out.insert(consts::FIELD_MACHINE_ID.to_string(), n);
        }
    }
    if let Some(v) = &raw.dmi_uuid {
        let n = fingerprint::normalize_uuid(v);
        if !n.is_empty() {
            out.insert(consts::FIELD_DMI_UUID.to_string(), n);
        }
    }
    if let Some(v) = &raw.disk_serial {
        let n = fingerprint::normalize_disk_serial(v);
        if !n.is_empty() {
            out.insert(consts::FIELD_DISK_SERIAL.to_string(), n);
        }
    }
    if let Some(v) = &raw.primary_mac {
        let n = fingerprint::normalize_mac(v);
        if !n.is_empty() {
            out.insert(consts::FIELD_PRIMARY_MAC.to_string(), n);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::device_info::IdentityFields;

    fn test_collected(machine_id: Option<&str>, mac: Option<&str>) -> CollectedDevice {
        CollectedDevice {
            raw: IdentityFields {
                machine_id: machine_id.map(str::to_string),
                dmi_uuid: None,
                disk_serial: None,
                primary_mac: mac.map(str::to_string),
            },
            environment: crate::device_info::DeviceEnvironment {
                hostname: "test-host".into(),
                os: "linux".into(),
                arch: "x86_64".into(),
                cpu_model: Some("Test CPU".into()),
                cpu_cores: 8,
                memory_gb: 64,
                wsl: false,
                containerized: false,
            },
        }
    }

    fn temp_path() -> (tempfile::TempDir, std::path::PathBuf) {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("device_fingerprint.json");
        (dir, path)
    }

    #[test]
    fn first_collection_freezes() {
        let (_dir, path) = temp_path();
        let device = test_collected(Some("aaa111"), Some("AA:BB:CC:DD:EE:FF"));
        let resolved = resolve_with_collected(&path, &device, false).expect("resolve");
        assert_eq!(resolved.source, ResolvedSource::NewlyFrozen);
        assert!(path.exists());
        // 0600 权限
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(&path).expect("meta").permissions().mode();
            assert_eq!(mode & 0o777, 0o600);
        }
    }

    #[test]
    fn consistent_reread_confirms_frozen() {
        let (_dir, path) = temp_path();
        let device = test_collected(Some("aaa111"), Some("AA:BB:CC:DD:EE:FF"));
        let first = resolve_with_collected(&path, &device, false).expect("first");
        let second = resolve_with_collected(&path, &device, false).expect("second");
        assert_eq!(second.source, ResolvedSource::FrozenConfirmed);
        assert_eq!(first.fingerprint.device_id, second.fingerprint.device_id);
    }

    #[test]
    fn environment_changes_preserve_identity_and_binding_time() {
        let (_dir, path) = temp_path();
        let before = test_collected(Some("aaa111"), Some("AA:BB:CC:DD:EE:FF"));
        let first = resolve_with_collected(&path, &before, false).expect("freeze");
        let mut current = before.clone();
        current.environment.hostname = "renamed-host".into();
        current.environment.cpu_cores = 16;
        current.environment.memory_gb = 128;
        current.environment.cpu_model = None;
        let resolved = resolve_with_collected(&path, &current, false).expect("resolve");
        assert_eq!(resolved.source, ResolvedSource::FrozenConfirmed);
        assert_eq!(resolved.fingerprint, first.fingerprint);
        assert_eq!(resolved.collected_at, first.collected_at);
        assert_eq!(resolved.environment.hostname, "renamed-host");
        assert_eq!(resolved.environment.cpu_cores, 16);
        assert_eq!(resolved.environment.memory_gb, 128);
        assert!(resolved.environment.cpu_model.is_none());
        // The original snapshot stays available for local diagnostics.
        let frozen = load_optional(&path).expect("load").expect("record");
        assert_eq!(frozen.environment.cpu_cores, 8);
    }

    #[test]
    fn drift_keeps_frozen_id() {
        let (_dir, path) = temp_path();
        let before = test_collected(Some("aaa111"), Some("AA:BB:CC:DD:EE:FF"));
        resolve_with_collected(&path, &before, false).expect("freeze");

        // 硬件变更：MAC 变了
        let after = test_collected(Some("aaa111"), Some("11:22:33:44:55:66"));
        let resolved = resolve_with_collected(&path, &after, false).expect("drift resolve");
        assert_eq!(resolved.source, ResolvedSource::FrozenDrifted);
        // device_id 维持冻结值（与变更前一致）
        let fp_before = fingerprint::compute(&before.raw).expect("fp before");
        assert_eq!(resolved.fingerprint.device_id, fp_before.device_id);
        assert!(resolved.drift.is_some());
        // drift 已写入冻结文件
        let stored = load_optional(&path).expect("load").expect("some");
        assert!(stored.drift.is_some());
    }

    #[test]
    fn refresh_rebinds() {
        let (_dir, path) = temp_path();
        let before = test_collected(Some("aaa111"), Some("AA:BB:CC:DD:EE:FF"));
        resolve_with_collected(&path, &before, false).expect("freeze");

        let after = test_collected(Some("bbb222"), Some("11:22:33:44:55:66"));
        let resolved = resolve_with_collected(&path, &after, true).expect("refresh");
        assert_eq!(resolved.source, ResolvedSource::Refreshed);
        let fp_after = fingerprint::compute(&after.raw).expect("fp after");
        assert_eq!(resolved.fingerprint.device_id, fp_after.device_id);
    }

    #[test]
    fn corrupted_file_fails_fast_with_hint() {
        let (_dir, path) = temp_path();
        std::fs::write(&path, "{ not json").expect("write");
        let err = load_optional(&path).expect_err("must fail");
        let msg = format!("{err:#}");
        assert!(
            msg.contains("--refresh"),
            "error should mention fix hint: {msg}"
        );
    }

    #[test]
    fn tampered_file_fails_integrity_check() {
        let (_dir, path) = temp_path();
        let device = test_collected(Some("aaa111"), Some("AA:BB:CC:DD:EE:FF"));
        let mut record = {
            // 先走真实冻结路径
            resolve_with_collected(&path, &device, false).expect("freeze");
            load_optional(&path).expect("load").expect("some")
        };
        // 篡改：改 field_hashes 但不改 device_id
        record
            .field_hashes
            .insert("machine_id".to_string(), "0".repeat(64));
        save(&path, &record).expect("save tampered");
        let err = load_optional(&path).expect_err("must fail");
        assert!(format!("{err:#}").contains("inconsistent"));
    }

    #[test]
    fn missing_file_returns_none() {
        let (_dir, path) = temp_path();
        assert!(load_optional(&path).expect("load").is_none());
    }

    #[test]
    fn partial_read_records_unreadable_drift() {
        // 非 root 运行时 DMI 读不到 → unreadable_now，不视为一致
        let (_dir, path) = temp_path();
        let full = CollectedDevice {
            raw: IdentityFields {
                machine_id: Some("aaa111".into()),
                dmi_uuid: Some("0e8d-5f8e".into()),
                disk_serial: None,
                primary_mac: Some("AA:BB:CC:DD:EE:FF".into()),
            },
            environment: test_collected(None, None).environment,
        };
        resolve_with_collected(&path, &full, false).expect("freeze");

        let partial = test_collected(Some("aaa111"), Some("AA:BB:CC:DD:EE:FF"));
        let resolved = resolve_with_collected(&path, &partial, false).expect("resolve");
        assert_eq!(resolved.source, ResolvedSource::FrozenDrifted);
        let drift = resolved.drift.expect("drift");
        assert_eq!(
            drift.changes.get("dmi_uuid").map(String::as_str),
            Some("unreadable_now")
        );
    }
}
