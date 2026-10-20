use std::fmt::Display;
use std::path::PathBuf;

use crate::{
    api_types::{EnhancedServiceManifest, PatchPackageInfo},
    architecture::Architecture,
    config::DockerChannel,
    constants::docker::get_compose_file_path,
    constants::docker::get_docker_work_dir,
    version::Version,
};
use anyhow::Result;
use tracing::{debug, info};

#[derive(Debug, Clone, PartialEq)]
pub enum DownloadType {
    /// 全量升级
    Full,
    /// 增量升级
    Patch,
    /// registry 瘦通道的全量升级：包内不含 `docker/images/*.tar`
    Registry,
}

impl Display for DownloadType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            DownloadType::Full => write!(f, "full"),
            DownloadType::Patch => write!(f, "patch"),
            DownloadType::Registry => write!(f, "registry"),
        }
    }
}

/// 升级策略类型
#[derive(Debug, Clone, PartialEq)]
pub enum UpgradeStrategy {
    /// 全量升级
    FullUpgrade {
        /// 下载URL
        url: String,
        /// 文件哈希
        hash: String,
        /// 签名
        signature: String,
        /// 目标版本
        target_version: Version,
        /// 下载类型
        download_type: DownloadType,
    },
    /// 增量升级（补丁）
    PatchUpgrade {
        /// 补丁包信息
        patch_info: PatchPackageInfo,
        /// 目标版本
        target_version: Version,
        /// 下载类型
        download_type: DownloadType,
    },
    /// 无需升级
    NoUpgrade {
        /// 目标版本
        target_version: Version,
    },
}

impl UpgradeStrategy {
    ///获取此次升级,变更的文件,或者目录,使用相对工作目录的路径,工作目录是:./docker ,如果是全量升级,只备份: ./data 目录; 增量升级,还需要额外备份增量升级变更的文件或者目录
    pub fn get_changed_files(&self) -> Vec<PathBuf> {
        let change_files = match self {
            UpgradeStrategy::FullUpgrade { .. } => vec!["data".to_string(), "upload".to_string()],
            UpgradeStrategy::PatchUpgrade { patch_info, .. } => patch_info.get_changed_files(),
            UpgradeStrategy::NoUpgrade { .. } => {
                vec![]
            }
        };
        change_files.into_iter().map(PathBuf::from).collect()
    }
}

/// 决策因素分析
#[derive(Debug, Clone)]
pub struct DecisionFactors {
    /// 版本兼容性分数（0.0-1.0）
    pub version_compatibility: f64,
    /// 网络条件分数（0.0-1.0）
    pub network_condition: f64,
    /// 磁盘空间分数（0.0-1.0）
    pub disk_space: f64,
    /// 风险评估分数（0.0-1.0，越低越好）
    pub risk_assessment: f64,
    /// 时间效率分数（0.0-1.0）
    pub time_efficiency: f64,
}

/// 升级策略管理器
#[derive(Debug)]
pub struct UpgradeStrategyManager {
    ///server端docker应用升级版本信息
    manifest: EnhancedServiceManifest,
    ///当前客户端版本
    current_version: String,
    ///是否强制全量升级
    force_full: bool,
    ///当前客户端架构
    architecture: Architecture,
    ///全量包所属通道（离线 full / registry 瘦包），决定下载目录与缓存身份
    full_download_type: DownloadType,
}

impl UpgradeStrategyManager {
    /// 创建新的升级策略管理器
    ///
    /// `channel` 决定全量包属于哪条发布通道：两条通道会为同一版本号发布不同的包，
    /// 下载目录与缓存收据身份必须区分开，否则瘦包会覆盖离线包、让另一条通道读到错误的收据。
    /// 增量（`DownloadType::Patch`）不由这里表达，它只属于离线通道，
    /// 所以参数用的是 `DockerChannel` 而不是 `DownloadType`。
    pub fn new(
        current_version: String,
        force_full: bool,
        manifest: EnhancedServiceManifest,
        channel: DockerChannel,
    ) -> Self {
        let full_download_type = match channel {
            DockerChannel::Images => DownloadType::Full,
            DockerChannel::Registry => DownloadType::Registry,
        };
        Self {
            manifest,
            current_version,
            force_full,
            architecture: Architecture::detect(),
            full_download_type,
        }
    }

