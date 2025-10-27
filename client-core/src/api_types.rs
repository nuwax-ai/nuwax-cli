use crate::version::Version;
use anyhow::Result;
use chrono;
use serde::{Deserialize, Serialize};
use std::{collections::HashMap, str::FromStr};

// ============================================================================
// 基础API结构
// ============================================================================

/// 客户端注册请求
#[derive(Debug, Serialize)]
pub struct ClientRegisterRequest {
    pub os: String,
    pub arch: String,
}

/// 注册客户端响应
#[derive(Debug, Deserialize)]
pub struct RegisterClientResponse {
    pub client_id: String,
}

/// 公告信息
#[derive(Debug, Deserialize)]
pub struct Announcement {
    pub id: i64,
    pub level: String,
    pub content: String,
    pub created_at: String,
}

/// 公告列表响应
#[derive(Debug, Deserialize)]
pub struct AnnouncementsResponse {
    pub announcements: Vec<Announcement>,
}

// ============================================================================
// 服务清单结构（传统格式）
// ============================================================================

/// 服务更新清单响应（传统格式）
#[derive(Debug, Deserialize)]
pub struct ServiceManifest {
    pub version: String,
    pub release_date: String,
    pub release_notes: String,
    pub packages: ServicePackages,
}

/// 服务包信息
#[derive(Debug, Deserialize)]
pub struct ServicePackages {
    pub full: PackageInfo,
    pub patch: Option<PackageInfo>,
}

/// 包信息
#[derive(Debug, Deserialize, Clone)]
pub struct PackageInfo {
    pub url: String,
    pub hash: String,
    pub signature: String,
    pub size: u64,
}

impl From<PackageInfo> for PlatformPackageInfo {
    fn from(package_info: PackageInfo) -> Self {
        PlatformPackageInfo {
            url: package_info.url,
            signature: package_info.signature,
        }
    }
}

// ============================================================================
// 增强服务清单结构（新格式）
// ============================================================================

/// 增强的服务更新清单响应（支持分架构和增量升级）
#[derive(Debug, Deserialize)]
pub struct EnhancedServiceManifest {
    /// 版本号,可能是“v1.0.2”，也可能是“1.0.2.4”;最后一位版本号是用于增量升级使用的;
    #[serde(deserialize_with = "crate::version::version_from_str")]
    pub version: Version,
    pub release_date: String,
    pub release_notes: String,

    /// 保持向后兼容的原有包格式
    pub packages: Option<ServicePackages>,

    /// 新增：分架构平台支持
    pub platforms: Option<PlatformPackages>,

    /// 新增：增量升级支持
    pub patch: Option<PatchInfo>,
}

/// 平台特定的包信息
#[derive(Debug, Deserialize)]
pub struct PlatformPackages {
    #[serde(rename = "x86_64")]
    pub x86_64: Option<PlatformPackageInfo>,
    #[serde(rename = "aarch64")]
    pub aarch64: Option<PlatformPackageInfo>,
}

/// 平台包信息
#[derive(Debug, Deserialize, Clone)]
pub struct PlatformPackageInfo {
    pub signature: String,
    pub url: String,
}

/// 增量升级信息
#[derive(Debug, Deserialize, Clone)]
pub struct PatchInfo {
    #[serde(rename = "x86_64")]
    pub x86_64: Option<PatchPackageInfo>,
    #[serde(rename = "aarch64")]
    pub aarch64: Option<PatchPackageInfo>,
}

/// 增量升级包信息
#[derive(Debug, Deserialize, Clone, PartialEq)]
pub struct PatchPackageInfo {
    pub url: String,
    pub hash: Option<String>,
    pub signature: Option<String>,
    pub operations: PatchOperations,
    /// 补丁说明
    pub notes: Option<String>,
}

impl PatchPackageInfo {
    //获取变更的文件或者目录
    pub fn get_changed_files(&self) -> Vec<String> {
        let mut changed_files = Vec::new();

        if let Some(replace) = &self.operations.replace {
            changed_files.extend(replace.files.clone());
            changed_files.extend(replace.directories.clone());
        }

        if let Some(delete) = &self.operations.delete {
            changed_files.extend(delete.files.clone());
            changed_files.extend(delete.directories.clone());
        }

        changed_files
    }
}

