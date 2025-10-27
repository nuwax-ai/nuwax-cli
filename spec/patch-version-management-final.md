# Duck CLI 复合版本号管理实现方案

## 🎯 方案2：复合版本号设计

### 版本号格式规范

```
完整版本号格式：{主版本}.{次版本}.{修订版本}.{补丁级别}
示例：
- 全量版本：0.0.13
- 复合版本：0.0.13.2 (在0.0.13基础上应用第2个补丁)
```

### config.toml 结构设计

```toml
[versions]
# 基础全量版本（兼容现有字段）
docker_service = "0.0.13"

# 复合版本号（新增字段）
current_version = "0.0.13.2"      # 当前实际运行的版本（包含补丁级别）

# 升级历史记录（可选）
last_full_upgrade = "2025-01-12T10:30:00Z"
last_patch_upgrade = "2025-01-12T15:45:00Z"
```

### 关键设计原则

1. **兼容性**：保持 `docker_service` 字段不变，确保向后兼容
2. **清晰性**：`current_version` 始终反映真实的运行版本
3. **简洁性**：避免复杂的补丁历史数组，简化配置结构

## 🏗️ Rust 代码实现

### 1. 版本结构定义

```rust
// client-core/src/version.rs

use serde::{Deserialize, Serialize};
use std::fmt;
use std::str::FromStr;

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct Version {
    pub major: u32,
    pub minor: u32,
    pub patch: u32,
    pub build: u32,  // 补丁级别
}

impl Version {
    pub fn new(major: u32, minor: u32, patch: u32, build: u32) -> Self {
        Self { major, minor, patch, build }
    }
    
    /// 获取基础版本（不包含补丁级别）
    pub fn base_version(&self) -> Self {
        Self {
            major: self.major,
            minor: self.minor,
            patch: self.patch,
            build: 0,
        }
    }
    
    /// 检查是否可以在此版本上应用补丁
    pub fn can_apply_patch(&self, patch_base_version: &Self) -> bool {
        self.base_version() == patch_base_version.base_version()
    }
    
    /// 应用补丁，返回新版本
    pub fn apply_patch(&self, patch_level: u32) -> Self {
        Self {
            major: self.major,
            minor: self.minor,
            patch: self.patch,
            build: patch_level,
        }
    }
    
    /// 检查是否是全量版本（补丁级别为0）
    pub fn is_full_version(&self) -> bool {
        self.build == 0
    }
    
    /// 获取下一个补丁版本
    pub fn next_patch_version(&self) -> Self {
        Self {
            major: self.major,
            minor: self.minor,
            patch: self.patch,
            build: self.build + 1,
        }
    }
}

impl FromStr for Version {
    type Err = VersionParseError;
    
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let parts: Vec<&str> = s.split('.').collect();
        
        match parts.len() {
            3 => {
                // 解析 "0.0.13" 格式（全量版本）
                Ok(Version {
                    major: parts[0].parse()?,
                    minor: parts[1].parse()?,
                    patch: parts[2].parse()?,
                    build: 0,
                })
            }
            4 => {
                // 解析 "0.0.13.2" 格式（复合版本）
                Ok(Version {
                    major: parts[0].parse()?,
                    minor: parts[1].parse()?,
                    patch: parts[2].parse()?,
                    build: parts[3].parse()?,
                })
            }
            _ => Err(VersionParseError::InvalidFormat(s.to_string())),
        }
    }
}

impl fmt::Display for Version {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.build == 0 {
            write!(f, "{}.{}.{}", self.major, self.minor, self.patch)
        } else {
            write!(f, "{}.{}.{}.{}", self.major, self.minor, self.patch, self.build)
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub enum VersionParseError {
    #[error("版本格式无效: {0}")]
    InvalidFormat(String),
    #[error("版本号解析错误: {0}")]
    ParseError(#[from] std::num::ParseIntError),
}

#[cfg(test)]
mod tests {
    use super::*;
    
    #[test]
    fn test_version_parsing() {
        // 全量版本
        let v1 = Version::from_str("0.0.13").unwrap();
        assert_eq!(v1, Version::new(0, 0, 13, 0));
        assert_eq!(v1.to_string(), "0.0.13");
        
        // 复合版本
        let v2 = Version::from_str("0.0.13.2").unwrap();
        assert_eq!(v2, Version::new(0, 0, 13, 2));
        assert_eq!(v2.to_string(), "0.0.13.2");
        
        // 版本比较
        assert!(v2 > v1);
        assert!(v2.can_apply_patch(&v1));
    }
    
    #[test]
    fn test_version_operations() {
        let base = Version::from_str("0.0.13").unwrap();
        let patched = base.apply_patch(2);
        
        assert_eq!(patched.to_string(), "0.0.13.2");
        assert!(!base.is_full_version() || base.build == 0);
        assert!(!patched.is_full_version());
    }
}
```

