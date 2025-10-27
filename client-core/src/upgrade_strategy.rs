use std::fmt::Display;
use std::path::PathBuf;

use crate::{
    api_types::{EnhancedServiceManifest, PatchPackageInfo},
    architecture::Architecture,
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
}

impl Display for DownloadType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            DownloadType::Full => write!(f, "full"),
            DownloadType::Patch => write!(f, "patch"),
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
            UpgradeStrategy::FullUpgrade { .. } => vec!["data".to_string(),"upload".to_string()],
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
}

impl UpgradeStrategyManager {
    /// 创建新的升级策略管理器
    pub fn new(
        current_version: String,
        force_full: bool,
        manifest: EnhancedServiceManifest,
    ) -> Self {
        Self {
            manifest,
            current_version,
            force_full,
            architecture: Architecture::detect(),
        }
    }

    /// 确定升级策略（简化版本）
    pub fn determine_strategy(&self) -> Result<UpgradeStrategy> {
        info!("🔍 开始升级策略决策");
        info!("   当前版本: {}", self.current_version);
        info!("   服务器版本: {}", self.manifest.version);
        info!("   目标架构: {}", self.architecture.as_str());
        info!("   强制全量: {}", self.force_full);

        // 1. 解析当前版本
        let current_ver = self.current_version.parse::<Version>()?;

        // 2. 首先与基础服务器版本比较，确定是否需要升级
        let server_ver = self.manifest.version.clone();
        //比较当前版本和服务器版本，判断是全量，还是增量升级，还是不需要升级
        let base_comparison = current_ver.compare_detailed(&server_ver);

        info!("📊 当前版本详细: {:?}", current_ver);
        info!("📊 服务器版本详细: {:?}", server_ver);
        info!("📊 基础版本比较结果: {:?}", base_comparison);

        // 3. 强制全量升级
        if self.force_full {
            info!("🔄 强制执行全量升级");
            return self.select_full_upgrade_strategy();
        }
        //判断工作目录下,是否有docker目录,如果没有docker目录,则也使用全量升级
        let work_dir = get_docker_work_dir();
        let compose_file_path = get_compose_file_path();
        if !work_dir.exists() || !compose_file_path.exists() {
            info!("❌ 工作目录下没有docker目录或compose文件，选择全量升级策略");
            return self.select_full_upgrade_strategy();
        }

        // 4. 根据版本比较结果决策
        match base_comparison {
            crate::version::VersionComparison::Equal | crate::version::VersionComparison::Newer => {
                info!("✅ 当前版本已是最新，无需升级");
                Ok(UpgradeStrategy::NoUpgrade {
                    target_version: self.manifest.version.clone(),
                })
            }
            crate::version::VersionComparison::PatchUpgradeable => {
                // 可以进行增量升级
                if !self.has_patch_for_architecture() {
                    info!("📦 当前架构无增量升级包，选择全量升级策略");
                    self.select_full_upgrade_strategy()
                } else {
                    info!("⚡ 选择增量升级策略");
                    self.select_patch_upgrade_strategy()
                }
            }
            crate::version::VersionComparison::FullUpgradeRequired => {
                // 需要全量升级
                info!("📦 选择全量升级策略");
                self.select_full_upgrade_strategy()
            }
        }
    }

    /// 选择全量升级策略
    pub fn select_full_upgrade_strategy(&self) -> Result<UpgradeStrategy> {
        debug!("🔍 选择全量升级策略");

        if let Some(_) = &self.manifest.platforms {
            //使用分架构的全量包
            let platform_info = self.get_platform_package()?;

            debug!("📦 使用架构特定的全量包: {}", &platform_info.url);
            Ok(UpgradeStrategy::FullUpgrade {
                url: platform_info.url.clone(),
                hash: "external".to_string(), // 平台包通常没有预设哈希
                signature: platform_info.signature.clone(),
                target_version: self.manifest.version.clone(),
                download_type: DownloadType::Full,
            })
        } else {
            if let Some(package_info) = &self.manifest.packages {
                let full_info = &package_info.full;
                debug!("📦 使用通用的全量包: {}", &full_info.url);
                Ok(UpgradeStrategy::FullUpgrade {
                    url: full_info.url.clone(),
                    hash: full_info.hash.clone(),
                    signature: full_info.signature.clone(),
                    target_version: self.manifest.version.clone(),
                    download_type: DownloadType::Full,
                })
            } else {
                //未找到对应架构的全量升级包，这里主动报错
                Err(anyhow::anyhow!("未找到对应架构的全量升级包"))
            }
        }
    }

