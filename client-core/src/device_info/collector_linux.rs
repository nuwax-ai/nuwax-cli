//! Linux 采集器：全部通过 sysfs / procfs 纯文件读取，无子进程、无新依赖。
//! 根路径参数化（默认 `/`），测试时注入 fake sysfs 树（设计文档 §3.1）。

use crate::device_info::{DisplayInfo, IdentityFields};
use anyhow::Result;
use std::fs;
use std::path::Path;

/// 采集身份字段
pub fn collect_identity(sysroot: &Path) -> Result<IdentityFields> {
    Ok(IdentityFields {
        machine_id: read_machine_id(sysroot),
        dmi_uuid: read_dmi_uuid(sysroot),
        disk_serial: read_disk_serial(sysroot),
        primary_mac: read_primary_mac(sysroot),
    })
}

/// 读取文件并 trim；文件不存在 / 为空 → None
fn read_trimmed(path: &Path) -> Option<String> {
    let content = fs::read_to_string(path).ok()?;
    let trimmed = content.trim();
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed.to_string())
    }
}

/// machine-id：/etc/machine-id，回退 /var/lib/dbus/machine-id
fn read_machine_id(root: &Path) -> Option<String> {
    read_trimmed(&root.join("etc/machine-id"))
        .or_else(|| read_trimmed(&root.join("var/lib/dbus/machine-id")))
}

/// DMI UUID：/sys/class/dmi/id/product_uuid（0400 仅 root，非 root 返回 None 容缺）
fn read_dmi_uuid(root: &Path) -> Option<String> {
    read_trimmed(&root.join("sys/class/dmi/id/product_uuid"))
}

/// 磁盘序列号：优先根分区所在磁盘，回退第一个非 loop/ram 块设备
fn read_disk_serial(root: &Path) -> Option<String> {
    root_disk_name(root)
        .and_then(|disk| disk_serial_of(root, &disk))
        .or_else(|| fallback_block_serial(root))
}

/// 解析根磁盘设备名：
/// /proc/self/mountinfo 中挂载点为 `/` 的行取 major:minor，
/// 经 /sys/dev/block/<maj:min> 符号链接归一到磁盘名（纯文件读取，不依赖 findmnt）
fn root_disk_name(root: &Path) -> Option<String> {
    let mountinfo = fs::read_to_string(root.join("proc/self/mountinfo")).ok()?;
    for line in mountinfo.lines() {
        let fields: Vec<&str> = line.split_whitespace().collect();
        // 格式：mountID parentID major:minor root mountpoint [options...] - fstype source superopts
        if fields.len() > 5 && fields[4] == "/" {
            let dev = fields[2];
            // 0:x 为虚拟设备（overlay 等），无法回溯物理磁盘
            if dev.starts_with("0:") {
                return None;
            }
            if let Some(file_name) = block_dev_name(root, dev) {
                return Some(partition_to_disk(&file_name));
            }
            return None;
        }
    }
    None
}

/// /sys/dev/block/<maj:min> 符号链接指向的设备名（分区名，如 sda1 / nvme0n1p2）
fn block_dev_name(root: &Path, major_minor: &str) -> Option<String> {
    let link = root.join("sys/dev/block").join(major_minor);
    let target = fs::read_link(&link).ok()?;
    let name = target.file_name()?.to_string_lossy().into_owned();
    if name.is_empty() { None } else { Some(name) }
}

/// 分区名归一到磁盘名：sda1→sda、nvme0n1p2→nvme0n1、mmcblk0p1→mmcblk0
fn partition_to_disk(part: &str) -> String {
    // "基础名p数字" 形式的分区（nvme/mmc 命名惯例）：nvme0n1p3 / mmcblk0p1
    if let Some(stripped) = strip_partition_suffix(part) {
        return stripped;
    }
    // nvme0n1 / mmcblk0 本身就是整盘名（结尾的 n<数字> 不是分区号）
    if is_whole_disk_name(part) {
        return part.to_string();
    }
    // sda1 / vdb2 → 去掉纯数字后缀（保留字母结尾）
    let trimmed = part.trim_end_matches(|c: char| c.is_ascii_digit());
    if trimmed.is_empty() {
        part.to_string()
    } else {
        trimmed.to_string()
    }
}

