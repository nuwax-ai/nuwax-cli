//! 瘦（registry）通道的镜像就绪逻辑：**在服务停止之前**把候选包 Compose 引用的
//! 镜像全部拉到本地，并按交付清单校验身份。
//!
//! 两条不变量：
//! 1. 本模块不调用任何停服 / `compose down` / prune / rmi 能力 —— 停机时长必须只等于
//!    "换文件 + 起服务"，不能等于"传镜像"；拉取失败时服务仍然跑在旧版本上。
//! 2. `DELIVERY_MANIFEST.json` 登记了身份的镜像，拉下来必须和凭据对上；tag 被覆盖过会在
//!    停服前失败，而不是"版本账记 0.0.120、实际跑别的构建"。
//!
//! 清单**没有**登记的引用（第三方镜像，以及发布侧尚未纳入凭据的自研镜像）按 compose
//! 原样拉取，浮动 `:latest` 也照拉 —— 2026-10-20 用户决定的瘦通道语义就是"这些镜像取
//! registry 当前的最新"。代价由发布侧承担：同一版本号在不同时间可能装出不同字节，
//! 回滚只回滚包内产物、不回滚这类镜像。

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;
use std::time::Duration;

use anyhow::{Context as _, Result, anyhow, bail};
use client_core::container::DockerManager;
use client_core::container::preflight::{DeliveryManifest, parse_delivery_manifest};
use tracing::{info, warn};

/// 候选包 Compose 里的一个镜像引用，以及发布侧对它登记的身份凭据。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RegistryImage {
    pub reference: String,
    /// `None` = 交付清单没有覆盖它（第三方镜像，或发布侧尚未纳入凭据的自研镜像）：
    /// 按引用原样拉取，不做身份校验，浮动 tag 也接受。
    pub receipt: Option<ImageReceipt>,
}

/// `DELIVERY_MANIFEST.components.*` 里对一个自研镜像的凭据。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ImageReceipt {
    pub component: String,
    pub image_id: String,
    pub config_id: String,
}

/// 归一化 digest：接受 `sha256:<64hex>` 与裸 `<64hex>`，其余报错。
///
/// 校验规则本身复用 `package_cache::normalize_sha256`（全仓库只有一份 sha256 文本判定），
/// 这里只补瘦通道多出来的那条要求：凭据必须存在，占位符不算。
fn normalize_digest(value: &str, what: &str) -> Result<String> {
    let invalid = || anyhow!("{what} is not a sha256 digest: {value}");
    let hex = client_core::package_cache::normalize_sha256(Some(value))
        .map_err(|_| invalid())?
        .ok_or_else(|| {
            anyhow!(
                "{what} is not a sha256 digest: {value} \
                 (the registry channel requires real digests, not placeholders)"
            )
        })?;
    Ok(format!("sha256:{hex}"))
}

/// 把候选包 Compose 的镜像引用集合与交付清单交叉校验，得出确定性的拉取计划。
///
/// 交付清单只登记有凭据的组件，所以规则是 `targets ⊆ compose 引用`，不是集合相等：
/// - 清单登记的镜像不在 Compose 里：说明包内 compose 与交付凭据不是同一份发布，失败；
/// - 同一 target 被两个组件登记成不同身份：无法判定该拉哪个，失败；
/// - Compose 引用了清单没登记的镜像：原样拉取（含 `:latest`），不做身份校验。
pub fn plan_candidate_images(
    references: &[String],
    delivery: &DeliveryManifest,
    local_architecture: &str,
) -> Result<Vec<RegistryImage>> {
    if !local_architecture.is_empty() && delivery.architecture != local_architecture {
        bail!(
            "DELIVERY_MANIFEST architecture '{}' does not match this host ('{local_architecture}')",
            delivery.architecture
        );
    }

    let mut receipts: BTreeMap<String, ImageReceipt> = BTreeMap::new();
    for (component, entry) in &delivery.components {
        let target = entry.target.trim();
        if target.is_empty() {
            bail!("DELIVERY_MANIFEST component {component} records an empty image target");
        }
        let receipt = ImageReceipt {
            component: component.clone(),
            image_id: normalize_digest(
                &entry.image_id,
                &format!("DELIVERY_MANIFEST component {component} image_id"),
            )?,
            config_id: normalize_digest(
                &entry.config_id,
                &format!("DELIVERY_MANIFEST component {component} config_id"),
            )?,
        };
        match receipts.get(target) {
            Some(existing) if *existing != receipt => {
                bail!(
                    "DELIVERY_MANIFEST registers {target} twice with conflicting identities ({} and {component})",
                    existing.component
                );
            }
            Some(_) => {}
            None => {
                receipts.insert(target.to_string(), receipt);
            }
        }
    }
    if receipts.is_empty() {
        bail!(
            "The registry channel needs DELIVERY_MANIFEST component image identities; \
             the candidate package declares none"
        );
    }

    let wanted: BTreeSet<&str> = references.iter().map(String::as_str).collect();
    let unexpected: Vec<&str> = receipts
        .keys()
        .map(String::as_str)
        .filter(|target| !wanted.contains(target))
        .collect();
    if !unexpected.is_empty() {
        bail!(
            "DELIVERY_MANIFEST images are absent from the candidate Compose: {} \
             (package compose and delivery receipt are not the same release)",
            unexpected.join(", ")
        );
    }

    Ok(wanted
        .into_iter()
        .map(|reference| RegistryImage {
            reference: reference.to_string(),
            receipt: receipts.get(reference).cloned(),
        })
        .collect())
}