    /// 确定升级策略（简化版本）
    pub fn determine_strategy(&self) -> Result<UpgradeStrategy> {
        info!("Starting upgrade strategy decision");
        info!("   Current version: {}", self.current_version);
        info!("   Server version: {}", self.manifest.version);
        info!("   Target architecture: {}", self.architecture.as_str());
        info!("   Force full upgrade: {}", self.force_full);

        // 1. 解析当前版本
        let current_ver = self.current_version.parse::<Version>()?;

        // 2. 首先与基础服务器版本比较，确定是否需要升级
        let server_ver = self.manifest.version.clone();
        //比较当前版本和服务器版本，判断是全量，还是增量升级，还是不需要升级
        let base_comparison = current_ver.compare_detailed(&server_ver);

        info!("Current version details: {:?}", current_ver);
        info!("Server version details: {:?}", server_ver);
        info!("Base version comparison result: {:?}", base_comparison);

        // 3. 强制全量升级
        if self.force_full {
            info!("Force executing full upgrade");
            return self.select_full_upgrade_strategy();
        }
        //判断工作目录下,是否有docker目录,如果没有docker目录,则也使用全量升级
        let work_dir = get_docker_work_dir();
        let compose_file_path = get_compose_file_path();
        if !work_dir.exists() || !compose_file_path.exists() {
            info!(
                "No docker directory or compose file found in working directory, selecting full upgrade strategy"
            );
            return self.select_full_upgrade_strategy();
        }

        // 4. 根据版本比较结果决策
        match base_comparison {
            crate::version::VersionComparison::Equal | crate::version::VersionComparison::Newer => {
                info!("Current version is already latest, no upgrade needed");
                Ok(UpgradeStrategy::NoUpgrade {
                    target_version: self.manifest.version.clone(),
                })
            }
            crate::version::VersionComparison::PatchUpgradeable => {
                // 可以进行增量升级
                if self.full_download_type == DownloadType::Registry {
                    // registry 通道只发布瘦全量包：补丁包走的是离线通道的缓存目录，
                    // 误发布到瘦通道清单会让两条通道共用同一个包收据，宁可拒绝执行。
                    return Err(anyhow::anyhow!(
                        "The registry channel does not support incremental patch packages; \
                         publish a registry full package instead"
                    ));
                }
                if !self.has_patch_for_architecture() {
                    info!(
                        "No incremental upgrade package for current architecture, selecting full upgrade strategy"
                    );
                    self.select_full_upgrade_strategy()
                } else {
                    info!("Selecting incremental upgrade strategy");
                    self.select_patch_upgrade_strategy()
                }
            }
            crate::version::VersionComparison::FullUpgradeRequired => {
                // 需要全量升级
                info!("Selecting full upgrade strategy");
                self.select_full_upgrade_strategy()
            }
        }
    }

    /// 选择全量升级策略
    pub fn select_full_upgrade_strategy(&self) -> Result<UpgradeStrategy> {
        debug!("Selecting full upgrade strategy");

        if self.manifest.platforms.is_some() {
            //使用分架构的全量包
            let platform_info = self.get_platform_package()?;

            debug!(
                "Using architecture-specific full package: {}",
                &platform_info.url
            );
            let published = platform_info.published_sha256(
                self.manifest
                    .packages
                    .as_ref()
                    .map(|packages| &packages.full),
            )?;
            Ok(UpgradeStrategy::FullUpgrade {
                url: platform_info.url.clone(),
                hash: self.required_full_hash(published.as_deref(), &platform_info.url)?,
                signature: platform_info.signature.clone(),
                target_version: self.manifest.version.clone(),
                download_type: self.full_download_type.clone(),
            })
        } else if let Some(package_info) = &self.manifest.packages {
            let full_info = &package_info.full;
            debug!("Using generic full package: {}", &full_info.url);
            Ok(UpgradeStrategy::FullUpgrade {
                url: full_info.url.clone(),
                hash: self.required_full_hash(Some(full_info.hash.as_str()), &full_info.url)?,
                signature: full_info.signature.clone(),
                target_version: self.manifest.version.clone(),
                download_type: self.full_download_type.clone(),
            })
        } else {
            //未找到对应架构的全量升级包，这里主动报错
            Err(anyhow::anyhow!(
                "Full upgrade package for corresponding architecture not found"
            ))
        }
    }

