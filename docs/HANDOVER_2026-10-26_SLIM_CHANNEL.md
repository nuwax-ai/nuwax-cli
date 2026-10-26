# 接手说明 —— 瘦（registry）通道：prod 已首发，客户端还剩三件事（2026-10-26）

给接手的 Claude Code：**这不再是"规划阶段"**。发布自动化和瘦通道 prod 首发都已由别的 agent 落地并实跑过，
本仓库剩下的活是客户端四处（§4 的 P0–P4）。先读 §1 坐标并把 §2 的只读命令跑一遍，再动代码。

---

## 0. 一句话现状

`auto-upgrade-deploy registry-deploy` 已经在生产可用：公网 `docker-version/prod/registry.json` = **0.0.96**，
指向 `docker-slim/20261025232426/docker-slim-{arch}.zip`，两平台真 sha256、`patch: null`（我今天 curl 实测）。
beta 侧**没有** `registry.json`（NoSuchKey），瘦通道跳过了 beta 演练直接发 prod。
发布自动化落在 `build-agent-docker`：beta 用 `make prepare-beta-*`，prod 手改 JSON → commit → tag —— 正是当初提的形状。

**但我今天实测到一个必须先处理的裂缝**（§2.8）：线上那份 prod 0.0.96 瘦包里的凭据 `image_id`（OCI index digest）
和现网 `:latest` 的 index digest **不相等**，而 `config_id` **逐字节相等** —— 镜像内容就是那一版，
index 却换了（最可能是 SKILL §5 那个 `docker manifest create/push` 索引修复动作，见 §2.8 的取舍说明）。
客户端判定是 `config_id || image_id`，于是
overlay2 客户机能过、**containerd 客户机会在停服前拒绝部署**。这是 §4 的 P0。

---

## 1. 仓库坐标（接手前先核对，别信本文的快照）

| 仓库 | 状态 |
|---|---|
| `nuwax-cli`（本仓库，工作目录） | 分支 `feat/registry-slim-deploy` = `604b879`，工作树干净，**已与 origin 同步**（已 push），比 `main` 领先 129 commits，**没有开 PR** |
| `nuwax-cli/docker-versions/` | **已冻结**：beta `0.0.125` / prod `0.0.91` 是最后一版。真相源已迁走，见 `build-agent-docker/docker-versions/migration.json`（`source_commit = 604b879`，note 明写"新 publisher 永不触发旧仓库"）。**不要再编辑这里**，legacy `sync-docker-version-to-oss.yml` 保留但已不参与发布 |
| `build-agent-docker` | HEAD `c31bdb5`。**权威操作流程 = `skills/compose-release/SKILL.md`**（§3 打包 / §4 beta / §5 prod / §5.1 实战记录 / §6 实机验证 / §7 排障） |
| `nuwax-ossutil` | 源码仓库 `/Volumes/zhitai/git_workspace/nuwax-ossutil`，`version = 0.1.8`，已构建于 `target/release/nuwax-ossutil`（实测 `--version` = 0.1.8，有 `--forbid-overwrite / --verify-crc64 / --expected-bucket / --report-json`）。**这台机器 PATH 上装的 `/usr/local/bin/nuwax-ossutil` 还是 0.1.7，没有这些 flag** —— 要用 0.1.8 就显式写全路径或先更新全局安装 |

我这一轮产出的四份 `docs/SLIM_*.md` 现在是**决策记录**，不是待办清单；§4 说明哪几条已被实现取代。

---

## 2. 已证（都有坐标，可自行复核）

1. **prod 瘦通道清单已发布**：`curl …/docker-version/prod/registry.json` → `version=0.0.96`、
   `platforms.{x86_64,aarch64}` 键为 `[hash, signature, url]`、hash 为真 64 hex、`patch=null`、URL 在
   `docker-slim/20261025232426/`。beta 同路径 **NoSuchKey**。tag `docker-registry-version-prod/v0.0.96` 已推。
2. **两条胖指针同批**：`docker-version/prod/latest.json` = `0.0.96`、`docker-version/beta/latest.json` = `0.0.135`，
   都指向 `docker/20261025192333/`（beta 0.0.135 promote 成 prod 0.0.96）。