/// 从候选包（zip 或 tar.gz）直接产出拉取计划。
///
/// 三个输入各有来历，缺一不可：
/// - compose 文本取自**包内**：停服之前磁盘上的 `docker-compose.yml` 还是上一个版本的，
///   按它拉镜像会拉到旧 tag；
/// - env 值用"操作员现值 + 包内默认值"合并后的结果，和停服前预检（`preflight_candidate_package`）
///   同一套语义，保证"预检查到的引用"与"实际拉取的引用"不会分叉；
/// - 交付清单取自包内，因为身份凭据必须和被下载的这份包同时生成。
///
/// 本函数只做集合与格式校验，不校验文件哈希；哈希由停服前的交付清单预检负责，
/// PR-4 的时序保证它先于这里执行。
pub fn plan_from_candidate_package(
    archive_path: &Path,
    user_env_path: &Path,
    local_architecture: &str,
) -> Result<Vec<RegistryImage>> {
    let entries = crate::utils::read_archive_entries(
        archive_path,
        &["docker-compose.yml", "DELIVERY_MANIFEST.json", ".env"],
    )
    .with_context(|| format!("Failed to read candidate package {archive_path:?}"))?;

    let compose_bytes = entries
        .get("docker-compose.yml")
        .cloned()
        .ok_or_else(|| anyhow!("candidate package has no docker-compose.yml"))?;
    let compose_text = std::str::from_utf8(&compose_bytes)
        .context("candidate package docker-compose.yml is not valid UTF-8")?;
    let delivery_bytes = entries.get("DELIVERY_MANIFEST.json").ok_or_else(|| anyhow!(
        "The registry channel needs the candidate package's DELIVERY_MANIFEST.json to know which \
         image identities are trustworthy; use the offline channel (auto-upgrade-deploy run) for \
         packages that carry none"
    ))?;
    let delivery_text = std::str::from_utf8(delivery_bytes)
        .context("candidate package DELIVERY_MANIFEST.json is not valid UTF-8")?;
    let delivery = parse_delivery_manifest(delivery_text)
        .context("candidate package carries an invalid DELIVERY_MANIFEST")?;

    let user_env = match std::fs::read_to_string(user_env_path) {
        Ok(contents) => contents,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => String::new(),
        Err(error) => {
            return Err(error).with_context(|| {
                format!(
                    "Failed to read selected environment file {}",
                    user_env_path.display()
                )
            });
        }
    };
    let package_env = match entries.get(".env") {
        Some(bytes) => {
            String::from_utf8(bytes.clone()).context("candidate package .env is not valid UTF-8")?
        }
        None => String::new(),
    };
    let merged = crate::utils::env_merge::merge_env_contents(&user_env, &package_env)?;
    let values = client_core::container::preflight::compose_env_values_from_text(&merged)?;

    let compose_label = archive_path.join("docker/docker-compose.yml");
    let references = client_core::container::compose_image_references_with_values(
        compose_text,
        &compose_label,
        &values,
    )?;
    plan_candidate_images(&references, &delivery, local_architecture)
}

/// 瘦通道可用的 Docker 能力集合。
///
/// `stop_services` 出现在这里是为了让"镜像没拉齐之前绝不停服"成为可证明的约束而不是
/// review 意见：[`pull_candidate_images`] 泛型于本 trait，测试 mock 一旦被调用停服就 panic。
/// PR-4 用同一个 trait 驱动整条部署时序，顺序约束可以同样被证明。
///
/// 引用按值传递：trait 方法返回装箱 future 时，`&self` 之外的第二个借用生命周期
/// 无法用 `'_` 表达，owned String 可以把这个问题一次性消掉。
pub type BoxFuture<'a, T> = std::pin::Pin<Box<dyn std::future::Future<Output = T> + Send + 'a>>;

pub trait RegistryDeployOps {
    /// 本地镜像身份；引用不存在时返回 `None`。
    fn image_identity(&self, reference: String) -> BoxFuture<'_, Result<Option<String>>>;
    fn pull_image(&self, reference: String) -> BoxFuture<'_, Result<()>>;
    /// 停服前的仓库可达性预检。**只用于日志**：`Ok(false)` 和 `Err` 都不构成"这台机器拉不到
    /// 镜像"的结论，判定权在 pull 循环。`registry_reachable` 之所以允许返回 `Err` 是因为
    /// 预检命令本身可能跑不起来（超时、daemon 异常），实现无法把这类失败压成 `false`。
    /// 预检能力缺失时实现自行降级为 `Ok(true)`。
    fn registry_reachable(&self, reference: String) -> BoxFuture<'_, Result<bool>>;
    /// 内容兜底判定：解析该引用现网 index 中 `arch` 平台子 manifest 的 `config.digest`。
    ///
    /// index 被 `docker manifest create/push` 重建后 digest 必变而内容不变（2026-10-26
    /// 双机实测：containerd 存储的 `.Id` 就是 index digest，仅按 `.Id` 比对会把内容一致
    /// 的镜像误判成被覆盖）。解析不出时实现返回 `Ok(None)`，调用方维持原判定 —— 兜底
    /// 只放宽"内容确实一致"的情形，任何失败都不放行。
    fn index_config_digest(
        &self,
        reference: String,
        arch: String,
    ) -> BoxFuture<'_, Result<Option<String>>>;
    /// 停服。镜像就绪之前禁止调用。
    ///
    /// 生产部署走 `docker_service::stop_docker_services_and_wait`（带等待与超时），本模块
    /// 用不到它；它留在这里是为了让"拉取阶段绝不停服"可被 mock 证明 —— mock 里被调用即 panic。
    #[allow(dead_code)]
    fn stop_services(&self) -> BoxFuture<'_, Result<()>>;
}

impl RegistryDeployOps for DockerManager {
    fn image_identity(&self, reference: String) -> BoxFuture<'_, Result<Option<String>>> {
        Box::pin(async move { DockerManager::image_identity(self, &reference).await })
    }

    fn pull_image(&self, reference: String) -> BoxFuture<'_, Result<()>> {
        Box::pin(async move { DockerManager::pull_image(self, &reference).await })
    }

    fn registry_reachable(&self, reference: String) -> BoxFuture<'_, Result<bool>> {
        Box::pin(async move { DockerManager::registry_reachable(self, &reference).await })
    }

    fn index_config_digest(
        &self,
        reference: String,
        arch: String,
    ) -> BoxFuture<'_, Result<Option<String>>> {
        Box::pin(async move { DockerManager::index_config_digest(self, &reference, &arch).await })
    }

    fn stop_services(&self) -> BoxFuture<'_, Result<()>> {
        Box::pin(DockerManager::stop_services(self))
    }
}

/// 拉取重试策略。失败只影响"要不要再来一次"，不改变停机时机。
#[derive(Debug, Clone)]
pub struct PullPolicy {
    pub attempts: u32,
    pub initial_backoff: Duration,
    pub max_backoff: Duration,
}

impl Default for PullPolicy {
    fn default() -> Self {
        Self {
            attempts: 3,
            initial_backoff: Duration::from_secs(5),
            max_backoff: Duration::from_secs(60),
        }
    }
}

/// 拉取结果：哪些本次执行了 pull、哪些凭据已命中所以不必重传。
///
/// `reused` 只可能来自有交付凭据的引用；无凭据引用（含浮动 `:latest`）每次都 pull。
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct PullReport {
    pub pulled: Vec<String>,
    pub reused: Vec<String>,
}

