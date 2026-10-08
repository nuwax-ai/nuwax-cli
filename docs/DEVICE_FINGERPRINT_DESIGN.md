# 设备指纹（Device Fingerprint）多平台方案设计

> 状态：P0 已实现并完成三平台真机验证（2026-10-08：macOS aarch64 / Linux x86_64 / Windows x86_64）
> 日期：2026-10-08
> 目标仓库：nuwax-cli（采集与注入）、build-agent-docker（compose / yml 透传）、Java backend（密钥校验，另行方案）

## 1. 背景与目标

backend 容器内的 Java 应用需要感知**宿主机**的设备信息，用于实现"密钥 + 机器指纹"绑定的授权管控。容器隔离导致 Java 无法在容器内直接获得宿主机的稳定标识（容器的 machine-id / hostname / MAC 均随容器重建变化），因此由运行在宿主机上的 nuwax-cli 负责采集并注入。

**硬性约束：**

- nuwax-cli 发布目标覆盖 5 个平台（见 `dist-workspace.toml`）：`aarch64/x86_64-apple-darwin`、`aarch64/x86_64-unknown-linux-gnu`、`x86_64-pc-windows-msvc`，指纹采集必须全平台可用；
- 生产部署环境以 Linux 服务器为主，macOS/Windows 主要是开发与边缘场景；
- 项目规范：禁止 unsafe（winreg / 子进程方案均满足）、禁止 unwrap/expect（生产路径）、Fail Fast、cargo nextest 测试。

## 2. 总体架构

```
┌─ nuwax-cli（宿主机进程）────────────────────────────────────────┐
│                                                                  │
│  DeviceInfoCollector (各平台实现, cfg(target_os))                │
│      │ 采集原始字段                                               │
│      ▼                                                           │
│  normalize → per-field SHA-256 → canonical string                │
│      │                                                           │
│      ▼                                                           │
│  device_id = "v1:" + SHA-256(canonical)                          │
│      │                                                           │
│      ▼                                                           │
│  冻结文件 ./data/device_fingerprint.json  (防漂移, 见 §5)         │
│      │                                                           │
│      ▼                                                           │
│  EnvManager.upsert → docker/.env                                 │
│      DEVICE_ID / DEVICE_FIELDS / DEVICE_INFO                     │
└──────────────────────────────────────────────────────────────────┘
               │ docker compose 读取 .env
               ▼
   backend.environment:  - DEVICE_ID=${DEVICE_ID} ...
               │
               ▼
   Spring 宽松绑定(无需改 yml):
     device.id     ← DEVICE_ID
     device.fields ← DEVICE_FIELDS
     device.info   ← DEVICE_INFO
```

模块分层（SOLID）：

| 层 | 位置 | 职责 |
|---|---|---|
| 采集 | `client-core/src/device_info/collector_{linux,macos,windows}.rs` | 各平台读原始字段，`#[cfg(target_os)]` 隔离（策略模式） |
| 算法 | `client-core/src/device_info/fingerprint.rs` | 纯函数：规范化 / 哈希 / 组合，100% 可单测 |
| 持久化 | `client-core/src/device_info/store.rs` | 冻结文件的读写（仓库模式） |
| 注入 | `nuwax-cli/src/utils/env_manager.rs` 扩展 | `.env` upsert（与现有 `update_frontend_port` 同层） |
| 命令 | `nuwax-cli/src/commands/device_info.rs` | `nuwax-cli device-info` 展示 / `--refresh` 重绑 / `--apply` 注入 |

## 3. 多平台指纹源调研

### 3.1 Linux（生产主力）

| 字段名 | 来源 | 权限 | 稳定性 | 备注 |
|---|---|---|---|---|
| `machine_id` | `/etc/machine-id`，回退 `/var/lib/dbus/machine-id` | 所有用户可读 | 重装系统才变 | 32 位小写 hex；**克隆云镜像可能重复**（见 §11） |
| `dmi_uuid` | `/sys/class/dmi/id/product_uuid` | **0400，仅 root** | 换主板才变 | nuwax-cli 部署通常以 root 运行（deploy.sh 亦要求 sudo）；ARM 开发板可能无 `/sys/class/dmi`，容缺 |
| `disk_serial` | sysfs：`/sys/block/<dev>/device/serial`（SATA）、`/sys/class/nvme/nvme*/serial`（NVMe）；统一回退 `lsblk -dno SERIAL,TYPE` | root 可读 | 换系统盘才变 | 取根分区所在磁盘：解析 `/proc/self/mountinfo` 得根设备 major:minor → `/sys/dev/block/<maj:min>` 符号链接归一到所属磁盘（纯文件读取）；取不到则取第一个非 loop/ram 块设备；QEMU 裸盘可能为空，容缺 |
| `primary_mac` | `/sys/class/net/*/address` | 所有用户可读 | VM 迁移可能变 | 过滤 `lo`/`veth*`/`docker*`/`br-*`/`virbr*`/`cali*`/`tunl*`/`zt*`，取 ifindex 最小的物理口 |
| hostname（仅展示） | `/proc/sys/kernel/hostname` | 可读 | — | 不参与身份哈希 |
| cpu_model / mem_total（仅展示） | `/proc/cpuinfo`、`/proc/meminfo` | 可读 | — | 供授权限额与支持排障用 |