3. **发布自动化真实存在**：`makefiles/27-compose-release.mk:19-35` 四个目标（`prepare-beta-docker` /
   `prepare-beta-slim` / `check-beta-docker` / `check-beta-slim`，外加 `release-beta-*` 作为别名），
   统一入口 `scripts/registry/beta_release.py --channel {full,slim}`，`EXECUTE` 默认 `0` →
   `beta_release.py:323-327` 里 `real=False`，打印 `DRY RUN: no OSS/Git writes`（:371），只有 `EXECUTE=1`/`--execute`
   才写 OSS；并且把 `ENV/environment/RELEASE_ENV/PROD/env/ENVIRONMENT` 导出成 `NUWAX_RELEASE_REQUESTED_*` 以便脚本拒绝越界环境。
   上传走 `beta_release.py:180-185`，带 `--forbid-overwrite --verify-crc64`，工具名由 makefile 的
   `OSSUTIL ?= nuwax-ossutil` 导出 —— **这台机器 PATH 上那个是 0.1.7，没有这两个 flag，直接跑会失败**；
   要显式指向 §1 里的 0.1.8 二进制。
   它**只到"写出可审阅的 JSON + 打印后续命令"为止，永不自动 commit/tag**；tag 由人打，CI
   （`.github/workflows/docker-version-beta.yml` / `-prod.yml` → `publish_beta.py` / `publish_prod.py`）才翻公网指针。
4. **交付凭据契约按建议改了**：`scripts/build/component_delivery.py:136` 现在对 registry 拿到的
   **完整 manifest/index 原始字节**取 digest 作 `image_id`（注释明写"never its config.digest field"），
   `config_id` 单独取 config blob（:117-118 还留了 containerd 会把 OCI index 当 `inspect.Id` 的说明），
   :204-205 / :219-222 是这两条的产出点。这正是"客户端在 containerd 存储上比对不上"那个阻塞的修法。
5. **六镜像覆盖已成门槛，但只在 beta 生效**：`scripts/registry/release_evidence.py:84`
   `Slim contract not ready: receipts must cover all six first-party images`，:86 还要求 run 目录里有
   `registry-validation.json`（双架构 × containerd/overlay2 的真实验收回执）。调用方只有 `beta_release.py` 与
   `publish_manifest.py`；SKILL.md:82（§5）明写"**prod slim 人工维护清单无此门槛**"。
   线上那份 prod 包实测 `components` 只有 2 个（见 §2.8），所以另外 4 张自研镜像
   （front/backend/mcp-proxy/rcoder）在客户机上按 `:latest` **原样拉取、不做身份比对**
   （`9e543ea` / `719341d` 的既定语义）——"包内产物新、镜像可能是旧构建"这个混合版本风险是**当前真实存在**的，
   不是理论。门设在 beta 而 prod 直接放行，等于门只拦住了没走过的路。
6. **客户端基线**（我今天在 `604b879` 上跑的）：`cargo nextest run -p client-core -p nuwax-cli` →
   **551 passed / 4 skipped，exit 0**。注意这不代表瘦通道端到端已验证——见 §3。
7. **客户端仍没有跨通道强制换包的入口**：`client-core/src/upgrade_strategy.rs:104,120,158` 有 `force_full`
   形参（:667 有测试），`nuwax-cli/src/commands/update.rs:77,86` 把 `args.force` 传进去；但
   `nuwax-cli/src/cli.rs:7-15` 的 `UpgradeArgs` 只有 `force`/`check`，且
   `nuwax-cli/src/commands/auto_upgrade_deploy.rs:1072-1075` 硬编码 `force: false, check: false`。