### 2. 配置结构扩展

```rust
// client-core/src/config.rs

use crate::version::Version;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::str::FromStr;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VersionConfig {
    /// 基础Docker服务版本（保持向后兼容）
    pub docker_service: String,
    
    /// 当前实际运行的版本（复合版本号）
    pub current_version: String,
    
    /// 最后一次全量升级时间
    pub last_full_upgrade: Option<DateTime<Utc>>,
    
    /// 最后一次补丁升级时间
    pub last_patch_upgrade: Option<DateTime<Utc>>,
}

impl Default for VersionConfig {
    fn default() -> Self {
        Self {
            docker_service: "0.0.1".to_string(),
            current_version: "0.0.1".to_string(),
            last_full_upgrade: None,
            last_patch_upgrade: None,
        }
    }
}

impl VersionConfig {
    /// 获取当前版本对象
    pub fn get_current_version(&self) -> Result<Version, VersionParseError> {
        Version::from_str(&self.current_version)
    }
    
    /// 获取基础版本对象
    pub fn get_base_version(&self) -> Result<Version, VersionParseError> {
        Version::from_str(&self.docker_service)
    }
    
    /// 更新到新的全量版本
    pub fn update_full_version(&mut self, new_version: String) -> Result<(), VersionParseError> {
        // 验证版本格式
        let _ = Version::from_str(&new_version)?;
        
        self.docker_service = new_version.clone();
        self.current_version = new_version; // 全量版本的补丁级别为0
        self.last_full_upgrade = Some(Utc::now());
        
        Ok(())
    }
    
    /// 应用补丁版本
    pub fn apply_patch_version(&mut self, patch_version: String) -> Result<(), VersionParseError> {
        // 验证补丁版本格式
        let patch_ver = Version::from_str(&patch_version)?;
        let current_ver = self.get_current_version()?;
        
        // 检查补丁是否适用
        if !current_ver.can_apply_patch(&patch_ver) {
            return Err(VersionParseError::InvalidFormat(
                format!("补丁版本 {} 不适用于当前版本 {}", patch_version, self.current_version)
            ));
        }
        
        // 检查补丁版本是否比当前版本新
        if patch_ver <= current_ver {
            return Err(VersionParseError::InvalidFormat(
                format!("补丁版本 {} 不比当前版本 {} 新", patch_version, self.current_version)
            ));
        }
        
        self.current_version = patch_version;
        self.last_patch_upgrade = Some(Utc::now());
        
        Ok(())
    }
    
    /// 检查是否需要全量升级
    pub fn needs_full_upgrade(&self, target_version: &str) -> Result<bool, VersionParseError> {
        let current = self.get_current_version()?;
        let target = Version::from_str(target_version)?;
        
        // 如果目标版本的基础版本比当前基础版本新，需要全量升级
        Ok(target.base_version() > current.base_version())
    }
    
    /// 检查是否可以应用补丁升级
    pub fn can_apply_patch(&self, patch_version: &str) -> Result<bool, VersionParseError> {
        let current = self.get_current_version()?;
        let patch = Version::from_str(patch_version)?;
        
        // 检查补丁是否适用且比当前版本新
        Ok(current.can_apply_patch(&patch) && patch > current)
    }
    
    /// 获取版本信息摘要
    pub fn get_version_summary(&self) -> String {
        format!(
            "基础版本: {}, 当前版本: {}, 最后全量升级: {}, 最后补丁升级: {}",
            self.docker_service,
            self.current_version,
            self.last_full_upgrade
                .map(|t| t.format("%Y-%m-%d %H:%M:%S UTC").to_string())
                .unwrap_or_else(|| "从未".to_string()),
            self.last_patch_upgrade
                .map(|t| t.format("%Y-%m-%d %H:%M:%S UTC").to_string())
                .unwrap_or_else(|| "从未".to_string())
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    
    #[test]
    fn test_version_config_operations() {
        let mut config = VersionConfig::default();
        
        // 测试全量升级
        config.update_full_version("0.0.13".to_string()).unwrap();
        assert_eq!(config.docker_service, "0.0.13");
        assert_eq!(config.current_version, "0.0.13");
        
        // 测试补丁升级
        config.apply_patch_version("0.0.13.2".to_string()).unwrap();
        assert_eq!(config.current_version, "0.0.13.2");
        
        // 测试升级检查
        assert!(!config.needs_full_upgrade("0.0.13.3").unwrap());
        assert!(config.needs_full_upgrade("0.0.14").unwrap());
        assert!(config.can_apply_patch("0.0.13.3").unwrap());
        assert!(!config.can_apply_patch("0.0.14.1").unwrap());
    }
}
```