**全部为纯文件读取，无子进程、无新依赖。**

### 3.2 macOS

| 字段名 | 来源 | 权限 | 备注 |
|---|---|---|---|
| `machine_id` | `ioreg -rd1 -c IOPlatformExpertDevice` 解析 `IOPlatformUUID` | 普通用户 | 等价于"系统级 UUID"，重装系统不变 |
| `dmi_uuid` | 同上解析 `IOPlatformSerialNumber` | 普通用户 | 序列号，硬件绑定 |
| `disk_serial` | 默认**不采集** | — | `system_profiler` 需 2–5 秒，收益低；macOS 非生产主力，跳过以保启动速度 |
| `primary_mac` | `ifconfig en0` 解析 `ether` 行，回退第一个 `en*` | 普通用户 | |
| hostname（仅展示） | `scutil --get LocalHostName` | 普通用户 | |
| cpu / mem（仅展示） | `sysctl -n machdep.cpu.brand_string`、`sysctl -n hw.memsize` | 普通用户 | |

均为系统自带工具子进程调用，无新依赖。

### 3.3 Windows

| 字段名 | 来源 | 权限 | 备注 |
|---|---|---|---|
| `machine_id` | 注册表 `HKLM\SOFTWARE\Microsoft\Cryptography\MachineGuid`（**winreg crate**） | Users 可读，无需管理员 | 注意 64 位键（`KEY_WOW64_64KEY`，winreg 已支持） |
| `dmi_uuid` | PowerShell `Get-CimInstance Win32_BIOS`/`Win32_BaseBoard` 的 `SerialNumber`；回退 `wmic bios get serialnumber` | 普通用户 | **Win11 24H2 起移除 wmic**，故 PowerShell 为主、wmic 为兜底 |
| `disk_serial` | PowerShell `Get-PhysicalDisk` 的 `SerialNumber` | 普通用户 | |
| `primary_mac` | `getmac /fo csv /nh` 取第一块非蓝牙物理口 | 普通用户 | **Wi-Fi 随机 MAC 特性**：优先有线网卡（见 §11） |
| hostname（仅展示） | `%COMPUTERNAME%` | — | |
| cpu / mem（仅展示） | `%PROCESSOR_IDENTIFIER%` | — | |

**唯一新增依赖：`winreg`（仅 Windows target，无 unsafe，维护活跃）。** sha2 / serde_json 已在工作区依赖中。

### 3.4 依赖变更汇总

```toml
# 根 Cargo.toml [workspace.dependencies]
winreg = "0.55"          # 仅 target_os = "windows" 时引入

# client-core/Cargo.toml
[target.'cfg(windows)'.dependencies]
winreg = { workspace = true }
```

不引入 `sysinfo`（重量级、无 machine-id/DMI）、不引入 `machine-uid`（单值输出、无法做逐字段宽容匹配、规范化不可控）。

## 4. 指纹算法

### 4.1 规范化规则（fingerprint.rs 纯函数）

| 规则 | 说明 |
|---|---|
| trim + 折叠连续空白 | 消除读取差异 |
| `machine_id` / `dmi_uuid` / `mac` | 转小写；MAC 去掉 `:` / `-` |
| `disk_serial` | 转大写（厂商大小写混杂） |
| 去除包裹引号 | ioreg 输出带 `"..."` |

### 4.2 组合与哈希

```
per_field_hash  = SHA-256(normalized_value)          // 每字段独立哈希，供宽容匹配
canonical       = "nuwax-device-fp-v1|machine_id=<h1>|dmi_uuid=<h2>|disk_serial=<h3>|primary_mac=<h4>"
                   // 域分隔前缀 + 固定字段顺序，仅拼接"采集成功"的字段
device_id       = "v1:" + SHA-256(canonical)         // 64 hex
```