8. **线上瘦包的 `image_id` 与现网 `:latest` 的 index digest 不等，而 `config_id` 逐字节相等**（我今天实测，命令见 §5.5）：
   `packages/docker-slim-x86_64.zip` 的 sha256 = `bd2e1996…`，与 `releases/prod/0.0.96/slim.json` 里登记的
   完全一致 —— 即我拆的这份就是线上那份包。包内 `DELIVERY_MANIFEST.components` **只有 2 个**
   （`nuwax-im`、`repo-collab`），对现网 `nuwax/<img>:latest` 逐条比：

   | 引用 | 收据 `image_id` | 现网 index digest | 收据 `config_id` | 现网 amd64 config digest |
   |---|---|---|---|---|
   | `nuwax-im` | `c5e88ada…` | `1a128289…` ✗ | `ac94ecc…` | `ac94ecc…` ✅ |
   | `repo-collab` | `840bd51e…` | `01814362…` ✗ | `20b80d8f…` | `20b80d8f…` ✅ |

   即**镜像内容（config）逐字节相等，只有 index 不等**（我用脚本严格比过，两个组件都是
   `image_id==live_index: False / config_id==live_amd64_config: True`，见 §5.5）。
   index 变化的**最可能**来源是 SKILL §5 记的那条修复：ACR 跨命名空间复制会把 `:latest` 索引写旧，
   用 `docker manifest create` + `docker manifest push` 重建 —— 重建必然换掉 index digest。
   我也没有排除"采集 `image_id` 时看的本来就不是 `nuwax/<img>:latest`"这一可能；
   但**无论成因是哪个，结论一样**：containerd 客户机拿现网 index 去比对，两条都不等。
   结合客户端判定 `registry_images.rs:407`（`identity == config_id || identity == image_id`）：
   overlay2/classic 客户机 `.Id` = config digest → **命中 `config_id`，通过**；
   containerd-snapshotter 客户机 `.Id` = OCI index digest → **两条都不等 → 在停服前拒绝部署**（不会造成损害，
   但升不上去）。`.45` 上那台 containerd 机今天没复测，这一半是从 `client-core/src/container/image.rs:103`
   的实现与 2026-10-21 的实测推出来的 —— §5.5 给了复测命令。

---

## 3. 未证 / 别当事实用（接手方需要自己判定）

1. **同版本号跨通道会不会死锁**：prod 胖 = prod 瘦 = `0.0.96`。按 §2.7，已用胖通道升到 0.0.96 的机器跑
   `registry-deploy` 会走 `NoUpgrade` 分支、**什么都不做**。我没有实机复现这一条（需要一台已装 0.0.96 胖通道的机器），
   只做静态判断。§2.8 那条是**已经实测到的不等**，比这条更硬，别把它当次要项。
2. **瘦通道成功路径的真实客户机数据**（总耗时 / 停机时长 / 实际下载字节）：我手里没有 prod 0.0.96 那批的实测。
   `releases/prod/0.0.96/slim.json` 有上传侧的 crc64+sha256+`forbid_overwrite` 回执，但**没有部署侧**数据。
3. **beta slim 从没发过**：本地 tag 列表里只有 `docker-version-beta/v0.0.128…135`，没有任何
   `docker-registry-version-beta/*`。
4. **containerd 客户机今天是否真的拒绝**（§2.8 的后半）：`192.168.31.45` 是 containerd 快照存储，
   我这一轮只做了 registry 侧取证，**没有**在它上面重跑 `registry-deploy`。这条一跑就定性，见 §5.5 + §4 P0。
5. **两台测试机上的遗留物**（我今天清点了）：`.45` 上 `192.168.31.45:/home/soddy/rig-verify-20261021-slim`
   仍在，**26 GB**；该机现有 30 个 `nuwax` 镜像、根盘 75% 已用（886 GB 空闲）。
   `.41` **需要密码**（`BatchMode` 直接被拒），本会话在它上面留的 scratch 我这次没进去看 —— 要清点请发布方自己跑。
   **删不删由发布方决定，我不会动。**

---

## 4. 客户端待办（本仓库，按优先级）

