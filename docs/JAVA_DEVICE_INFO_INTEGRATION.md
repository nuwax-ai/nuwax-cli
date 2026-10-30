# 设备指纹对接指南（Java 侧）

> 面向 backend（Java / Spring Boot）开发同事。nuwax-cli 部署时会把宿主机设备指纹注入容器环境变量，本文说明如何读取与使用。
> 完整设计背景见 nuwax-cli 仓库 `docs/DEVICE_FINGERPRINT_DESIGN.md`，对接只需这一篇。

## 1. 一句话说明

nuwax-cli 部署/升级服务时，采集**宿主机**（物理机/虚拟机）的硬件指纹，逐字段写入 `docker/.env`，经 compose 透传到 backend 容器环境变量；`application-external.yml` 里的 `device.*` 用**原生 map 占位符**接收它们。Java 侧纯原生绑定（零 JSON 解析），与平台签发的授权密钥配合实现"密钥 + 机器"绑定。

## 2. 配置总览

### 2.1 application-external.yml（部署包已包含，最终形态）

```yaml
# 设备指纹(nuwax-cli 采集宿主机硬件信息后逐字段注入 docker/.env, 经 compose 透传到容器;
# 用于授权管控的"密钥+机器"绑定。以下占位符的值全部由 nuwax-cli 自动生成, 勿手改)
device:
  # 机器主标识: "v1:" + 64位hex; 同机跨重启/重建/升级不变, 授权严格绑定用
  id: ${DEVICE_ID:}
  # 身份字段哈希(SHA-256, 64位hex); 参与授权判定(宽容匹配)
  # 平台差异或权限原因可能缺失(空串 = 该平台未采集, 不是篡改):
  #   macOS 无 disk_serial / 部分ARM无 dmi_uuid / 非root首次部署可能缺 dmi_uuid
  fields:
    machine_id: ${DEVICE_FIELDS_MACHINE_ID:}      # OS级身份: /etc/machine-id(重装系统才变)
    dmi_uuid: ${DEVICE_FIELDS_DMI_UUID:}          # 硬件级身份: 主板UUID(换主板才变)
    disk_serial: ${DEVICE_FIELDS_DISK_SERIAL:}    # 系统盘序列号(换系统盘才变)
    primary_mac: ${DEVICE_FIELDS_PRIMARY_MAC:}    # 主网卡MAC(换网卡/VM迁移才变)
  # 展示/限额信息(不参与身份判定)
  info:
    hostname: ${DEVICE_INFO_HOSTNAME:}            # 宿主机名, 管理界面展示"授权绑定机器"
    os: ${DEVICE_INFO_OS:}                        # linux / macos / windows
    arch: ${DEVICE_INFO_ARCH:}                    # x86_64 / aarch64
    cpu_model: ${DEVICE_INFO_CPU_MODEL:}          # CPU型号, 展示用, 可能缺失
    cpu_cores: ${DEVICE_INFO_CPU_CORES:0}         # 逻辑核数: 按"核数限额"授权时校验
    memory_gb: ${DEVICE_INFO_MEMORY_GB:0}         # 内存GB: 按"内存限额"授权时校验
    fingerprint_version: ${DEVICE_INFO_FINGERPRINT_VERSION:0}  # 指纹算法版本(与id前缀一致)
    collected_at: ${DEVICE_INFO_COLLECTED_AT:}    # 指纹冻结时间(RFC3339), 展示用
    wsl: ${DEVICE_INFO_WSL:false}                 # true=部署在WSL环境(界面可提示)
    containerized: ${DEVICE_INFO_CONTAINERIZED:false}  # true=CLI跑在容器内(指纹为容器视角)
```

### 2.2 docker-compose.yml 侧（部署包已包含，backend environment 段）