    /// 选择增量升级策略
    pub fn select_patch_upgrade_strategy(&self) -> Result<UpgradeStrategy> {
        debug!("🔍 选择增量升级策略");

        let patch_info = self.get_patch_package()?;

        debug!("📦 使用架构特定的补丁包: {}", &patch_info.url);
        Ok(UpgradeStrategy::PatchUpgrade {
            patch_info: patch_info.clone(),
            target_version: self.manifest.version.clone(),
            download_type: DownloadType::Patch,
        })
    }

    /// 获取指定架构的平台包信息
    fn get_platform_package<'a>(&self) -> Result<crate::api_types::PlatformPackageInfo> {
        if let Some(platforms) = self.manifest.platforms.as_ref() {
            match self.architecture {
                Architecture::X86_64 => platforms
                    .x86_64
                    .clone()
                    .ok_or_else(|| anyhow::anyhow!("未找到对应架构的全量升级包")),
                Architecture::Aarch64 => platforms
                    .aarch64
                    .clone()
                    .ok_or_else(|| anyhow::anyhow!("未找到对应架构的全量升级包")),
                Architecture::Unsupported(_) => Err(anyhow::anyhow!("未找到对应架构的全量升级包")),
            }
        } else {
            //未找到对应架构的全量升级包，这里主动报错
            Err(anyhow::anyhow!("未找到对应架构的全量升级包"))
        }
    }

    /// 获取指定架构的补丁包信息
    fn get_patch_package(&self) -> Result<&PatchPackageInfo> {
        let patch_info = self
            .manifest
            .patch
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("服务器不支持增量升级"))?;
        match self.architecture {
            Architecture::X86_64 => patch_info
                .x86_64
                .as_ref()
                .ok_or_else(|| anyhow::anyhow!("x86_64架构的补丁包不可用")),
            Architecture::Aarch64 => patch_info
                .aarch64
                .as_ref()
                .ok_or_else(|| anyhow::anyhow!("aarch64架构的补丁包不可用")),
            Architecture::Unsupported(_) => Err(anyhow::anyhow!("不支持的架构")),
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
                    signature: "x86_64_signature".to_string(),
                    url: "https://example.com/x86_64/docker.zip".to_string(),
                }),
                aarch64: Some(PlatformPackageInfo {
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

        let manager =
            UpgradeStrategyManager::new("0.0.13.2".to_string(), false, create_test_manifest());

        // 当前版本与服务器版本相同
        let strategy = manager.determine_strategy().unwrap();

        assert!(matches!(strategy, UpgradeStrategy::NoUpgrade { .. }));
    }

    #[test]
    fn test_current_version_newer() {
        // 设置测试环境
        let _temp_dir = setup_test_environment();

        let manager =
            UpgradeStrategyManager::new("0.0.13.4".to_string(), false, create_test_manifest());

        // 当前版本比服务器版本新
        let strategy = manager.determine_strategy().unwrap();

        assert!(matches!(strategy, UpgradeStrategy::NoUpgrade { .. }));
    }

    #[test]
    fn test_full_upgrade_different_base_version() {
        // 设置测试环境
        let _temp_dir = setup_test_environment();

        let manager =
            UpgradeStrategyManager::new("0.0.12".to_string(), false, create_test_manifest());

        // 不同基础版本，需要全量升级
        let strategy = manager.determine_strategy().unwrap();

        match strategy {
            UpgradeStrategy::FullUpgrade {
                url,
                target_version,
                ..
            } => {
                assert_eq!(url, "https://example.com/aarch64/docker.zip");
                assert_eq!(target_version, "0.0.13.2".parse::<Version>().unwrap());
            }
            _ => panic!("应该选择全量升级策略"),
        }
    }

    #[test]
    fn test_patch_upgrade_same_base_version() {
        // 设置测试环境
        let _temp_dir = setup_test_environment();

        let manager =
            UpgradeStrategyManager::new("0.0.13".to_string(), false, create_test_manifest());

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

        let manager =
            UpgradeStrategyManager::new("0.0.13.2".to_string(), true, create_test_manifest());

        // 强制全量升级，即使可以增量升级
        let strategy = manager.determine_strategy().unwrap();

        assert!(matches!(strategy, UpgradeStrategy::FullUpgrade { .. }));
    }
}