| 级别 | 事项 | 落点 | 验收要求 |
|---|---|---|---|
| **P0** | **让凭据的 index 那一腿不再被发布流程自己打掉**（§2.8：现网 index digest ≠ 收据 `image_id`，而 config 完全相等 → containerd 客户机会拒绝部署 prod 0.0.96 瘦包）。三个选项，**要发布方选，别自己拍**：<br>(a) 发布侧把 `image_id` 的采集/重算挪到索引修复（`docker manifest create/push`）**之后**，或修完索引后重出瘦包与清单；<br>(b) 客户端在 identity 两条都不等时，再解析"该引用现网 index 的平台子 manifest 的 `config.digest`"与 `config_id` 比对（内容判定，对索引重序列化免疫，代价是要在线查 registry 且需认证）；<br>(c) 两者都做。<br>我倾向 (a)：不动客户端判定强度。(b) 等于承认"内容相同即可"，是语义变更 | 发布侧：`build-agent-docker/scripts/build/component_delivery.py:134-143`（`pinned_remote`/`pull_identity` 取原始 index 字节）、SKILL.md:82（§5 的索引重建步骤）。客户端：`nuwax-cli/src/docker_service/registry_images.rs:400-415` + `client-core/src/container/image.rs:103` | 必须**双存储驱动**各跑一次真实 `registry-deploy`：containerd 机（`192.168.31.45`）与 overlay2 机（`192.168.31.41`）两条都通过，并留下 §5.5 那张"收据 vs 现网"对照表的前后差异。只在一台上绿不算绿 |
| **P1** | 打通"同版本跨通道换包"：给 `auto-upgrade-deploy run` 与 `registry-deploy` 各加一个 flag（建议 `--force-full`，默认 false，现行为不变），接到 `UpgradeArgs` → `run_upgrade` → `UpgradeManager::check_for_updates(force_full, channel)` | `nuwax-cli/src/cli.rs:7-15`、`nuwax-cli/src/commands/auto_upgrade_deploy.rs:1072-1075` | 必须两条都有：①不传 flag 时同版本仍判 `NoUpgrade`（回归，别把默认行为改了）；②传 flag 时确实进入 `FullUpgrade` 并按目标通道取清单 |
| **P2** | 改掉误导的错误文案 | `nuwax-cli/src/docker_service/registry_images.rs:412-413`（现写"The tag was most likely overwritten … refusing to deploy an unaccounted image"） | 今天实测到的真实情形是**索引被重建过、镜像内容就是那一版**，所以"多半被覆盖 + 来路不明的镜像"这个断言会误导运维。文案要列可核实的几种可能（`:latest` 索引被重建／`manifest create` 换过 index／凭据早于最后一次 push／本机存储驱动差异），并直接给出自查命令 `docker buildx imagetools inspect <ref>` 与"比 config digest"的提示。**只改文案，别顺手改判定逻辑** |
| **P3** | 复核通道记账与回滚边界是否覆盖新现实（prod 两条指针同版本共存了） | `nuwax-cli/src/commands/auto_upgrade_deploy.rs` 的 `channel_to_record`（无包可换时不翻通道，`9c1f1a9`）、`9e543ea`（无凭据引用每次都 pull）、`719341d`（按发布侧语义原样拉浮动 `:latest`）、`9635f65`（仓库可达性预检只告警） | 这四条 commit 是既有结论，先读再改；尤其**不要**把"浮动 `:latest` 强制 pin digest"重新提出来——那是 2026-10-20 已否决的方向 |
| **P4** | 把本仓库已过期的规划文档标注成"已被 build-agent-docker 实现取代"，避免下一个 agent 重复勘察 | `docs/SLIM_REGISTRY_OSS_PUBLISH_PLAN.md`（D1 采用 `docker-slim/{ts}/`✅、D2 采用同序列⚠️但 flag 未接、D3 未走 beta 演练、D4 由新 CI workflow 承担）、`docs/SLIM_BETA_PUBLISH_AUTOMATION_FOR_CODEX.md`（已实现）、`docs/SLIM_RECEIPT_CONTRACT_FOR_BUILD_AGENT.md`（§1 已修，但见本文 §2.8 的新形态）、`docs/SLIM_REGISTRY_RELEASE_WORKORDER.md`（§1 分工已变更：清单不再由本仓库维护） | 只加一段"结局"注记，不要重写正文（它是决策证据） |

---

## 5. 只读命令块（接手第一件事）