    /// 全量包的发布方 SHA。两条通道的原始文本都先过 `package_cache::normalize_sha256`，
    /// 让 `external` 占位、空值、`sha256:` 前缀和 64 位十六进制只有一份判定规则。
    ///
    /// registry 瘦包必须给出可用的 SHA：瘦部署跳过 `docker load`，下载完整性是唯一能证明
    /// "解出来的就是发布的那份"的环节，拿不到就拒绝开始部署。离线通道保持历史行为
    /// （缺省填 `external` 占位，下游同样解释为"不校验"）。
    fn required_full_hash(&self, published: Option<&str>, package: &str) -> Result<String> {
        match crate::package_cache::normalize_sha256(published)? {
            Some(hash) => Ok(hash),
            None if self.full_download_type == DownloadType::Registry => Err(anyhow::anyhow!(
                "registry package {package} publishes no SHA-256; \
                 refusing an unverified slim download"
            )),
            None => Ok("external".to_string()),
        }
    }

    /// 选择增量升级策略
    pub fn select_patch_upgrade_strategy(&self) -> Result<UpgradeStrategy> {
        debug!("Selecting incremental upgrade strategy");

        let patch_info = self.get_patch_package()?;

        debug!(
            "Using architecture-specific patch package: {}",
            &patch_info.url
        );
        Ok(UpgradeStrategy::PatchUpgrade {
            patch_info: patch_info.clone(),
            target_version: self.manifest.version.clone(),
            download_type: DownloadType::Patch,
        })
    }

    /// 获取指定架构的平台包信息
    fn get_platform_package(&self) -> Result<crate::api_types::PlatformPackageInfo> {
        if let Some(platforms) = self.manifest.platforms.as_ref() {
            match self.architecture {
                Architecture::X86_64 => platforms.x86_64.clone().ok_or_else(|| {
                    anyhow::anyhow!("Full upgrade package for x86_64 architecture not found")
                }),
                Architecture::Aarch64 => platforms.aarch64.clone().ok_or_else(|| {
                    anyhow::anyhow!("Full upgrade package for aarch64 architecture not found")
                }),
                Architecture::Unsupported(_) => Err(anyhow::anyhow!(
                    "Full upgrade package for this architecture not found"
                )),
            }
        } else {
            //未找到对应架构的全量升级包，这里主动报错
            Err(anyhow::anyhow!(
                "Full upgrade package for the current architecture not found"
            ))
        }
    }