```yaml
      - DEVICE_ID=${DEVICE_ID}
      - DEVICE_FIELDS_MACHINE_ID=${DEVICE_FIELDS_MACHINE_ID}
      - DEVICE_FIELDS_DMI_UUID=${DEVICE_FIELDS_DMI_UUID}
      - DEVICE_FIELDS_DISK_SERIAL=${DEVICE_FIELDS_DISK_SERIAL}
      - DEVICE_FIELDS_PRIMARY_MAC=${DEVICE_FIELDS_PRIMARY_MAC}
      - DEVICE_INFO_HOSTNAME=${DEVICE_INFO_HOSTNAME}
      - DEVICE_INFO_OS=${DEVICE_INFO_OS}
      - DEVICE_INFO_ARCH=${DEVICE_INFO_ARCH}
      - DEVICE_INFO_CPU_MODEL=${DEVICE_INFO_CPU_MODEL}
      - DEVICE_INFO_CPU_CORES=${DEVICE_INFO_CPU_CORES}
      - DEVICE_INFO_MEMORY_GB=${DEVICE_INFO_MEMORY_GB}
      - DEVICE_INFO_FINGERPRINT_VERSION=${DEVICE_INFO_FINGERPRINT_VERSION}
      - DEVICE_INFO_COLLECTED_AT=${DEVICE_INFO_COLLECTED_AT}
      - DEVICE_INFO_WSL=${DEVICE_INFO_WSL}
      - DEVICE_INFO_CONTAINERIZED=${DEVICE_INFO_CONTAINERIZED}
```

### 2.3 docker/.env（nuwax-cli 托管、自动写入，用户无需手配）

```dotenv
# --- Managed by nuwax-cli: device fingerprint, DO NOT EDIT ---
DEVICE_ID=v1:aff7c6444dcd6d7bfb98890d0daaa6d004626157e58c7dbd62ba172e44c345e
DEVICE_FIELDS_MACHINE_ID=3fb59a395a93a73cfdf93528db13642d25fa7ed166c201be8f59ca9232c8aff9
DEVICE_FIELDS_DMI_UUID=c8743baf574a672e72e17e4eb99507b12600982e4eec8acb7cd2930320084ba4
DEVICE_FIELDS_DISK_SERIAL=f28d3bca917672d4d3c52eb6aa9476941451de660f0229ce805e80bb07984cbd
DEVICE_FIELDS_PRIMARY_MAC=035f79cf6426abcb5701c1fea6128a48ca00c6d673dda11fe917eff3065ada37
DEVICE_INFO_HOSTNAME='prod-web-01'
DEVICE_INFO_OS=linux
DEVICE_INFO_ARCH=x86_64
DEVICE_INFO_CPU_MODEL='Intel(R) Xeon(R) Platinum 8269CY CPU @ 2.50GHz'
DEVICE_INFO_CPU_CORES=8
DEVICE_INFO_MEMORY_GB=64
DEVICE_INFO_FINGERPRINT_VERSION=1
DEVICE_INFO_COLLECTED_AT='2026-10-08T10:00:00+08:00'
DEVICE_INFO_WSL=false
DEVICE_INFO_CONTAINERIZED=false
```

注意：**缺失字段不写键**（如 macOS 没有 `DEVICE_FIELDS_DISK_SERIAL`），yml 占位符默认空串/默认值兜底——这就是"该平台未采集"的表达方式。

CLI 再次注入时会删除已缺失字段的旧定义，避免刷新后 `DEVICE_ID` 与字段哈希不一致。
已有 v1 身份与冻结时间继续保留；`hostname`、CPU 和内存等环境信息每次采用当前采集值，
扩容无需重新绑定授权。

## 3. Java 侧绑定（原生 POJO，推荐）

```java
@ConfigurationProperties(prefix = "device")
public class DeviceProperties {
    private String id;                                  // "v1:..." 或空
    private final Fields fields = new Fields();
    private final Info info = new Info();
    // getter 省略（fields/info 为 final，无需 setter）

    public static class Fields {
        private String machineId;    // ← fields.machine_id（宽松绑定自动映射）
        private String dmiUuid;
        private String diskSerial;
        private String primaryMac;
        // getter/setter 省略
    }

    public static class Info {
        private String hostname;
        private String os;
        private String arch;
        private String cpuModel;
        private Integer cpuCores;
        private Long memoryGb;
        private Integer fingerprintVersion;
        private String collectedAt;
        private Boolean wsl = Boolean.FALSE;
        private Boolean containerized = Boolean.FALSE;
        // getter/setter 省略
    }
}
```

直接使用：`props.getId()`、`props.getFields().getMachineId()`、`props.getInfo().getCpuCores()`——**零 JSON 解析、零自定义 Converter**。

判空约定：`device.id` 为空串/null = 机器信息未提供（旧版 CLI / 非 CLI 部署），统一 `StringUtils.isBlank()` 判断；fields 各哈希同理。

