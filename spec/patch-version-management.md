# Duck CLI 增量版本号管理设计

## 🎯 版本号体系设计

### 版本号结构
```
完整版本号 = {主版本}.{次版本}.{修订版本}.{补丁级别}
示例：0.0.13.5
```

- **0.0.13**: 全量版本号（对应全量docker.zip包）
- **5**: 补丁级别（在0.0.13基础上应用的第5个补丁）

### config.toml 扩展

```toml
[versions]
# 基础全量版本
docker_service = "0.0.13"

# 补丁版本管理  
patch_version = "0.0.1"          # 当前服务器patch版本
local_patch_level = 0            # 本地已应用的补丁级别
full_version_with_patches = "0.0.13.0"  # 完整版本号

# 升级历史记录
last_full_upgrade = "2025-01-12T10:30:00Z"
last_patch_upgrade = "2025-01-12T15:45:00Z"

# 应用的补丁历史（可选，用于回滚）
applied_patches = [
    { version = "0.0.1", level = 1, applied_at = "2025-01-12T15:45:00Z" }
]
```

### 版本比较逻辑

```rust
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct Version {
    pub major: u32,
    pub minor: u32, 
    pub patch: u32,
    pub build: u32,  // patch level
}

impl Version {
    pub fn from_str(version_str: &str) -> Result<Self> {
        // 解析 "0.0.13.5" 格式
    }
    
    pub fn base_version(&self) -> Version {
        // 返回不包含patch level的基础版本
        Version {
            major: self.major,
            minor: self.minor, 
            patch: self.patch,
            build: 0,
        }
    }
    
    pub fn can_apply_patch(&self, patch_base_version: &Version) -> bool {
        // 检查是否可以在当前版本上应用补丁
        self.base_version() == patch_base_version.base_version()
    }
}
```

### 升级决策流程

```rust
pub fn determine_upgrade_strategy(
    current_version: &Version,
    server_manifest: &EnhancedServiceManifest,
) -> UpgradeStrategy {
    let server_version = Version::from_str(&server_manifest.version)?;
    
    // 1. 检查是否需要全量升级
    if current_version.base_version() < server_version.base_version() {
        return UpgradeStrategy::FullUpgrade;
    }
    
    // 2. 检查是否可以增量升级
    if let Some(patch_info) = &server_manifest.patch {
        let patch_version = Version::from_str(&patch_info.version)?;
        
        // 检查patch是否适用于当前基础版本
        if current_version.can_apply_patch(&server_version) {
            // 检查patch版本是否比当前版本新
            if patch_version > current_version {
                return UpgradeStrategy::PatchUpgrade;
            }
        }
    }
    
    // 3. 已经是最新版本
    UpgradeStrategy::NoUpgrade
}
```

## 📝 配置管理更新

### AppConfig 结构扩展

```rust
// client-core/src/config.rs
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VersionConfig {
    /// 基础Docker服务版本
    pub docker_service: String,
    
    /// 补丁版本信息
    pub patch_version: String,
    
    /// 本地已应用的补丁级别
    pub local_patch_level: u32,
    
    /// 完整版本号（包含补丁级别）
    pub full_version_with_patches: String,
    
    /// 最后一次全量升级时间
    pub last_full_upgrade: Option<chrono::DateTime<chrono::Utc>>,
    
    /// 最后一次补丁升级时间  
    pub last_patch_upgrade: Option<chrono::DateTime<chrono::Utc>>,
    
    /// 已应用的补丁历史
    pub applied_patches: Vec<AppliedPatch>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AppliedPatch {
    pub version: String,
    pub level: u32,
    pub applied_at: chrono::DateTime<chrono::Utc>,
}

impl VersionConfig {
    /// 更新全量版本
    pub fn update_full_version(&mut self, new_version: String) {
        self.docker_service = new_version.clone();
        self.local_patch_level = 0;  // 重置补丁级别
        self.full_version_with_patches = format!("{}.0", new_version);
        self.last_full_upgrade = Some(chrono::Utc::now());
        self.applied_patches.clear();  // 清空补丁历史
    }
    
    /// 应用补丁
    pub fn apply_patch(&mut self, patch_version: String) {
        self.patch_version = patch_version.clone();
        self.local_patch_level += 1;
        self.full_version_with_patches = format!("{}.{}", 
            self.docker_service, self.local_patch_level);
        self.last_patch_upgrade = Some(chrono::Utc::now());
        
        // 记录补丁历史
        self.applied_patches.push(AppliedPatch {
            version: patch_version,
            level: self.local_patch_level,
            applied_at: chrono::Utc::now(),
        });
    }
    
    /// 获取当前完整版本
    pub fn get_current_version(&self) -> Version {
        Version::from_str(&self.full_version_with_patches)
            .unwrap_or_else(|_| Version::from_str(&self.docker_service).unwrap())
    }
}
``` 