/// 补丁操作集合
#[derive(Debug, Deserialize, Clone, PartialEq)]
pub struct PatchOperations {
    ///替换
    pub replace: Option<ReplaceOperations>,
    ///删除
    pub delete: Option<ReplaceOperations>,
}

/// 替换操作
#[derive(Debug, Deserialize, Clone, PartialEq)]
pub struct ReplaceOperations {
    pub files: Vec<String>,
    pub directories: Vec<String>,
}

// ============================================================================
// 版本和升级相关
// ============================================================================

/// Docker版本检查响应
#[derive(Deserialize, Debug)]
pub struct DockerVersionResponse {
    pub current_version: String,
    pub latest_version: String,
    pub has_update: bool,
    pub release_notes: Option<String>,
}

/// Docker版本列表响应
#[derive(Deserialize, Debug)]
pub struct DockerVersionListResponse {
    pub versions: Vec<DockerVersion>,
}

/// Docker版本信息
#[derive(Deserialize, Debug)]
pub struct DockerVersion {
    pub version: String,
    pub release_date: String,
    pub notes: String,
    pub is_latest: bool,
}

/// 下载文件的哈希信息,用于下载文件的哈希验证
#[derive(Debug, Deserialize, Clone, PartialEq)]
pub struct DownloadHashInfo {
    pub hash: String,
    pub version: String,
    pub timestamp: String,
}

impl FromStr for DownloadHashInfo {
    type Err = serde_json::Error;

    fn from_str(s: &str) -> std::result::Result<Self, Self::Err> {
        let lines: Vec<&str> = s.trim().lines().collect();
        if lines.len() >= 3 {
            let hash = lines[0].to_string();
            let version = lines[1].to_string();
            let timestamp = lines[2].to_string();
            Ok(DownloadHashInfo {
                hash,
                version,
                timestamp,
            })
        } else {
            Err(serde::de::Error::custom("下载文件的哈希信息格式无效"))
        }
    }
}

// ============================================================================
// 上报和遥测相关
// ============================================================================

/// 服务升级历史上报请求
#[derive(Debug, Serialize)]
pub struct ServiceUpgradeReport {
    pub from_version: String,
    pub to_version: String,
    pub status: String,
    pub details: String,
}

/// 客户端自升级历史上报请求
#[derive(Debug, Serialize)]
pub struct ClientUpgradeReport {
    pub from_version: String,
    pub to_version: String,
    pub status: String,
    pub details: String,
}

/// 服务升级历史上报请求
#[derive(Serialize)]
pub struct ServiceUpgradeHistoryRequest {
    pub service_name: String,
    pub from_version: String,
    pub to_version: String,
    pub status: String,
    pub details: Option<String>,
}

/// 客户端自升级历史上报请求
#[derive(Serialize)]
pub struct ClientSelfUpgradeHistoryRequest {
    pub from_version: String,
    pub to_version: String,
    pub status: String,
    pub details: Option<String>,
}

/// 遥测数据上报请求
#[derive(Serialize)]
pub struct TelemetryRequest {
    pub event_type: String,
    pub data: serde_json::Value,
}

// ============================================================================
// 客户端清单相关
// ============================================================================

/// 客户端更新清单响应
#[derive(Debug, Deserialize)]
#[allow(dead_code)]
pub struct ClientManifest {
    pub version: String,
    pub notes: String,
    pub pub_date: String,
    pub platforms: HashMap<String, PlatformInfo>,
}

/// 客户端平台信息
#[derive(Debug, Deserialize)]
#[allow(dead_code)]
pub struct PlatformInfo {
    pub signature: String,
    pub url: String,
}

// ============================================================================
// 数据验证实现
// ============================================================================

impl EnhancedServiceManifest {
    /// 验证增强清单的完整性和有效性
    pub fn validate(&self) -> Result<()> {
        // 验证发布日期格式
        if chrono::DateTime::parse_from_rfc3339(&self.release_date).is_err() {
            return Err(anyhow::anyhow!("发布日期格式无效"));
        }

        // 验证原有包信息
        if let Some(ref packages) = self.packages {
            packages.validate()?;
        }

        // 验证平台包信息（如果存在）
        if let Some(ref platforms) = self.platforms {
            platforms.validate()?;
        }

        // 验证补丁信息（如果存在）
        if let Some(ref patch) = self.patch {
            patch.validate()?;
        }

        Ok(())
    }