    /// 获取指定架构的补丁包信息
    fn get_patch_package(&self) -> Result<&PatchPackageInfo> {
        let patch_info = self
            .manifest
            .patch
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("Server does not support incremental upgrade"))?;
        match self.architecture {
            Architecture::X86_64 => patch_info.x86_64.as_ref().ok_or_else(|| {
                anyhow::anyhow!("Patch package for x86_64 architecture not available")
            }),
            Architecture::Aarch64 => patch_info.aarch64.as_ref().ok_or_else(|| {
                anyhow::anyhow!("Patch package for aarch64 architecture not available")
            }),
            Architecture::Unsupported(_) => Err(anyhow::anyhow!("Unsupported architecture")),
        }
    }

    /// 检查指定架构是否有可用的补丁
    fn has_patch_for_architecture(&self) -> bool {
        self.manifest
            .patch
            .as_ref()
            .map(|patch| match self.architecture {
                Architecture::X86_64 => patch.x86_64.is_some(),
                Architecture::Aarch64 => patch.aarch64.is_some(),
                Architecture::Unsupported(_) => false,
            })
            .unwrap_or(false)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::api_types::*;
    use std::fs;
    use tempfile::TempDir;

    // 创建测试用的增强服务清单
    fn create_test_manifest() -> EnhancedServiceManifest {
        EnhancedServiceManifest {
            version: "0.0.13.2".parse::<Version>().unwrap(),
            release_date: "2025-01-12T10:00:00Z".to_string(),
            release_notes: "测试版本".to_string(),
            packages: Some(ServicePackages {
                full: PackageInfo {
                    url: "https://example.com/docker.zip".to_string(),
                    hash: "sha256:full_hash".to_string(),
                    signature: "full_signature".to_string(),
                    size: 100 * 1024 * 1024, // 100MB
                },
                patch: None,
            }),
            platforms: Some(PlatformPackages {
                x86_64: Some(PlatformPackageInfo {
                    hash: None,
                    signature: "x86_64_signature".to_string(),
                    url: "https://example.com/x86_64/docker.zip".to_string(),
                }),
                aarch64: Some(PlatformPackageInfo {
                    hash: None,
                    signature: "aarch64_signature".to_string(),
                    url: "https://example.com/aarch64/docker.zip".to_string(),
                }),
            }),
            patch: Some(PatchInfo {
                x86_64: Some(PatchPackageInfo {
                    url: "https://example.com/patches/x86_64-patch.tar.gz".to_string(),
                    hash: Some("sha256:patch_hash_x86_64".to_string()),
                    signature: Some("patch_signature_x86_64".to_string()),
                    operations: PatchOperations {
                        replace: Some(ReplaceOperations {
                            files: vec!["app.jar".to_string(), "config.yml".to_string()],
                            directories: vec!["front/".to_string()],
                        }),
                        delete: Some(ReplaceOperations {
                            files: vec![
                                "old-files/app.jar".to_string(),
                                "old-files/config.yml".to_string(),
                            ],
                            directories: vec!["old-files/front/".to_string()],
                        }),
                    },
                    notes: None,
                }),
                aarch64: Some(PatchPackageInfo {
                    url: "https://example.com/patches/aarch64-patch.tar.gz".to_string(),
                    hash: Some("sha256:patch_hash_aarch64".to_string()),
                    signature: Some("patch_signature_aarch64".to_string()),
                    operations: PatchOperations {
                        replace: Some(ReplaceOperations {
                            files: vec!["app.jar".to_string(), "config.yml".to_string()],
                            directories: vec!["front/".to_string()],
                        }),
                        delete: Some(ReplaceOperations {
                            files: vec![
                                "old-files/app.jar".to_string(),
                                "old-files/config.yml".to_string(),
                            ],
                            directories: vec!["old-files/front/".to_string()],
                        }),
                    },
                    notes: None,
                }),
            }),
        }
    }

    // 创建测试环境，包括必要的docker目录和文件
    fn setup_test_environment() -> TempDir {
        let temp_dir = TempDir::new().unwrap();

        // 创建docker目录
        let docker_dir = temp_dir.path().join("docker");
        fs::create_dir(&docker_dir).unwrap();

        // 创建docker-compose.yml文件
        let compose_file = docker_dir.join("docker-compose.yml");
        fs::write(
            &compose_file,
            "version: '3.8'\nservices:\n  test:\n    image: hello-world",
        )
        .unwrap();

        // 设置当前工作目录为临时目录
        std::env::set_current_dir(&temp_dir).unwrap();

        temp_dir
    }

    #[test]
    fn test_no_upgrade_needed() {
        // 设置测试环境
        let _temp_dir = setup_test_environment();

        let manager = UpgradeStrategyManager::new(
            "0.0.13.2".to_string(),
            false,
            create_test_manifest(),
            DockerChannel::Images,
        );

        // 当前版本与服务器版本相同
        let strategy = manager.determine_strategy().unwrap();

        assert!(matches!(strategy, UpgradeStrategy::NoUpgrade { .. }));
    }

    #[test]
    fn test_current_version_newer() {
        // 设置测试环境
        let _temp_dir = setup_test_environment();

        let manager = UpgradeStrategyManager::new(
            "0.0.13.4".to_string(),
            false,
            create_test_manifest(),
            DockerChannel::Images,
        );

        // 当前版本比服务器版本新
        let strategy = manager.determine_strategy().unwrap();

        assert!(matches!(strategy, UpgradeStrategy::NoUpgrade { .. }));
    }

    #[test]
    fn test_full_upgrade_different_base_version() {
        // 设置测试环境
        let _temp_dir = setup_test_environment();

        let manager = UpgradeStrategyManager::new(
            "0.0.12".to_string(),
            false,
            create_test_manifest(),
            DockerChannel::Images,
        );

        // 不同基础版本，需要全量升级
        let strategy = manager.determine_strategy().unwrap();

        match strategy {
            UpgradeStrategy::FullUpgrade {
                url,
                target_version,
                ..
            } => {
                assert_eq!(
                    url,
                    format!(
                        "https://example.com/{}/docker.zip",
                        Architecture::detect().as_str()
                    )
                );
                assert_eq!(target_version, "0.0.13.2".parse::<Version>().unwrap());
            }
            _ => panic!("应该选择全量升级策略"),
        }
    }

    /// 两条通道会为同一版本号发布不同的包：registry 策略必须自带 registry 下载类型，
    /// 下载目录与缓存收据才不会和离线包混用
    #[test]
    fn registry_channel_stamps_full_package_with_registry_download_type() {
        let mut manifest = create_test_manifest();
        match Architecture::detect() {
            Architecture::X86_64 => {
                manifest
                    .platforms
                    .as_mut()
                    .unwrap()
                    .x86_64
                    .as_mut()
                    .unwrap()
                    .hash = Some("a".repeat(64))
            }
            Architecture::Aarch64 => {
                manifest
                    .platforms
                    .as_mut()
                    .unwrap()
                    .aarch64
                    .as_mut()
                    .unwrap()
                    .hash = Some("a".repeat(64))
            }
            _ => panic!("test host architecture unsupported"),
        }
        let manager = UpgradeStrategyManager::new(
            "0.0.12".to_string(),
            false,
            manifest,
            DockerChannel::Registry,
        );
        let UpgradeStrategy::FullUpgrade {
            url, download_type, ..
        } = manager.select_full_upgrade_strategy().unwrap()
        else {
            panic!("应该选择全量升级策略");
        };
        assert_eq!(download_type, DownloadType::Registry);
        assert_eq!(download_type.to_string(), "registry");
        assert_eq!(
            url,
            format!(
                "https://example.com/{}/docker.zip",
                Architecture::detect().as_str()
            )
        );
    }

    #[test]
    fn registry_channel_refuses_an_incremental_patch_package() {
        let _temp_dir = setup_test_environment();
        let manager = UpgradeStrategyManager::new(
            "0.0.13".to_string(),
            false,
            create_test_manifest(),
            DockerChannel::Registry,
        );
        let error = manager.determine_strategy().unwrap_err().to_string();
        assert!(
            error.contains("registry channel does not support incremental patch"),
            "unexpected error: {error}"
        );
    }

    /// 瘦包跳过 docker load，SHA 是唯一的下载完整性证明：清单不给就必须拒绝
    #[test]
    fn registry_channel_requires_a_published_package_sha() {
        let manager = UpgradeStrategyManager::new(
            "0.0.12".to_string(),
            false,
            create_test_manifest(),
            DockerChannel::Registry,
        );
        let error = manager.select_full_upgrade_strategy().unwrap_err();
        assert!(
            error.to_string().contains("publishes no SHA-256"),
            "unexpected error: {error}"
        );

        // 离线通道保持历史行为：没有可用 SHA 时填 external 占位继续部署
        let offline = UpgradeStrategyManager::new(
            "0.0.12".to_string(),
            false,
            create_test_manifest(),
            DockerChannel::Images,
        )
        .select_full_upgrade_strategy()
        .unwrap();
        let UpgradeStrategy::FullUpgrade { hash, .. } = offline else {
            panic!("应该选择全量升级策略");
        };
        assert_eq!(hash, "external");

        let mut manifest = create_test_manifest();
        let digest = "c".repeat(64);
        match Architecture::detect() {
            Architecture::X86_64 => {
                manifest
                    .platforms
                    .as_mut()
                    .unwrap()
                    .x86_64
                    .as_mut()
                    .unwrap()
                    .hash = Some(digest.clone());
            }
            Architecture::Aarch64 => {
                manifest
                    .platforms
                    .as_mut()
                    .unwrap()
                    .aarch64
                    .as_mut()
                    .unwrap()
                    .hash = Some(digest.clone());
            }
            _ => panic!("test host architecture unsupported"),
        }
        let manager = UpgradeStrategyManager::new(
            "0.0.12".to_string(),
            false,
            manifest,
            DockerChannel::Registry,
        );
        let UpgradeStrategy::FullUpgrade { hash, .. } =
            manager.select_full_upgrade_strategy().unwrap()
        else {
            panic!("应该选择全量升级策略");
        };
        assert_eq!(hash, digest);
    }

    #[test]
    fn test_patch_upgrade_same_base_version() {
        // 设置测试环境
        let _temp_dir = setup_test_environment();

        let manager = UpgradeStrategyManager::new(
            "0.0.13".to_string(),
            false,
            create_test_manifest(),
            DockerChannel::Images,
        );

        // 相同基础版本，可以增量升级
        let strategy = manager.determine_strategy().unwrap();

        match strategy {
            UpgradeStrategy::PatchUpgrade { target_version, .. } => {
                assert_eq!(target_version, "0.0.13.2".parse::<Version>().unwrap());
            }
            _ => panic!("应该选择增量升级策略"),
        }
    }

    #[test]
    fn test_force_full_upgrade() {
        // 设置测试环境
        let _temp_dir = setup_test_environment();

        let manager = UpgradeStrategyManager::new(
            "0.0.13.2".to_string(),
            true,
            create_test_manifest(),
            DockerChannel::Images,
        );

        // 强制全量升级，即使可以增量升级
        let strategy = manager.determine_strategy().unwrap();

        assert!(matches!(strategy, UpgradeStrategy::FullUpgrade { .. }));
    }

    #[test]
    fn cache_identity_uses_selected_platform_sha_and_url_matched_legacy_fallback() {
        let mut manifest = create_test_manifest();
        let platform = match Architecture::detect() {
            Architecture::X86_64 => manifest
                .platforms
                .as_mut()
                .unwrap()
                .x86_64
                .as_mut()
                .unwrap(),
            Architecture::Aarch64 => manifest
                .platforms
                .as_mut()
                .unwrap()
                .aarch64
                .as_mut()
                .unwrap(),
            _ => panic!("test host architecture unsupported"),
        };
        platform.hash = Some("a".repeat(64));
        let manager =
            UpgradeStrategyManager::new("0.0.12.0".into(), false, manifest, DockerChannel::Images);
        let UpgradeStrategy::FullUpgrade { hash, .. } =
            manager.select_full_upgrade_strategy().unwrap()
        else {
            panic!("expected full upgrade");
        };
        assert_eq!(hash, "a".repeat(64));

        let mut manifest = create_test_manifest();
        manifest.packages.as_mut().unwrap().full.hash = "b".repeat(64);
        let platform = match Architecture::detect() {
            Architecture::X86_64 => manifest
                .platforms
                .as_mut()
                .unwrap()
                .x86_64
                .as_mut()
                .unwrap(),
            Architecture::Aarch64 => manifest
                .platforms
                .as_mut()
                .unwrap()
                .aarch64
                .as_mut()
                .unwrap(),
            _ => panic!("test host architecture unsupported"),
        };
        platform.url = manifest.packages.as_ref().unwrap().full.url.clone();
        let manager =
            UpgradeStrategyManager::new("0.0.12.0".into(), false, manifest, DockerChannel::Images);
        let UpgradeStrategy::FullUpgrade { hash, .. } =
            manager.select_full_upgrade_strategy().unwrap()
        else {
            panic!("expected full upgrade");
        };
        assert_eq!(hash, "b".repeat(64));

        let manifest = create_test_manifest();
        let manager =
            UpgradeStrategyManager::new("0.0.12.0".into(), false, manifest, DockerChannel::Images);
        let UpgradeStrategy::FullUpgrade { hash, .. } =
            manager.select_full_upgrade_strategy().unwrap()
        else {
            panic!("expected full upgrade");
        };
        // Generic full.url differs from both platform URLs: no cross-arch digest.
        assert_eq!(hash, "external");
    }

    #[test]
    fn platform_digest_sentinels_do_not_hide_a_valid_same_url_generic_digest() {
        for placeholder in [None, Some(String::new()), Some("external".to_owned())] {
            let mut manifest = create_test_manifest();
            manifest.packages.as_mut().unwrap().full.hash = "b".repeat(64);
            let url = manifest.packages.as_ref().unwrap().full.url.clone();
            let platform = match Architecture::detect() {
                Architecture::X86_64 => manifest
                    .platforms
                    .as_mut()
                    .unwrap()
                    .x86_64
                    .as_mut()
                    .unwrap(),
                Architecture::Aarch64 => manifest
                    .platforms
                    .as_mut()
                    .unwrap()
                    .aarch64
                    .as_mut()
                    .unwrap(),
                _ => panic!("test host architecture unsupported"),
            };
            platform.hash = placeholder;
            platform.url = url;
            let manager = UpgradeStrategyManager::new(
                "0.0.12.0".into(),
                false,
                manifest,
                DockerChannel::Images,
            );
            let UpgradeStrategy::FullUpgrade { hash, .. } =
                manager.select_full_upgrade_strategy().unwrap()
            else {
                panic!("expected full upgrade");
            };
            assert_eq!(hash, "b".repeat(64));
        }
    }
}