> ⚠️ 若你更想绑定 `Map<String, String> fields`：注意环境变量 `DEVICE_FIELDS_MACHINE_ID` 经宽松绑定会**额外**产生一个 `machine.id`（点分）别名键，与 yml 的 `machine_id` 键并存、值相同——按机器匹配计数时需只认下划线键。**用 POJO 绑定则无此问题**（两种来源写入同一字段），故推荐 POJO。

## 4. 字段语义

### 4.1 `device.id`

```
v1:aff7c6444dcd6d7bfb98890d0daaa6d004626157e58c7dbd62ba172e44c345e
```

- 同一台机器跨重启/容器重建/CLI 升级**不变**；硬件变更后**仍不变**（CLI 冻结机制，授权连续性优先），需 CLI 侧显式 `--refresh` 才重新绑定
- 前缀 `v1` 是指纹算法版本，遇到未知前缀按"未知版本"告警处理

### 4.2 `device.fields.*`（身份哈希，参与授权判定）

各字段变化时机（宽容匹配依据）：

| 字段 | 变化时机 | 建议权重 |
|---|---|---|
| `machine_id` | 重装系统 / 整机克隆 | 高（OS 级身份） |
| `dmi_uuid` | 换主板 | 高（硬件级身份） |
| `disk_serial` | 换系统盘 | 中 |
| `primary_mac` | 换网卡 / VM 迁移 | 低 |

### 4.3 `device.info.*`（展示/限额，不参与身份判定）

`hostname` 管理界面展示用（用户可随意改主机名，**绝不参与判定**）；`cpu_cores`/`memory_gb` 用于**按资源限额授权**（如 ≤8 核 ≤64G）。

## 5. 建议的匹配规则（授权判定，可按业务调整）

```
① 密钥载荷 device_id == device.id                 → 同机（严格命中）
② 否则: machine_id 相等 或 dmi_uuid 相等          → 同机（重装OS/换主板互为兜底）
③ 否则: 非空字段中 ≥2 个相等                       → 同机
④ 否则                                            → 新机器，要求重新授权
```

含义：客户换网卡或数据盘不掉授权；重装系统 + 换主板同时发生才需重新授权。

## 6. 防篡改（可选但推荐）

compose 给 backend 加只读挂载（仅 Linux 宿主）：

```yaml
    volumes:
      - /etc/machine-id:/etc/host-machine-id:ro
```

Java 启动时校验：

```java
String raw = Files.readString(Path.of("/etc/host-machine-id")); // 文件不存在则跳过（非 Linux 宿主）
String check = sha256Hex(raw.trim().toLowerCase());             // 仅 trim + 转小写，与 CLI 规范化一致
if (StringUtils.isNotBlank(props.getFields().getMachineId())
        && !check.equals(props.getFields().getMachineId())) {
    // 注入的指纹被伪造 → 拒绝启用授权功能
}
```

防住"用户手改 `.env` 伪造已授权机器指纹"的攻击。

## 7. 空值与异常速查

| 场景 | 表现 | 建议处理 |
|---|---|---|
| 旧版 CLI / 非 CLI 部署 | `device.id` 空 | 自行决定降级行为 |
| 收到密钥但 `device.id` 为空 | 配置不完整 | 告警 + 拒绝 |
| `device.id` 前缀非 `v1:` | 算法版本不识别 | 告警 |
| 单个 fields 键为空 | 平台未采集 | 正常，不算不匹配 |

## 8. 如何自测

```bash
# 容器里确认变量注入
docker exec docker-backend-1 printenv | grep -E "^DEVICE_(ID|FIELDS|INFO)"

# 宿主机看指纹详情（nuwax-cli 1.0.136-beta.1 起）
nuwax-cli device-info --json
```

本地开发不起容器时，直接在 application-dev.yml 写字面量（结构与 §2.1 完全一致，把占位符换成真实值即可）。

授权签发流程建议：客户执行 `nuwax-cli device-info --json` 把输出发给你们 → 你们按 `device_id` / 字段哈希签发密钥 → 客户填入 → Java 验证。

## 9. 版本要求

- 逐字段注入自 nuwax-cli **1.0.136-beta.1（beta 渠道）** 起；`npm install -g nuwax-cli@beta` 获取
- 需要配套的平台部署包（compose 透传 + application-external.yml 声明，均在 build-agent-docker 的 docker/ 目录，随下一次平台包发版生效）
- 老部署升级 CLI 后，任意部署/启动命令或 `nuwax-cli device-info --apply` 即可补注入，无需重新部署