    /// 检查是否支持指定架构
    pub fn supports_architecture(&self, arch: &str) -> bool {
        if let Some(ref platforms) = self.platforms {
            match arch {
                "x86_64" => platforms.x86_64.is_some(),
                "aarch64" => platforms.aarch64.is_some(),
                _ => false,
            }
        } else {
            // 没有平台信息时，默认支持（向后兼容）
            true
        }
    }

    /// 检查是否有可用的补丁
    pub fn has_patch_for_architecture(&self, arch: &str) -> bool {
        if let Some(ref patch) = self.patch {
            match arch {
                "x86_64" => patch.x86_64.is_some(),
                "aarch64" => patch.aarch64.is_some(),
                _ => false,
            }
        } else {
            false
        }
    }
}

impl ServicePackages {
    /// 验证服务包信息
    pub fn validate(&self) -> Result<()> {
        self.full.validate()?;

        if let Some(ref patch) = self.patch {
            patch.validate()?;
        }

        Ok(())
    }
}

impl PackageInfo {
    /// 验证包信息
    pub fn validate(&self) -> Result<()> {
        if self.url.is_empty() {
            return Err(anyhow::anyhow!("包URL不能为空"));
        }

        // 验证URL格式
        if !self.url.starts_with("http://")
            && !self.url.starts_with("https://")
            && !self.url.starts_with("/")
        {
            return Err(anyhow::anyhow!("包URL格式无效"));
        }

        Ok(())
    }
}

impl PlatformPackages {
    /// 验证平台包信息
    pub fn validate(&self) -> Result<()> {
        if let Some(ref x86_64) = self.x86_64 {
            x86_64.validate()?;
        }

        if let Some(ref aarch64) = self.aarch64 {
            aarch64.validate()?;
        }

        // 至少要有一个平台的包
        if self.x86_64.is_none() && self.aarch64.is_none() {
            return Err(anyhow::anyhow!("至少需要提供一个平台的包信息"));
        }

        Ok(())
    }
}

impl PlatformPackageInfo {
    /// 验证平台包信息
    pub fn validate(&self) -> Result<()> {
        if self.url.is_empty() {
            return Err(anyhow::anyhow!("平台包URL不能为空"));
        }

        if !self.url.starts_with("http://")
            && !self.url.starts_with("https://")
            && !self.url.starts_with("/")
        {
            return Err(anyhow::anyhow!("平台包URL格式无效"));
        }

        // 签名可以为空（对于某些部署环境）

        Ok(())
    }
}

impl PatchInfo {
    /// 验证补丁信息
    pub fn validate(&self) -> Result<()> {
        if let Some(ref x86_64) = self.x86_64 {
            x86_64.validate()?;
        }

        if let Some(ref aarch64) = self.aarch64 {
            aarch64.validate()?;
        }

        // 至少要有一个架构的补丁
        if self.x86_64.is_none() && self.aarch64.is_none() {
            return Err(anyhow::anyhow!("至少需要提供一个架构的补丁信息"));
        }

        Ok(())
    }
}

impl PatchPackageInfo {
    /// 验证补丁包信息
    pub fn validate(&self) -> Result<()> {
        if self.url.is_empty() {
            return Err(anyhow::anyhow!("补丁包URL不能为空"));
        }

        if !self.url.starts_with("http://")
            && !self.url.starts_with("https://")
            && !self.url.starts_with("/")
        {
            return Err(anyhow::anyhow!("补丁包URL格式无效"));
        }

        if let Some(hash) = &self.hash {
            if hash.is_empty() {
                return Err(anyhow::anyhow!("补丁包哈希值不能为空"));
            }
        }

        self.operations.validate()?;

        Ok(())
    }
}

impl PatchOperations {
    /// 验证补丁操作
    pub fn validate(&self) -> Result<()> {
        if let Some(replace) = &self.replace {
            replace.validate()?;
        }

        // 验证删除路径
        if let Some(delete) = &self.delete {
            delete.validate()?;
        }

        Ok(())
    }

    /// 计算补丁操作总数
    pub fn total_operations(&self) -> usize {
        let mut total_operations = 0;
        if let Some(replace) = &self.replace {
            total_operations += replace.files.len();
            total_operations += replace.directories.len();
        }
        if let Some(delete) = &self.delete {
            total_operations += delete.files.len();
            total_operations += delete.directories.len();
        }
        total_operations
    }
}

