# 需求：把 **beta** 的上传与发版收进 `build-agent-docker`（prod 继续手动）—— 请进仓库勘察后给出实施方案

> **结局（2026-10-26 注记，需求已实现，本文仅存档）**：beta 发布自动化已在 build-agent-docker 落地：
> `makefiles/27-compose-release.mk` + `scripts/registry/beta_release.py`（EXECUTE 默认 dry，只到
> "可审阅 JSON"，人工 commit/tag，CI `docker-version-beta.yml` 翻公网指针）。beta 瘦通道首发 0.0.136
> （tag `docker-registry-version-beta/v0.0.136`）。权威流程见那边 `skills/compose-release/SKILL.md` §4。
给你（codex）的任务是**先勘察、再出方案**，不是直接改代码。请读 `/Volumes/zhitai/git_workspace/build-agent-docker`
的真实 Make/脚本定义，回答 §5 的每个问题，然后按 §6 交付一份实施计划。

---

## 1. 需求原文（范围以此为准）

现状：发布者**手动**做两件事——① 在 `nuwax-cli` 仓库的 `docker-versions/{env}/latest.json` 里手填版本号和
下载地址；② 在构建仓库手跑 `nuwax-ossutil upload-docker ./packages/` 把打好的包传到阿里云 OSS。

要变成：

| 环境 | 要什么 |
|---|---|
| **beta** | 胖包（`docker-{arch}.zip`）和瘦包（`docker-slim-{arch}.zip`）的**上传 + 发版**都通过 `build-agent-docker` 里**新增的 make 命令**完成 |
| **prod** | **保持手动**：手改 JSON → git 提交 → 打 tag。不在本轮范围内，且新命令必须**结构上碰不到** prod |

"发版"指：让 OSS 上那份可覆盖的清单指针（`docker-version/{env}/latest.json`，瘦通道还有 `registry.json`）
指向刚上传的那批产物。**瘦包/胖包产物本身一次写入不覆盖。**

本轮**不改客户端代码**（`nuwax-cli` 仓库由发布方自己维护），方案里若认为客户端要配合改，写进 §5 的问题里
交回给我判断，不要动手。

---

## 2. 已实测的现状（这些是我在这台机器上量到的，可以直接当事实用）

**构建仓库产物目录是混的**——`packages/` 下现在同时有 8 个 zip：

```
docker-x86_64.zip      24,913,136,679   (23.2 GiB)   Oct 22 15:06
docker-aarch64.zip     23,576,477,546   (21.9 GiB)   Oct 22 15:17
docker-slim-x86_64.zip    523,726,192   (500 MiB)    Oct 21 20:36
docker-slim-aarch64.zip   523,725,402   (500 MiB)    Oct 21 20:36
docker-computer.zip / docker-computer-test.zip / docker-userapp-computer*.zip   （另外 4 个，与本次无关）
```

每个包旁边有 `*.sha256`，格式是 `sha256sum` 默认输出（`<64hex>␣␣<文件名>`，两个空格）。

**OSS 上现有约定**（bucket `nuwa-packages`，endpoint `oss-rg-china-mainland`，公网基址
`https://nuwa-packages.oss-rg-china-mainland.aliyuncs.com`）：

| 用途 | 键形态 |
|---|---|
| 胖包产物 | `docker/{YYYYMMDDHHMMSS}/docker-{arch}.zip` |
| 瘦包产物 | `docker-slim/{YYYYMMDDHHMMSS}/docker-slim-{arch}.zip`（+ `.sha256`）——本轮临时验证已按此传过 |
| 胖通道清单指针 | `docker-version/{beta,prod}/latest.json`（可覆盖） |
| 瘦通道清单指针 | `docker-version/{beta,prod}/registry.json`（客户端**编译期**写死这两个地址） |
| 清单历史 | `docker-version/history/{env}/{ts}.json` |

实测 `nuwax-ossutil list` 的结果（只读命令，见 §4）：`docker/20261022071920/` 下**只有** 2 个胖包 zip，
`docker-slim/20261021193721/` 下 4 个对象（2 zip + 2 sha256）。

**上传工具能力**（`nuwax-ossutil` 是我自己写的工具，已在这台机器配好凭据，配置在
`~/.config/nuwax-ossutil.toml`）：

```
nuwax-ossutil upload        --remote <任意前缀> <文件/目录…>   # --remote 必填；可指定目录
nuwax-ossutil upload-docker [PATHS…]                            # 只会自动写 docker/{ts}/，接受显式文件路径
nuwax-ossutil list --prefix <前缀>
nuwax-ossutil rm …
```

