# 瘦包上传 OSS 与 beta/prod 发布规划（待拍板）

范围：`build-agent-docker` 已在本地产出瘦包，**尚未上传**。本文只解决三件事：
① 瘦包放 OSS 哪个前缀；② `registry.json` 怎么发布、beta 与 prod 怎么分开；③ beta 演练怎么在
不动生产的前提下把"成功路径"真跑通。客户端代码不动，需要你拍板的点集中在 §5。

---

## 1. 现状（本机实测，2026-10-21）

| 产物 | 字节 | 折算 | sha256 |
|---|---|---|---|
| `packages/docker-slim-x86_64.zip` | 523,715,212 | 499.5 MiB | `0643cef9b101aa29e26837cbbdf3d2b61ffc1ef760b0eb95ddf19b9afdb03723` |
| `packages/docker-slim-aarch64.zip` | 523,725,402 | 499.5 MiB | `16e47af52532325f11569df0b110fa08b3e4d590eb2b32c788d19b5787f4acf5` |
| 同批胖包 `docker-x86_64.zip` | 15,127,662,997 | 14.09 GiB | 胖包链路未改（工作副本今日重建过，与上午量到的尺寸不同，以当前值为准） |

瘦/胖 ≈ **3.5%**（约 28 倍）。瘦包 499.5 MiB 里几乎全是 JAR：`docker/app/app.jar` 321.3 MiB +
`docker/im-app/*.jar` 159.5 MiB ≈ 481 MiB —— **剩下的大头不是镜像，是 Java 产物**；14 GiB 的镜像部分
确实一个 tar 都没带。

包内自检（我自己独立跑过，非引用发布 agent 的结论）：

- 2307 条目 / 94 个目录项；`docker/images/` **目录项存在且为空**，`docker/images/*.tar` **0 个** ✅
- `docker/DELIVERY_MANIFEST.json` 顶层键恰为 `architecture, components, compose, contract_version, mysql, release_sha256`
  （`deny_unknown_fields` 满足）✅，`architecture=amd64`、`ARCH_INFO=ARCHITECTURE=x86_64`（两套词汇各就各位）✅
- 我重算 `sha256(docker/docker-compose.yml)` **等于**清单 `compose.sha256` ✅
- 清单登记 2 个组件：`repo-collab` / `nuwax-im`，`target` 逐字节命中 compose ✅
- compose 内 25 处 `image:`（`image_references_of` 用 BTreeSet 去重后是 **17 个引用**）**全部**落在
  `nuwax-docker-images-registry.cn-hangzhou.cr.aliyuncs.com/nuwax/`
  （生产命名空间、字面量、无 `${}`）✅ —— 这一条是 §4 beta 难题的根源。
  17 个里 **6 个是浮动 `:latest`**（自研：`agent-platform-front`/`agent-platform-backend`/`mcp-proxy`/
  `rcoder`/`repo-collab`/`nuwax-im`），**11 个是固定 tag 第三方**（mysql/redis/minio/milvus/…）
- 保留的 `docker/im-app/.delivery.json`、`docker/repo-collab-app/.delivery.json` 是组件收据，客户端不读，无害

发布侧提交：`build-agent-docker` `01e5521`（`prepare --slim-no-image-tars` + `docker-slim-*` 目标），未 push。

---

## 2. OSS 现有约定（照此对齐，不另立体系）

bucket `nuwa-packages`，endpoint `oss-rg-china-mainland`，公网基址
`https://nuwa-packages.oss-rg-china-mainland.aliyuncs.com`。

