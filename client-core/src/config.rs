use crate::architecture::Architecture;
use crate::constants::{backup, config, docker, updates, version};
use crate::version::Version; // 新增：导入Version类型
use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::cell::RefCell;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Arc};
use toml;

/// 应用配置结构
#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct AppConfig {
    pub versions: VersionConfig,
    pub docker: DockerConfig,
    pub backup: BackupConfig,
    pub cache: CacheConfig,
    pub updates: UpdatesConfig,
}

/// 版本配置结构（支持增量版本管理）
#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct VersionConfig {
    /// 基础Docker服务版本（向后兼容字段）
    pub docker_service: String,

    /// 补丁版本信息
    #[serde(default)]
    pub patch_version: String,

    /// 本地已应用的补丁级别
    #[serde(default)]
    pub local_patch_level: u32,

    /// 完整版本号（包含补丁级别）
    #[serde(default)]
    pub full_version_with_patches: String,

    /// 最后一次全量升级时间
    #[serde(default)]
    pub last_full_upgrade: Option<chrono::DateTime<chrono::Utc>>,

    /// 最后一次补丁升级时间
    #[serde(default)]
    pub last_patch_upgrade: Option<chrono::DateTime<chrono::Utc>>,

    /// 已应用的补丁历史
    #[serde(default)]
    pub applied_patches: Vec<AppliedPatch>,
}

/// 已应用的补丁记录
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AppliedPatch {
    pub version: String,
    pub level: u32,
    pub applied_at: chrono::DateTime<chrono::Utc>,
}

// 为了向后兼容，保留Versions类型别名
pub type Versions = VersionConfig;

impl VersionConfig {
    /// 创建新的版本配置
    pub fn new() -> Self {
        let docker_service = version::version_info::DEFAULT_DOCKER_SERVICE_VERSION.to_string();
        let full_version = format!("{docker_service}.0");

        Self {
            docker_service: docker_service.clone(),
            patch_version: "0.0.0".to_string(),
            local_patch_level: 0,
            full_version_with_patches: full_version,
            last_full_upgrade: None,
            last_patch_upgrade: None,
            applied_patches: Vec::new(),
        }
    }

    /// 更新全量版本
    pub fn update_full_version(&mut self, new_version: String) {
        self.docker_service = new_version.clone();
        self.local_patch_level = 0; // 重置补丁级别
        self.full_version_with_patches = format!("{new_version}.0");
        self.last_full_upgrade = Some(chrono::Utc::now());
        self.applied_patches.clear(); // 清空补丁历史

        tracing::info!("全量版本已更新: {} -> {}", self.docker_service, new_version);
    }

    /// 应用补丁
    pub fn apply_patch(&mut self, patch_version: String) {
        self.patch_version = patch_version.clone();
        self.local_patch_level += 1;
        self.full_version_with_patches =
            format!("{}.{}", self.docker_service, self.local_patch_level);
        self.last_patch_upgrade = Some(chrono::Utc::now());

        // 记录补丁历史
        self.applied_patches.push(AppliedPatch {
            version: patch_version.clone(),
            level: self.local_patch_level,
            applied_at: chrono::Utc::now(),
        });

        tracing::info!(
            "补丁已应用: {} (级别: {})",
            patch_version,
            self.local_patch_level
        );
    }

    /// 获取当前完整版本
    pub fn get_current_version(&self) -> Result<Version> {
        if !self.full_version_with_patches.is_empty() {
            self.full_version_with_patches.parse::<Version>()
        } else {
            // 向后兼容：如果没有完整版本信息，基于docker_service构建
            format!("{}.0", self.docker_service).parse::<Version>()
        }
    }

    /// 检查是否需要版本配置迁移
    pub fn needs_migration(&self) -> bool {
        self.full_version_with_patches.is_empty()
            || (self.local_patch_level == 0 && !self.applied_patches.is_empty())
    }