/// nvme0n1 / nvme1n2 / mmcblk0 这类"整盘"命名
fn is_whole_disk_name(name: &str) -> bool {
    if let Some(rest) = name.strip_prefix("nvme") {
        // nvme<ctrl>n<ns>
        let Some((ctrl, ns)) = rest.split_once('n') else {
            return false;
        };
        return !ctrl.is_empty()
            && ctrl.chars().all(|c| c.is_ascii_digit())
            && !ns.is_empty()
            && ns.chars().all(|c| c.is_ascii_digit());
    }
    if let Some(rest) = name.strip_prefix("mmcblk") {
        return !rest.is_empty() && rest.chars().all(|c| c.is_ascii_digit());
    }
    false
}

/// 处理 "基础名p数字" 形式的分区（nvme/mmc 命名惯例）：
/// nvme0n1p2 → nvme0n1、mmcblk0p1 → mmcblk0。
/// 要求 'p' 前是数字（nvme/mmc 命名里盘名以数字结尾），避免误剥 sdap1 这类盘名。
fn strip_partition_suffix(name: &str) -> Option<String> {
    let pos = name.rfind('p')?;
    let base = &name[..pos];
    let digits = &name[pos + 1..];
    if !digits.is_empty()
        && digits.chars().all(|c| c.is_ascii_digit())
        && !base.is_empty()
        && base.chars().last().is_some_and(|c| c.is_ascii_digit())
    {
        return Some(base.to_string());
    }
    None
}

/// 磁盘序列号的多路径查找：
/// SATA/SAS/virtio：/sys/block/<disk>/device/serial
/// NVMe：/sys/class/nvme/<ctrl>/serial（nvme0n1 → 控制器 nvme0）
fn disk_serial_of(root: &Path, disk: &str) -> Option<String> {
    if let Some(serial) = read_trimmed(&root.join("sys/block").join(disk).join("device/serial")) {
        return Some(serial);
    }
    if disk.starts_with("nvme") {
        // nvme0n1 → nvme0（去掉命名空间 n\d+）
        let ctrl = {
            let t = disk.trim_end_matches(|c: char| c.is_ascii_digit());
            t.strip_suffix('n').unwrap_or(t).to_string()
        };
        if let Some(serial) = read_trimmed(&root.join("sys/class/nvme").join(ctrl).join("serial")) {
            return Some(serial);
        }
    }
    // 部分虚拟化环境直接暴露在 /sys/block/<disk>/serial
    read_trimmed(&root.join("sys/block").join(disk).join("serial"))
}

/// 回退：遍历 /sys/block，取第一个非 loop/ram/dm 设备的序列号
fn fallback_block_serial(root: &Path) -> Option<String> {
    let block_dir = root.join("sys/block");
    let mut entries: Vec<String> = fs::read_dir(&block_dir)
        .ok()?
        .filter_map(|e| e.ok())
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .filter(|name| {
            !(name.starts_with("loop")
                || name.starts_with("ram")
                || name.starts_with("dm-")
                || name.starts_with("sr"))
        })
        .collect();
    entries.sort();
    for name in entries {
        if let Some(serial) = disk_serial_of(root, &name) {
            return Some(serial);
        }
    }
    None
}

/// 主网卡 MAC：过滤虚拟接口后取 ifindex 最小者
fn read_primary_mac(root: &Path) -> Option<String> {
    let net_dir = root.join("sys/class/net");
    let entries = fs::read_dir(&net_dir).ok()?;
    let mut candidates: Vec<(u32, String)> = Vec::new();
    for entry in entries.filter_map(|e| e.ok()) {
        let name = entry.file_name().to_string_lossy().into_owned();
        if is_virtual_nic(&name) {
            continue;
        }
        let nic_dir = entry.path();
        // sysfs 的 net/<name> 可能是符号链接，read_dir 给出的 path 能直接拼接子路径
        let Some(ifindex) =
            read_trimmed(&nic_dir.join("ifindex")).and_then(|s| s.parse::<u32>().ok())
        else {
            continue;
        };
        let Some(addr) = read_trimmed(&nic_dir.join("address")) else {
            continue;
        };
        // 全零地址（未绑定）视为无效
        if addr.replace(':', "").trim_start_matches('0').is_empty() {
            continue;
        }
        candidates.push((ifindex, addr));
    }
    candidates.sort_by_key(|(idx, _)| *idx);
    candidates.into_iter().next().map(|(_, addr)| addr)
}

/// 虚拟/容器相关接口过滤（设计文档 §3.1）
fn is_virtual_nic(name: &str) -> bool {
    if name == "lo" {
        return true;
    }
    const VIRTUAL_NIC_PREFIXES: [&str; 14] = [
        "veth",
        "docker",
        "br-",
        "virbr",
        "cali",
        "tunl",
        "zt",
        "flannel",
        "cni",
        "dummy",
        "wg",
        "tailscale",
        "tun",
        "tap",
    ];
    VIRTUAL_NIC_PREFIXES.iter().any(|p| name.starts_with(p))
}

