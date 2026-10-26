# 瘦（registry）通道发布侧改造工单 —— 交给 build-agent-docker 的执行 agent

> **结局（2026-10-26 注记，工单已由 build-agent-docker 完成，本文仅存档）**：瘦包路径、`registry.json`
> schema（`packages.full` 回退槽位与 x86_64 一致）、OSS 严格上传协议均已在那边落地。**分工已变更**：
> 清单真相源迁至 `build-agent-docker/docker-versions/`（见其 `migration.json`，source_commit=604b879），
> nuwax-cli 的 `docker-versions/` 已冻结勿编辑；tag `docker-registry-version-*` 打在 build-agent-docker。
> 客户端命令入口是 `auto-upgrade-deploy registry-deploy`（瘦通道机器的升级用这条，不是 `run`）。
**目标一句话**：在 `/Volumes/zhitai/git_workspace/build-agent-docker` 新增一条"瘦包"打包与上传路径，产出
`docker-slim-{x86_64,aarch64}.zip`（与今天的胖包**同一份 compose、同一份 DELIVERY_MANIFEST、同样齐全的非镜像产物，
只少 `docker/images/*.tar`**）；现有胖包的行为、文件名、清单、上传方式**一律不改**。

客户端（`nuwax-cli`，分支 `feat/registry-slim-deploy`）已经实现并验证完毕，本工单只讲发布侧。

---

## 1. 分工边界（越界会互相覆盖，务必遵守）

| 事项 | 归属 |
|---|---|
| 瘦包打包、`.sha256` 计算、ACR 镜像 push、`unzip -l` 自检 | **本工单（build-agent-docker）** |
| `docker-versions/{beta,prod}/registry.json` 文件本身、它的 OSS 同步 | **nuwax-cli 仓库侧（我方）**，本工单**不要**碰 `nuwax-cli` 仓库 |
| `docker-version/{beta,prod}/latest.json`、`sync-docker-version-to-oss.yml`、胖包链路 | 不动。瘦通道靠**新增文件**，不靠改这两个 |

发布 agent 需要交付给我方的信息只有三项：**两个瘦包的 OSS 返回 URL、两个 `.sha256` 值、对应 ACR 镜像的
`image_id/config_id` 已经进了包内 `DELIVERY_MANIFEST.json`**。

---

## 2. 客户端会怎么判（这些是硬约束，违反 = 部署在停服前失败）