### 3. 升级策略管理器

```rust
// client-core/src/upgrade_strategy.rs

use crate::version::Version;
use crate::config::VersionConfig;
use std::str::FromStr;

#[derive(Debug, Clone)]
pub enum UpgradeStrategy {
    /// 无需升级
    NoUpgrade {
        current_version: String,
    },
    /// 全量升级
    FullUpgrade {
        from_version: String,
        to_version: String,
        architecture: String,
        download_url: String,
        signature: String,
    },
    /// 增量升级
    PatchUpgrade {
        from_version: String,
        to_version: String,
        architecture: String,
        download_url: String,
        hash: String,
        signature: String,
    },
    /// 回退到传统升级
    LegacyUpgrade {
        from_version: String,
        to_version: String,
        download_url: String,
    },
}

impl UpgradeStrategy {
    pub fn get_description(&self) -> String {
        match self {
            UpgradeStrategy::NoUpgrade { current_version } => {
                format!("✅ 当前版本 {} 已是最新", current_version)
            }
            UpgradeStrategy::FullUpgrade { from_version, to_version, .. } => {
                format!("📦 全量升级: {} → {}", from_version, to_version)
            }
            UpgradeStrategy::PatchUpgrade { from_version, to_version, .. } => {
                format!("🔄 增量升级: {} → {}", from_version, to_version)
            }
            UpgradeStrategy::LegacyUpgrade { from_version, to_version, .. } => {
                format!("🔧 传统升级: {} → {}", from_version, to_version)
            }
        }
    }
}

pub struct UpgradeStrategyManager;

impl UpgradeStrategyManager {
    /// 根据服务清单和当前配置确定升级策略
    pub fn determine_strategy(
        version_config: &VersionConfig,
        manifest: &EnhancedServiceManifest,
        architecture: &str,
        force_full: bool,
    ) -> Result<UpgradeStrategy, Box<dyn std::error::Error>> {
        let current_version = version_config.get_current_version()?;
        let current_version_str = current_version.to_string();
        
        // 强制全量升级
        if force_full {
            return Self::select_full_upgrade_strategy(
                &current_version_str,
                manifest,
                architecture,
            );
        }
        
        // 检查是否需要全量升级（基础版本变化）
        if version_config.needs_full_upgrade(&manifest.version)? {
            return Self::select_full_upgrade_strategy(
                &current_version_str,
                manifest,
                architecture,
            );
        }
        
        // 检查是否可以增量升级
        if let Some(patch_info) = &manifest.patch {
            if version_config.can_apply_patch(&patch_info.version)? {
                return Self::select_patch_upgrade_strategy(
                    &current_version_str,
                    patch_info,
                    architecture,
                );
            }
        }
        
        // 检查是否已经是最新版本
        let server_version = Version::from_str(&manifest.version)?;
        if current_version >= server_version {
            return Ok(UpgradeStrategy::NoUpgrade {
                current_version: current_version_str,
            });
        }
        
        // 回退到传统升级
        Self::select_legacy_upgrade_strategy(&current_version_str, manifest)
    }
    
    fn select_full_upgrade_strategy(
        current_version: &str,
        manifest: &EnhancedServiceManifest,
        architecture: &str,
    ) -> Result<UpgradeStrategy, Box<dyn std::error::Error>> {
        // 优先使用新的平台特定包
        if let Some(platforms) = &manifest.platforms {
            let platform_info = match architecture {
                "x86_64" => platforms.x86_64.as_ref(),
                "aarch64" => platforms.aarch64.as_ref(),
                _ => None,
            };
            
            if let Some(platform_info) = platform_info {
                return Ok(UpgradeStrategy::FullUpgrade {
                    from_version: current_version.to_string(),
                    to_version: manifest.version.clone(),
                    architecture: architecture.to_string(),
                    download_url: platform_info.url.clone(),
                    signature: platform_info.signature.clone(),
                });
            }
        }
        
        // 回退到传统方式
        Self::select_legacy_upgrade_strategy(current_version, manifest)
    }
    
    fn select_patch_upgrade_strategy(
        current_version: &str,
        patch_info: &PatchInfo,
        architecture: &str,
    ) -> Result<UpgradeStrategy, Box<dyn std::error::Error>> {
        let patch_platform_info = match architecture {
            "x86_64" => patch_info.x86_64.as_ref(),
            "aarch64" => patch_info.aarch64.as_ref(),
            _ => None,
        };
        
        if let Some(patch_platform_info) = patch_platform_info {
            Ok(UpgradeStrategy::PatchUpgrade {
                from_version: current_version.to_string(),
                to_version: patch_info.version.clone(),
                architecture: architecture.to_string(),
                download_url: patch_platform_info.url.clone(),
                hash: patch_platform_info.hash.clone(),
                signature: patch_platform_info.signature.clone(),
            })
        } else {
            Err(format!("不支持的架构: {}", architecture).into())
        }
    }
    
    fn select_legacy_upgrade_strategy(
        current_version: &str,
        manifest: &EnhancedServiceManifest,
    ) -> Result<UpgradeStrategy, Box<dyn std::error::Error>> {
        Ok(UpgradeStrategy::LegacyUpgrade {
            from_version: current_version.to_string(),
            to_version: manifest.version.clone(),
            download_url: manifest.packages.full.url.clone(),
        })
    }
}

// 临时结构定义（需要在实际实现中补充）
#[derive(Debug)]
pub struct EnhancedServiceManifest {
    pub version: String,
    pub platforms: Option<PlatformPackages>,
    pub patch: Option<PatchInfo>,
    pub packages: ServicePackages,
}

#[derive(Debug)]
pub struct PlatformPackages {
    pub x86_64: Option<PlatformPackageInfo>,
    pub aarch64: Option<PlatformPackageInfo>,
}

#[derive(Debug)]
pub struct PlatformPackageInfo {
    pub url: String,
    pub signature: String,
}

#[derive(Debug)]
pub struct PatchInfo {
    pub version: String,
    pub x86_64: Option<PatchPackageInfo>,
    pub aarch64: Option<PatchPackageInfo>,
}

#[derive(Debug)]
pub struct PatchPackageInfo {
    pub url: String,
    pub hash: String,
    pub signature: String,
}

#[derive(Debug)]
pub struct ServicePackages {
    pub full: PackageInfo,
}

#[derive(Debug)]
pub struct PackageInfo {
    pub url: String,
}
```