- 缺失字段直接跳过（ARM 板无 DMI、macOS 无磁盘序列号均合法），同机缺字段结果确定；
- **Fail Fast 边界**：四个身份字段全部采集失败 → 报错终止部署（说明宿主机环境异常，宁可失败不可给出不稳定 ID）；
- 带版本前缀 `v1:`，未来算法调整可平滑升级（v2 并行生成、Java 端按前缀路由）。

### 4.3 宽容匹配策略（Java / 服务端执行，此处仅约定数据）

`.env` 中携带**逐字段哈希**（`DEVICE_FIELDS`），授权绑定时服务端记录的是字段哈希集合而非单一 device_id：

- `machine_id` **或** `dmi_uuid` 匹配 → 判定同一设备（换网卡/数据盘不掉授权）；
- 其余情况 ≥2/4 匹配 → 同一设备（覆盖重装系统 + 换主板这种低频组合）;
- 全不匹配 → 新设备，需重新授权。

阈值属于业务策略，放在 Java/服务端可调，协议层只保证数据充分。

## 5. 防漂移：冻结机制（关键设计）

**问题**：每次 `start` 重算指纹时，若某字段偶发读取失败（权限、时序），组合哈希会变化 → device_id 漂移 → 授权失效。这是本方案最大的生产风险点。

**方案**：首次采集结果**冻结**到 `./data/device_fingerprint.json`（与 `duck_client.db` 同目录，沿用现有状态持久化约定；权限 0600，Windows 下仅当前用户可访问）：

```json
{
  "fp_version": 1,
  "collected_at": "2026-10-08T10:00:00+08:00",
  "raw": { "machine_id": "...", "dmi_uuid": "...", "disk_serial": "...", "primary_mac": "..." },
  "device_id": "v1:ab3f...",
  "field_hashes": { "machine_id": "...", "dmi_uuid": "..." },
  "environment": { "os": "linux", "arch": "x86_64", "wsl": false, "containerized": false }
}
```

后续每次部署/启动/`device-info --apply`：

1. 重新采集 → 与冻结值比对；
2. **一致** → 照常写入 `.env`（幂等）；
3. **不一致（硬件变更）** → 默认**维持冻结的 device_id 不变**，仅把漂移详情记录进冻结文件的 `drift` 字段并打 warn 日志；授权连续性优先；
4. 用户显式执行 `nuwax-cli device-info --refresh` 才接受新值重新冻结（对应"硬件变更需重新授权"的售后流程）。

复用 `client-core/src/container/environment.rs` 已有的 WSL 检测；检测到 `/.dockerenv` 时标记 `containerized: true` 并 warn（提示 nuwax-cli 被装进了容器，指纹是容器视角）。

## 6. 注入机制

### 6.1 `.env` 键设计

```dotenv
# --- Managed by nuwax-cli: device fingerprint, DO NOT EDIT ---
DEVICE_ID=v1:3fa8c0...（64 hex）
DEVICE_FIELDS='{"machine_id":"...","dmi_uuid":"...","disk_serial":"...","primary_mac":"..."}'
DEVICE_INFO='{"hostname":"prod-1","os":"linux","arch":"x86_64","cpu_model":"...","cpu_cores":8,"memory_gb":64,"fingerprint_version":1,"collected_at":"...","wsl":false,"containerized":false}'
```

- `FIELDS` / `INFO` 为 JSON，**用单引号包裹**（dotenvy 与 docker compose 均剥单引号、保留内部双引号，已验证 EnvManager 的 `QuoteType::Single` 路径支持）；
- `INFO` 内的自由文本（hostname、cpu_model）写入前需**剔除单引号与控制字符**——值被单引号包裹，内嵌 `'` 会破坏 dotenvy/compose 解析；`FIELDS` 全是 hex 哈希天然安全；
- `FIELDS` 只含哈希不含原始序列号，降低敏感信息暴露面；`device-info` 命令输出同样只含哈希，原始值仅存在于冻结文件（0600）中，支持排障时直接查看该文件。

### 6.2 EnvManager 扩展（nuwax-cli）

现状：`set_variable` 只能修改已存在的键，键不存在时 bail（env_manager.rs:214）。需要新增：

```rust
impl EnvManager {
    /// 键存在则更新，不存在则以注释头 + 追加方式插入文件末尾
    pub fn upsert_variable(&mut self, key: &str, value: &str, quote: QuoteType) -> Result<()>;
}
```

配套单测：新增键追加、已有键覆盖、注释与引号保留、幂等重复调用。

### 6.3 集成点（nuwax-cli）