瘦通道子命令是 `auto-upgrade-deploy registry-deploy`，读 `docker-version/{env}/registry.json`
（[constants.rs:402,405](file:///Volumes/zhitai/git_workspace/nuwax-cli/client-core/src/constants.rs#L402)），
下载瘦包后在**停服之前**按包内 compose 引用拉镜像（[registry_images.rs:278](file:///Volumes/zhitai/git_workspace/nuwax-cli/nuwax-cli/src/docker_service/registry_images.rs#L278)）。

| # | 客户端规则 | 报错原文（用于反查） | 代码 |
|---|---|---|---|
| 1 | `registry.json` 里 `packages.full.hash` / `platforms.*.hash` 必须是真 64 hex；`null` 或 `"external"` **直接拒绝下载** | `registry package … publishes no SHA-256; refusing an unverified slim download` | [upgrade_strategy.rs `required_full_hash`](file:///Volumes/zhitai/git_workspace/nuwax-cli/client-core/src/upgrade_strategy.rs) |
| 2 | `registry.json` 的 `patch` 必须缺省或 `null`，出现 patch 块直接失败 | `The registry channel does not support incremental patch packages; …` | [upgrade_strategy.rs:182-188](file:///Volumes/zhitai/git_workspace/nuwax-cli/client-core/src/upgrade_strategy.rs#L182) |
| 3 | 瘦包必须含 `DELIVERY_MANIFEST.json`，否则拒绝 | `The registry channel needs the candidate package's DELIVERY_MANIFEST.json …use the offline channel` | [registry_images.rs:164-168](file:///Volumes/zhitai/git_workspace/nuwax-cli/nuwax-cli/src/docker_service/registry_images.rs#L164) |
| 4 | 清单至少要有 **1 个**登记身份的组件（现在生成器写 repo-collab + nuwax-im，满足） | `The registry channel needs DELIVERY_MANIFEST component image identities; the candidate package declares none` | [registry_images.rs:106-111](file:///Volumes/zhitai/git_workspace/nuwax-cli/nuwax-cli/src/docker_service/registry_images.rs#L106) |
| 5 | 清单每个 `components.*.target` 必须与包内 compose 的 `image:` 字符串**逐字节相等**（含全域名与 tag） | `DELIVERY_MANIFEST images are absent from the candidate Compose: …` | [registry_images.rs:114-125](file:///Volumes/zhitai/git_workspace/nuwax-cli/nuwax-cli/src/docker_service/registry_images.rs#L114) |
| 6 | `image_id`/`config_id` 只接受紧凑写法 `sha256:<64hex>` 或裸 `<64hex>`；带空格、`external`、空串都拒 | `… is not a sha256 digest: … (the registry channel requires real digests, not placeholders)` | [registry_images.rs:45-56](file:///Volumes/zhitai/git_workspace/nuwax-cli/nuwax-cli/src/docker_service/registry_images.rs#L45) |
| 7 | 清单 `architecture` 用 docker 词汇 `amd64`/`arm64`（与包内 `ARCH_INFO` 的 `x86_64` 是**两套词汇**，别混） | `DELIVERY_MANIFEST architecture '…' does not match this host ('…')` | [registry_images.rs:69-74](file:///Volumes/zhitai/git_workspace/nuwax-cli/nuwax-cli/src/docker_service/registry_images.rs#L69) |
| 8 | 客户端会**重算** `release_sha256`（键码点排序 + 紧凑分隔符 + UTF-8 直通，payload 不含 `release_sha256` 自身）；compose/mysql/artifacts 三类哈希逐个对盘校验，artifact 缺失或 0 字节即失败 | `DELIVERY_MANIFEST release_sha256 mismatch: files do not form the declared release` / `component artifact … is missing or empty` | [preflight.rs:1384-1470](file:///Volumes/zhitai/git_workspace/nuwax-cli/client-core/src/container/preflight.rs#L1384) |
| 9 | 清单结构是 `deny_unknown_fields`：`contract_version/architecture/components/mysql/compose/release_sha256` 六个顶层键，组件七个字段。**不要加任何新字段**（例如 `"slim": true`）——加了老 CLI 直接解析失败 | `DELIVERY_MANIFEST.json is not a valid v1 document` | [preflight.rs:1311-1349](file:///Volumes/zhitai/git_workspace/nuwax-cli/client-core/src/container/preflight.rs#L1311) |
| 10 | 拉到本地的镜像若与清单凭据不符（tag 在封包后被覆盖重推），**停服前失败**，服务继续跑旧版本 | `Image … does not match its delivery receipt: expected config … local store has …` | [registry_images.rs:379-410](file:///Volumes/zhitai/git_workspace/nuwax-cli/nuwax-cli/src/docker_service/registry_images.rs#L379) |
| 11 | 清单没登记的引用（mysql/redis/minio/alpine… 以及任何自研镜像只要没进 components）按 compose 原样 `pull`，浮动 `:latest` 每次都重新拉（**这是 2026-10-20 定下的语义，不要改成固定 digest**） | — | [registry_images.rs:316-330](file:///Volumes/zhitai/git_workspace/nuwax-cli/nuwax-cli/src/docker_service/registry_images.rs#L316) |
| 12 | 瘦包解出来的 `docker/` 目录形状按 `FullUpgrade` 全量替换，`images` **不在受保护目录清单**里（[constants.rs:271-283](file:///Volumes/zhitai/git_workspace/nuwax-cli/client-core/src/constants.rs#L271)）→ 瘦包一旦装上，机器上历史 tar 会被清掉 | — | — |
| 13 | registry 通道下 `prepare_services` 跳过 `docker load` 与 retag（[manager.rs:100-114](file:///Volumes/zhitai/git_workspace/nuwax-cli/nuwax-cli/src/docker_service/manager.rs#L100)），也不再强制 `docker/images` 存在（[manager.rs:148](file:///Volumes/zhitai/git_workspace/nuwax-cli/nuwax-cli/src/docker_service/manager.rs#L148)） | — | — |

**瘦包读取入口的相对路径**：客户端按 `docker/` 前缀归一后取 `docker-compose.yml`、`DELIVERY_MANIFEST.json`、`.env`
（[utils/mod.rs:540-586](file:///Volumes/zhitai/git_workspace/nuwax-cli/nuwax-cli/src/utils/mod.rs#L540)）——即 zip 内的
`docker/docker-compose.yml`、`docker/DELIVERY_MANIFEST.json`、`docker/.env`。**瘦包必须保持这个前缀**（与胖包一致即可）。

---

## 3. 现状（我已核过行号，直接照此改）

- 打包 staging 与 zip：[makefiles/04-package.mk](file:///Volumes/zhitai/git_workspace/build-agent-docker/makefiles/04-package.mk)
  —— `prepare-compose-package` 宏 :100-116（含 6 行 `--required-tar` :104-109），`zip-package*` :119-150
  （`cd packages/x86_64 && 7z a -tzip -mx=6 -mmt=12 ../docker-x86_64.zip docker` + `archive_checksum.py`）。
- 真正的"必须有 tar"闸门只有一处：`scripts/build/component_delivery.py` 的 `prepare/_prepare_locked`
  :558-636 —— `--required-tar` 非空文件断言 :602-603、repo-collab/nuwax-im 组件 tar 与其
  `.tar.delivery.json` 收据的身份校验 :588-598、compose 镜像引用交叉校验 :596-598、
  `(staged/"images").mkdir()` :618、tar + sidecar 复制 :619-626、app 快照 artifact 复制 :627-632、
  `ARCH_INFO` :633-634、`write_delivery_manifest` :635（函数 :525-555）。
- 交付清单**已经在生成**：字段与第 2 节契约一致，digest 已是紧凑写法（见胖包
  `packages/x86_64/docker/DELIVERY_MANIFEST.json`）。瘦包沿用即可，**不要新写一份生成器**。
- compose 内 `image:` 全是**字面量全域名**，无 `${}`：`…/nuwax/agent-platform-front:latest`、
  `…/nuwax/repo-collab:latest`、`…/nuwax/nuwax-im:latest` 等（[docker/docker-compose.yml:5,47,143,619,915,988,1071](file:///Volumes/zhitai/git_workspace/build-agent-docker/docker/docker-compose.yml#L5)）。
  这对瘦通道正好：客户端只读 `services.*.image`，不认 `build:`/`deploy.image`。
- zip 内已有目录项（`docker/images/` 等 93 个目录项），`7z -tzip` 天然写目录项。
- 上传：`.gitea/workflows/package-docker.yml:674-715`，`nuwax-ossutil upload-docker ./upload/`
  → `docker/{timestamp}/`；注意 :690-699 的 `find` 只收 `*.zip`/`*.tgz`，**`.sha256` 不会上传**。
- 本机 `packages/` 现状：胖包 `docker-x86_64.zip` ≈18 GB、`docker-aarch64.zip` ≈17 GB，各含 ~20 个 tar；
  `beta/latest.json:17,22` 有真 hash，**prod 无 hash**（`packages.full.hash = "external"`）。

⚠️ 先读一处再动手：CI 的 thin-package 分支会写 `PACKAGE_IMAGE_POLICY.json`
（[package-docker.yml:302](file:///Volumes/zhitai/git_workspace/build-agent-docker/.gitea/workflows/package-docker.yml#L302)）。
如果它已经是"按策略省 tar"的机制，就在它上面加瘦包模式，**不要再造第二个策略文件**。

---

## 4. 要改的（最小差异，全部新增，不改默认行为）

### 4.1 打包：给 `component_delivery.py prepare` 加一个显式开关

建议 `--slim-no-image-tars`（默认关闭，关=现行为逐字节不变）。开启时：

- **跳过** 6 行 `--required-tar` 的非空断言与 `docker/images/*-<arch>.tar` 的复制（:619-626）；
- **保留**：`docker/images` 目录项的创建（:618，保持与胖包同一目录形状，客户端不为瘦包特判）、
  compose↔镜像引用交叉校验（:596-598）、repo-collab/nuwax-im 的**身份校验**、app 快照 artifact 复制（:627-632）、
  `ARCH_INFO`、`write_delivery_manifest`。
- **不要**改成"从 ACR 现查 digest 来填凭据"：瘦包的 `image_id/config_id` 必须继续来自导出时那份 tar
  （也就是本批真正构建出来的镜像身份）。现查 `:latest` 会读到别人覆盖后的内容，凭据就失去意义，而且第 2 节
  第 10 条会在客户机上失败。所以瘦包流水线**仍然要跑 `make export`**，只是不把 tar 装进 zip。

Makefile 侧新增（示意，命名与现有风格一致）：

```make
# 04-package.mk 追加，勿改 zip-package-*
prepare-compose-package-slim = \
  python3 scripts/build/component_delivery.py prepare --ref-dir .cache/component-delivery \
    --docker-dir ./docker --dest packages/$(1)/docker --arch $(2) \
    --repo-version $(REPO_COLLAB_VERSION) --im-version $(IM_VERSION) \
    --slim-no-image-tars

zip-package-slim-x86_64:
	test -d packages/x86_64/docker
	rm -f packages/docker-slim-x86_64.zip
	cd packages/x86_64 && 7z a -tzip -mx=6 -mmt=12 ../docker-slim-x86_64.zip docker
	python3 scripts/build/archive_checksum.py packages/docker-slim-x86_64.zip
```

aarch64 同理。**文件名必须是新的**（`docker-slim-*`），瘦包与胖包在客户端缓存里按
`DownloadType::Registry` 已经分目录（`…/{version}/registry` vs `…/{version}/full`），同名会互相覆盖收据。

### 4.2 自检断言（CI 里加，本地也要能跑）

```bash
# 必须为空：瘦包不含镜像 tar
unzip -l packages/docker-slim-x86_64.zip | grep -E 'docker/images/[^/]+\.tar' && { echo "FAIL: slim 包含 tar"; exit 1; }
# 必须存在：目录项 + 清单 + compose + .env + 产物快照
unzip -l packages/docker-slim-x86_64.zip | grep -E '^\s+[0-9]+\s+[0-9-]+ [0-9:]+\s+docker/images/$'
for n in docker/DELIVERY_MANIFEST.json docker/docker-compose.yml docker/.env docker/ARCH_INFO; do
  unzip -l packages/docker-slim-x86_64.zip | grep -q " $n$" || { echo "FAIL: 缺 $n"; exit 1; }
done
# 清单自查（只读，不打印任何 .env 值）
python3 - <<'PY'
import json,zipfile
z=zipfile.ZipFile('packages/docker-slim-x86_64.zip')
m=json.loads(z.read('docker/DELIVERY_MANIFEST.json'))
compose=z.read('docker/docker-compose.yml').decode()
refs={l.split('image:')[1].strip().split()[0] for l in compose.splitlines() if l.strip().startswith('image:')}
assert m['contract_version']==1 and m['architecture'] in ('amd64','arm64')
assert set(m)=={'contract_version','architecture','components','mysql','compose','release_sha256'}, set(m)
for name,c in m['components'].items():
    assert c['target'] in refs, f'{name} target 不在 compose 里: {c["target"]}'
    for k in ('image_id','config_id'):
        v=c[k]; assert v.startswith('sha256:') and len(v)==71 and all(ch in '0123456789abcdef' for ch in v[7:]), (name,k,v)
print('OK', m['architecture'], len(m['components']), 'receipted;', 'release', m['release_sha256'][:12])
PY
# compose 哈希必须对得上（客户端会重算）
python3 -c "import hashlib,zipfile;z=zipfile.ZipFile('packages/docker-slim-x86_64.zip');print(hashlib.sha256(z.read('docker/docker-compose.yml')).hexdigest())"
```

最后一条打印值应等于清单里的 `compose.sha256`。**封包之后不得再改 compose**（含行尾、BOM、缩进），否则客户
端在停服前报 `compose … hash mismatch`。

### 4.3 镜像必须真的在 compose 写的那个命名空间里

**这是瘦通道最容易出事的地方，也是胖通道结构上不会暴露的问题**：胖包的 tar 里镜像名字是导出时就烤死的，
而瘦包靠客户机去 ACR 拉 compose 里写的 `…/nuwax/xxx:latest`。

- compose 硬编码生产命名空间 `nuwax`；`EXPORT_ALIYUN_NAMESPACE ?= $(ALIYUN_NAMESPACE_PROD)`
  （[makefiles/00-vars.mk:118](file:///Volumes/zhitai/git_workspace/build-agent-docker/makefiles/00-vars.mk#L118)）。
- 而 `make push-test`、`ALIYUN_NAMESPACE ?= $(ALIYUN_NAMESPACE_TEST)`（:80）推的是 `nuwax-test`。
- ⇒ **beta 演练若只 `push-test`，客户机上瘦部署会去拉生产命名空间的旧 `:latest`**：轻则装出与本次发布不符的构建
  （清单凭据不匹配 → 停服前失败），重则（该组件没进 components 时）静默装成生产仓里别人推的内容。
- 规则：**哪个命名空间被 compose 引用，就必须往哪个命名空间 push，且 push 必须早于 `prepare` 写清单**；
  beta 演练想避开生产仓，就显式 `EXPORT_ALIYUN_NAMESPACE=nuwax-test` 并同步换 compose 引用来源——但本工单
  不建议改 compose，beta 演练请按现有 `push-prod` 授权流程走，未获授权不得推生产仓。

push 后核对（只读）：

```bash
docker manifest inspect -v nuwax-docker-images-registry.cn-hangzhou.cr.aliyuncs.com/nuwax/repo-collab:latest \
  | python3 -c "import sys,json;d=json.load(sys.stdin);print([x.get('Descriptor',x).get('digest') for x in (d if isinstance(d,list) else [d])])"
```

### 4.4 上传

沿用 `nuwax-ossutil upload-docker`，但 `.sha256` 目前不被 CI 上传（:690-699 的 `find` 只匹配 zip/tgz）。
**瘦包的 hash 是客户端硬门槛（第 2 节第 1 条）**，所以必须拿到真值：本地直接
`sha256sum packages/docker-slim-*.zip`，或把 sidecar 一并放进 upload 目录。把
「两个瘦包 URL + 两个 sha256 + 版本号」回传给我方填 `registry.json`（我方负责，勿改 `latest.json`）。

---

## 5. 不许动的清单

1. 不改 `package-compose` / `zip-package*` / `prepare-compose-package` 现有目标的默认行为；瘦包走新目标。
2. 不改 `latest.json`、不改 `nuwax-cli` 的 `sync-docker-version-to-oss.yml`、不动胖包 URL 规则。
3. 不给 `DELIVERY_MANIFEST.json` 加新字段、不加 `slim`/`channel` 标记位（`deny_unknown_fields`，第 2 节第 9 条）。
4. 不把 compose 的 `:latest` 改成固定 digest，也不做客户端 digest 固定（2026-10-20 已定语义）。
5. 不在日志/回执里打印 `docker/.env` 内容（含凭据）；自检脚本只读文件名与哈希。
6. 不删历史胖包目录；瘦包与胖包同期共存，客户可按需选命令。

---

## 6. 验收（按顺序，全绿才算完成）

1. `make package-compose-slim`（或等价新目标）两架构各出一个 zip，`unzip -l` 断言全过（4.2）。
2. `sha256sum` 值与清单内 `compose.sha256` 一并与发布回执留档。
3. 包体积应从 ~17–18 GB 降到非镜像产物量级（估数百 MB，请留实测值）。
4. 真机端到端（干净机器，或已装 0.0.119 的演练机）：
   ```bash
   NUWAX_API_DOCKER_REGISTRY_VERSION_URL=http://<发布机或演练 OSS 地址>/registry.json \
     ./nuwax-cli auto-upgrade-deploy registry-deploy --port 8080
   ```
   期望日志顺序：`🧭 Registry deployment must ensure N image(s) for amd64 before stopping services`
   → 逐条 `⬇️ [i/N] pulling … (services are still running)` → `✅ Images ready in …s: a pulled, b already matched`
   → 才出现停服。**先出现停服、后拉镜像即判失败。**
5. 部署后 `docker images` 里 repo-collab/nuwax-im 的 ID 与包内清单 `config_id`/`image_id` 之一相等；
   `nuwax-cli auto-upgrade-deploy status` 显示 `Image delivery channel: registry`。
6. **失败路径演练一次**（可用一个故意改坏的清单副本指向的 zip，或封包后再覆盖重推一次 `:latest`）：
   必须停在 `does not match its delivery receipt` 或 `after N attempts; deployment aborted before any service
   was stopped`，且旧服务仍在跑、`config.toml` 的 `docker_channel` 仍是 `images`。
7. 断网演练：拉不到镜像时同样只停服前失败，不产生半部署。

---

## 7. IM 版（可直接粘贴）

> 瘦通道发布侧改造（build-agent-docker）：新增 `docker-slim-{x86_64,aarch64}.zip` 打包目标 +
> `component_delivery.py prepare --slim-no-image-tars` 开关（只跳过 tar 断言与复制，
> **保留** `docker/images/` 目录项、compose 校验、组件身份校验来自导出 tar 的 `image_id/config_id`、
> app 快照、`DELIVERY_MANIFEST.json` 与 `release_sha256` 生成）。胖包链路一行不改。
> 客户端已实现：包内必须有 DELIVERY_MANIFEST（不加新字段），digest 用紧凑 `sha256:<64hex>`，
> `architecture` 用 `amd64/arm64`，registry.json 必须发布真 sha256 且 `patch: null`。
> 最大坑：compose 硬编码生产命名空间 `nuwax`，而 `push-test` 推 `nuwax-test` —— 瘦包靠客户机去 ACR 拉，
> 所以**镜像必须推进 compose 引用的那个命名空间，且 push 早于封包写清单**。
> 完成后给我：两个瘦包 URL、两个 sha256、包体积实测；registry.json 和 OSS 同步在 nuwax-cli 侧我们做，别碰
> `latest.json` 和 sync workflow。详见 `nuwax-cli/docs/SLIM_REGISTRY_RELEASE_WORKORDER.md`。