**关键发现：上传这件事在构建仓库里已经存在，只是全在 CI 里，一条 make 命令都没有。** 请先把这三条读清楚再谈新增，
否则你会重复实现或把两条通道的键布局搅混：

| 位置 | 做什么 | 与本需求的关系 |
|---|---|---|
| `.gitea/workflows/package-docker.yml:672-715`（Job `upload-oss`） | `find artifacts/ -name "*.zip" -o -name "*.tgz"` 全量收进 `upload/` 再 `nuwax-ossutil upload-docker ./upload/` → `docker/{ts}/`。**没有文件名白名单**；secrets 缺失时 `exit 0` **静默跳过上传** | 瘦包一旦进这个 job，就会被**无审批地顺带上传**，而且 URL 长得像胖包。这条既是坑也是现成的上传实现 |
| `.gitea/workflows/offline-bundle-test.yml:88-109` | `TS=$(date +%Y%m%d%H%M%S)`；显式 `ls dist/nuwax-*-*.tar.gz` 列文件；`nuwax-ossutil upload --remote "offline-test/$TS" $FILES` | **这就是本需求要的形态**——自选 ts、显式选文件、自定义前缀。照它的形状写，不要新发明 |
| `.github/workflows/ci-package-test.yml:260-321` + `ci-promote-release.yml` | 用**官方 aliyun `ossutil` v2**（不是 nuwax-ossutil）传 `docker/test/{run_id}-{run_number}/`，再 `promote` job 复制到 `docker/release/{ts}/`，靠 `environment: release` 审批 | 键布局与主线 `docker/{ts}/` **不是同一套**。需要你确认这套是否还在用（见 Q9）——别把自动化接到死流程上 |

清单（`docker-version/{env}/latest.json`、`registry.json`）**构建仓库里完全没有发布逻辑**——那边唯一的
`latest.json` 代码是 `build_config/agent-client/fetch_nuwaclaw_url.py`，它是**消费方**且 schema 是另一回事（deb 包）。

**两份清单的 schema 不对称，这是事实不是笔误**：

```
beta/latest.json  version 0.0.124  platforms.{x86_64,aarch64} keys = [hash, signature, url]
prod/latest.json  version 0.0.90   platforms.{x86_64,aarch64} keys = [signature, url]     ← prod 故意不带 hash
两者的 packages.full.hash 都是 "external"；两者的 patch 都是 null
```

**构建仓库里已有的、和本需求相邻的东西**（请优先复用，不要另立一套）：

- `makefiles/04-package.mk:153-238`：瘦包链路。`prepare-package-slim-*` / `zip-package-slim-*` /
  `check-package-slim` / `package-slim-publish` / `package-slim-publish-plan`。
- `scripts/registry/slim_publish.py`（288 行）：已经有**冻结 run 选择**、`--dry-run`、版本与架构一致性校验、
  `verify_zip_files()`。它的 `--version` 默认取自 `k8s/helm/nuwax-platform/VERSION`（即 `0.1.x` 那条序列）。
- `makefiles/15-version.mk`：只 bump **Helm/chart** 版本（0.1.x），与 `latest.json` 的 0.0.x 发布序列**没有任何关系**。
- 没有任何 make 目标做过 OSS 上传或清单渲染。

**瘦通道的客户端硬规则**（清单字段填错 = 客户机装不上，比不填更糟）：见同目录
`SLIM_REGISTRY_RELEASE_WORKORDER.md` §2 和 `SLIM_RECEIPT_CONTRACT_FOR_BUILD_AGENT.md`，方案里当作已知约束引用，
不必重复推导。

---

## 3. 这个需求本质上要新写的三件东西

1. **一条发布脚本**：显式选文件 → 算/取 sha256 → 上传到正确前缀 → **回读断言** → 渲染清单 JSON →
   git 提交 → 打 tag。
2. **两个 make 目标**：分别管胖通道 beta 与瘦通道 beta，环境**硬编码为 beta**，不接受 `ENV=` 覆盖。
3. **一道 prod 防呆**：新命令在任何执行路径下都不能写 prod 的对象或清单。这条不是"记得别传 PROD"，
   而是要有可验证的拒绝行为（验收见 §6）。

---

## 4. 勘察任务（全部只读，可以直接复制执行）

