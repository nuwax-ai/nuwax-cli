mod test_helpers;
use test_helpers::ENHANCED_MANIFEST_JSON;

#[cfg(test)]
mod tests {
    use anyhow::Result;
    use client_core::EnhancedServiceManifest;
    use serde_json::from_str;
    use tracing::{error, info};

    use super::*;
    #[test]
    fn test_docker_upgrade() -> Result<(), anyhow::Error> {
        let check_upgrade_json = from_str(ENHANCED_MANIFEST_JSON)?;

        // 有 platforms 字段，按增强格式解析
        match serde_json::from_value::<EnhancedServiceManifest>(check_upgrade_json) {
            Ok(manifest) => {
                info!("📋 成功解析增强服务清单");
                manifest.validate()?; // 进行数据验证
                Ok(())
            }
            Err(e) => {
                error!("💥 应用服务升级解析失败 - 增强格式: {}", e);
                Err(anyhow::anyhow!("应用服务升级解析失败 - 增强格式: {}", e))
            }
        }
    }

    #[test]
    fn test_json_parsing_enhanced_service_manifest() -> Result<(), anyhow::Error> {
        let json_str = from_str(ENHANCED_MANIFEST_JSON)?;

        // 使用serde_json解析JSON字符串
        let manifest: EnhancedServiceManifest = serde_json::from_value(json_str)?;

        // 验证解析结果
        assert_eq!(manifest.version.to_short_string(), "0.0.13");
        assert_eq!(manifest.release_notes, "测试发布说明");
        assert_eq!(manifest.release_date, "2025-07-12T13:49:59Z");

        // 验证packages字段
        assert!(manifest.packages.is_some());
        let full_package = manifest.packages.unwrap().full;
        assert_eq!(
            full_package.url,
            "https://nuwa-packages.oss-rg-china-mainland.aliyuncs.com/docker/20250715141736/docker.zip"
        );
        assert_eq!(full_package.hash, "external");
        assert_eq!(full_package.size, 0);

        // 验证platforms字段
        assert!(manifest.platforms.is_some());

        let x86_64_platform = manifest.platforms.unwrap().x86_64.unwrap();
        assert_eq!(
            x86_64_platform.url,
            "https://nuwa-packages.oss-rg-china-mainland.aliyuncs.com/other/docker-x86_64.zip"
        );

        // 验证patch字段
        assert!(manifest.patch.clone().is_some());
        let x86_64_patch = manifest.patch.clone().unwrap().x86_64.unwrap();
        assert_eq!(
            x86_64_patch.url,
            "https://nuwa-packages.oss-rg-china-mainland.aliyuncs.com/patches/20250712133533/x86_64-patch.tar.gz"
        );
        assert_eq!(x86_64_patch.hash.unwrap(), "sha256:patch_hash_x86_64");

        // 验证operations
        let operations = x86_64_patch.operations;
        assert!(operations.replace.is_some());
        assert!(operations.delete.is_some());

        let replace_ops = operations.replace.unwrap();
        assert!(replace_ops.files.contains(&"app/app.jar".to_string()));
        assert!(replace_ops.directories.contains(&"front/".to_string()));

        let delete_ops = operations.delete.unwrap();

        assert!(delete_ops.files.contains(&"old.jar".to_string()));

        // 验证aarch64 patch
        assert!(manifest.patch.is_some());
        let aarch64_patch = manifest.patch.unwrap().aarch64.unwrap();
        assert_eq!(
            aarch64_patch.notes.unwrap(),
            "patch包目录结构与docker.zip保持一致"
        );

        println!("✅ JSON解析验证成功！所有字段都正确解析");

        Ok(())
    }

    #[test]
    fn test_minimal_json_parsing() -> Result<(), anyhow::Error> {
        // 测试最小化的JSON字符串
        let minimal_json = r#"
        {
            "version": "1.0.0",
            "release_notes": "最小化测试",
            "release_date": "2025-01-01T00:00:00Z",
            "packages": {
                "full": {
                    "url": "https://example.com/package.zip",
                    "hash": "sha256:abc123",
                    "signature": "sig123",
                    "size": 100
                }
            }
        }"#;

        let manifest: EnhancedServiceManifest = serde_json::from_str(minimal_json)?;

        assert_eq!(manifest.version.to_short_string(), "1.0.0");
        assert_eq!(manifest.release_notes, "最小化测试");
        assert!(manifest.platforms.is_none());
        assert!(manifest.patch.is_none());

        println!("✅ 最小化JSON解析验证成功！");

        Ok(())
    }

    #[test]
    fn test_original_json_content() -> Result<(), anyhow::Error> {
        let original_json = from_str(ENHANCED_MANIFEST_JSON)?;

        let manifest: EnhancedServiceManifest = serde_json::from_value(original_json)?;

        // 验证基本字段
        assert_eq!(manifest.version.to_short_string(), "0.0.13");
        assert_eq!(manifest.release_notes, "测试发布说明");
        assert_eq!(manifest.release_date, "2025-07-12T13:49:59Z");

        // 验证packages
        assert!(manifest.packages.is_some());
        let full_package = manifest.packages.unwrap().full;
        assert_eq!(
            full_package.url,
            "https://nuwa-packages.oss-rg-china-mainland.aliyuncs.com/docker/20250715141736/docker.zip"
        );

        // 验证platforms
        assert!(manifest.platforms.is_some());

        // 验证patch
        assert!(manifest.patch.is_some());

        println!("✅ 原始JSON内容解析验证成功！");

        Ok(())
    }
}