/// 在停服之前把计划里的镜像全部拉到本地并校验身份。
///
/// 全程只调用 `registry_reachable` / `image_identity` / `pull_image` /
/// `index_config_digest`。`arch` 是 docker 词汇（amd64/arm64），供身份不匹配时的
/// 内容兜底查询挑平台子 manifest 用。
pub async fn pull_candidate_images<O: RegistryDeployOps + ?Sized>(
    ops: &O,
    plan: &[RegistryImage],
    policy: &PullPolicy,
    arch: &str,
) -> Result<PullReport> {
    if plan.is_empty() {
        bail!("The candidate package Compose declares no images; nothing to pull");
    }
    let attempts = policy.attempts.max(1);

    // 预检打在哪个引用上是有讲究的：计划按字典序排，`plan[0]` 很可能是 Docker Hub 的
    // `alpine`，而瘦通道真正依赖的是发布侧自己的仓库。有凭据的引用一定来自那个仓库，
    // 所以优先探它；探通了才说明"这台机器能从发布仓库拉镜像"。
    //
    // 探不通只告警，不拦部署：`docker manifest inspect` 不走 daemon 的 registry-mirrors，
    // 实测在配了镜像加速的 Docker 29 主机上对 docker.io 引用必然失败，而同一时刻
    // `docker pull` 是成功的 —— 拿它当硬闸门会把整条瘦通道挡死在能装成的机器上。真正
    // 拉不到时下面的 pull 循环会报错，实测 DNS 失败只要 35ms，省不了多少时间。
    //
    // 连"预检命令本身跑不起来"（daemon 套接字异常、仓库被防火墙静默丢包直到 60s 超时）
    // 也只能告警：这一段的判定权归 pull 循环，任何一条从预检流出的失败都不许在拉取开始
    // 之前终结部署，否则超时这种最不像"连不上"的情况反而成了唯一的拦停理由。
    let probe_target = plan
        .iter()
        .find(|image| image.receipt.is_some())
        .unwrap_or(&plan[0]);
    match ops.registry_reachable(probe_target.reference.clone()).await {
        Ok(true) => {}
        Ok(false) => warn!(
            "⚠️ Registry pre-check for {} did not pass; continuing with the real pulls. \
             This CLI may not use registry mirrors for manifest inspect, so it is not proof \
             that images cannot be pulled",
            probe_target.reference
        ),
        Err(error) => warn!(
            "⚠️ Registry pre-check for {} could not run: {error}; \
             continuing with the real pulls",
            probe_target.reference
        ),
    }

    let mut report = PullReport::default();
    let total = plan.len();
    for (index, image) in plan.iter().enumerate() {
        let position = format!("[{}/{}]", index + 1, total);
        // 只有"能证明本地这份就是本批发布内容"时才允许跳过传输，也就是凭据命中。
        // 没有凭据的引用（含浮动 `:latest`）直接 pull：本地已存在不代表 registry 没有更新
        // 的构建，先 inspect 一次也换不来任何信息。代价可控 —— 内容没变时 Docker 自己
        // 只解析 manifest、不重传层。
        if let Some(receipt) = &image.receipt
            && let Some(identity) = ops.image_identity(image.reference.clone()).await?
            && identity_matches_receipt(&identity, receipt)
        {
            info!(
                "📦 {position} {} is already present with the expected identity; skipping download",
                image.reference
            );
            report.reused.push(image.reference.clone());
            continue;
        }

        let mut last_error = None;
        for attempt in 1..=attempts {
            if attempt > 1 {
                let backoff = (policy.initial_backoff * (1u32 << (attempt - 2).min(16)))
                    .min(policy.max_backoff);
                warn!(
                    "🔁 {position} retrying {} (attempt {attempt}/{attempts}) after {}s",
                    image.reference,
                    backoff.as_secs()
                );
                tokio::time::sleep(backoff).await;
            }
            info!(
                "⬇️ {position} pulling {} from the registry (services are still running)",
                image.reference
            );
            match ops.pull_image(image.reference.clone()).await {
                Ok(()) => {
                    last_error = None;
                    break;
                }
                Err(error) => {
                    warn!("⚠️ {position} pull of {} failed: {error}", image.reference);
                    last_error = Some(error);
                }
            }
        }
        if let Some(error) = last_error {
            return Err(error.context(format!(
                "Registry image {} could not be fetched after {attempts} attempts; \
                 deployment aborted before any service was stopped. Check DNS/registry access \
                 and `docker login`, or use the offline channel (auto-upgrade-deploy run)",
                image.reference
            )));
        }

        // 校验发生在 pull 之后：退出码为 0 只说明命令跑完，不说明拉到的是清单里那个构建
        //（同一个 tag 在包发布之后可能被人覆盖重推）。
        let identity = ops
            .image_identity(image.reference.clone())
            .await?
            .ok_or_else(|| {
                anyhow!(
                    "docker pull reported success but {} is not in the local image store",
                    image.reference
                )
            })?;
        if let Some(receipt) = &image.receipt
            && !identity_matches_receipt(&identity, receipt)
        {
            // 兜底：index 重建（`docker manifest create/push`）会让 index digest 变而
            // 镜像内容不变。`.Id` 是 index digest 的存储（containerd，Docker 28+ 默认）
            // 会在这里被误拒 —— 2026-10-26 双机实测 prod 0.0.96 全部命中的正是这种
            // 情形。所以 identity 两条都不等时，再解析现网 index 的平台子 manifest
            // 的 config.digest 与凭据 config_id 比对：内容一致即放行，任何解析失败
            // 维持拒绝。
            let mismatch = match ops
                .index_config_digest(image.reference.clone(), arch.to_string())
                .await
            {
                Ok(Some(config_digest)) if config_digest == receipt.config_id => {
                    warn!(
                        "🔀 {position} {} index digest {identity} differs from the receipt, \
                         but its {arch} manifest config digest matches the receipt config_id; \
                         the index was re-serialized without changing image content",
                        image.reference
                    );
                    None
                }
                fallback => Some(fallback),
            };
            if let Some(fallback) = mismatch {
                return Err(mismatch_error(image, receipt, &identity, arch, fallback));
            }
        }
        info!("✅ {position} {} ready at {identity}", image.reference);
        report.pulled.push(image.reference.clone());
    }
    Ok(report)
}

/// 本地镜像身份是否与交付凭据一致。
///
/// 经典 image store 暴露 config digest，containerd store 可能暴露 OCI index digest，
/// 发布侧两个都记，所以任一命中即通过 —— 与 `backup_release::validate_runtime_delivery`
/// 的回滚校验同一套语义。
///
/// 参数是凭据本身而不是 `RegistryImage`：没有凭据的引用不参与身份判定，把 `None ⇒ 通过`
/// 写进这个函数里，迟早会被误用成"无凭据的 `:latest` 不用重拉"。
fn identity_matches_receipt(identity: &str, receipt: &ImageReceipt) -> bool {
    identity == receipt.config_id || identity == receipt.image_id
}