    /// 执行版本配置迁移
    pub fn migrate(&mut self) -> Result<()> {
        if self.full_version_with_patches.is_empty() {
            // 基于docker_service构建完整版本号
            self.full_version_with_patches =
                format!("{}.{}", self.docker_service, self.local_patch_level);
            tracing::info!(
                "配置迁移: 构建完整版本号 {}",
                self.full_version_with_patches
            );
        }

        // 验证配置的一致性
        self.validate()?;

        Ok(())
    }

    /// 验证版本配置的一致性
    pub fn validate(&self) -> Result<()> {
        // 验证基础版本号格式
        if self.docker_service.is_empty() {
            return Err(anyhow::anyhow!("docker_service不能为空"));
        }

        // 验证完整版本号格式
        if !self.full_version_with_patches.is_empty() {
            let _version = self
                .full_version_with_patches
                .parse::<Version>()
                .map_err(|e| anyhow::anyhow!(format!("无效的完整版本号格式: {e}")))?;
        }

        // 验证补丁级别与历史记录的一致性
        if self.applied_patches.len() != self.local_patch_level as usize {
            tracing::warn!(
                "补丁级别与历史记录不一致: level={}, history_count={}",
                self.local_patch_level,
                self.applied_patches.len()
            );
        }

        Ok(())
    }

    /// 获取补丁应用历史摘要
    pub fn get_patch_summary(&self) -> String {
        if self.applied_patches.is_empty() {
            format!("版本: {} (无补丁)", self.docker_service)
        } else {
            format!(
                "版本: {} (已应用{}个补丁，当前级别: {})",
                self.docker_service,
                self.applied_patches.len(),
                self.local_patch_level
            )
        }
    }

    /// 回滚最后一个补丁
    pub fn rollback_last_patch(&mut self) -> Result<Option<AppliedPatch>> {
        if let Some(last_patch) = self.applied_patches.pop() {
            if self.local_patch_level > 0 {
                self.local_patch_level -= 1;
            }

            self.full_version_with_patches =
                format!("{}.{}", self.docker_service, self.local_patch_level);

            // 更新patch_version为前一个补丁的版本（如果存在）
            if let Some(prev_patch) = self.applied_patches.last() {
                self.patch_version = prev_patch.version.clone();
            } else {
                self.patch_version = "0.0.0".to_string();
            }

            tracing::info!(
                "已回滚补丁: {} (级别: {})",
                last_patch.version,
                last_patch.level
            );
            Ok(Some(last_patch))
        } else {
            Ok(None)
        }
    }
}

impl Default for VersionConfig {
    fn default() -> Self {
        Self::new()
    }
}

/// Docker相关配置
#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct DockerConfig {
    #[serde(default = "default_compose_file_path")]
    pub compose_file: String,
    #[serde(default = "default_env_file_path")]
    pub env_file: String,
}
// 默认值函数, 用于获取默认的环境文件路径
fn default_env_file_path() -> String {
    docker::get_env_file_path_str()
}

fn default_compose_file_path() -> String {
    docker::get_compose_file_path_str()
}

/// 备份相关配置
#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct BackupConfig {
    pub storage_dir: String,
}

/// 缓存相关配置
#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct CacheConfig {
    pub cache_dir: String,
    pub download_dir: String,
}

/// 更新相关配置
#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct UpdatesConfig {
    pub check_frequency: String,
}

impl Default for AppConfig {
    fn default() -> Self {
        Self {
            versions: VersionConfig::new(),
            docker: DockerConfig {
                compose_file: docker::get_compose_file_path_str(),
                env_file: docker::get_env_file_path_str(),
            },
            backup: BackupConfig {
                storage_dir: backup::get_default_storage_dir()
                    .to_string_lossy()
                    .to_string(),
            },
            cache: CacheConfig {
                cache_dir: config::get_default_cache_dir()
                    .to_string_lossy()
                    .to_string(),
                download_dir: config::get_default_download_dir()
                    .to_string_lossy()
                    .to_string(),
            },
            updates: UpdatesConfig {
                check_frequency: updates::DEFAULT_CHECK_FREQUENCY.to_string(),
            },
        }
    }
}

impl AppConfig {
    /// 获取 docker 应用版本配置
    pub fn get_docker_versions(&self) -> String {
        self.versions.docker_service.clone()
    }