| 用途 | 现有键形态 | 出处 |
|---|---|---|
| 胖包对象 | `docker/{YYYYMMDDHHMMSS}/docker-{arch}.zip` | `nuwax-ossutil upload-docker` 自动生成；beta 与 prod 的 `latest.json` **当前指向同一个时间戳目录** |
| 胖通道清单 | `docker-version/{beta,prod}/latest.json`（可覆盖的指针） | [sync-docker-version-to-oss.yml:116](file:///Volumes/zhitai/git_workspace/nuwax-cli/.github/workflows/sync-docker-version-to-oss.yml#L116) |
| 清单历史 | `docker-version/history/{env}/{YYYYMMDD-HHMMSS}.json` | 同上 :126 |

工具能力（已核 `--help`）：`nuwax-ossutil upload --remote <任意前缀>` 可指定目录，
`upload-docker` 只会自动写 `docker/{ts}/`。所以瘦包用独立前缀**不需要改工具**。

`registry.json` 的读取地址是**编译期常量**，发布位置没有选择余地：
`docker-version/prod/registry.json` 与 `docker-version/beta/registry.json`
（[constants.rs:402,405](file:///Volumes/zhitai/git_workspace/nuwax-cli/client-core/src/constants.rs#L402)）。

---

## 3. 建议的目标布局

```
nuwa-packages/
├── docker/{ts}/docker-x86_64.zip                      # 胖包：现状不动
├── docker-slim/{ts}/docker-slim-x86_64.zip            # 瘦包：新前缀，一次写入不覆盖
├── docker-slim/{ts}/docker-slim-x86_64.zip.sha256     # 供人工核对，客户端不读
├── docker-version/beta/registry.json                  # beta 指针（客户端固定读）
├── docker-version/prod/registry.json                  # prod 指针
└── docker-version/history/{env}/{ts}-registry.json    # registry.json 历史快照，回滚用
```

要点：

1. **一次构建一个 `docker-slim/{ts}/`，只写不改**；beta 与 prod 的区别只体现在"哪份 registry.json 指向它"，
   与胖通道现有习惯完全一致（同一份产物，环境靠清单分）。晋级 = 改指针，不重传大文件。
2. **回滚 = 从 `history/` 取上一份 registry.json 覆盖指针**，对象目录不动。
3. 不复用 `docker/{ts}/` 放瘦包，是因为发布 CI 的上传步骤是"把 artifacts 里所有 `*.zip` 收进 `./upload/`
   再 `upload-docker`"（[package-docker.yml:690-715](file:///Volumes/zhitai/git_workspace/build-agent-docker/.gitea/workflows/package-docker.yml#L690)）。
   瘦包一旦进同一个 job，就会被**无审批地**顺带上传，且 URL 长得像胖包，人工填清单容易拿错。
   分前缀后，瘦包上传必须是一条显式命令，即一次显式审批。
4. 保留策略：`docker-slim/` 留最近 5 个目录（≈5 GiB，相对胖包 14 GiB/版可忽略），更早的按季度清理;
   `docker/{ts}/` 现有习惯不动。
5. **上传后必须回读校验**再发清单：`curl -sS <url> | sha256sum` 与本地 `.sha256` 相等才允许写 registry.json。
   瘦通道对 hash 是硬门槛（`null`/`external` 直接拒绝下载），**填错的 hash 会让本来能装的机器装不上**，
   比不填更糟；这一条是新增的、胖通道从来没有过的纪律。

上传命令（beta，需你授权后执行）：

```bash
cd /Volumes/zhitai/git_workspace/build-agent-docker
TS=$(date +%Y%m%d%H%M%S)
nuwax-ossutil upload --remote "docker-slim/$TS" \
  packages/docker-slim-x86_64.zip packages/docker-slim-x86_64.zip.sha256 \
  packages/docker-slim-aarch64.zip packages/docker-slim-aarch64.zip.sha256
# 回读核对（两条都要等于本地值）
for a in x86_64 aarch64; do
  curl -sS "https://nuwa-packages.oss-rg-china-mainland.aliyuncs.com/docker-slim/$TS/docker-slim-$a.zip" \
    | shasum -a 256
done
```

---

## 4. `registry.json` 的写法（客户端逐条核过）

```json
{
  "version": "<见 §5-D2>",
  "release_notes": "nuwax智能体-slim-beta",
  "release_date": "2026-10-21T11:20:00Z",
  "packages": { "full": { "url": "…/docker-slim/<ts>/docker-slim-x86_64.zip",
                          "hash": "<64hex>", "signature": "", "size": 0 } },
  "platforms": {
    "x86_64":  { "url": "…/docker-slim/<ts>/docker-slim-x86_64.zip",  "hash": "0643cef9…", "signature": "" },
    "aarch64": { "url": "…/docker-slim/<ts>/docker-slim-aarch64.zip", "hash": "16e47af5…", "signature": "" }
  },
  "patch": null
}
```

- `platforms.*.hash` 必填真值（架构词汇 `x86_64`/`aarch64`，与交付清单的 `amd64`/`arm64` 是两套，别混）。
- `patch` 必须 `null` 或缺省，出现 patch 块直接失败
  （[upgrade_strategy.rs:182-188](file:///Volumes/zhitai/git_workspace/nuwax-cli/client-core/src/upgrade_strategy.rs#L182)）。
- 有 `platforms` 时走分架构分支，`packages.full` 只为兼容人读，可留可不留，留就要一致。
- 发布文件仍然落在 `nuwax-cli/docker-versions/{env}/registry.json`，随 tag 同步上 OSS（改 workflow 是下一步，见 §6）。

**beta 演练的拦路问题（必须先定策略）**：compose 去重后的 17 个引用全在生产命名空间 `nuwax/`，
而清单只给 `repo-collab`、`nuwax-im` 两个组件配了凭据。于是 beta 真机跑瘦通道时：

- 这 2 个有凭据的组件：本地/仓库拉到的是生产仓**旧** `:latest`，与本次构建的凭据不符 →
  **停服前**失败（`does not match its delivery receipt`）。失败保护是有效的，但成功路径走不通。
- 其余 4 个自研镜像（front / backend / mcp-proxy / rcoder）**没有凭据** → 客户端按引用原样拉生产
  `:latest`，会**静默装成旧构建**，只有包内产物是新的。这是"混合版本"的真实来源，比前一种更危险，
  因为它不报错。

三个选项（成本已量化）：

| 方案 | 做法 | 得到什么 | 代价 |
|---|---|---|---|
| **P1（推荐）** | 先让发布侧把 front/backend/mcp-proxy/rcoder 也纳入 `DELIVERY_MANIFEST.components`（扩 `component_delivery.py` 的 COMPONENTS 集合，**不加新字段**），再一次性 `make push-prod` 把本批镜像推到 `nuwax/:latest`，然后跑 beta 成功路径 | 5 个自研镜像全部有凭据保护；成功路径真实可演；无凭据的只剩第三方固定 tag（mysql/redis/minio…） | 生产 `:latest` 被未验证构建覆盖一次，窗口内新装胖通道的机器会拿到它（胖通道不拉 registry，影响面=同命名空间的其他消费者）；发布侧一点工作量 |
| P2 | beta 专用瘦包用 `nuwax-test/` 引用的 compose 变体（beta 推 `push-test`，全程不碰生产） | 干净、可反复演练 | compose 变化会让 `compose.sha256`/`release_sha256` 跟着变，必须"先替换再出清单"；从此 beta 包与 prod 包**内容不同**，与"一份产物两个环境"的原则冲突；打包侧改动中等 |
| P3 | beta 只演失败路径与计划/预检（拿现网 `nuwax/:latest`） | 零风险、零改动 | 拿不到"成功路径 + 停机时长 + 下载字节"这三项最想要的实测数据 |

补充：**不建议**把 compose 改成固定 digest 或版本 tag —— `:latest` 浮动语义是 2026-10-20 定过的，
P1/P2 都保留它。

---

## 5. 需要你拍板的四点

| # | 选项 | 我的推荐 | 代价 |
|---|---|---|---|
| **D1** 瘦包前缀 | A `docker-slim/{ts}/`（新前缀，显式上传）／B 复用 `docker/{ts}/`（工具零改动，但 CI 顺带上传、URL 易混） | **A** | 一条新上传命令；工具不用改 |
| **D2** `registry.json` 的版本号 | A 与胖通道同一条发布序列（`0.0.124`…，同一构建同一号）／B 独立序列跟 compose 构建号（`0.1.372`…） | **A + 客户端加 `--force-full`** | A 有个真实的坑：客户端 `UpgradeArgs{force:false}` 是硬编码（[auto_upgrade_deploy.rs:1072-1074](file:///Volumes/zhitai/git_workspace/nuwax-cli/nuwax-cli/src/commands/auto_upgrade_deploy.rs#L1072)），同一版本号跨通道切换会被判 `NoUpgrade`，**什么都不做** —— 即"从胖切瘦"在已装该版本的机器上不会生效。加 `--force-full`（`run` 与 `registry-deploy` 各一个 flag，默认 false，现行为不变）是客户端一处小改动，我这边可以做。B 不需要改代码，但以后运维要习惯两个编号并存、`status` 显示的版本随通道变化 |
| **D3** beta 成功路径 | P1 / P2 / P3（§4） | **P1**（先扩 components，再一次性 push-prod 演练，需你明确授权推生产仓） | P1 有一次生产 `:latest` 覆盖窗口 |
| **D4** registry.json 上 OSS 的方式 | A 扩展现有 `sync-docker-version-to-oss.yml`：同一个 tag 触发，`latest.json` 照传，`registry.json` 存在才传（缺失就跳过，不影响纯胖发布），历史快照写 `history/{env}/{ts}-registry.json`／B 新建独立 workflow + 独立 tag 前缀 | **A** | A 改一个文件、一次发布动作同时处理两份清单；B 隔离更彻底但发布要打两个 tag，容易漏 |

---

## 6. 落地顺序（拍板后）

1. 发布侧：把 4 个自研镜像纳入 components（P1 前置），重出瘦包，自检 + 胖包回归不变。
2. 发布侧：`nuwax-ossutil upload --remote docker-slim/{ts}` 上传 beta 用瘦包，**回读 sha256 核对**。
3. 我方（nuwax-cli）：写 `docker-versions/beta/registry.json`（真 hash、`patch:null`）；
   按 D2 决定是否加 `--force-full`；按 D4 扩展 sync workflow；beta 真机跑
   `NUWAX_API_DOCKER_REGISTRY_VERSION_URL` 指向本地文件先做一次离线自检，再用 OSS beta 地址跑正式一次。
4. 拿到三项实测（总耗时 / 停机时长 / 下载字节）与失败路径演练结果后，才谈 prod：
   `docker-versions/prod/registry.json` + `docker-version/prod/registry.json`，prod 上传与推送逐次授权。
5. prod 首发期间 `latest.json` 与胖通道保持原样可用 —— 两条通道并行是既定前提，瘦通道出问题时
   `auto-upgrade-deploy run` 必须仍是可随时回去的路。

---

## 7. IM 版

> 瘦包本地已好（各 499.5 MiB，胖包 14.09 GiB，包内 0 个 tar、清单与 compose 哈希自洽，
> sha256 已实测）。OSS 方案建议：瘦包放**新前缀** `docker-slim/{构建时间戳}/`，一次写入不覆盖；
> 环境靠 `docker-version/{beta,prod}/registry.json` 这两个固定指针区分（客户端编译期写死），
> 历史快照进 `history/{env}/{ts}-registry.json`，回滚只覆盖指针。上传必须回读 sha256 再发清单 ——
> 瘦通道 hash 是硬门槛，填错比不填更糟。
> 要你定四件事：① 前缀 `docker-slim/` 还是混进 `docker/`；② registry.json 用 0.0.x 同序列（那我给
> 客户端加 `--force-full`，否则同版本跨通道切换会被判 NoUpgrade 什么都不做）还是独立 0.1.x；
> ③ beta 成功路径：我建议先把 front/backend/mcp-proxy/rcoder 也纳入交付清单凭据（现在只有 2 个组件有凭据，
> 其余自研镜像会静默拉生产旧 `:latest`，装成混合版本），再一次性 push-prod 演练；④ registry.json
> 走现有 sync workflow 顺带传（文件不存在就跳过），不新建 workflow。
