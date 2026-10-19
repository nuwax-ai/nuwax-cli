# 瘦部署（镜像走 Registry）实施方案

状态：**待评审**，未开始编码。
目标读者：nuwax-cli 维护者 + CI 发布侧。

---

## 1. 目标与硬约束

| # | 约束 | 说明 |
|---|---|---|
| C1 | `latest.json` 与全量包发布流程**不动** | 胖通道长期保留，内网/离线客户继续用 |
| C2 | 发布侧不维护"每版本差异包" | CI 只多产出：1 个 slim zip + 1 个 registry manifest + 镜像 push |
| C3 | 现有 `auto-upgrade-deploy run` 行为不变 | 新能力走**新子命令** |
| C4 | 停机时长不得包含镜像传输 | 拉取必须发生在停服**之前** |
| C5 | 客户机器要能直连 ACR | 否则只能用胖通道（本产品决定） |

要解决的问题（实测数据，x86_64 生产包）：
- 下载 15,166,545,257 B ≈ 14.1 GiB，其中绝大部分是 `docker/images/*.tar`
- 每次部署**无条件** `docker load` 全部 tar（[image_loader.rs:267](file:///Volumes/zhitai/git_workspace/nuwax-cli/nuwax-cli/src/docker_service/image_loader.rs#L267) 无跳过逻辑），耗时数十分钟
- 峰值磁盘 ~42 GiB

结论：只减下载字节数不够，**load 步骤必须一起干掉**。本方案两个都解决。

---

## 2. 方案总览

```
胖通道（不动）                          瘦通道（新增）
latest.json                            docker-version/registry/latest.json
  └─ fat zip（含 images/*.tar）           └─ slim zip（不含 images/*.tar）
auto-upgrade-deploy run                 auto-upgrade-deploy <新子命令>
  download 14GiB → load → up              download ~数百MB → pull → up
```

两条通道**共用**同一套替换、预检、部署、迁移机器：`PackageReplacement`、`preflight_*`、
`run_staged_deployment`、Live Diff 全部复用不改语义。差异只有两点：

1. **manifest 来源**不同（多一个 URL）
2. **镜像获取方式**不同（`load tar` → `pull` 且必须在停服前）

### 2.1 slim zip 内容

与 fat zip 完全同构，仅去掉 `docker/images/*.tar`。

> PR-1 已把 `check_environment` 的"images 目录必须存在"改成按通道判断
> （[manager.rs:141-152](file:///Volumes/zhitai/git_workspace/nuwax-cli/nuwax-cli/src/docker_service/manager.rs#L141-L152)），
> registry 通道不再要求这个目录，所以 slim zip 不必为它保留空目录项。

### 2.2 registry manifest

复用现有 `EnhancedServiceManifest` schema（`version` / `packages.full` / `platforms.{x86_64,aarch64}` / `patch`），
只换 URL 与 zip 文件名，**不引入新结构**。这样 `UpgradeStrategyManager::select_full_upgrade_strategy`
原样可用，无需新解析代码。瘦通道的 `patch` 字段恒为 `null`（不做差分增量，镜像层增量由 registry 负责）。

镜像身份不进 manifest —— 它是 `DELIVERY_MANIFEST.json` 的事（见 §3.3）。

### 2.3 新子命令

`auto-upgrade-deploy registry`（暂定名，见 §8 待确认项）。参数与 `Run` 一致：
`--port` / `--config` / `--project`。在 [cli.rs:45-88](file:///Volumes/zhitai/git_workspace/nuwax-cli/nuwax-cli/src/cli.rs#L45-L88)
的 `AutoUpgradeDeployCommand` 加一个 variant。

---

## 3. 数据契约

### 3.1 manifest URL 解析

现有优先级（[api.rs:662](file:///Volumes/zhitai/git_workspace/nuwax-cli/client-core/src/api.rs#L662)）：
`NUWAX_API_DOCKER_VERSION_URL` env > `NUWAX_API_DOCKER_ENV=test` → beta OSS > prod OSS（[constants.rs:392/395](file:///Volumes/zhitai/git_workspace/nuwax-cli/client-core/src/constants.rs#L392)）。

新增并列一套：常量 `NUWAX_API_DOCKER_REGISTRY_VERSION_URL_*` + env override
`NUWAX_API_DOCKER_REGISTRY_VERSION_URL`。`get_enhanced_service_manifest` 加一个"取哪个通道"的入参，
不复制函数体。

### 3.2 slim zip 的 SHA

**prod `latest.json` 现在 `platforms.*.hash` 是 null**，被
[package_cache.rs:56 normalize_sha256](file:///Volumes/zhitai/git_workspace/nuwax-cli/client-core/src/package_cache.rs#L56)
归一成 `"external"`，等于**下载完整性完全不校验**（beta 有真 SHA）。瘦通道包虽小，但没有 SHA 就 pull
十几个 GB 的镜像是拿客户环境赌。所以 PR-1 要顺手把 prod manifest 的 `hash` 补上（CI 侧一行）。

### 3.3 镜像身份：`DELIVERY_MANIFEST.json` v1 直接复用，不用升 contract

关键发现：现有 v1 已经有我们需要的字段，而且**从来没被读过**——
[preflight.rs:1326-1335](file:///Volumes/zhitai/git_workspace/nuwax-cli/client-core/src/container/preflight.rs#L1326-L1335)：

```rust
pub struct DeliveryComponent {
    pub version: serde_json::Value,
    pub image_id: String,   // 镜像 config blob 之外的 image id
    pub config_id: String,  // 镜像配置 blob 的 sha256
    pub source: String,     // ← 死字段
    pub target: String,     // ← 死字段
    pub artifacts: HashMap<String, String>,
}
```

对照 fixture（[delivery-from-producer.json](file:///Volumes/zhitai/git_workspace/nuwax-cli/client-core/tests/fixtures/delivery-from-producer.json)）：
- `target` = `nuwax-docker-images-registry.cn-hangzhou.cr.aliyuncs.com/nuwax/nuwax-im:latest` —— 正是完整镜像引用
- `artifacts` 只列应用产物（`im-app/*.jar`、`repo-collab-app/dist/index.js`），**不含 `images/*.tar`**

所以：瘦通道的 `DELIVERY_MANIFEST.json` 继续 `contract_version: 1`，`verify_delivery_manifest`
（哈希磁盘文件，[preflight.rs:1384](file:///Volumes/zhitai/git_workspace/nuwax-cli/client-core/src/container/preflight.rs#L1384)）
一行不改就能跑；只需把 `target` 从"发布侧写了没人看"变成**部署前的交叉校验依据**。

> 待核实：真实生产 `DELIVERY_MANIFEST.json` 的 `artifacts` 是否也没列 tar。若列了，slim 包必须同步剔除
> 该条目，否则 `verify_delivery_manifest` 会因为"artifact 缺失或为空"失败（preflight.rs:1424-1428）。
> **这是 PR-1 的第一个动作：拿一份真实 manifest 确认。**

### 3.4 生产 compose 的镜像引用现状

[build-agent-docker/docker/docker-compose.yml](file:///Volumes/soddy/git_workspace/build-agent-docker/docker/docker-compose.yml)，
25 处 `image:` **全字面量，无变量**：

- 可变 `:latest`，6 个，全是自研应用：`agent-platform-front`、`agent-platform-backend`、`mcp-proxy`、
  `rcoder`、`repo-collab`、`nuwax-im`（两处）
- 钉死版本，19 个第三方：`mysql:8.0`、`redis:7.0`、`milvus:v2.5.8`、`elasticsearch:9.2.1`、
  `rocketmq:5.5.1`、`minio:RELEASE.2025-09-07T16-13-09Z`、`alpine:3.22.4`、`busybox:1.36-uclibc` 等

推论（正面）：镜像身份 100% 活在这个文件里，而它是包里被逐文件替换的管理文件。CI 新增容器/改镜像版本，
客户端零感知 —— "拉最新 compose、按最新 compose 拉镜像"天然自洽。原设计中"`DOCKER_REGISTRY` 归谁写 /
CLI 要不要改写 `.env`"整块逻辑**删除**。

推论（唯一残留风险）：`latest` 可被覆盖。失败模式两种，都致命：
- 升级记的是 `0.0.120`，pull 到的是当天 `latest` 指向的别的构建，而
  [update_config_version, auto_upgrade_deploy.rs:254](file:///Volumes/zhitai/git_workspace/nuwax-cli/nuwax-cli/src/commands/auto_upgrade_deploy.rs#L254)
  老实写入 120 → 账实不符
- 回滚时旧 compose 引用的仍是 `latest` → **回滚后跑的还是新镜像**，回滚是假的

处置：**CI 把这 6 个引用改成不可变**，两种写法选一：
- `:0.0.120`（可读、可直接 pull、人眼可审）—— **推荐**
- `@sha256:<digest>`（最严但 compose 不可读，且 CI 必须先 push 再取 digest 再渲染 compose，是硬顺序依赖）

CLI 侧不猜：pull 完用 `config_id` 比对本地实际镜像，不一致即失败。

---

## 4. 新子命令执行时序

对照现有 `run_auto_upgrade_deploy`（[auto_upgrade_deploy.rs:942](file:///Volumes/zhitai/git_workspace/nuwax-cli/nuwax-cli/src/commands/auto_upgrade_deploy.rs#L942)）
的 9 步，只有两处顺序/内容不同（**加粗**）：

| 步 | 现有 `run` | 新命令 |
|---|---|---|
| 0 | `main.rs:142-161` CLI 自更新（可能 `exit(0)` 重启） | 同 |
| 1 | `DeploymentContext::new`（强制 `./docker/docker-compose.yml`） | 同 |
| 2 | `run_upgrade` 读 manifest + 定策略 | 读 **registry manifest** |
| 3 | `preflight_current_config` | 同 |
| 4 | `preflight_candidate_package` | 同（对 slim zip） |
| 5 | 下载 14GiB fat zip | 下载 slim zip |
| **6** | — | **停服前：解 compose → 抽镜像引用 → 与 `target` 交叉校验 → `docker pull` → `config_id` 校验** |
| 7 | `stop_docker_services_and_wait`（:1014，2s 轮询/30s 上限，超时即硬错） | 同 |
| 8 | `PackageReplacement::apply`（:1055）+ `fix_script_permissions` | 同（见 §5.1 的 images 目录后果） |
| 9 | `run_staged_deployment`（:317-445） | 同，但 `prepare_services` 的 step5/6 **换成 no-op** |
| 10 | `finish_package_deployment`（:1083） | 同 + 记录 `docker_channel = registry` |

第 9 步的细节：`prepare_services`（[manager.rs:76-107](file:///Volumes/zhitai/git_workspace/nuwax-cli/nuwax-cli/src/docker_service/manager.rs#L76-L107)）
的 step5 `load_images` 和 step6 `setup_image_tags_with_ducker_validation` 是唯一与镜像相关的两步，
其余（挂载目录、MySQL 配置权限、脚本权限）瘦通道一样需要。所以引入一个枚举：

```rust
pub enum ImageAcquisition { Load, Pull }   // Pull 时 step5/6 跳过
```

step 6 的 `setup_image_tags_with_mappings`（[image_loader.rs:333](file:///Volumes/zhitai/git_workspace/nuwax-cli/nuwax-cli/src/docker_service/image_loader.rs#L333)）
现在是"把 load 出来的名字去掉 `-amd64/-arm64` 后缀 re-tag"，**从不读 compose**。瘦通道里镜像名就是
compose 里的引用，不需要 retag —— 这正是 `Pull` 分支要跳过它的原因。

下游（`get_service_dependency_closure("mysql")` → 起 mysql → `wait_for_mysql_connection` →
`execute_sql_diff_upgrade` → 其余层 → MySQL container-id 前后相等守卫 :418-423 → 全量健康检查 →
最后写版本）全部原样，包括 mysql container-id 守卫：pull 出来的新镜像换了 container id 会照旧被抓住。

---

## 5. 两个必须现在接受的后果

### 5.1 切到瘦通道后，`docker/images/*.tar` 会被清掉

`apply_inner`（[package_replace.rs:780-883](file:///Volumes/zhitai/git_workspace/nuwax-cli/nuwax-cli/src/utils/package_replace.rs#L780-L883)）
的语义是：把**旧的受管文件**全部 rename 进 backup，再把 staged 文件 rename 进来。`images` 不在
`EXCLUDE_DIRS`（[constants.rs:271-286](file:///Volumes/zhitai/git_workspace/nuwax-cli/client-core/src/constants.rs#L271-L286)）
里，而 slim 包不提供新 tar → 旧 tar 被移除且无对应物。磁盘上白捡回 ~14 GiB，但：

- 之后若 `run`（胖通道）会用
  `scan_architecture_images`（[image_loader.rs:205/250](file:///Volumes/zhitai/git_workspace/nuwax-cli/nuwax-cli/src/docker_service/image_loader.rs#L205)）
  找不到 tar 而报错

所以**通道是配置状态，不能靠目录嗅探**。要显式记 `versions.docker_channel`，且两个子命令各自只认自己的通道，
切换须由用户显式执行。PR-1 先把这个字段建出来（默认 `images`，行为不变）。

⚠️ 落这个字段有个坑：`config.toml` 不是 `toml::to_string` 写的，是拿
`client-core/templates/config.toml.template` 做占位符替换（[config.rs:336-345](file:///Volumes/zhitai/git_workspace/nuwax-cli/client-core/src/config.rs#L336-L345)）。
只加 struct 字段而不加 template 占位符 + 对应 `.replace()`，字段会在保存时**静默丢失**。

### 5.2 文件回滚 ≠ 镜像回滚

`recover_error`（[package_replace.rs:1018](file:///Volumes/zhitai/git_workspace/nuwax-cli/nuwax-cli/src/utils/package_replace.rs#L1018)）
只还原文件，不动 docker 状态。瘦通道里如果 6 个引用还写 `latest`，回滚后旧 compose 引用的还是 `latest`，
本地 `latest` 已被新 pull 覆盖 → 回滚无效（见 §3.4）。钉死引用后，旧 compose 指向旧 tag，本地镜像层还在
（**PR-3 明确禁止在部署链路里 `docker image prune` / `rmi`**），回滚后 `up` 直接命中本地。

MySQL 的既有边界不变：MySQL 起来之前失败 = 完整 `recover_error`；之后失败 =
`retain_for_recovery`（[auto_upgrade_deploy.rs:1083](file:///Volumes/zhitai/git_workspace/nuwax-cli/nuwax-cli/src/commands/auto_upgrade_deploy.rs#L1083)），
永不自动回滚。另外提醒：`run` 目前**不做数据备份**（`backup::run_list_backups` 只被 `status` 用），
瘦通道同样不做 —— 不在本方案范围内，但要知道。

---

## 6. 实施分解（PR-1 → PR-5）

每个 PR 可独立合入、独立发布；前 4 个都不改变现网 `run` 的行为。

### PR-1｜镜像获取方式抽象 + 通道状态 + 事实核实
**状态：已完成**（分支 `feat/registry-slim-deploy`，mac 全绿 512 tests / clippy / fmt）。
落地与原计划的三处偏差：
- 决策入口用**配置状态** `versions.docker_channel` 而不是给 `prepare_services` 加参数：通道是必须持久化的
  部署事实（§5.1），从配置读可以让 `prepare_services`/`deploy_services` 签名全部不变，改动面最小。
- `check_environment` 的 images 目录检查改成**按通道判断**（见 §2.1），比原计划更宽松也更正确。
- `image_loader.rs` 的"按 digest 跳过已加载镜像"**没有**放进 PR-1，它属于 PR-3 的 pull 幂等逻辑一起做，
  否则同一个能力在两个 PR 里各写一半。

**剩余待办（PR-1 范围内、不阻塞后续 PR）**：拿一份真实生产 `DELIVERY_MANIFEST.json` 确认 `artifacts`
不含 tar（§3.3）—— 这条只影响 PR-5 的 CI 打包断言。

**改**：`manager.rs`（`ImageAcquisition`，`Pull` 时跳 step5/6 但仍保留 `check_environment` 对 `images` 目录的检查）、
`config.rs` + `templates/config.toml.template`（`versions.docker_channel`，默认 `images`）、
`image_loader.rs`（可选：`load` 前按 `config_id` 跳过已存在镜像，**只加不改默认行为**）。
**先做**：拿一份真实生产 `DELIVERY_MANIFEST.json` 确认 `artifacts` 不含 tar（§3.3）。
**验收**：胖通道 `run` 在 x86_64 + aarch64 结果与 PR-0 完全一致（同一 compose、同镜像 tag、同 SQL 产物）；
`Pull` 分支有单测证明 step5/6 未执行；`docker_channel` 写入后 `load_from_file` 能读回（防静默丢失）。
**为什么第一个做**：这是唯一会碰现有代码的部分，越早合入，后面 4 个 PR 越是纯新增。

### PR-2｜manifest 来源参数化
**改**：`constants.rs`（prod/beta 两个新 URL + env 名）、`api.rs:662`（加通道入参，复用同一函数体）、
`upgrade_strategy.rs`（不需要改，瘦通道只有一个 full 包）。
**验收**：默认路径 URL 与今天逐字节一致（回归测试）；env override 生效；瘦 manifest 的 `hash` 缺失时
按 `normalize_sha256` 语义拒绝或明确告警，不允许静默 `"external"` 进瘦通道。
**顺带**：prod `latest.json` 的 CI 补 `hash`（发布侧一行）。

### PR-3｜pull-before-stop 模块（本方案的技术核心）
**新增** `nuwax-cli/src/docker_service/registry_images.rs`：
1. 从 **staged 包**里的 compose（不是磁盘旧文件）取引用：
   `load_compose_config_with_env`（[config.rs:371](file:///Volumes/zhitai/git_workspace/nuwax-cli/client-core/src/container/config.rs#L371)）
   已能拿到 typed `services[*].image`，用它，**不要用正则**
2. 与 `DELIVERY_MANIFEST.components.*.target` 交叉校验：集合必须相等，多一个少一个都失败
3. 逐引用 `pull`（失败重试 N 次；带进度日志）。`pull_images()`
   （[image.rs:62](file:///Volumes/zhitai/git_workspace/nuwax-cli/client-core/src/container/image.rs#L62)）
   是 `docker compose pull`，但它读磁盘 compose —— 停服前磁盘还是旧 compose，**不能用**，
   要么按引用逐个 `docker pull`，要么对 staged 目录做 `compose -f <staged> pull`。倾向前者（可控、可校验）
4. 每个引用落地后 `docker inspect` 取 config digest，与 `config_id` 比对，不符即失败
5. 已存在且 digest 匹配则跳过（幂等，断点重跑不重传）
6. 全程不 prune、不 rmi

**硬性验收**：**pull 全部完成之前，代码路径上不存在任何 `stop` 调用**（写一个测试：mock 停服函数被调用即 panic）。
这是 C4 的机器证明，不是 review 意见。理由：`prepare_services` 今天跑在 `compose down` 之后，
照原位置放 pull 就等于"停机时长 = 镜像传输时长"，且 pull 失败留下服务全停状态。
另需一条预检：客户机能不能解析/访问 ACR（`docker manifest inspect` 一个引用即可），失败时在停服前给出
可读错误 + 指向胖通道。

### PR-4｜新子命令
**改**：`cli.rs:45-88` 加 variant、`main.rs`（同 `Run` 走 CLI 自更新前置）、
`auto_upgrade_deploy.rs`（新 `run_registry_deploy`，按 §4 时序复用 `run_staged_deployment`）、
`locales/*.yml` 三语新键、`status` 输出增加当前通道。
**验收**：`--help` 三语；错误路径全部有 i18n 键；失败语义与 §5.2 一致；在 dev box（192.168.32.117）
真机跑通一次 0.0.119 → 0.0.120 并记录各阶段耗时。

### PR-5｜发布侧
CI 产出：slim zip（断言 `unzip -l` 不含 `images/*.tar` **且** 含 `images/` 目录项）+ registry manifest +
镜像 push（**必须先把 `:latest` 换成不可变 tag**，再渲染 compose，再打 zip，再传 manifest）+
`DELIVERY_MANIFEST.json` 的 `target/config_id/image_id` 填真值 + 上传前断言 manifest 引用的 digest 在 ACR 已存在。
**验收**：CI 里跑一条端到端：干净机器 → `init` → 新子命令 → 服务健康 → `docker images` 全部引用与
manifest 一致 → 总耗时/停机时长/下载字节三项数据留档。

---

## 7. 预期收益（需 PR-5 实测证实）

| 指标 | 胖通道 | 瘦通道 |
|---|---|---|
| 下载字节 | 14.1 GiB | slim zip（估数百 MB）+ **镜像增量层** |
| 镜像就绪耗时 | `docker load` 全部 tar，数十分钟，无法增量 | 只传变化层；未变的 19 个第三方镜像**零传输** |
| 停机时长 | ≥ load 时长 | ≈ `compose up` + MySQL ready + Live Diff |
| 峰值磁盘 | ~42 GiB | 无 tar、无 fat zip |
| 发布侧维护 | 全量包 ×1 | 全量包 ×1 + slim 包 ×1 + manifest ×1（无差异包） |

第三行的收益是结构性的：`load` 每次全量、`pull` 天生增量。

---

## 8. 待你确认（4 项）

1. **子命令命名**：`auto-upgrade-deploy registry` / `slim-deploy` / `deploy-from-registry`？
2. **6 个 `:latest` 改成不可变 tag** —— 这是 CI 侧改动，PR-3 的校验强度取决于它。不做的话回滚是假的（§5.2），
   我不建议合入瘦通道。
3. **内网/离线客户**继续走胖通道，CLI 不做 mirror 逻辑 —— 按你"各走各的、互不影响"我按是处理，若要支持
   客户内网仓库需要另开设计。
4. **prod `latest.json` 补 `hash`** 是否允许动这条 CI（PR-2 顺带，纯收益，不影响老客户端）。

另外三项需要你提供（不是决策，是输入）：一份真实生产 `DELIVERY_MANIFEST.json`；slim 包估算体积；
以及（可选）连续两个版本 fat zip 的尾部 8 MiB，用来量化 §1 里剩下的那点镜像构建不确定性。

确认后我从 PR-1 开始动代码。