/// 展示类信息：hostname / CPU 型号 / 内存 GB
pub fn read_display_info(sysroot: &Path) -> DisplayInfo {
    let (cpu_model, memory_gb) = read_cpu_and_memory(sysroot);
    DisplayInfo {
        hostname: read_hostname(sysroot),
        cpu_model,
        memory_gb,
    }
}

/// hostname（仅展示）
fn read_hostname(root: &Path) -> Option<String> {
    read_trimmed(&root.join("proc/sys/kernel/hostname"))
}

/// CPU 型号与内存 GB（仅展示）
fn read_cpu_and_memory(root: &Path) -> (Option<String>, u64) {
    let cpu_model = fs::read_to_string(root.join("proc/cpuinfo"))
        .ok()
        .and_then(|content| {
            content
                .lines()
                .find_map(|line| line.strip_prefix("model name"))
                .map(|rest| rest.trim_start_matches([' ', '\t', ':']).trim().to_string())
        })
        .filter(|s| !s.is_empty());
    // MemTotal 行格式："MemTotal:       16308856 kB"
    let memory_gb = fs::read_to_string(root.join("proc/meminfo"))
        .ok()
        .and_then(|content| {
            let total_kb: u64 = content
                .lines()
                .find_map(|line| line.strip_prefix("MemTotal:"))
                .and_then(|rest| {
                    rest.trim()
                        .strip_suffix("kB")
                        .map(str::trim)
                        .and_then(|num| num.parse().ok())
                })?;
            Some(total_kb / (1024 * 1024))
        })
        .unwrap_or(0);
    (cpu_model, memory_gb)
}

#[cfg(all(test, target_os = "linux"))]
mod tests {
    use super::*;

    fn write_file(root: &Path, rel: &str, content: &str) {
        let path = root.join(rel);
        fs::create_dir_all(path.parent().expect("parent")).expect("mkdir");
        fs::write(path, content).expect("write");
    }

    fn make_symlink(root: &Path, link_rel: &str, target: &str) {
        let path = root.join(link_rel);
        fs::create_dir_all(path.parent().expect("parent")).expect("mkdir");
        std::os::unix::fs::symlink(target, path).expect("symlink");
    }

    fn fake_tree() -> tempfile::TempDir {
        let dir = tempfile::tempdir().expect("tempdir");
        let root = dir.path();
        write_file(root, "etc/machine-id", "7f05a4e28c9b3d6f1a0e5c8d2b4f7a91\n");
        write_file(
            root,
            "sys/class/dmi/id/product_uuid",
            "0E8D5F8E-6C1A-4B2D-9E3F-A1B2C3D4E5F6\n",
        );
        // 根挂载在 8:1（sda1）
        write_file(
            root,
            "proc/self/mountinfo",
            "30 23 0:22 / /sys rw,nosuid ...\n36 35 8:1 / / rw,relatime - ext4 /dev/sda1 rw\n",
        );
        make_symlink(
            root,
            "sys/dev/block/8:1",
            "../../devices/pci0000:00/0000:00:1f.2/ata1/host0/target0:0:0/0:0:0:0/block/sda/sda1",
        );
        write_file(root, "sys/block/sda/device/serial", "WD-WCC4N7XRZ2VA\n");
        // 两块物理网卡 + 虚拟接口
        write_file(root, "sys/class/net/enp3s0/ifindex", "3\n");
        write_file(root, "sys/class/net/enp3s0/address", "AA:BB:CC:11:22:33\n");
        write_file(root, "sys/class/net/eth0/ifindex", "2\n");
        write_file(root, "sys/class/net/eth0/address", "11:22:33:44:55:66\n");
        write_file(root, "sys/class/net/lo/ifindex", "1\n");
        write_file(root, "sys/class/net/lo/address", "00:00:00:00:00:00\n");
        write_file(root, "sys/class/net/vethabc123/ifindex", "4\n");
        write_file(
            root,
            "sys/class/net/vethabc123/address",
            "AA:BB:CC:00:00:01\n",
        );
        write_file(root, "sys/class/net/docker0/ifindex", "5\n");
        write_file(root, "sys/class/net/docker0/address", "02:42:AC:11:00:00\n");
        dir
    }