## 🔄 升级命令集成

### CLI 命令更新

```rust
// duck-cli/src/commands/update.rs

use client_core::config::VersionConfig;
use client_core::upgrade_strategy::{UpgradeStrategy, UpgradeStrategyManager};
use client_core::version::Version;

pub async fn run_enhanced_upgrade(
    app: &mut CliApp,
    full: bool,
    force: bool,
    check: bool,
    show_strategy: bool,
) -> Result<()> {
    // 1. 架构检测
    let arch = std::env::consts::ARCH;
    info!("🔍 检测到架构: {}", arch);
    
    // 2. 获取增强的升级清单
    let manifest = app.api_client.get_enhanced_service_manifest().await?;
    
    // 3. 确定升级策略
    let strategy = UpgradeStrategyManager::determine_strategy(
        &app.config.versions,
        &manifest,
        arch,
        full,
    )?;
    
    // 4. 显示策略信息
    info!("📋 升级策略: {}", strategy.get_description());
    
    if show_strategy {
        print_detailed_strategy_info(&strategy, &app.config.versions);
        return Ok(());
    }
    
    if check {
        print_version_check_info(&strategy, &manifest);
        return Ok(());
    }
    
    // 5. 执行升级
    match strategy {
        UpgradeStrategy::NoUpgrade { .. } => {
            info!("✅ 无需升级");
            Ok(())
        }
        UpgradeStrategy::FullUpgrade { to_version, download_url, .. } => {
            execute_full_upgrade(app, &to_version, &download_url).await
        }
        UpgradeStrategy::PatchUpgrade { to_version, download_url, hash, .. } => {
            execute_patch_upgrade(app, &to_version, &download_url, &hash).await
        }
        UpgradeStrategy::LegacyUpgrade { to_version, download_url, .. } => {
            execute_legacy_upgrade(app, &to_version, &download_url).await
        }
    }
}

async fn execute_patch_upgrade(
    app: &mut CliApp,
    to_version: &str,
    download_url: &str,
    hash: &str,
) -> Result<()> {
    info!("🔄 开始执行增量升级...");
    
    // 1. 下载补丁包
    let patch_path = download_patch_package(download_url, hash).await?;
    
    // 2. 应用补丁
    apply_patch_package(&patch_path).await?;
    
    // 3. 更新配置版本
    app.config.versions.apply_patch_version(to_version.to_string())?;
    app.config.save_to_file("config.toml")?;
    
    info!("✅ 增量升级完成: {}", to_version);
    Ok(())
}

fn print_detailed_strategy_info(strategy: &UpgradeStrategy, version_config: &VersionConfig) {
    println!("\n📊 详细升级策略信息:");
    println!("════════════════════════════════");
    println!("{}", version_config.get_version_summary());
    println!("推荐策略: {}", strategy.get_description());
    
    match strategy {
        UpgradeStrategy::PatchUpgrade { .. } => {
            println!("💡 优势: 下载量小，升级速度快");
            println!("⚠️  限制: 仅适用于小版本更新");
        }
        UpgradeStrategy::FullUpgrade { .. } => {
            println!("💡 优势: 完整升级，支持跨版本");
            println!("⚠️  注意: 下载量较大，升级时间较长");
        }
        _ => {}
    }
}
```

