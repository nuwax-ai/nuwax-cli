# 给 build-agent-docker 执行 agent：瘦包交付凭据的三处调整（附实测证据与只读复现命令）

结论先说：**瘦包本身做得没问题**（包内 0 个镜像 tar、`docker/images/` 目录项保留、40 个产物哈希自洽、
`release_sha256` 用独立实现复算一致，客户端预检已实测通过）。要调整的是**凭据（identity）的语义、覆盖面和
出凭据的时序**这三点，否则瘦通道在客户机上要么静默装旧构建，要么永远拒绝部署。

客户端代码这轮不改，下列规则是你对照用的硬事实（都是我在这台机器上跑真实产物验证过的）。

---

## 1. 必须改：`image_id` 现在记的是"构建宿主本地镜像 ID"，客户端在 containerd 存储上看不到这个值

**实测到的失败**（Ubuntu 26.04 / Docker 29.5.1 / `Storage Driver: overlayfs` +
`driver-type: io.containerd.snapshotter.v1`，用真实瘦包 `docker-slim-x86_64.zip` 跑
`auto-upgrade-deploy registry-deploy`，前 10 个镜像拉取成功，第 11 个被拒）：

```
ERROR ❌ Operation failed: Registry images were not ready; services were left running on the old version;
caused by: Image …/nuwax/nuwax-im:latest does not match its delivery receipt:
  expected config sha256:a6f911d6…(or index sha256:d2d45c3f…),
  local store has     sha256:43958c93…
```

**为什么这两个值不可能相等**（同一台机、同一个引用，只读取证；下列命令都在 `build-agent-docker` 仓库根目录跑，
全是只读命令，不改任何镜像或文件）：

```bash
R=nuwax-docker-images-registry.cn-hangzhou.cr.aliyuncs.com/nuwax
# 客户端的比较对象：docker image inspect 的 .Id
docker image inspect --format '{{.Id}}' $R/nuwax-im:latest
# 仓库侧这个 tag 解析到什么：MediaType 是 image.index，Digest 就是客户端看到的那个数
docker buildx imagetools inspect $R/nuwax-im:latest | grep -E 'MediaType|^ *Digest'
# 两个架构包里的 image_id 是否相同（index 摘要与架构无关，所以"相同"才说明它是 index）
for p in x86_64 aarch64; do
  unzip -p packages/docker-slim-$p.zip docker/DELIVERY_MANIFEST.json \
    | python3 -c "import sys,json;m=json.load(sys.stdin);print(m['architecture'],{k:v['image_id'] for k,v in m['components'].items()})"
done
```