```bash
# 5.1 本仓库
cd /Volumes/zhitai/git_workspace/nuwax-cli
git status --short && git log --oneline -3 && git rev-list --left-right --count origin/main...HEAD
cargo nextest run -p client-core -p nuwax-cli          # 基线：551 passed / 4 skipped

# 5.2 公网清单现状（只读 GET，小文件）
for f in beta prod; do for n in latest registry; do
  printf "%s/%s: " $f $n
  curl -sS --max-time 30 "https://nuwa-packages.oss-rg-china-mainland.aliyuncs.com/docker-version/$f/$n.json" \
    | python3 -c 'import json,sys;t=sys.stdin.read();
print("NoSuchKey" if "<Error>" in t else json.loads(t).get("version"))' 2>/dev/null || echo "?"
done; done

# 5.3 发布侧自动化与门（只读）
cd /Volumes/zhitai/git_workspace/build-agent-docker
sed -n '1,40p' makefiles/27-compose-release.mk
sed -n '49,120p' skills/compose-release/SKILL.md
sed -n '70,100p' scripts/registry/release_evidence.py
make prepare-beta-slim RELEASE_RUN=<run 目录名>   # 不带 EXECUTE=1 → 打印计划后 return（:370-371，之前无任何写操作）
# 注意 RELEASE_RUN 是必填：execute() 明写 "RELEASE_RUN is required; never infer a batch from packages/"
# 越界环境在 execute() 第一行就被 guard_environment(os.environ) 拒绝
git tag | sort

# 5.4 测试机遗留清点（只列不改；BatchMode 避免卡在密码提示）
for h in 192.168.31.41 192.168.31.45; do echo "== $h"; ssh -o BatchMode=yes -o ConnectTimeout=5 $h 'bash -lc "df -h / | tail -1; docker images | wc -l; ls -d ~/*slim* ~/*verify* 2>/dev/null"'; done

# 5.5 凭据 vs 现网 digest 对照（P0 的取证；全是只读，注意 zsh 的 :l 修饰符陷阱，必须带花括号）
R=nuwax-docker-images-registry.cn-hangzhou.cr.aliyuncs.com/nuwax
unzip -p packages/docker-slim-x86_64.zip docker/DELIVERY_MANIFEST.json \
  | python3 -c 'import json,sys;d=json.load(sys.stdin);[print(k,v["image_id"],v["config_id"]) for k,v in d["components"].items()]'
for img in nuwax-im repo-collab; do
  echo "== ${img} 收据 vs 现网 index"
  docker buildx imagetools inspect --raw "${R}/${img}:latest" > /tmp/$img.idx 2>/tmp/$img.err \
    || { head -2 /tmp/$img.err; continue; }
  shasum -a 256 /tmp/$img.idx          # 客户端在 containerd 存储上看到的 .Id 就是这个
  python3 -c "
import json;m=json.load(open('/tmp/$img.idx'))
for e in m['manifests']:
    if e['platform']['architecture']=='amd64': print('  amd64 child', e['digest'])
"
done
# 再把 amd64 child 的 config.digest 取出来，与收据 config_id 比（overlay2 客户机的 .Id 等于它）
# docker buildx imagetools inspect --raw "${R}/nuwax-im@<child digest>" | python3 -c 'import json,sys;print(json.load(sys.stdin)["config"]["digest"])'
```

**zsh 陷阱**：本机 shell 是 zsh，`$img:latest` 会被解析成历史修饰符 `:l` + 残留串（我实测得到
`nuwax-imatest:latest` 这种假 not-found）。任何 `$var:suffix` 都要写成 `"${var}:suffix"`。

---

## 6. 纪律（发布方明确定过，违反代价高）

- **真实副作用需要逐次显式授权**：OSS 上传、ACR push、打 tag、prod 发布，都不要自己做。本文档写的时候
  `make prepare-beta-*` 的默认 `EXECUTE=0` 就是这条的产物，别绕过它。
- 生产 `:latest` 的镜像语义是**已定决策**（浮动，不 pin）。
- 测试机：`192.168.31.45`（soddy，个人开发机）**key 免密可用**，containerd 快照存储；
  `192.168.31.41`（swufe，overlay2）**今天实测免密被拒**（`Permission denied (publickey,password)`），
  要密码 → 由发布方自己跑，不要试图代跑密码 ssh；**`192.168.32.117` 不要用**（凭据待轮换）。
  `ssh … 'bash -lc …'`；`.45` 上有不属于本任务的存量
  （K3s、用户自己的 `nuwax-*` 部署、2379 端口冲突需要 `specs/*/linux-beta-45-preflight/derive_beta45.py` 派生包），
  不要清理/复用/覆盖别人的目录。cargo target 留在 `/Volumes/zhitai`，不要落到系统盘。
- 本机是 macOS（BSD `sed`/`shasum`，**没有 `timeout`**）；"mac 绿不等于绿"，跨平台相关改动要在真实 Linux 上验。
  Windows 机器（`.235`）上不要跑全量 workspace 测试。
- 提交纪律：分片本地提交、不中途停下等确认、**没有明确要求不 push**；"仔细检查下再 commit"是真门槛
  （数字、机制、文案都要核，别引用没复核过的数字）。