    /// 写入 docker 应用版本配置
    pub fn write_docker_versions(&mut self, docker_service: String) {
        self.versions.docker_service = docker_service;
    }

    /// 智能查找并加载配置文件
    /// 按优先级查找：config.toml -> /app/config.toml
    pub fn find_and_load_config() -> Result<Self> {
        let config_files = ["config.toml", "/app/config.toml"];

        for config_file in &config_files {
            if Path::new(config_file).exists() {
                tracing::info!("找到配置文件: {}", config_file);
                return Self::load_from_file(config_file);
            }
        }

        // 如果没找到配置文件，创建默认配置
        tracing::warn!("未找到配置文件，创建默认配置: config.toml");
        let default_config = Self::default();
        default_config.save_to_file("config.toml")?;
        Ok(default_config)
    }

    /// 从指定文件加载配置
    pub fn load_from_file<P: AsRef<Path>>(path: P) -> Result<Self> {
        let content = fs::read_to_string(&path)?;
        let config: AppConfig = toml::from_str(&content)?;

        Ok(config)
    }

    /// 保存配置到文件
    pub fn save_to_file<P: AsRef<Path>>(&self, path: P) -> Result<()> {
        let content = self.to_toml_with_comments();
        fs::write(&path, content)?;
        Ok(())
    }

    /// 生成带注释的TOML配置
    fn to_toml_with_comments(&self) -> String {
        const TEMPLATE: &str = include_str!("../templates/config.toml.template");

        // 将所有路径的反斜杠替换为正斜杠，确保TOML兼容性
        let compose_file = self.docker.compose_file.replace('\\', "/");
        let backup_storage_dir = self.backup.storage_dir.replace('\\', "/");
        let cache_dir = self.cache.cache_dir.replace('\\', "/");
        let download_dir = self.cache.download_dir.replace('\\', "/");

        TEMPLATE
            .replace(
                "{docker_service_version}",
                &self.get_docker_versions()
            )
            .replace("{compose_file}", &compose_file)
            .replace("{backup_storage_dir}", &backup_storage_dir)
            .replace("{cache_dir}", &cache_dir)
            .replace("{download_dir}", &download_dir)
            .replace("{check_frequency}", &self.updates.check_frequency)
    }

    /// 确保缓存目录存在
    pub fn ensure_cache_dirs(&self) -> Result<()> {
        fs::create_dir_all(&self.cache.cache_dir)?;
        fs::create_dir_all(&self.cache.download_dir)?;
        Ok(())
    }

    /// 获取下载目录路径
    pub fn get_download_dir(&self) -> PathBuf {
        PathBuf::from(&self.cache.download_dir)
    }

    /// 获取指定版本的全量下载目录路径
    pub fn get_version_download_dir(&self, version: &str, download_type: &str) -> PathBuf {
        PathBuf::from(&self.cache.download_dir)
            .join(version)
            .join(download_type)
    }

    /// 获取指定版本的全量下载文件路径
    pub fn get_version_download_file_path(
        &self,
        version: &str,
        download_type: &str,
        filename: Option<&str>,
    ) -> PathBuf {
        match filename {
            Some(filename) => self
                .get_version_download_dir(version, download_type)
                .join(filename),
            None => {
                //根据当前系统架构,使用不同的docker全量升级包的文件名
                let docker_file_name = Architecture::detect().get_docker_file_name();

                self.get_version_download_dir(version, download_type)
                    .join(docker_file_name)
            }
        }
    }

    /// 确保指定版本的下载目录存在
    pub fn ensure_version_download_dir(
        &self,
        version: &str,
        download_type: &str,
    ) -> Result<PathBuf> {
        let dir = self.get_version_download_dir(version, download_type);
        fs::create_dir_all(&dir)?;
        Ok(dir)
    }