| 位置 | 时机 | 动作 |
|---|---|---|
| `commands/docker_service.rs::prepare_docker_services` | 部署准备阶段（首个部署与升级均经过） | 调用 `ensure_device_env(get_env_file_path())`：冻结文件加载/首次生成 → upsert 三个键 |
| `commands/docker_service.rs::start_docker_services` | 每次 `docker-service start` | 同上（幂等，秒级） |
| `commands/auto_upgrade_deploy.rs` | offline-deploy | 复用 staged 部署路径 → 已被 `prepare_docker_services` 覆盖，无需单独改动 |

放置在 `update_frontend_port`（frontend port 注入）之后同一段落，语义一致："prepare 阶段可能更新 .env"。

实现注意：

1. `ensure_device_env` 写完 `.env` 后需使 DockerManager 的 compose 配置缓存失效——与 `update_frontend_port` 同一注意点（`prepare_docker_services` 调用方已有失效逻辑，`start_docker_services` 路径实现时需确认补齐）；
2. upsert 每次写入的都是冻结文件中的真实值，因此部署/启动路径可**自愈**用户对 `DEVICE_*` 键的手改（升级流程对 `.env` 的"保留旧值"合并不影响——注入永远发生在合并之后）。

### 6.4 build-agent-docker 改动

`docker/docker-compose.yml` backend 服务 environment 追加三个透传：

```yaml
      - DEVICE_ID=${DEVICE_ID}
      - DEVICE_FIELDS=${DEVICE_FIELDS}
      - DEVICE_INFO=${DEVICE_INFO}
```

`application-external.yml` **无需任何改动**：配置前缀定为 `device.*`，Spring Boot 的宽松绑定（relaxed binding）自动把环境变量 `DEVICE_ID` 映射到 `device.id`、`DEVICE_FIELDS` 映射到 `device.fields`、`DEVICE_INFO` 映射到 `device.info`，不需要 yml 占位符中转；`device` 前缀已核验与该文件既有键（含顶层扁平键 `license`）无冲突。授权密钥的配置命名与透传由 Java 团队自行决定，不在本方案范围内。完整契约见 §13。

（旧版 nuwax-cli 部署时环境变量缺失 → Java 侧拿到 null → 机器信息视为未提供，Java 侧自行决定降级行为。）

**防篡改交叉验证（可选但推荐）**：backend 增加只读挂载 `- /etc/machine-id:/etc/host-machine-id:ro`，Java 将文件实际值哈希后与 `device-fields.machine_id` 比对，不一致则拒绝启动。防住"用户改 `.env` 伪造已授权机器指纹"的攻击。仅 Linux 宿主机适用；macOS/Windows 宿主（Docker Desktop）可豁免该检查。

## 7. 新 CLI 命令：`nuwax-cli device-info`

授权工作流需要：客户跑命令 → 把输出发给平台 → 平台按 device_id / 字段哈希签发密钥。

```
nuwax-cli device-info                     # 表格展示 device_id、各字段哈希、环境信息、冻结状态、漂移告警
nuwax-cli device-info --json              # 机器可读输出（供 GUI / 上报集成）
nuwax-cli device-info --apply             # 采集(冻结优先)并写入部署目录 docker/.env —— 独立注入入口
nuwax-cli device-info --apply --refresh   # 忽略冻结重新采集后写入（硬件变更重新绑定 + 注入一步完成）
```

`--apply` 是独立的注入入口，与部署/启动路径共用同一个 `ensure_device_env()`（同一份代码、同样的幂等 upsert），覆盖三类场景：

1. **原生 compose 部署后想启用授权**——无需重新部署，一条命令补注入（§14 场景 B 的恢复路径）；
2. **手动换包升级丢了 `.env` 指纹**——一键恢复（§14 场景 C）；
3. **用户误改 `.env`**——重跑即还原真实指纹（对防篡改是正向的：命令只会写入真实采集值）。

行为细节：

- 命令需在**部署根目录**运行（`.env` 与 `./data/` 均为相对路径，与 nuwax-cli 既有工作目录约定一致）；
- 按 `get_env_file_path()` 定位 `.env`；文件不存在则 Fail Fast 报错提示（未部署或工作目录不对）；
- 即使 `./data/` 冻结文件也丢了也没关系：同一台机器重新采集结果确定，device_id 不变；
- 写入后若 `docker-compose.yml` 未引用 `DEVICE_ID`（用户自定义 compose 缺透传），打 warn 提示；
- `--apply` 尊重冻结机制（默认不改变 device_id）；与 `--refresh` 组合才重新冻结；
- `--json` 输出结构与 §5 冻结文件一致（含 frozen / drift 状态），供 GUI 与上报集成依赖稳定 schema。