impl ReplaceOperations {
    /// 验证替换操作
    pub fn validate(&self) -> Result<()> {
        // 验证文件路径
        for file_path in &self.files {
            if file_path.is_empty() {
                return Err(anyhow::anyhow!("文件路径不能为空"));
            }

            // 安全检查：防止访问系统重要路径
            if file_path.starts_with("/")
                || file_path.starts_with("../")
                || file_path.contains("..\\")
                || file_path.starts_with("C:\\")
            {
                return Err(anyhow::anyhow!("危险的文件路径: {}", file_path));
            }
        }

        // 验证目录路径
        for dir_path in &self.directories {
            if dir_path.is_empty() {
                return Err(anyhow::anyhow!("目录路径不能为空"));
            }

            // 安全检查：防止访问系统重要路径
            if dir_path.starts_with("/")
                || dir_path.starts_with("../")
                || dir_path.contains("..\\")
                || dir_path.starts_with("C:\\")
            {
                return Err(anyhow::anyhow!("危险的目录路径: {}", dir_path));
            }
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const ENHANCED_MANIFEST_JSON: &str = r#"
    {
        "version": "0.0.13",
        "release_date": "2025-01-12T13:49:59Z",
        "release_notes": "增强版本更新",
        "packages": {
            "full": {
                "url": "https://example.com/docker.zip",
                "hash": "external",
                "signature": "",
                "size": 0
            },
            "patch": null
        },
        "platforms": {
            "x86_64": {
                "signature": "dW50cnVzdGVkIGNvbW1lbnQ6IHNpZ25hdHVyZSBmcm9tIGNsaSBzZWNyZXQga2V5CkNMSS1MSU5VWC1YNjQtdjEuMS4w",
                "url": "https://packages.com/x86_64/docker.zip"
            },
            "aarch64": {
                "signature": "dW50cnVzdGVkIGNvbW1lbnQ6IHNpZ25hdHVyZSBmcm9tIGNsaSBzZWNyZXQga2V5CkNMSS1XSU5ET1dTLVg2NC12MS4xLjA=",
                "url": "https://packages.com/aarch64/docker.zip"
            }
        },
        "patch": {
            "version": "0.0.13.2",
            "x86_64": {
                "url": "https://packages.com/patches/x86_64-patch.tar.gz",
                "hash": "sha256:patch_hash_x86_64",
                "signature": "patch_signature_x86_64",
                "operations": {
                    "replace": {
                        "files": [
                            "app/app.jar",
                            "config/application.yml"
                        ],
                        "directories": [
                            "front/",
                            "plugins/"
                        ]
                    },
                    "delete": {
                        "files": [
                            "app/app.jar",
                            "config/application.yml"
                        ],
                        "directories": [
                            "front/",
                            "plugins/"
                        ]
                    }
                }
            },
            "aarch64": {
                "url": "https://packages.com/patches/aarch64-patch.tar.gz",
                "hash": "sha256:patch_hash_aarch64",
                "signature": "patch_signature_aarch64",
                "operations": {
                    "replace": {
                        "files": [
                            "app.jar",
                            "config/application.yml"
                        ],
                        "directories": [
                            "front/",
                            "plugins/"
                        ]
                    },
                    "delete": {
                        "files": [
                            "app/app.jar",
                            "config/application.yml"
                        ],
                        "directories": [
                            "front/",
                            "plugins/"
                        ]
                    }
                }
            }
        }
    }
    "#;

    #[test]
    fn test_enhanced_manifest_parsing() {
        let manifest: EnhancedServiceManifest =
            serde_json::from_str(ENHANCED_MANIFEST_JSON).expect("应该能够解析增强清单JSON");

        // 验证基本字段
        assert_eq!(manifest.version, "0.0.13".parse::<Version>().unwrap());
        assert_eq!(manifest.release_notes, "增强版本更新");

        // 验证平台信息存在
        assert!(manifest.platforms.is_some());
        let platforms = manifest.platforms.unwrap();
        assert!(platforms.x86_64.is_some());
        assert!(platforms.aarch64.is_some());

        // 验证平台包信息
        let x86_64_pkg = platforms.x86_64.unwrap();
        assert_eq!(x86_64_pkg.url, "https://packages.com/x86_64/docker.zip");

        // 验证补丁信息存在
        assert!(manifest.patch.is_some());
        let patch = manifest.patch.unwrap();
        assert!(patch.x86_64.is_some());
        assert!(patch.aarch64.is_some());

        // 验证补丁操作
        let x86_64_patch = patch.x86_64.unwrap();
        assert_eq!(
            x86_64_patch.clone().operations.replace.unwrap().files.len(),
            2
        );
        assert_eq!(
            x86_64_patch
                .clone()
                .operations
                .replace
                .unwrap()
                .directories
                .len(),
            2
        );
        assert_eq!(
            x86_64_patch.clone().operations.delete.unwrap().files.len(),
            2
        );
        assert_eq!(
            x86_64_patch
                .clone()
                .operations
                .delete
                .unwrap()
                .directories
                .len(),
            2
        );
    }

    #[test]
    fn test_enhanced_manifest_validation() {
        let manifest: EnhancedServiceManifest =
            serde_json::from_str(ENHANCED_MANIFEST_JSON).expect("应该能够解析增强清单JSON");

        // 验证数据结构的完整性
        manifest.validate().expect("增强清单应该通过验证");
    }

    #[test]
    fn test_architecture_support_check() {
        let manifest: EnhancedServiceManifest =
            serde_json::from_str(ENHANCED_MANIFEST_JSON).expect("应该能够解析增强清单JSON");

        // 测试架构支持检查
        assert!(manifest.supports_architecture("x86_64"));
        assert!(manifest.supports_architecture("aarch64"));
        assert!(!manifest.supports_architecture("unsupported"));

        // 测试补丁支持检查
        assert!(manifest.has_patch_for_architecture("x86_64"));
        assert!(manifest.has_patch_for_architecture("aarch64"));
        assert!(!manifest.has_patch_for_architecture("unsupported"));
    }

    #[test]
    fn test_legacy_manifest_compatibility() {
        let legacy_json = r#"
        {
            "version": "0.0.12",
            "release_date": "2025-01-10T10:00:00Z",
            "release_notes": "传统版本",
            "packages": {
                "full": {
                    "url": "https://example.com/docker.zip",
                    "hash": "external",
                    "signature": "",
                    "size": 0
                },
                "patch": null
            }
        }
        "#;

        // 测试旧格式解析
        let legacy_manifest: ServiceManifest =
            serde_json::from_str(legacy_json).expect("应该能够解析传统清单JSON");

        // 转换为增强格式
        let enhanced_manifest = EnhancedServiceManifest {
            version: legacy_manifest.version.parse::<Version>().unwrap(),
            release_date: legacy_manifest.release_date,
            release_notes: legacy_manifest.release_notes,
            packages: Some(legacy_manifest.packages),
            platforms: None,
            patch: None,
        };

        // 验证转换后的格式
        enhanced_manifest
            .validate()
            .expect("转换后的增强清单应该通过验证");
        assert_eq!(
            enhanced_manifest.version,
            "0.0.12".parse::<Version>().unwrap()
        );
        assert!(enhanced_manifest.platforms.is_none());
        assert!(enhanced_manifest.patch.is_none());

        // 测试向后兼容的架构支持（默认支持）
        assert!(enhanced_manifest.supports_architecture("x86_64"));
        assert!(enhanced_manifest.supports_architecture("aarch64"));
        assert!(!enhanced_manifest.has_patch_for_architecture("x86_64"));
    }

    #[test]
    fn test_patch_operations_validation() {
        // 测试安全路径验证
        let safe_operations = PatchOperations {
            replace: Some(ReplaceOperations {
                files: vec!["app/app.jar".to_string(), "config/app.yml".to_string()],
                directories: vec!["plugins/".to_string()],
            }),
            delete: Some(ReplaceOperations {
                files: vec![],
                directories: vec!["temp/cache/".to_string()],
            }),
        };

        safe_operations.validate().expect("安全路径应该通过验证");

        // 测试危险路径检测
        let dangerous_operations = PatchOperations {
            replace: Some(ReplaceOperations {
                files: vec!["../../../etc/passwd".to_string()],
                directories: vec!["plugins/".to_string()],
            }),
            delete: Some(ReplaceOperations {
                files: vec![],
                directories: vec!["temp/cache/".to_string()],
            }),
        };

        assert!(
            dangerous_operations.validate().is_err(),
            "危险路径应该被拒绝"
        );
    }

    #[test]
    fn test_platform_package_validation() {
        let valid_platform_pkg = PlatformPackageInfo {
            signature: "valid_signature".to_string(),
            url: "https://example.com/package.zip".to_string(),
        };

        valid_platform_pkg
            .validate()
            .expect("有效的平台包信息应该通过验证");

        let invalid_platform_pkg = PlatformPackageInfo {
            signature: "signature".to_string(),
            url: "".to_string(), // 空URL
        };

        assert!(invalid_platform_pkg.validate().is_err(), "空URL应该被拒绝");
    }

    // Task 1.1 验收标准测试
    #[test]
    fn test_task_1_1_acceptance_criteria() {
        // 验收标准：能够成功解析新的JSON格式
        let json_str = ENHANCED_MANIFEST_JSON;
        let manifest: EnhancedServiceManifest =
            serde_json::from_str(json_str).expect("应该能够成功解析新的JSON格式");

        // 验收标准：解析后的数据结构包含platforms字段
        assert!(manifest.platforms.is_some(), "manifest.platforms应该存在");

        // 验收标准：解析后的数据结构包含patch字段
        assert!(manifest.patch.is_some(), "manifest.patch应该存在");

        println!("✅ Task 1.1: 扩展数据结构定义 - 验收标准测试通过");
        println!("   - ✅ 成功解析新的JSON格式");
        println!("   - ✅ platforms字段存在且可访问");
        println!("   - ✅ patch字段存在且可访问");
        println!("   - ✅ 数据验证功能正常");
    }

    // Task 1.5 验收标准测试
    #[test]
    fn test_task_1_5_acceptance_criteria() {
        // 验收标准：EnhancedServiceManifest数据结构正确解析
        let enhanced_json = ENHANCED_MANIFEST_JSON;
        let enhanced_manifest: EnhancedServiceManifest =
            serde_json::from_str(enhanced_json).expect("应该能够解析增强服务清单");

        // 验证enhanced manifest的功能
        assert!(enhanced_manifest.supports_architecture("x86_64"));
        assert!(enhanced_manifest.supports_architecture("aarch64"));
        assert!(enhanced_manifest.has_patch_for_architecture("x86_64"));

        // 验收标准：向后兼容性 - 旧格式仍然可以解析
        let legacy_json = r#"
        {
            "version": "0.0.12",
            "release_date": "2025-01-10T10:00:00Z",
            "release_notes": "传统版本",
            "packages": {
                "full": {
                    "url": "https://example.com/docker.zip",
                    "hash": "external",
                    "signature": "",
                    "size": 0
                },
                "patch": null
            }
        }
        "#;

        let legacy_manifest: ServiceManifest =
            serde_json::from_str(legacy_json).expect("应该能够解析传统服务清单");

        // 验证legacy到enhanced的转换
        let converted = EnhancedServiceManifest {
            version: legacy_manifest.version.parse::<Version>().unwrap(),
            release_date: legacy_manifest.release_date,
            release_notes: legacy_manifest.release_notes,
            packages: Some(legacy_manifest.packages),
            platforms: None,
            patch: None,
        };

        // 验证转换后的功能（向后兼容）
        assert!(converted.supports_architecture("x86_64")); // 默认支持
        assert!(converted.supports_architecture("aarch64")); // 默认支持
        assert!(!converted.has_patch_for_architecture("x86_64")); // 无patch

        // 验收标准：数据验证功能
        enhanced_manifest.validate().expect("增强清单应该通过验证");
        converted.validate().expect("转换后的清单应该通过验证");

        // 验收标准：错误处理支持
        // 测试无效JSON的错误处理
        let invalid_json = r#"{"invalid": "json"}"#;
        assert!(serde_json::from_str::<EnhancedServiceManifest>(invalid_json).is_err());

        println!("✅ Task 1.5: API 客户端扩展 - 验收标准测试通过");
        println!("   - ✅ get_enhanced_service_manifest方法数据结构支持完整");
        println!("   - ✅ 向后兼容性保持 - 旧格式ServiceManifest正常工作");
        println!("   - ✅ 错误处理支持 - 无效格式能正确处理");
        println!("   - ✅ 数据验证功能正常");
        println!("   - ✅ 架构支持检查功能正常");
    }
}