    /// 获取备份目录路径
    pub fn get_backup_dir(&self) -> PathBuf {
        PathBuf::from(&self.backup.storage_dir)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_version_config_new() {
        let config = VersionConfig::new();

        assert!(!config.docker_service.is_empty());
        assert_eq!(config.patch_version, "0.0.0");
        assert_eq!(config.local_patch_level, 0);
        assert!(config.full_version_with_patches.ends_with(".0"));
        assert!(config.applied_patches.is_empty());
        assert!(config.last_full_upgrade.is_none());
        assert!(config.last_patch_upgrade.is_none());
    }

    #[test]
    fn test_update_full_version() {
        let mut config = VersionConfig::new();
        config.apply_patch("0.0.1".to_string());
        assert_eq!(config.local_patch_level, 1);

        // 更新全量版本应该重置补丁信息
        config.update_full_version("0.0.14".to_string());

        assert_eq!(config.docker_service, "0.0.14");
        assert_eq!(config.local_patch_level, 0);
        assert_eq!(config.full_version_with_patches, "0.0.14.0");
        assert!(config.applied_patches.is_empty());
        assert!(config.last_full_upgrade.is_some());
    }

    #[test]
    fn test_apply_patch() {
        let mut config = VersionConfig::new();
        let initial_service_version = config.docker_service.clone();

        // 应用第一个补丁
        config.apply_patch("patch-0.0.1".to_string());

        assert_eq!(config.patch_version, "patch-0.0.1");
        assert_eq!(config.local_patch_level, 1);
        assert_eq!(
            config.full_version_with_patches,
            format!("{initial_service_version}.1")
        );
        assert_eq!(config.applied_patches.len(), 1);
        assert!(config.last_patch_upgrade.is_some());

        // 应用第二个补丁
        config.apply_patch("patch-0.0.2".to_string());

        assert_eq!(config.patch_version, "patch-0.0.2");
        assert_eq!(config.local_patch_level, 2);
        assert_eq!(
            config.full_version_with_patches,
            format!("{initial_service_version}.2")
        );
        assert_eq!(config.applied_patches.len(), 2);

        // 验证补丁历史记录
        assert_eq!(config.applied_patches[0].version, "patch-0.0.1");
        assert_eq!(config.applied_patches[0].level, 1);
        assert_eq!(config.applied_patches[1].version, "patch-0.0.2");
        assert_eq!(config.applied_patches[1].level, 2);
    }

    #[test]
    fn test_get_current_version() {
        let mut config = VersionConfig::new();

        // 测试基础版本
        let version = config.get_current_version().unwrap();
        assert_eq!(version.build, 0);

        // 应用补丁后测试
        config.apply_patch("patch-0.0.1".to_string());
        let version = config.get_current_version().unwrap();
        assert_eq!(version.build, 1);
    }

    #[test]
    fn test_backward_compatibility() {
        // 测试向后兼容性：旧配置只有docker_service字段
        let old_config = VersionConfig {
            docker_service: "0.0.13".to_string(),
            patch_version: String::new(),
            local_patch_level: 0,
            full_version_with_patches: String::new(),
            last_full_upgrade: None,
            last_patch_upgrade: None,
            applied_patches: Vec::new(),
        };

        assert!(old_config.needs_migration());

        let version = old_config.get_current_version().unwrap();
        assert_eq!(version.to_string(), "0.0.13.0");
    }

    #[test]
    fn test_migration() {
        let mut config = VersionConfig {
            docker_service: "0.0.13".to_string(),
            patch_version: String::new(),
            local_patch_level: 2,
            full_version_with_patches: String::new(),
            last_full_upgrade: None,
            last_patch_upgrade: None,
            applied_patches: Vec::new(),
        };

        assert!(config.needs_migration());

        config.migrate().unwrap();

        assert!(!config.needs_migration());
        assert_eq!(config.full_version_with_patches, "0.0.13.2");
    }

    #[test]
    fn test_validation() {
        let mut config = VersionConfig::new();

        // 有效配置应该通过验证
        assert!(config.validate().is_ok());

        // docker_service为空应该失败
        config.docker_service = String::new();
        assert!(config.validate().is_err());

        // 无效的版本号格式应该失败
        config.docker_service = "0.0.13".to_string();
        config.full_version_with_patches = "invalid.version".to_string();
        assert!(config.validate().is_err());
    }

    #[test]
    fn test_rollback_last_patch() {
        let mut config = VersionConfig::new();

        // 没有补丁时回滚应该返回None
        assert!(config.rollback_last_patch().unwrap().is_none());

        // 应用两个补丁
        config.apply_patch("patch-1".to_string());
        config.apply_patch("patch-2".to_string());
        assert_eq!(config.local_patch_level, 2);
        assert_eq!(config.applied_patches.len(), 2);

        // 回滚最后一个补丁
        let rolled_back = config.rollback_last_patch().unwrap();
        assert!(rolled_back.is_some());
        assert_eq!(rolled_back.unwrap().version, "patch-2");
        assert_eq!(config.local_patch_level, 1);
        assert_eq!(config.applied_patches.len(), 1);
        assert_eq!(config.patch_version, "patch-1");

        // 回滚剩余的补丁
        let rolled_back = config.rollback_last_patch().unwrap();
        assert!(rolled_back.is_some());
        assert_eq!(rolled_back.unwrap().version, "patch-1");
        assert_eq!(config.local_patch_level, 0);
        assert_eq!(config.applied_patches.len(), 0);
        assert_eq!(config.patch_version, "0.0.0");
    }

    #[test]
    fn test_patch_summary() {
        let mut config = VersionConfig::new();

        // 无补丁时的摘要
        let summary = config.get_patch_summary();
        assert!(summary.contains("无补丁"));

        // 有补丁时的摘要
        config.apply_patch("patch-1".to_string());
        config.apply_patch("patch-2".to_string());
        let summary = config.get_patch_summary();
        assert!(summary.contains("已应用2个补丁"));
        assert!(summary.contains("当前级别: 2"));
    }

    #[test]
    fn test_serde_compatibility() {
        // 测试序列化和反序列化
        let mut config = VersionConfig::new();
        config.apply_patch("test-patch".to_string());

        // 序列化
        let serialized = toml::to_string(&config).unwrap();

        // 反序列化
        let deserialized: VersionConfig = toml::from_str(&serialized).unwrap();

        assert_eq!(config.docker_service, deserialized.docker_service);
        assert_eq!(config.patch_version, deserialized.patch_version);
        assert_eq!(config.local_patch_level, deserialized.local_patch_level);
        assert_eq!(
            config.full_version_with_patches,
            deserialized.full_version_with_patches
        );
        assert_eq!(
            config.applied_patches.len(),
            deserialized.applied_patches.len()
        );
    }

    // Task 1.3 验收标准测试
    #[test]
    fn test_task_1_3_acceptance_criteria() {
        // 验收标准：扩展VersionConfig结构体
        let mut config = VersionConfig::new();
        assert!(!config.docker_service.is_empty());
        assert!(config.patch_version.is_empty() || config.patch_version == "0.0.0");

        // 验收标准：update_full_version方法
        config.update_full_version("0.0.14".to_string());
        assert_eq!(config.full_version_with_patches, "0.0.14.0");

        // 验收标准：apply_patch方法
        config.apply_patch("0.0.1".to_string());
        assert_eq!(config.full_version_with_patches, "0.0.14.1");

        // 验收标准：get_current_version方法
        let version = config.get_current_version().unwrap();
        assert_eq!(version.to_string(), "0.0.14.1");

        // 验收标准：配置迁移逻辑（向后兼容）
        let old_config = VersionConfig {
            docker_service: "0.0.13".to_string(),
            patch_version: String::new(),
            local_patch_level: 0,
            full_version_with_patches: String::new(),
            last_full_upgrade: None,
            last_patch_upgrade: None,
            applied_patches: Vec::new(),
        };
        assert!(old_config.needs_migration());

        println!("✅ Task 1.3: 配置文件结构扩展 - 验收标准测试通过");
        println!("   - ✅ VersionConfig结构体扩展完成");
        println!("   - ✅ update_full_version方法正常工作");
        println!("   - ✅ apply_patch方法正常工作");
        println!("   - ✅ get_current_version方法正常工作");
        println!("   - ✅ 配置迁移逻辑（向后兼容）正常工作");
    }
}