- `cli.rs` 增加命令定义；`commands/device_info.rs` 实现；
- 用户可见文案走 i18n：`locales/{en,zh-CN,zh-TW}.yml` 增加 `device_info_cmd.*` 键（补全 `scripts/check_i18n.py` 校验范围）。

## 8. 代码改动清单

### nuwax-cli（本仓库）

| 文件 | 改动 |
|---|---|
| `client-core/src/device_info/mod.rs` | 新模块：`DeviceInfo`、`Fingerprint` 类型与公共入口 `collect()` |
| `client-core/src/device_info/fingerprint.rs` | 规范化 / 哈希 / canonical / 匹配建议纯函数 |
| `client-core/src/device_info/collector_linux.rs` | sysfs + /proc 读取（读取根路径参数化，便于 fake sysfs 测试） |
| `client-core/src/device_info/collector_macos.rs` | ioreg / sysctl / ifconfig 子进程 + 输出解析函数 |
| `client-core/src/device_info/collector_windows.rs` | winreg + PowerShell/wmic/getmac 子进程 + 解析函数 |
| `client-core/src/device_info/store.rs` | 冻结文件读写、漂移检测 |
| `client-core/src/lib.rs` | 导出新模块 |
| `client-core/src/constants.rs` | 新增 `device_info` 常量段：冻结文件名、env 键名、字段优先级顺序 |
| `nuwax-cli/src/utils/env_manager.rs` | `upsert_variable` + 单测 |
| `nuwax-cli/src/utils/device_env.rs`（新增） | `ensure_device_env()`：冻结加载/生成 + EnvManager upsert（prepare/start 与 `--apply` 三处共用） |
| `nuwax-cli/src/commands/docker_service.rs` | prepare/start 两个入口调用 `ensure_device_env` |
| `nuwax-cli/src/commands/device_info.rs` + `cli.rs` + `commands/mod.rs` | 新命令 |
| `locales/*.yml` | i18n 文案 |
| 根 `Cargo.toml` + `client-core/Cargo.toml` | winreg（cfg windows） |

### build-agent-docker（另一仓库）

`docker-compose.yml`（backend env 透传 + 可选 machine-id 只读挂载）、随包发布说明；`application-external.yml` **无需改动**（宽松绑定，见 §6.4）。

## 9. 测试方案（cargo nextest）

| 层级 | 用例 |
|---|---|
| fingerprint 纯函数 | 规范化各字段大小写/空白/引号；canonical 固定快照（防算法意外变更导致全量设备 ID 变化）；缺字段跳过；v1 前缀 |
| Linux collector | 临时目录构造 fake sysfs（machine-id、DMI、网卡、NVMe serial），路径参数化注入；veth/docker 网卡过滤；全缺 → Fail Fast 错误 |
| macOS/Windows collector | 解析函数用固化的 ioreg/getmac/PowerShell 输出样本做快照测试（命令本体在 CI 对应平台跑集成冒烟） |
| store | 首次冻结 / 幂等重读 / 漂移不覆盖 / refresh 覆盖 / 损坏 JSON 报错（Fail Fast 不静默重建，报错信息附 `device-info --refresh` 修复指引） |
| env_manager upsert | 见 §6.2 |
| 端到端 | 部署后断言容器内 `DEVICE_ID` 存在且两次部署一致（复用现有 docker 集成测试基建）；`--apply` 独立注入路径单独覆盖（§14 场景 B/C 恢复） |

## 10. 密钥协议建议（Java 侧，另行立项，此处仅接口约定）

- 密钥格式：`Base64(Ed25519签名 || JSON{device_id, field_hashes?, expiry, features, customer})`，Java 内置公钥**验签**，并校验 payload 内设备标识与注入值一致；
- 周期性在线心跳可选（复用 eco-market / market-api.nuwax.com 的客户端注册通道），签名保离线可用、心跳保可吊销；
- Java 侧匹配策略按 §4.3，阈值可配置。

## 11. 风险与已知边界