实测结果：`docker image inspect .Id` == `imagetools` 的 **index Digest**（`43958c93…`）；而两个架构包里的
`image_id` 分别是 `d2d45c3f…`（amd64 包）和 `8132eada…`（arm64 包）——**不同**，所以它不可能是 index 摘要，
它是每架构的 manifest 摘要。来源在
[scripts/build/component_delivery.py:99-109](file:///Volumes/zhitai/git_workspace/build-agent-docker/scripts/build/component_delivery.py#L99)
的 `inspect_image()`：取的是 `docker image inspect <ref>` 的根 `.Id`。

客户端接受规则见
[nuwax-cli/src/docker_service/registry_images.rs:407](file:///Volumes/zhitai/git_workspace/nuwax-cli/nuwax-cli/src/docker_service/registry_images.rs#L407)：
`identity == receipt.config_id || identity == receipt.image_id`；`identity` 取法见
[client-core/src/container/image.rs:103](file:///Volumes/zhitai/git_workspace/nuwax-cli/client-core/src/container/image.rs#L103)
（`docker image inspect --format {{.Id}}`，注释里已写明"经典存储给 config digest，containerd 可能给 OCI index digest"）。

**要改成什么**：

| 字段 | 现在的值 | 应当的值 | 为什么这样分 |
|---|---|---|---|
| `image_id` | 构建宿主本地 `.Id`（每架构 manifest 摘要） | **推送后、该引用在仓库里解析到的 digest**（多架构时就是 index digest；单架构时就是那个 manifest digest） | containerd 存储的客户机 `.Id` = 这个值 |
| `config_id` | 每架构 config blob 摘要 ✅ 保持不变 | 不变 | 经典 overlay2 客户机 `.Id` = config digest，靠它匹配 |

两字段合起来才同时覆盖两种镜像存储；只留一个必然在另一类机器上失败。
你们已经有算这个值的代码：`pinned_remote()`（[同文件 :134-143](file:///Volumes/zhitai/git_workspace/build-agent-docker/scripts/build/component_delivery.py#L134)）
对 `skopeo inspect --raw` 的**原始字节**取 sha256，得到的正是仓库侧该引用的 digest（多架构时=index digest）。
瘦包路径下把它的结果写进 `image_id` 即可，别再加新字段（客户端 `deny_unknown_fields`，加字段两边都要改）。
注意 `pinned_remote()` 返回的是 `repo@sha256:…` 引用形态，而 `image_id` 要的是 digest 段
（`sha256:<64hex>`，客户端只接受这个写法或裸 64 hex）。

**我没能证伪的一条，请你补**：上面说"经典 overlay2 主机 `.Id` = config digest、所以 `config_id` 能匹配"是按 Docker
语义推的，我手上两台 Linux 机（Docker 29.5.1 / 29.9.0）**都是 containerd 存储**，没实测过经典存储那条腿。
请在任一台 `docker info` 显示 `Storage Driver: overlay2` 的机器上跑一次，把 `docker image inspect .Id` 与
清单 `config_id` 对比。如果经典存储也报 index/manifest 摘要，那 `config_id` 这条就不成立，要另想办法。

---

## 2. 必须扩：只有 2 个组件有凭据，另外 4 个自研镜像正在**静默装旧构建**（已实测）

真实 compose 有 25 个 service、去重后 **17 个引用**，其中 **6 个是浮动 `:latest`**（全部自研），
**11 个是固定 tag 第三方**。`DELIVERY_MANIFEST.components` 只登记了 `repo-collab`、`nuwax-im` 两个。

刚才那次实跑里，无凭据的自研引用有 3 个已经被静默接受并落在本地（拉取按引用字典序，第 11 位被拒停后剩下的还没轮到）：

```
[1/17] agent-platform-backend:latest ready at sha256:d5502e54…
[2/17] agent-platform-front:latest    ready at sha256:ea1c433b…
[6/17] mcp-proxy:latest               ready at sha256:71bb7d56…
```

这三个是**生产仓当前 `:latest`**，不是本批构建，而包里的 JAR/前端产物是本批的 —— 装出来的就是混合版本，
而且不报错。第 4 个自研 `rcoder` 排在 `nuwax-im` 之后（第 12 位），这次因为第 11 位被拒停而没轮到，
按同样规则它也会被静默接受。客户端的规则是"没被 receipt 的引用按 compose 原样拉取、不做身份校验"，
所以这个洞只能由发布侧封：

**请把 compose 里 6 个自研 `:latest` 全部纳入 `DELIVERY_MANIFEST.components`**
（`agent-platform-front`、`agent-platform-backend`、`mcp-proxy`、`rcoder`、`repo-collab`、`nuwax-im`），
复用现有 `COMPONENTS` 集合的形状，**不要加新字段**。第三方固定 tag 不用登记（tag 不可覆盖，且重传为零）。

---

## 3. 时序要倒过来：先 push 到不可变构建 tag，再出清单封包，最后把 `:latest` 提升过去

第 1 点决定了 `image_id` 只能从**仓库**取，所以凭据必须在镜像已进仓库之后才能生成；第 2 点又要求
`:latest` 在被拉取时正好指向这批构建。直接把"push-prod"排在验证之前会留下一个真实风险：**生产 `:latest`
挂着还没验收过的构建**。可以用不可变 tag 把这个窗口消掉，而且不违反你们已定的"`:latest` 浮动、客户端不做
digest 固定"：

```
build（双架构）
  └─ push 到不可变构建 tag，例如 nuwax/rcoder:0.1.372          ← 不动 :latest
       └─ 从该 tag 取 digest（skopeo inspect --raw 原始字节）    ← 填进 image_id
            └─ 写 DELIVERY_MANIFEST → 封 slim zip → 自检/回归
                 └─ 通过后把 :latest retag 到同一 digest          ← 纯改 tag，不重推字节
                      └─ 发布 registry.json（客户端这时拉 :latest 就必然等于凭据）
```

digest 与 tag 无关（同一批字节推两个 tag 是同一个 digest），所以"提升 `:latest`"是仓库侧一次 retag，
既能被 registry.json 发布前断言（"manifest 引用的 digest 在 ACR 已存在"这条你们工单里已经写了），
又不会让未验证构建在生产 `:latest` 上驻留。

⚠️ 一个会咬人的细节：`image_id` 必须来自**推进仓库的那批字节**。如果出完清单后又重 build 了一次
（layer 里有时间戳，digest 会变），凭据就废了 —— 所以断言要在 push 之后、封包之前做一次，
`registry.json` 发布前再做一次。

---

## 客户端侧规则（这轮不改，给你自检用）

- 清单四层都 `deny_unknown_fields`：顶层 `contract_version/architecture/components/mysql/compose/release_sha256`；
  组件 `version/image_id/config_id/source/target/artifacts`；`mysql.{manifest,files}`；`compose.{path,sha256}`。
  多一个键就直接拒解析。
- digest 文本只接受 `sha256:<64hex>` 或裸 64 hex（占位符、多架构 `linux/amd64` 之类会被拒）。
- 架构词汇是 docker 的 `amd64`/`arm64`（客户端拿 `docker_service::get_system_architecture()` 比），
  与包名的 `x86_64`/`aarch64` 是两套，别混。
- `components.*.target` 必须逐字节等于某个 compose `image:` 引用；`target ⊄ compose 引用` 会失败。
  （反过来"components 必须等于 compose 集合"不成立，所以第 2 点要靠你们扩登记，不是靠客户端兜。）
- `compose.sha256` 会跟磁盘真实文件比；`release_sha256` 按 canonical JSON（键排序、紧凑分隔、UTF-8 直通）复算。
  **改任何一个字段都会连带 `release_sha256` 变**，包括第 1 点把 `image_id` 换成语义。
- 瘦通道对包 hash 是硬门槛：`registry.json` 的 `platforms.*.hash` 不能是 `null`/`external`。

## 你那边什么不用动

胖通道全部不变（离线 tar 仍然按现有链路走，客户端 `Images` 通道仍逐个 load + retag）；
`latest.json` 与 `sync-docker-version-to-oss.yml` 不要碰；瘦包的 `docker/images/` 空目录项、
compose 交叉校验、身份取自导出 tar 这些保持现状 —— 这次实测证明它们是对的。

## 验收（两轮，缺一个平台不算过）

1. 在 **containerd 存储**主机（如 Docker 29.5.1 + `io.containerd.snapshotter.v1`）用真实瘦包跑
   `auto-upgrade-deploy registry-deploy`：6 个自研引用全部过身份校验、服务在停服前就绪、`0 个容器被停`。
2. 在 **overlay2 存储**主机跑同一条：同样过校验（这条走 `config_id`，也是第 1 点里要我补的那条腿）。
3. 两个架构包对比：`image_id` **相同**（index 语义生效的标志），`config_id` **不同**，`architecture` 各为
   `amd64`/`arm64`。
4. 失败路径仍要保住：`:latest` 被覆盖后必须拒绝且**不停服**（本轮已实测有效，别在改造中弄丢）。

---

## IM 版

> 瘦包本身没问题（包内 0 tar、产物哈希与 release_sha256 都自洽，客户端预检实测通过）。要改三处：
> ① `image_id` 现在取构建宿主 `docker image inspect .Id`，但客户端在 containerd 存储上看到的是**多架构
> index digest**（实测 `43958c93…`，而你们两架构包里 `image_id` 是 `d2d45c3f`/`8132eada`，说明它是每架构
> manifest 摘要）→ 这条校验在 containerd 客户机上永远过不去；请把 `image_id` 改成"推送后仓库里该引用解析到
> 的 digest"（你们 `pinned_remote()` 已经在算它），`config_id` 保持每架构 config digest 不变，两个字段合起来
> 才覆盖 containerd + overlay2。
> ② 现在只有 repo-collab/nuwax-im 两个组件有凭据，实测另外 3 个自研 `:latest`（back/front/mcp-proxy）被
> 静默装成生产旧构建 → 把 compose 里 6 个自研 `:latest` 全纳入 components，别加新字段。
> ③ 时序倒过来：build → push 到**不可变构建 tag**（不动 `:latest`）→ 从该 tag 取 digest 写清单 → 封包验证 →
> 通过后把 `:latest` retag 到同一 digest → 发 registry.json。digest 与 tag 无关，所以不会有"生产 :latest 挂着
> 未验证构建"的窗口，也保留 :latest 浮动语义。
> 另：`image_id` 必须来自实际推上去的那批字节，出完清单别再重 build（layer 时间戳会变 digest）。
> 还想请你补一条我没证过的：在 overlay2 主机确认 `docker image inspect .Id` 是否等于清单 `config_id`。