## 📝 配置文件更新示例

### 更新后的 config.toml

```toml
[versions]
# 保持现有字段（向后兼容）
docker_service = "0.0.13"

# 新增复合版本号字段
current_version = "0.0.13.2"

# 升级历史
last_full_upgrade = "2025-01-12T10:30:00Z"
last_patch_upgrade = "2025-01-12T15:45:00Z"

[docker]
compose_file = "./docker/docker-compose.yml"

[backup]
storage_dir = "./backups"

[cache]
cache_dir = "./cacheDuckData"
download_dir = "./cacheDuckData/download"

[updates]
check_frequency = "daily"
```

## 🎯 关键优势

### 1. 简洁清晰
- ✅ 只需两个版本字段：`docker_service` 和 `current_version`
- ✅ 版本关系一目了然：`0.0.13` → `0.0.13.2`

### 2. 向后兼容
- ✅ 保持 `docker_service` 字段不变
- ✅ 现有代码无需修改

### 3. 灵活扩展
- ✅ 支持任意数量的补丁级别
- ✅ 易于实现版本比较和升级决策

### 4. 错误防护
- ✅ 版本格式验证
- ✅ 补丁适用性检查
- ✅ 版本回退检测

这个复合版本号方案既保持了简洁性，又完全满足了增量升级的需求！ 