| 风险 | 影响 | 缓解 |
|---|---|---|
| 克隆云镜像 machine-id 重复（黄金镜像未重置） | 不同物理机算出相同 device_id；宽容匹配中 machine_id 单独命中即判同机 | 文档要求制作镜像时清空 machine-id（cloud-init `runcmd: truncate -s0 /etc/machine-id && systemd-machine-id-setup`）；服务端签发时可附加 mac/disk 组合校验（阈值可调为"machine_id 且 ≥1 个硬件字段"） |
| Win11 24H2 移除 wmic | Windows dmi/disk 采集失败 | PowerShell `Get-CimInstance` 为主，wmic 仅兜底 |
| Wi-Fi 随机 MAC（Windows/macOS） | primary_mac 抖动 | 冻结机制已吸收（§5）；采集时优先有线口 |
| ARM 板卡无 DMI / QEMU 空序列号 | 字段缺失 | 算法容缺；四字段全失才 Fail Fast |
| 用户手改 `.env` 伪造指纹 | 授权被绕过 | 签名密钥绑定原始指纹（改 env 无法自造有效签名）+ machine-id 只读挂载交叉验证（§6.4） |
| Docker Desktop（macOS/Win 宿主） | 容器跑在 Linux VM，但 nuwax-cli 在宿主运行，指纹=宿主机 | 符合"客户视角的宿主机"语义，无需特殊处理；文档说明 |
| WSL2 下部署 | 指纹=WSL 发行版（machine-id 为 distro 级） | 标记 `wsl: true` 进 device_info；语义为"承载 Docker 的环境"，行为一致可预期 |
| `system_profiler` 慢 | macOS 采集慢 | 已默认跳过磁盘序列号字段 |
| 冻结文件被篡改 | `.env` 随之注入伪造值 | 与 `.env` 篡改同类：签名密钥绑定原始指纹 + §13.6 交叉验证兜底；文件权限 0600 降低误改概率 |
| 首次采集非 root（DMI 0400 读不到），之后 root 部署 | 字段集合变化触发漂移告警 | 维持冻结值，授权连续（§5）；部署文档建议始终以 root/sudo 部署以采集完整字段 |
| 同机多部署目录（prod/test 并存） | 各目录独立冻结，device_id 相同 | 符合"按机器授权"语义；若需按实例区分，属 Java/服务端策略，数据已足够（可加实例标识） |

## 12. 实施分期

| 阶段 | 内容 | 验收标准 |
|---|---|---|
| **P0（核心）** | client-core device_info 模块（Linux/macOS/Windows 采集 + 算法 + 冻结）+ EnvManager upsert + prepare/start 注入 + `device-info` 命令（含 `--apply`）+ 全部单测 | 三平台 `device-info` 输出稳定；部署后容器内可见三个 env；升级/重启 ID 不漂移；`--apply` 在原生部署目录可独立注入 |
| **P1（接入）** | build-agent-docker compose 改动 + 打包发版 | Java 侧能从配置读到值 |
| **P2（加固）** | machine-id 只读挂载交叉验证、Java 签名校验与在线心跳（Java 团队） | 篡改 `.env` 场景被拒绝 |

## 13. Java 侧机器信息契约（v1）

> 本节定义 nuwax-cli 向 Java backend **提供的机器信息**；授权/密钥体系（生成、校验、配置命名、与老 `license` 键的关系）由 Java 团队自行设计，不在本方案范围。
> 原则：**原始硬件序列号不进入 Java**（唯一的例外是 §13.6 的交叉验证只读挂载），Java 拿到的身份数据全部是 SHA-256 哈希。

### 13.1 提供的配置项总览

Java 通过 **`device` 前缀**读取，环境变量经 Spring Boot 宽松绑定自动映射（`DEVICE_ID` → `device.id`），**不需要 yml 占位符**：

| 配置项 | 环境变量 | 类型 | 用途 |
|---|---|---|---|
| `device.id` | `DEVICE_ID` | String | 机器的**主标识**，严格绑定用 |
| `device.fields` | `DEVICE_FIELDS` | JSON String | 逐字段哈希，**宽容匹配**用 |
| `device.info` | `DEVICE_INFO` | JSON String | 展示 / 限额 / 排障，**不参与身份判定** |
| （交叉验证） | 挂载文件 `/etc/host-machine-id` | 文件 | 防篡改校验，可选，仅 Linux 宿主（见 §13.6） |

选 `device` 前缀：已核验与 Java 项目现有配置代码、application-external.yml 既有键（含顶层扁平键 `license`）、compose 环境变量均无冲突；本方案不引入任何 `license` 命名空间的配置。

### 13.2 `device.id`

```
格式:  "v1:" + 64 位小写 hex
示例:  v1:aff7c6444dcd6d7bfb98890d0daaa6d004e626157e58c7dbd62ba172e44c345e
```

语义保证：

- 同一台机器上，跨重启、跨 Docker 重建、跨 nuwax-cli 升级**值不变**；
- CLI 冻结机制保证它**不会自动变化**（硬件变更后默认维持旧值，仅 `device-info --refresh` 显式重新绑定）；
- 前缀 `v1` 为指纹算法版本，Java 遇到未知前缀（如未来的 `v2`）应按"未知版本"策略处理（告警/拒绝，不要当普通字符串比对）。