    #[test]
    fn collects_all_fields_from_fake_sysfs() {
        let dir = fake_tree();
        let fields = collect_identity(dir.path()).expect("collect");
        assert_eq!(
            fields.machine_id.as_deref(),
            Some("7f05a4e28c9b3d6f1a0e5c8d2b4f7a91")
        );
        assert_eq!(
            fields.dmi_uuid.as_deref(),
            Some("0E8D5F8E-6C1A-4B2D-9E3F-A1B2C3D4E5F6")
        );
        assert_eq!(fields.disk_serial.as_deref(), Some("WD-WCC4N7XRZ2VA"));
        // ifindex 最小的物理口是 eth0(2)，enp3s0(3) 次之；veth/docker/lo 被过滤
        assert_eq!(fields.primary_mac.as_deref(), Some("11:22:33:44:55:66"));
    }

    #[test]
    fn nvme_partition_to_controller_serial() {
        let dir = tempfile::tempdir().expect("tempdir");
        let root = dir.path();
        write_file(root, "etc/machine-id", "aaa111\n");
        write_file(
            root,
            "proc/self/mountinfo",
            "36 35 259:2 / / rw - ext4 /dev/nvme0n1p2 rw\n",
        );
        make_symlink(
            root,
            "sys/dev/block/259:2",
            "../../devices/pci.../nvme/nvme0/nvme0n1/nvme0n1p2",
        );
        write_file(root, "sys/class/nvme/nvme0/serial", "S64GNE0R123456\n");
        write_file(root, "sys/class/net/enp0s3/ifindex", "2\n");
        write_file(root, "sys/class/net/enp0s3/address", "52:54:00:aa:bb:cc\n");

        let fields = collect_identity(root).expect("collect");
        assert_eq!(fields.disk_serial.as_deref(), Some("S64GNE0R123456"));
    }

    #[test]
    fn missing_dmi_and_empty_machine_id_tolerated() {
        let dir = tempfile::tempdir().expect("tempdir");
        let root = dir.path();
        write_file(root, "etc/machine-id", "\n"); // 未初始化的空文件
        write_file(root, "var/lib/dbus/machine-id", "fallback-dbus-id\n");
        write_file(root, "sys/class/net/enp0s3/ifindex", "2\n");
        write_file(root, "sys/class/net/enp0s3/address", "52:54:00:aa:bb:cc\n");

        let fields = collect_identity(root).expect("collect");
        assert_eq!(fields.machine_id.as_deref(), Some("fallback-dbus-id"));
        assert!(fields.dmi_uuid.is_none());
        assert!(fields.disk_serial.is_none());
        assert!(fields.primary_mac.is_some());
    }

    #[test]
    fn partition_to_disk_rules() {
        assert_eq!(partition_to_disk("sda1"), "sda");
        assert_eq!(partition_to_disk("sda"), "sda");
        assert_eq!(partition_to_disk("nvme0n1p2"), "nvme0n1");
        assert_eq!(partition_to_disk("nvme0n1"), "nvme0n1");
        assert_eq!(partition_to_disk("mmcblk0p1"), "mmcblk0");
        assert_eq!(partition_to_disk("mmcblk0"), "mmcblk0");
        // sdap 是盘名，sdap1 的分区不能被误剥成 sda
        assert_eq!(partition_to_disk("sdap1"), "sdap");
    }

    #[test]
    fn virtual_nic_filter() {
        assert!(is_virtual_nic("lo"));
        assert!(is_virtual_nic("vethabc123"));
        assert!(is_virtual_nic("docker0"));
        assert!(is_virtual_nic("br-abcdef1234"));
        assert!(is_virtual_nic("tun0"));
        assert!(!is_virtual_nic("eth0"));
        assert!(!is_virtual_nic("enp3s0"));
    }

    #[test]
    fn hostname_and_meminfo_parsing() {
        let dir = fake_tree();
        write_file(dir.path(), "proc/sys/kernel/hostname", "prod-web-01\n");
        write_file(
            dir.path(),
            "proc/cpuinfo",
            "flags\t: x\nmodel name\t: Intel(R) Xeon(R) Test CPU\n",
        );
        write_file(
            dir.path(),
            "proc/meminfo",
            "MemTotal:       16308856 kB\nSwapTotal: 0 kB\n",
        );
        let display = read_display_info(dir.path());
        assert_eq!(display.hostname.as_deref(), Some("prod-web-01"));
        assert_eq!(
            display.cpu_model.as_deref(),
            Some("Intel(R) Xeon(R) Test CPU")
        );
        assert_eq!(display.memory_gb, 15); // 16308856 kB ≈ 15.56 GB
    }
}