```bash
cd /Volumes/zhitai/git_workspace/build-agent-docker
# 4.1 现有瘦包发布脚本到底做了什么、产物落哪、有没有碰过上传
sed -n '1,288p' scripts/registry/slim_publish.py
make package-slim-publish-plan                      # 只读预览，不上传不 push

# 4.2 已有的三条 OSS 上传实现（必读，决定你是复用还是新增）
sed -n '660,720p' .gitea/workflows/package-docker.yml
sed -n '70,115p'  .gitea/workflows/offline-bundle-test.yml
sed -n '255,325p' .github/workflows/ci-package-test.yml
sed -n '1,140p'   .github/workflows/ci-promote-release.yml

# 4.3 胖包链路的清单/版本从哪来（确认构建仓库里确实没有清单发布逻辑）
grep -rn "latest.json\|docker-versions\|version-patch" makefiles/ scripts/ script/ | grep -v Binary | head -40
grep -rn "^[a-z0-9_-]*:" makefiles/15-version.mk makefiles/08-ci.mk | head -40

# 4.4 tag 触发的清单同步在另一个仓库，确认它现在管什么（只读）
sed -n '1,162p' /Volumes/zhitai/git_workspace/nuwax-cli/.github/workflows/sync-docker-version-to-oss.yml

# 4.5 客户端读清单的地址与"hash 是硬门槛"这两条，逐字核一遍
sed -n '395,410p' /Volumes/zhitai/git_workspace/nuwax-cli/client-core/src/constants.rs
grep -n "external\|required_full_hash\|NoUpgrade" /Volumes/zhitai/git_workspace/nuwax-cli/client-core/src/upgrade_strategy.rs | head -20

# 4.6 OSS 现状只读确认（不产生任何写入）
nuwax-ossutil list --prefix docker/20261022071920
nuwax-ossutil list --prefix docker-slim/
nuwax-ossutil list --prefix docker-version/beta/
nuwax-ossutil list --prefix docker/test/
nuwax-ossutil --help; nuwax-ossutil upload --help; nuwax-ossutil upload-docker --help
```

要重点搞清楚的：**`upload-docker` 传目录时会不会把 `docker-slim-*` / `docker-computer*` 一起扫进
`docker/{ts}/`**（这决定了必须显式列文件还是可以让它展开）。测法：先 `--help`/读工具源码或做零成本验证，
不要靠真上传 23 GiB 来试。这里有一处**互相矛盾、需要你解释**：我的习惯是 `nuwax-ossutil upload-docker ./packages/`，
而 `packages/` 里现在有 8 个 zip；可今天那批 beta 前缀 `docker/20261022071920/` 实测**只有 2 个胖包对象**。
要么我那次其实传的是空目录/别的目录，要么工具有过滤——两种答案会直接改变脚本的写法。

---

## 5. 需要你回答的问题（拿不准就停下问我，别自己拍）