### 13.3 `device.fields`

```json
{
  "machine_id":  "3fb59a395a93a73cfdf93528db13642d25fa7ed166c201be8f59ca9232c8aff9",
  "dmi_uuid":    "c8743baf574a672e72e17e4eb99507b12600982e4eec8acb7cd2930320084ba4",
  "disk_serial": "f28d3bca917672d4d3c52eb6aa9476941451de660f0229ce805e80bb07984cbd",
  "primary_mac": "035f79cf6426abcb5701c1fea6128a48ca00c6d673dda11fe917eff3065ada37"
}
```

- 值 = `SHA-256(规范化后的原始值)`，64 位小写 hex；
- **四个键都是可选的**：macOS 无 `disk_serial`、部分 ARM 板无 `dmi_uuid` 属正常。**键缺失 = 该平台未采集，不是篡改，不算不匹配**；
- 解析必须容忍**未知键**（前向兼容，未来 v2 加字段）。

字段稳定性语义（匹配策略的依据）：

| 字段 | 变化时机 | 匹配权重 |
|---|---|---|
| `machine_id` | 重装 OS / 整机克隆 | 高（OS 级身份） |
| `dmi_uuid` | 换主板 | 高（硬件级身份） |
| `disk_serial` | 换系统盘 | 中 |
| `primary_mac` | 换网卡 / VM 迁移 | 低 |

**建议的判定规则**（可在 Java/服务端配置化）：

```
1. 载荷 device_id == device.id                          → 同机（严格命中）
2. 否则按字段:
   a. machine_id 相等 或 dmi_uuid 相等                   → 同机
   b. 其余情况: 4 个字段中 ≥2 个相等                      → 同机
   c. 否则                                                 → 新机器, 拒绝并提示重新授权
```

### 13.4 `device.info`

```json
{
  "hostname": "prod-web-01",
  "os": "linux",
  "arch": "x86_64",
  "cpu_model": "Intel(R) Xeon(R) Platinum 8269CY CPU @ 2.50GHz",
  "cpu_cores": 8,
  "memory_gb": 64,
  "fingerprint_version": 1,
  "collected_at": "2026-10-08T10:00:00+08:00",
  "wsl": false,
  "containerized": false
}
```

| 键 | 说明 | Java 用法 |
|---|---|---|
| `hostname` | 宿主机名（展示用） | 管理界面展示"授权绑定机器"；**绝不参与身份判定**（用户可随意改主机名） |
| `os` / `arch` | `linux`/`macos`/`windows`；`x86_64`/`aarch64` | 授权特性可按平台区分 |
| `cpu_model` / `cpu_cores` / `memory_gb` | CPU 型号 / 逻辑核数 / 内存 GB | **按资源限额授权**（如"≤8 核 ≤64G 可用"）时校验 |
| `fingerprint_version` | 与 device-id 前缀一致的数字版本 | 版本路由 |
| `collected_at` | 冻结时间（RFC3339） | 展示 |
| `wsl` / `containerized` | 部署环境标记 | `true` 时可在管理界面提示环境特殊 |

### 13.5 空值与兼容语义（Java 侧注意）

| 场景 | 表现 | 说明 |
|---|---|---|
| 旧版 nuwax-cli 部署（未升级） | `device.*` 为 null | 机器信息未提供，Java 侧自行决定降级行为 |
| 部分 JSON 键缺失 | 见 §13.3 | 正常，按"未采集"处理（平台差异） |
| `device.id` 前缀非 `v1:` | 未知版本 | 指纹算法版本不识别，建议告警 |
| JSON 解析失败 | 值损坏 | 属异常状态，建议 Fail Fast 暴露 |

与既有顶层扁平键 `license`（老 AES 时间锁）**天然无冲突**——本方案不引入任何 `license` 命名空间的配置；新授权体系与老键如何共存/替换，由 Java 团队自行设计。

配置绑定示例（Spring，宽松绑定直取环境变量，无需 yml）：

```java
@ConfigurationProperties(prefix = "device")
public class DeviceProperties {
    private String id;                  // "v1:..." 或 null; ← DEVICE_ID
    private String fields;              // JSON 串; ← DEVICE_FIELDS
    private String info;                // JSON 串; ← DEVICE_INFO
    // ...getter/setter; fields/info 在业务层用 Jackson 解析
    // (Map<String,String>/DeviceInfo, @JsonIgnoreProperties(ignoreUnknown = true))
}
```

代码侧统一用 `StringUtils.isBlank()` 判空后再 JSON 解析（宽松绑定下缺失环境变量得到 null，非空串）。