fn mismatch_error(
    image: &RegistryImage,
    receipt: &ImageReceipt,
    identity: &str,
    arch: &str,
    fallback: Result<Option<String>, anyhow::Error>,
) -> anyhow::Error {
    let fallback_note = match fallback {
        Ok(Some(config_digest)) => format!(
            "the registry {arch} manifest config digest is {config_digest}, which also does \
             not match the receipt, so this is NOT a re-serialized index: the image content \
             itself differs from this package's build"
        ),
        Ok(None) => format!(
            "the registry index could not be resolved to a single {arch} manifest config \
             digest (not an index, or the platform manifest is not unique)"
        ),
        Err(error) => format!("the fallback registry query failed: {error:#}"),
    };
    anyhow!(
        "Image {} does not match its delivery receipt: expected config {} (or index {}), \
         local store has {identity}. Fallback check: {fallback_note}. \
         Verifiable causes (the receipt was taken from the registry when the package was \
         built; inspect with `docker buildx imagetools inspect {}` and compare the {arch} \
         child manifest's config.digest with the receipt config_id): \
         (1) the tag was overwritten by a different build after the package was built; \
         (2) the receipt was collected before the last push of this tag; \
         (3) this host's storage driver reports a different identity (classic image store \
         reports the config digest, containerd store reports the index digest — the \
         config-digest fallback above already covers re-serialized indexes). \
         Refusing to deploy an unaccounted image",
        image.reference,
        receipt.config_id,
        receipt.image_id,
        image.reference
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use client_core::container::preflight::parse_delivery_manifest;
    use std::sync::Mutex;
    use std::sync::atomic::{AtomicBool, Ordering};

    const RECEIPTED: &str = "registry.example.cn/nuwax/nuwax-im:0.0.120";
    const INDEX_ID: &str =
        "sha256:1111111111111111111111111111111111111111111111111111111111111111";
    const CONFIG_ID: &str =
        "sha256:2222222222222222222222222222222222222222222222222222222222222222";
    const OTHER_ID: &str =
        "sha256:5555555555555555555555555555555555555555555555555555555555555555";

    /// 记录调用的 mock。`stop_services` 被调用即 panic —— 这就是"拉取阶段不停服"的证明。
    ///
    /// 用 `Mutex` 而不是 `RefCell`：`BoxFuture` 要求 `Send`，async 块捕获 `&RefCell`
    /// 就不满足，换成 `Sync` 的锁即可，语义不变。
    struct MockOps {
        calls: Mutex<Vec<String>>,
        local: Mutex<BTreeMap<String, String>>,
        pull_failures: Mutex<BTreeMap<String, u32>>,
        pull_result: Mutex<Option<String>>,
        /// `manifest inspect` 那条预检的结果。与 `net_ok` 分开是实测要求：配了镜像加速的
        /// 主机上预检失败而 pull 成功，两者混在一个开关里就演不出这个场景。
        probe_ok: AtomicBool,
        /// 预检命令自身跑不起来（超时、daemon 异常）。真机上防火墙静默丢包就是这个形状。
        probe_err: AtomicBool,
        /// 这台机器能不能真的把镜像拉下来。
        net_ok: AtomicBool,
        /// 内容兜底查询（`index_config_digest`）的返回。默认 `Ok(None)`：不影响旧用例。
        /// 错误存 `String`（`anyhow::Error` 不可 Clone）。
        index_config: Mutex<Result<Option<String>, String>>,
    }

    impl MockOps {
        fn new() -> Self {
            Self {
                calls: Mutex::new(Vec::new()),
                local: Mutex::new(BTreeMap::new()),
                pull_failures: Mutex::new(BTreeMap::new()),
                pull_result: Mutex::new(Some(CONFIG_ID.to_string())),
                probe_ok: AtomicBool::new(true),
                probe_err: AtomicBool::new(false),
                net_ok: AtomicBool::new(true),
                index_config: Mutex::new(Ok(None)),
            }
        }

        fn with_local(self, reference: &str, identity: &str) -> Self {
            self.local
                .lock()
                .unwrap()
                .insert(reference.to_string(), identity.to_string());
            self
        }

        fn set_pulled_identity(self, identity: Option<&str>) -> Self {
            *self.pull_result.lock().unwrap() = identity.map(str::to_string);
            self
        }

        fn fail_pulls(self, reference: &str, times: u32) -> Self {
            self.pull_failures
                .lock()
                .unwrap()
                .insert(reference.to_string(), times);
            self
        }

        /// 这台机器既探不到也拉不到发布仓库。
        fn unreachable(self) -> Self {
            self.probe_ok.store(false, Ordering::SeqCst);
            self.net_ok.store(false, Ordering::SeqCst);
            self
        }

        /// 实测过的镜像加速主机：预检（`manifest inspect` 不走 mirrors）失败，pull 成功。
        fn precheck_only_failure(self) -> Self {
            self.probe_ok.store(false, Ordering::SeqCst);
            self
        }

        /// 预检命令本身跑不起来：仓库被防火墙静默丢包直到超时，或 daemon 异常。
        fn probe_error(self) -> Self {
            self.probe_err.store(true, Ordering::SeqCst);
            self
        }

        /// 设置内容兜底查询的结果：`Some(digest)` = index 解析出的平台 config digest，
        /// `None` = 不是 index/解析不出，`Err` = 查询本身失败。
        fn with_index_config(self, outcome: Result<Option<String>, String>) -> Self {
            *self.index_config.lock().unwrap() = outcome;
            self
        }

        fn record(&self, call: String) {
            self.calls.lock().unwrap().push(call);
        }

        fn calls(&self) -> Vec<String> {
            self.calls.lock().unwrap().clone()
        }
    }

    /// mock 的一次 pull 结果：网络失败，或命令成功（成功时镜像可能没落地）
    enum Outcome {
        NetworkFailure,
        CommandSucceeded(Option<String>),
    }

    impl RegistryDeployOps for MockOps {
        fn image_identity(&self, reference: String) -> BoxFuture<'_, Result<Option<String>>> {
            self.record(format!("inspect {reference}"));
            let found = self.local.lock().unwrap().get(&reference).cloned();
            Box::pin(async move { Ok(found) })
        }

        fn pull_image(&self, reference: String) -> BoxFuture<'_, Result<()>> {
            self.record(format!("pull {reference}"));
            if !self.net_ok.load(Ordering::SeqCst) {
                return Box::pin(async move { Err(anyhow!("registry is not reachable")) });
            }
            let outcome = {
                let mut failures = self.pull_failures.lock().unwrap();
                match failures.get_mut(&reference) {
                    Some(remaining) if *remaining > 0 => {
                        *remaining -= 1;
                        Outcome::NetworkFailure
                    }
                    _ => Outcome::CommandSucceeded(
                        // None 模拟"pull 命令成功但本地镜像存储里查不到"的病态情况
                        self.pull_result.lock().unwrap().clone(),
                    ),
                }
            };
            let local = &self.local;
            Box::pin(async move {
                match outcome {
                    Outcome::NetworkFailure => Err(anyhow!("network hiccup")),
                    Outcome::CommandSucceeded(Some(identity)) => {
                        local.lock().unwrap().insert(reference, identity);
                        Ok(())
                    }
                    Outcome::CommandSucceeded(None) => Ok(()),
                }
            })
        }

        fn registry_reachable(&self, reference: String) -> BoxFuture<'_, Result<bool>> {
            self.record(format!("probe {reference}"));
            let probe_ok = self.probe_ok.load(Ordering::SeqCst);
            let probe_err = self.probe_err.load(Ordering::SeqCst);
            Box::pin(async move {
                if probe_err {
                    Err(anyhow!(
                        "docker manifest inspect timed out after 60 seconds"
                    ))
                } else {
                    Ok(probe_ok)
                }
            })
        }

        fn index_config_digest(
            &self,
            reference: String,
            arch: String,
        ) -> BoxFuture<'_, Result<Option<String>>> {
            self.record(format!("index-config {reference}/{arch}"));
            let outcome = self.index_config.lock().unwrap().clone();
            Box::pin(async move { outcome.map_err(|message| anyhow!(message)) })
        }

        fn stop_services(&self) -> BoxFuture<'_, Result<()>> {
            panic!("stop_services must never be called before every image is pulled");
        }
    }

    fn delivery(entries: &[(&str, &str, &str, &str)]) -> DeliveryManifest {
        // 按 fixture 形状拼 JSON 再走真实 parse_delivery_manifest 校验路径，
        // 免得测试构造的结构和生产发布的结构漂移。
        let components: Vec<String> = entries
            .iter()
            .map(|(name, target, image_id, config_id)| {
                format!(
                    "\"{name}\": {{\"version\": \"1\", \"image_id\": \"{image_id}\", \
                     \"config_id\": \"{config_id}\", \"source\": \"{image_id}\", \
                     \"target\": \"{target}\", \"artifacts\": {{}}}}"
                )
            })
            .collect();
        let joined = components.join(",");
        let compose_sha = "a".repeat(64);
        let release_sha = "b".repeat(64);
        let text = format!(
            "{{\"contract_version\":1,\"architecture\":\"amd64\",\"components\":{{{joined}}},\
             \"mysql\": {{\"manifest\":\"config/mysql-schema-manifest.json\",\"files\":{{}}}},\
             \"compose\": {{\"path\":\"docker-compose.yml\",\"sha256\":\"{compose_sha}\"}},\
             \"release_sha256\":\"{release_sha}\"}}"
        );
        parse_delivery_manifest(&text).expect("test delivery manifest must parse")
    }

    fn receipted_delivery() -> DeliveryManifest {
        delivery(&[("nuwax-im", RECEIPTED, INDEX_ID, CONFIG_ID)])
    }

    fn references(values: &[&str]) -> Vec<String> {
        values.iter().map(|value| value.to_string()).collect()
    }

    fn plan_of(values: &[&str]) -> Vec<RegistryImage> {
        plan_candidate_images(&references(values), &receipted_delivery(), "amd64").unwrap()
    }

    #[test]
    fn plans_receipted_and_pinned_third_party_images_in_deterministic_order() {
        let plan = plan_of(&["mysql:8.0", RECEIPTED]);
        assert_eq!(plan.len(), 2);
        // 顺序确定性（字典序）：断点重跑与日志对齐都依赖它
        assert_eq!(plan[0].reference, "mysql:8.0");
        assert_eq!(plan[1].reference, RECEIPTED);
        let receipt = plan[1].receipt.as_ref().unwrap();
        assert_eq!(receipt.component, "nuwax-im");
        assert_eq!(receipt.config_id, CONFIG_ID);
        assert!(plan[0].receipt.is_none());
    }

    #[test]
    fn deduplicates_references_that_appear_in_several_services() {
        let plan = plan_of(&["mysql:8.0", "mysql:8.0", RECEIPTED]);
        assert_eq!(plan.len(), 2);
    }

    #[test]
    fn plans_unreceipted_references_exactly_as_the_compose_declares_them() {
        // 浮动 tag 是发布侧选定的语义：必须进计划、按原样拉，不能在这里被拒。
        let plan = plan_of(&["nginx:latest", "busybox", RECEIPTED]);
        assert_eq!(plan.len(), 3);
        assert!(
            plan.iter()
                .any(|image| image.reference == "nginx:latest" && image.receipt.is_none())
        );
        assert!(
            plan.iter()
                .any(|image| image.reference == "busybox" && image.receipt.is_none())
        );
        assert!(
            plan.iter()
                .any(|image| image.reference == RECEIPTED && image.receipt.is_some())
        );
    }

    #[test]
    fn accepts_digest_pinned_third_party_references() {
        let pinned = "registry.example.cn/nuwax/rcoder@sha256:3333333333333333333333333333333333333333333333333333333333333333";
        let plan = plan_of(&[pinned, RECEIPTED]);
        assert!(plan.iter().any(|image| image.reference == pinned));
    }

    #[test]
    fn rejects_receipts_for_images_the_compose_does_not_run() {
        let error =
            plan_candidate_images(&references(&["mysql:8.0"]), &receipted_delivery(), "amd64")
                .unwrap_err()
                .to_string();
        assert!(
            error.contains("absent from the candidate Compose"),
            "{error}"
        );
        assert!(error.contains(RECEIPTED), "{error}");
    }

    #[test]
    fn rejects_placeholder_digests() {
        let fake = delivery(&[("nuwax-im", RECEIPTED, "external", "external")]);
        let error = plan_candidate_images(&references(&[RECEIPTED]), &fake, "amd64")
            .unwrap_err()
            .to_string();
        assert!(error.contains("not a sha256 digest"), "{error}");
    }

    #[test]
    fn rejects_a_delivery_manifest_with_no_component_images() {
        let empty = delivery(&[]);
        let error =
            plan_candidate_images(&references(&["mysql:8.0"]), &empty, "amd64").unwrap_err();
        assert!(
            error
                .to_string()
                .contains("needs DELIVERY_MANIFEST component image identities"),
            "{error}"
        );
    }

    #[test]
    fn rejects_a_receipt_belonging_to_another_architecture() {
        let error =
            plan_candidate_images(&references(&[RECEIPTED]), &receipted_delivery(), "arm64")
                .unwrap_err()
                .to_string();
        assert!(
            error.contains(
                "DELIVERY_MANIFEST architecture 'amd64' does not match this host ('arm64')"
            ),
            "{error}"
        );
    }

    /// 交付清单的 architecture 用 docker 词汇（`amd64`/`arm64`），与
    /// `verify_delivery_manifest` 的调用方一致。调用方若传 `client_core::architecture`
    /// 的 `x86_64`/`aarch64`，每一次部署都会被判成架构不符 —— PR-4 的接线必须用前者。
    #[test]
    fn rejects_the_golang_architecture_vocabulary_from_the_caller() {
        let error =
            plan_candidate_images(&references(&[RECEIPTED]), &receipted_delivery(), "x86_64")
                .unwrap_err()
                .to_string();
        assert!(
            error.contains("does not match this host ('x86_64')"),
            "{error}"
        );
    }

    #[test]
    fn rejects_the_same_image_registered_with_conflicting_identities() {
        let manifest = delivery(&[
            ("nuwax-im", RECEIPTED, INDEX_ID, CONFIG_ID),
            ("repo-collab", RECEIPTED, OTHER_ID, OTHER_ID),
        ]);
        let error = plan_candidate_images(&references(&[RECEIPTED]), &manifest, "amd64")
            .unwrap_err()
            .to_string();
        assert!(error.contains("conflicting identities"), "{error}");
    }

    #[tokio::test]
    async fn pulling_never_stops_services() {
        let plan = plan_of(&["mysql:8.0", RECEIPTED]);
        let ops = MockOps::new();
        let report = pull_candidate_images(&ops, &plan, &PullPolicy::default(), "amd64")
            .await
            .unwrap();
        // MockOps::stop_services 是 panic!，所以能走到这里本身就是结论；
        // 再断言调用序列确实只含预检 / inspect / pull。
        let calls = ops.calls();
        assert_eq!(calls[0], format!("probe {RECEIPTED}"));
        assert!(
            calls.iter().all(|call| {
                call.starts_with("probe")
                    || call.starts_with("inspect ")
                    || call.starts_with("pull ")
            }),
            "unexpected docker call: {calls:?}"
        );
        assert_eq!(
            report.pulled,
            vec!["mysql:8.0".to_string(), RECEIPTED.to_string()]
        );
        assert!(report.reused.is_empty());
    }

    /// 跳过传输只在凭据命中时发生：`mysql:8.0` 虽然本地已有，但没有凭据能证明它就是本批
    /// 要的那份，所以必须 pull（registry 内容没变时 Docker 自己不会重传层）。
    /// 无凭据的引用连那次预检 inspect 都不做 —— 结果不会影响任何决定。
    #[tokio::test]
    async fn reuses_only_the_local_image_whose_receipt_already_matches() {
        let ops = MockOps::new()
            .with_local(RECEIPTED, CONFIG_ID)
            .with_local("mysql:8.0", OTHER_ID);
        let plan = plan_of(&["mysql:8.0", RECEIPTED]);
        let report = pull_candidate_images(&ops, &plan, &PullPolicy::default(), "amd64")
            .await
            .unwrap();
        assert_eq!(report.reused, vec![RECEIPTED.to_string()]);
        assert_eq!(report.pulled, vec!["mysql:8.0".to_string()]);
        assert_eq!(
            ops.calls(),
            vec![
                format!("probe {RECEIPTED}"),
                "pull mysql:8.0".to_string(),
                "inspect mysql:8.0".to_string(),
                format!("inspect {RECEIPTED}"),
            ]
        );
    }

    /// `:latest` 的语义就是"取 registry 当前的最新"：本地哪怕已经存在同 tag 的旧构建，
    /// 也绝不能跳过 pull，否则升级命令只是把上一次拉的东西又用了一遍。
    #[tokio::test]
    async fn always_pulls_a_floating_latest_even_when_it_is_already_local() {
        let floating = "registry.example.cn/nuwax/rcoder:latest";
        let ops = MockOps::new()
            .with_local(floating, OTHER_ID)
            .with_local(RECEIPTED, CONFIG_ID);
        let plan = plan_of(&[floating, RECEIPTED]);
        let report = pull_candidate_images(&ops, &plan, &PullPolicy::default(), "amd64")
            .await
            .unwrap();
        assert!(
            report.reused.contains(&RECEIPTED.to_string()),
            "有凭据的镜像仍应免重传：{:?}",
            report
        );
        assert!(
            report.pulled.contains(&floating.to_string()),
            "浮动 latest 必须重新 pull：{:?}",
            report
        );
        let calls = ops.calls();
        assert_eq!(
            calls
                .iter()
                .filter(|call| **call == format!("pull {floating}"))
                .count(),
            1,
            "{calls:?}"
        );
        assert!(
            !calls
                .iter()
                .any(|call| call == &format!("pull {RECEIPTED}")),
            "凭据已命中的镜像不该重传：{calls:?}"
        );
    }

    #[tokio::test]
    async fn reuses_a_local_image_whose_identity_is_the_oci_index() {
        let ops = MockOps::new().with_local(RECEIPTED, INDEX_ID);
        let plan = plan_of(&[RECEIPTED]);
        let report = pull_candidate_images(&ops, &plan, &PullPolicy::default(), "amd64")
            .await
            .unwrap();
        assert_eq!(report.reused, vec![RECEIPTED.to_string()]);
    }

    #[tokio::test]
    async fn accepts_a_pulled_image_whose_identity_is_the_oci_index() {
        let ops = MockOps::new().set_pulled_identity(Some(INDEX_ID));
        let plan = plan_of(&[RECEIPTED]);
        let report = pull_candidate_images(&ops, &plan, &PullPolicy::default(), "amd64")
            .await
            .unwrap();
        assert_eq!(report.pulled, vec![RECEIPTED.to_string()]);
    }

    /// 2026-10-26 双机实测的主场景：index 被 `docker manifest create/push` 重建，
    /// containerd 存储的 `.Id`（index digest）与收据两条都不等，但平台 config.digest
    /// 与收据 config_id 一致 —— 内容就是那一版，必须放行而不是拒绝部署。
    #[tokio::test]
    async fn accepts_when_the_index_drifts_but_the_config_digest_still_matches() {
        let ops = MockOps::new()
            .set_pulled_identity(Some(OTHER_ID))
            .with_index_config(Ok(Some(CONFIG_ID.to_string())));
        let plan = plan_of(&[RECEIPTED]);
        let report = pull_candidate_images(&ops, &plan, &PullPolicy::default(), "amd64")
            .await
            .unwrap();
        assert_eq!(report.pulled, vec![RECEIPTED.to_string()]);
        assert!(
            ops.calls()
                .iter()
                .any(|call| call.starts_with("index-config "))
        );
    }

    #[tokio::test]
    async fn rejects_when_the_fallback_config_digest_also_differs() {
        let ops = MockOps::new()
            .set_pulled_identity(Some(OTHER_ID))
            .with_index_config(Ok(Some(OTHER_ID.to_string())));
        let plan = plan_of(&[RECEIPTED]);
        let error = pull_candidate_images(&ops, &plan, &PullPolicy::default(), "amd64")
            .await
            .unwrap_err()
            .to_string();
        assert!(error.contains("image content itself differs"), "{error}");
        assert!(error.contains("imagetools inspect"), "{error}");
    }

    #[tokio::test]
    async fn rejects_when_the_fallback_cannot_resolve_the_index() {
        let ops = MockOps::new()
            .set_pulled_identity(Some(OTHER_ID))
            .with_index_config(Ok(None));
        let plan = plan_of(&[RECEIPTED]);
        let error = pull_candidate_images(&ops, &plan, &PullPolicy::default(), "amd64")
            .await
            .unwrap_err()
            .to_string();
        assert!(
            error.contains("could not be resolved"),
            "兜底解析不出必须维持拒绝：{error}"
        );
    }

    #[tokio::test]
    async fn rejects_when_the_fallback_query_itself_fails() {
        let ops = MockOps::new()
            .set_pulled_identity(Some(OTHER_ID))
            .with_index_config(Err("registry unreachable".to_string()));
        let plan = plan_of(&[RECEIPTED]);
        let error = pull_candidate_images(&ops, &plan, &PullPolicy::default(), "amd64")
            .await
            .unwrap_err()
            .to_string();
        assert!(error.contains("fallback registry query failed"), "{error}");
        assert!(error.contains("registry unreachable"), "{error}");
    }

    #[tokio::test]
    async fn retries_and_then_names_the_reference_that_failed() {
        let ops = MockOps::new().fail_pulls(RECEIPTED, 5);
        let plan = plan_of(&[RECEIPTED]);
        let policy = PullPolicy {
            attempts: 3,
            initial_backoff: Duration::from_millis(1),
            max_backoff: Duration::from_millis(2),
        };
        let error = pull_candidate_images(&ops, &plan, &policy, "amd64")
            .await
            .unwrap_err()
            .to_string();
        assert!(error.contains(RECEIPTED), "{error}");
        assert!(error.contains("after 3 attempts"), "{error}");
        assert!(error.contains("before any service was stopped"), "{error}");
        assert_eq!(
            ops.calls()
                .iter()
                .filter(|call| call.starts_with("pull "))
                .count(),
            3
        );
    }

    #[tokio::test]
    async fn rejects_an_image_whose_tag_was_overwritten_after_the_package_was_built() {
        let ops = MockOps::new().set_pulled_identity(Some(OTHER_ID));
        let plan = plan_of(&[RECEIPTED]);
        let error = pull_candidate_images(&ops, &plan, &PullPolicy::default(), "amd64")
            .await
            .unwrap_err()
            .to_string();
        assert!(
            error.contains("does not match its delivery receipt"),
            "{error}"
        );
        assert!(error.contains(CONFIG_ID), "{error}");
    }

    #[tokio::test]
    async fn rejects_a_successful_pull_that_left_no_image_behind() {
        let ops = MockOps::new().set_pulled_identity(None);
        let plan = plan_of(&[RECEIPTED]);
        let error = pull_candidate_images(&ops, &plan, &PullPolicy::default(), "amd64")
            .await
            .unwrap_err()
            .to_string();
        assert!(
            error.contains("reported success but"),
            "本地镜像缺失必须可见：{error}"
        );
    }

    /// 预检失败不再拦部署：判定权在 pull 循环，它报错时同样发生在停服之前，而实测
    /// DNS 层面的失败只要毫秒级，预检省不下时间，却能假阳性挡死能用 pull 的机器。
    #[tokio::test]
    async fn an_unreachable_host_fails_in_the_pull_loop_before_any_stop() {
        let ops = MockOps::new().unreachable();
        let plan = plan_of(&[RECEIPTED]);
        let policy = PullPolicy {
            attempts: 3,
            initial_backoff: Duration::from_millis(1),
            max_backoff: Duration::from_millis(2),
        };
        let error = pull_candidate_images(&ops, &plan, &policy, "amd64")
            .await
            .unwrap_err()
            .to_string();
        assert!(error.contains(RECEIPTED), "{error}");
        assert!(error.contains("after 3 attempts"), "{error}");
        assert!(error.contains("before any service was stopped"), "{error}");
        assert!(error.contains("offline channel"), "{error}");
        assert_eq!(
            ops.calls(),
            vec![
                format!("probe {RECEIPTED}"),
                format!("inspect {RECEIPTED}"),
                format!("pull {RECEIPTED}"),
                format!("pull {RECEIPTED}"),
                format!("pull {RECEIPTED}"),
            ]
        );
    }

    /// 这台 Ubuntu 24 / Docker 29 主机上实测到的形状：`manifest inspect` 因为不走
    /// registry mirrors 而失败，`docker pull` 完全正常。预检必须只做告警，否则整条
    /// 瘦通道在这类机器上永远装不成。
    #[tokio::test]
    async fn a_failed_precheck_still_pulls_every_image_that_is_actually_available() {
        let ops = MockOps::new().precheck_only_failure();
        let plan = plan_of(&["mysql:8.0", RECEIPTED]);
        let report = pull_candidate_images(&ops, &plan, &PullPolicy::default(), "amd64")
            .await
            .expect("预检失败不该拦住拉得到的镜像");
        assert_eq!(
            report.pulled,
            vec!["mysql:8.0".to_string(), RECEIPTED.to_string()]
        );
        assert_eq!(
            ops.calls()
                .iter()
                .filter(|call| call.starts_with("probe"))
                .count(),
            1
        );
    }

    /// 预检命令自己跑不起来（仓库被防火墙静默丢包直到 60s 超时、daemon 异常）时，也不能
    /// 让它拦停部署：这条通道上唯一有资格判定"拉不到"的是 pull 循环，而它已经在下面
    /// 重试并把引用名报出来。若在这里 `?` 出去，超时这种最不像"连不上"的失败反而成了
    /// 唯一的拦停理由，而且报错发生在任何一次拉取尝试之前。
    #[tokio::test]
    async fn a_precheck_that_could_not_run_still_reaches_the_pull_loop() {
        let ops = MockOps::new().probe_error();
        let plan = plan_of(&["mysql:8.0", RECEIPTED]);
        let report = pull_candidate_images(&ops, &plan, &PullPolicy::default(), "amd64")
            .await
            .expect("预检跑不起来不该拦住拉得到的镜像");
        assert_eq!(
            report.pulled,
            vec!["mysql:8.0".to_string(), RECEIPTED.to_string()]
        );
        assert_eq!(
            ops.calls().first().map(String::as_str),
            Some(format!("probe {RECEIPTED}").as_str()),
            "预检仍然要跑一次并把失败记进日志：{:?}",
            ops.calls()
        );
    }

    #[tokio::test]
    async fn probes_the_release_registry_not_the_alphabetically_first_image() {
        // 字典序把 `mysql:8.0`（Docker Hub）排在发布仓库前面。探通了公共仓库不能说明
        // 这台机器拉得到本次发布的镜像，告警信息就该打在发布仓库的引用上。
        let ops = MockOps::new().unreachable();
        let plan = plan_of(&["mysql:8.0", RECEIPTED]);
        let policy = PullPolicy {
            attempts: 1,
            initial_backoff: Duration::from_millis(1),
            max_backoff: Duration::from_millis(2),
        };
        let error = pull_candidate_images(&ops, &plan, &policy, "amd64")
            .await
            .unwrap_err()
            .to_string();
        let calls = ops.calls();
        assert_eq!(calls[0], format!("probe {RECEIPTED}"), "{calls:?}");
        assert!(
            !calls.iter().any(|call| call == "probe mysql:8.0"),
            "预检探错了仓库：{calls:?}"
        );
        // 判定来自 pull 循环，报的是计划里第一个拉不掉的引用。
        assert!(error.contains("mysql:8.0"), "{error}");
    }

    #[tokio::test]
    async fn rejects_an_empty_plan_instead_of_claiming_success() {
        let ops = MockOps::new();
        let error = pull_candidate_images(&ops, &[], &PullPolicy::default(), "amd64")
            .await
            .unwrap_err()
            .to_string();
        assert!(error.contains("declares no images"), "{error}");
    }

    fn zip_package(
        directory: &Path,
        compose_text: &str,
        delivery: Option<&DeliveryManifest>,
        package_env: &str,
    ) -> Result<std::path::PathBuf> {
        use std::io::Write as _;
        let path = directory.join("candidate.zip");
        let mut zip = zip::ZipWriter::new(std::fs::File::create(&path)?);
        let options = zip::write::SimpleFileOptions::default();
        let delivery_text = delivery.map(serde_json::to_string).transpose()?;
        let mut entries: Vec<(&str, Vec<u8>)> = vec![(
            "docker/docker-compose.yml",
            compose_text.as_bytes().to_vec(),
        )];
        if let Some(text) = delivery_text {
            entries.push(("docker/DELIVERY_MANIFEST.json", text.into_bytes()));
        }
        if !package_env.is_empty() {
            entries.push(("docker/.env", package_env.as_bytes().to_vec()));
        }
        for (name, bytes) in entries {
            zip.start_file(name, options)?;
            zip.write_all(&bytes)?;
        }
        zip.finish()?;
        Ok(path)
    }

    const CANDIDATE_COMPOSE: &str = concat!(
        "services:\n",
        "  im:\n    image: ${NUWAX_TEST_REGISTRY}/nuwax/nuwax-im:0.0.120\n",
        "  mysql:\n    image: mysql:8.0\n",
    );

    #[test]
    fn plans_from_the_candidate_package_not_from_the_deployed_compose() -> Result<()> {
        let directory = tempfile::tempdir()?;
        // 镜像引用带变量，且变量只在包内 .env 里有默认值：证明计划走的是
        // "包内 compose + 合并 env"，而不是操作员磁盘上的旧值
        let archive = zip_package(
            directory.path(),
            CANDIDATE_COMPOSE,
            Some(&receipted_delivery()),
            "NUWAX_TEST_REGISTRY=registry.example.cn\n",
        )?;
        let plan =
            plan_from_candidate_package(&archive, &directory.path().join("missing.env"), "amd64")?;
        assert_eq!(plan.len(), 2);
        assert_eq!(plan[0].reference, "mysql:8.0");
        assert_eq!(plan[1].reference, RECEIPTED);
        assert_eq!(
            plan[1]
                .receipt
                .as_ref()
                .map(|receipt| receipt.component.clone()),
            Some("nuwax-im".to_string())
        );
        Ok(())
    }

    #[test]
    fn a_package_without_a_delivery_manifest_points_at_the_offline_channel() -> Result<()> {
        let directory = tempfile::tempdir()?;
        let archive = zip_package(
            directory.path(),
            "services:\n  mysql:\n    image: mysql:8.0\n",
            None,
            "",
        )?;
        let error =
            plan_from_candidate_package(&archive, &directory.path().join("missing.env"), "amd64")
                .unwrap_err()
                .to_string();
        assert!(error.contains("DELIVERY_MANIFEST.json"), "{error}");
        assert!(error.contains("offline channel"), "{error}");
        Ok(())
    }

    /// 发布侧选定的语义：清单没登记的浮动 `:latest` 照原样拉，不在计划阶段被拒。
    /// 顺带证明可达性预检仍打在发布仓库（有凭据的那个引用）上，而不是字典序第一个的
    /// `agent-platform-front`。
    #[tokio::test]
    async fn a_floating_latest_reference_is_pulled_verbatim_before_services_stop() -> Result<()> {
        let floating = "registry.example.cn/nuwax/agent-platform-front:latest";
        let directory = tempfile::tempdir()?;
        let archive = zip_package(
            directory.path(),
            concat!(
                "services:\n",
                "  im:\n    image: registry.example.cn/nuwax/nuwax-im:0.0.120\n",
                "  front:\n    image: registry.example.cn/nuwax/agent-platform-front:latest\n",
            ),
            Some(&receipted_delivery()),
            "",
        )?;
        let plan =
            plan_from_candidate_package(&archive, &directory.path().join("missing.env"), "amd64")?;
        assert_eq!(plan.len(), 2);
        assert_eq!(plan[0].reference, floating);
        assert!(plan[0].receipt.is_none());

        let ops = MockOps::new();
        let report = pull_candidate_images(&ops, &plan, &PullPolicy::default(), "amd64").await?;
        assert_eq!(
            report.pulled,
            vec![floating.to_string(), RECEIPTED.to_string()]
        );
        let calls = ops.calls();
        assert_eq!(calls[0], format!("probe {RECEIPTED}"));
        assert!(
            calls.contains(&format!("pull {floating}")),
            "浮动引用必须按 compose 里的名字原样拉取：{calls:?}"
        );
        Ok(())
    }
}