- 凭据（`~/.config/nuwax-ossutil.toml`、ACR/OSS key）不读进代码、不打印、不进 git、不进文档。

---

## 7. IM 版（转发给接手的 agent）

> 接手 `nuwax-cli` 的瘦（registry）升级通道。状态变了，别按旧规划做：**发布自动化和 prod 瘦通道首发都已经由别的
> agent 落地**——公网 `docker-version/prod/registry.json` = 0.0.96，包在 `docker-slim/20261025232426/`，
> 发布走 `build-agent-docker` 的 `make prepare-beta-{docker,slim}` + 人工 commit/tag + 那边自己的
> `.github/workflows/docker-version-*.yml`。**清单真相源已从 `nuwax-cli/docker-versions/` 迁到
> `build-agent-docker/docker-versions/`（见 `migration.json`），本仓库那份已冻结，别再编辑。**
> 先读 `build-agent-docker/skills/compose-release/SKILL.md`，那是权威流程。
> 客户端第一件事是我今天实测到的凭据裂缝（P0）：**线上 prod 0.0.96 瘦包**（本地
> `packages/docker-slim-x86_64.zip`，sha256 `bd2e1996…` 与 `releases/prod/0.0.96/slim.json` 一致，所以我拆的就是
> 线上那份）里 `image_id` = `sha256:c5e88ada…`（nuwax-im）/ `840bd51e…`（repo-collab），而现网 `:latest` 的
> **index digest** 是 `1a128289…` / `01814362…` —— 不等；但**amd64 config digest 与收据 `config_id` 逐字节相等**。
> 也就是说镜像内容就是那一版，index 却换了（最可能是 SKILL §5 那个索引重建修复动作，成因我没有排除其它可能）。
> 客户端判定是 `config_id || image_id`（`registry_images.rs:407`），所以 overlay2 客户机通过、
> **containerd 客户机会在停服前拒绝部署**。三个选项（要发布方选）：(a) 发布侧把 `image_id` 采集/重算挪到索引修复
> 之后或修完索引重出包与清单（我倾向这个）；(b) 客户端在两条都不等时再查一次现网 index 的平台子 manifest
> 的 `config.digest` 与 `config_id` 比；(c) 都做。验收必须 containerd（`.45`）与 overlay2 两条都真跑一次。
> 剩余三件：① **P1** prod 胖瘦两条指针同为 0.0.96，而 `auto_upgrade_deploy.rs:1072` 硬编码 `force:false`、
> `cli.rs` 的 `UpgradeArgs` 没有 `--force-full`，已装胖 0.0.96 的机器跑 `registry-deploy` 会判 `NoUpgrade`
> 什么都不做 —— 加 flag 接到已有的 `force_full`（`upgrade_strategy.rs:158`），默认行为不许变，要有"不传 flag
> 仍 NoUpgrade"的回归；② **P2** `registry_images.rs:412` 那句"The tag was most likely overwritten / unaccounted
> image"要改成可核实的几种可能 + 自查命令，**只改文案别改判定**；③ **P4** 给 `docs/SLIM_*.md` 四份文档加"结局"注记。
> 另一条已证风险：beta 侧有"六张自研镜像凭据覆盖 + `registry-validation.json`"的硬门（`release_evidence.py:84`），
> 但 SKILL:82 明写 prod slim 人工清单**没这道门**，且线上包实测 `components` 只有 2 张 → front/backend/mcp-proxy/rcoder
> 现在就是"静默拉生产 `:latest`、不做身份比对"。
> 硬纪律：不许真上传/推镜像/打 tag（要发布方逐次授权）；`:latest` 浮动是已定决策，不要重提 pin；
> 本机 PATH 上的 `nuwax-ossutil` 是 0.1.7（缺 `--verify-crc64/--forbid-overwrite`），0.1.8 在
> `/Volumes/zhitai/git_workspace/nuwax-ossutil/target/release/`；`.41` 免密今天被拒、要密码就交回发布方，
> `192.168.32.117` 不要用；本机 shell 是 zsh，`$var:latest` 会被当历史修饰符，必须写 `"${var}:latest"`；
> 提交分片、不 push、数字自己复核。基线：`cargo nextest run -p client-core -p nuwax-cli` = 551 passed / 4 skipped。