| # | 问题 | 我这边的默认倾向（供参考，不是命令） |
|---|---|---|
| **Q1** | **beta 的版本号计数器放哪个仓库？** `latest.json` 的 `0.0.124` 目前手改在 `nuwax-cli/docker-versions/beta/`。如果 make 命令在 `build-agent-docker` 里，它要去读/写**另一个仓库**的文件，然后由那边的 tag 触发同步。可行但你得说清怎么定位那个仓库、要不要 `--nuwax-cli-repo` 参数、跨仓库提交怎么办。**或者**：清单文件本身也搬进 `build-agent-docker` 生成、直接 `nuwax-ossutil upload` 上 OSS，不再走 `nuwax-cli` 的 tag 同步 | 二选一，我需要一个明确推荐 |
| **Q2** | 瘦包清单 `registry.json` 的**版本号**用哪条序列：跟胖通道同号（`0.0.125`…）还是独立（`0.1.374`…）？坑在这里：客户端 `auto-upgrade-deploy` 里 `UpgradeArgs{force:false, check:false}` 是**硬编码**（[nuwax-cli/src/commands/auto_upgrade_deploy.rs:1072-1075](file:///Volumes/zhitai/git_workspace/nuwax-cli/nuwax-cli/src/commands/auto_upgrade_deploy.rs#L1072)），同一版本号在两条通道之间切换会被判 `NoUpgrade`、**什么都不做**。选同序列就要我在客户端加 `--force-full`；选独立序列不用改代码，但以后 `status` 显示的版本会随通道跳变 | 我倾向"同序列 + 客户端加 `--force-full`"，这决定权在我，你只需告诉我脚本按哪种渲染 |
| **Q3** | `upload-docker` 自动生成 `docker/{ts}/` 的时间戳，怎么让脚本**拿到它实际用的那个 ts** 再回填清单 URL？如果工具不打印/不可预测，就必须改用 `upload --remote docker/$TS`（脚本自己 `date +%Y%m%d%H%M%S`）。给出你的读码结论 | 我倾向脚本自己定 ts、只用 `upload --remote`，两条通道对称 |
| **Q4** | 回读校验怎么做断言：`curl -sS <url>` 再算 sha256，对 23 GiB 的胖包要多久、要不要改用 OSS 返回的 ETag/CRC 代替？瘦包 500 MiB 必须逐字节回读（瘦通道 hash 是硬门槛）。胖通道要不要回读由你权衡后建议 | 瘦包：必须回读；胖包：给建议 |
| **Q5** | 清单里 `signature` 字段现在填什么（我看到两份文件都是空串/占位）？有没有我遗漏的签名流程？脚本该原样保留还是能生成 | 沿用现值，别自己发明 |
| **Q6** | `packages/` 混杂 8 个 zip、而且上一批的包不会被清理：脚本**凭什么认定"这批"**？现成答案是复用 `slim_publish.py` 的冻结 run 身份 + `verify_zip_files()`，胖包侧有没有等价的"本批次"证明（`DEV_LOG_DIR` 里的 run？）。**绝不能靠 mtime 猜最新** | 必须绑定 run 身份，理由见 `slim_publish.py` 的注释 |
| **Q7** | 上传失败/回读不过的**半途状态**怎么处理：OSS 上前 3 个对象已写、第 4 个失败 → 清单还没发，所以没人受害。要不要脚本自己 `rm` 清理，还是留目录让下次构建覆盖新 ts？（注意 `docker-slim/{ts}/` 是写一次的语义） | 我倾向"失败即停、不清理、不发清单"，你评估 |
| **Q8** | 有没有可能复用 `nuwax-cli` 那个 `sync-docker-version-to-oss.yml`（tag 触发、已在生产用）——即新命令只负责**上传产物 + 改清单文件 + 打 tag**，把"清单上 OSS"这一步仍交给现有 CI 的 beta tag？这样新命令不直接写 OSS 清单，prod 防呆也更容易成立 | 我倾向这个：动作最少、复用已验证的同步 |
| **Q9** | `.gitea/workflows/package-docker.yml` 的 `upload-oss` job 与我的本地上传**是不是两条都在活的路**？如果是，新 make 命令和它如何分工（CI 继续管它自己的批次，本地 make 只管 beta 演练？还是把瘦包从 CI 那个"全量 zip 一锅端"的步骤里显式排除掉，避免同一条命令的两个入口互相覆盖）。查一下 Gitea 那边最近的 run 记录/artifacts，答不了就标"未知，需发布方确认"，不要猜 | 至少要给出"CI 不会把瘦包顺手传成胖包 URL"的证据 |
| **Q10** | `.github/workflows/` 里那套用官方 `ossutil` 的 `docker/test/{ts}` + `docker/release/{ts}` 方案（键布局与主线 `docker/{ts}/` 不同，靠 `environment: release` 审批）现在还在用吗？本需求**要不要**把它一起收敛。**只勘察、不动它**，除非我明确说 | 我怀疑它已废弃；请给证据（最近触发记录 / 是否引用了不存在的 make 目标） |

---

## 6. 交付与验收

**先给方案（含 §5 逐条回答、要新建/修改的文件清单、每个 make 目标的参数表），我确认后再写代码。**

写代码后的验收必须包含（缺任何一项我会退回）：

1. **不产生任何真实副作用**：所有自检走 `--dry-run` / 打印计划；**不允许**在这轮里真上传 OSS、真打 tag、
   真 push 镜像。任何真实发布动作由我逐次授权后我自己触发。
2. **prod 防呆的"拒绝真的咬人"收据**：不只是"命令里没有 prod 字样"。要实际执行一次最像误操作的调用
   （例如 `make release-beta-docker ENV=prod`、`--env prod`、把 `PROD` 塞进变量或参数），贴出**真实报错与
   非零退出码**。只证明 argv 被忽略是不够的，要证明写路径被挡。
3. **文件选择的证据**：`--dry-run` 打印出将要上传的**确切文件路径与目标键**，能看出 `docker-computer*.zip`
   和上一批的包**没有**被扫进来。
4. **回读断言的证据**：一次对已存在对象的真实 `curl -sS … | shasum -a 256` 比对（这是只读下载，允许），
   展示匹配输出；再展示断言失败时的报错文案。
5. **清单渲染测试**：脚本渲染出的 beta `latest.json` 与 `registry.json`，对着
   `nuwax-cli/docker-versions/beta/latest.json` 的真值做字段级 diff；`registry.json` 要覆盖
   `patch` 必须为 null、hash 必须是真 64 hex、`platforms.*.url` 与包名/前缀一致这几条硬规则的单测。
6. **不合并 prod 的 schema**：任何"把 prod/beta 用同一个模板渲染"的做法都是错的（prod 的 `platforms` 没有
   `hash` 键）。方案里如果出现了共用模板，必须明确说明 prod 分支被排除的证据。
7. 双平台：发布脚本是 Python，注意这台是 macOS（BSD `sed`/`shasum`，没有 `timeout`），CI 侧是 Linux。
   命令要写成两边都能跑的，或者明确说明只在哪个平台验证过。

---

## 7. 明确不要动的东西

- 胖包产物链路（`prepare-package-*` / `zip-package-*` / `export`）与 `check-package-slim` 的现有语义。
- `nuwax-cli` 仓库里的任何文件（包括 `docker-versions/`、`.github/workflows/`）——本轮由我改。
- `docker-version/prod/*` 的 OSS 对象。
- 凭据：`~/.config/nuwax-ossutil.toml` 内容不要读进代码、不要打印、不要写进脚本注释或 git。
- ACR 镜像 push 的时序与命名空间语义（`:latest` 浮动是已定决策，别改成固定 digest）。

**一条依赖关系要记在方案里**：瘦通道的**成功路径**目前被交付凭据契约卡住（`image_id` 记的是构建宿主本地
镜像 ID，客户端在 containerd 存储上看不到那个值；详见 `SLIM_RECEIPT_CONTRACT_FOR_BUILD_AGENT.md` §1）。
所以 `release-beta-slim` 这个命令可以现在就写，但它的**首次真实运行**要在那个契约修好之后；
`release-beta-docker`（胖通道）不依赖它，可以先自动化起来。请把这条门控写进方案，别让自动化把两个命令
当成同一成熟度。

---

## 8. IM 版

> 需求：把 **beta** 的"上传 OSS + 发版"从我的手工作变成 `build-agent-docker` 里的新 make 命令——胖包
> （`docker-{arch}.zip`）和瘦包（`docker-slim-{arch}.zip`）两条都要；**prod 继续手动**改 JSON→提交→打 tag，
> 新命令必须结构上碰不到 prod。
> 现在的手工动作是：在 `nuwax-cli/docker-versions/{env}/latest.json` 手填版本号和下载地址，再
> `nuwax-ossutil upload-docker ./packages/` 上传；`nuwax-ossutil` 是我本地配好凭据的自研工具。
> 请注意这几件实测到的事：`packages/` 里混着 8 个 zip（23 GiB 的胖包、500 MiB 的瘦包、还有 4 个不相干的），
> 所以不能靠目录展开选文件；beta 的 `platforms` 带 `hash`、prod 的不带，**两套 schema 不能共用一个模板**；
> 瘦包对 hash 是硬门槛，上传后必须回读校验再发清单；`slim_publish.py` 已经有冻结 run 选择 + `--dry-run` +
> zip 校验，能复用就别另写一套。
> 先出方案不要写代码，要回答 10 个问题（清单计数器放哪个仓库、`registry.json` 版本号用 0.0.x 还是 0.1.x、
> ts 谁决定、23 GiB 要不要逐字节回读、signature 填什么、怎么证明"这批"、半途失败要不要清理、清单上 OSS 能否
> 继续走现有 tag 触发的 CI 同步、Gitea CI 的 `upload-oss` 和我的手动上传怎么分工、GitHub 那套
> `docker/test`+`docker/release` 是不是已经废弃）。验收里必须有"prod 防呆真的会拒绝"的实际报错收据，且这一轮
> **不许真的上传或打 tag**。另外：瘦包命令可以先写，但首次真跑要等交付凭据契约（`image_id` 语义）修好，
> 胖包命令不受影响。
> 勘察时最重要的发现留给你：**上传逻辑仓库里已经有了，但全在 CI**（`package-docker.yml` 的 `upload-oss` 是
> `find *.zip` 一锅端进 `docker/{ts}/`，`offline-bundle-test.yml` 才是"显式列文件 + `upload --remote` 自定义前缀"
> 的正确形状），`make` 侧一条都没有，清单发布更是完全没有。所以这是"把 CI 的形状搬到 make 并加选择性与门控"，
> 不是从零发明。