### 13.6 交叉验证（防篡改，可选启用）

compose 增加只读挂载（仅 Linux 宿主适用）：

```yaml
    volumes:
      - /etc/machine-id:/etc/host-machine-id:ro
```

Java 校验算法（与 CLI 侧规范化规则保持一致）：

```
raw   = 读取 /etc/host-machine-id 内容
norm  = lowercase(strip(raw))                # 仅去首尾空白 + 转小写
check = SHA-256Hex(norm)
要求: check == device_fields.machine_id
```

不一致 → 判定 `.env` 被篡改 → 拒绝启动授权功能。文件不存在（非 Linux 宿主）→ 跳过该校验。

### 13.7 授权密钥与机器信息的组合方式（参考，授权体系由 Java 团队设计）

密钥为平台签发的签名 blob：`Base64(Ed25519签名 || JSON载荷)`，载荷示例：

```json
{
  "v": 1,
  "device_id": "v1:aff7c644...",
  "field_hashes": { "machine_id": "3fb5...", "dmi_uuid": "c874..." },
  "expiry": "2027-10-08T00:00:00+08:00",
  "features": { "max_cores": 16, "modules": ["agent", "rcoder"] },
  "customer": "customer-0421"
}
```

Java 内置 Ed25519 公钥，校验流程：验签 → 比对载荷内 `device_id` / `field_hashes` 与注入值（按 §13.3 规则）→ 校验 `expiry` / 资源限额（对照 `device-info` 的 `cpu_cores`/`memory_gb`）。平台签发时同时记录 `field_hashes` 快照，用户换硬件后 `--refresh` 重新绑定时可凭"宽容匹配通过"自动换绑新 device_id（是否允许自动换绑由业务决定）。

## 14. 原生 docker compose 使用场景的兼容性

**结论：首次部署经 nuwax-cli、后续全部用原生 `docker compose` 管理容器——完全兼容**；纯原生部署（从未经 CLI）——机器信息缺失，落入 §13.5 的"未提供"场景。

兼容性的来源是一个关键设计选择：指纹的注入点是 **`.env` 文件**（部署目录内的持久状态），而不是 nuwax-cli 的进程行为。`docker compose up` 创建容器时统一从 `docker/.env` 插值——由 CLI 触发还是用户手动触发，结果一致。（若当初选择通过 Docker API 直接向容器塞环境变量，原生 compose 一重建容器指纹就丢了。）

逐场景分析：

| 场景 | 行为 | 结果 |
|---|---|---|
| A. CLI 首部署 → 之后原生 `up` / `restart` / `down`+`up` | `.env` 已含指纹且不变；`restart` 不重建容器、沿用创建时的 env；`down`+`up` 重新读 `.env` 得到相同值 | ✅ 指纹稳定一致 |
| B. 纯原生部署（从未用 CLI） | 包内默认 `.env` 无 `DEVICE_*`；compose 插值得到空串（会打印 "defaulting to blank" 警告，无害） | ⚠️ 机器信息缺失，Java 按空值契约处理；恢复路径：`nuwax-cli device-info --apply` 补注入，无需重新部署 |
| C. 原生方式"升级"（解压新包覆盖 `docker/` 目录） | 新包的 `.env` 不含 `DEVICE_*`（用户未保留旧 `.env` 时） | ⚠️ 指纹丢失；保留旧 `.env`，或 `device-info --apply` 一键重新注入（同机重采结果确定，device_id 不变） |
| D. 整个部署目录拷贝到另一台机器运行 | `.env` 与 `./data/` 冻结文件随目录迁移，新机器上报**旧机器**的指纹 | ⚠️ 一份授权跑两台：Linux 上由 §13.6 交叉验证挂载拦截（新机 machine-id 对不上）；非 Linux 为文件型注入的固有边界，可由服务端"同 device_id 多来源心跳"在线检测兜底 |

场景 D 补充：拷贝目录后若在新机器上重新使用 nuwax-cli，冻结文件的漂移检测会告警（默认维持旧 ID 不自动变更），需要显式 `device-info --refresh` 才重新绑定——与防漂移设计一致。

**面向用户的部署文档应写明**：

1. 需要授权功能的客户，**首次部署与升级走 nuwax-cli**；
2. 日常 `docker compose start/stop/restart/logs` 随意使用，互不影响；
3. 原生部署 / 手动换包后需要机器信息：`nuwax-cli device-info --apply` 一条命令注入；
4. 若手动换包升级，建议保留旧 `.env`（与 `./data/`），丢了也可用上一条恢复。
