// client-core/src/patch_executor/patch_processor.rs
//! 补丁包处理器
//!
//! 负责补丁包的下载、验证、解压等操作

use super::error::{PatchExecutorError, Result};
use crate::api_types::PatchPackageInfo;
use base64;
use flate2::read::GzDecoder;
use reqwest::Client;
use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};
use tar::Archive;
use tempfile::TempDir;
use tokio::fs;
use tokio::io::AsyncWriteExt;
use tracing::{debug, info, warn};

/// 补丁包处理器
pub struct PatchProcessor {
    /// 临时工作目录
    temp_dir: TempDir,
    /// HTTP 客户端
    http_client: Client,
}

impl PatchProcessor {
    /// 创建新的补丁处理器
    pub fn new() -> Result<Self> {
        let temp_dir = TempDir::new()
            .map_err(|e| PatchExecutorError::custom(format!("创建临时目录失败: {e}")))?;

        // 创建带超时的HTTP客户端
        let http_client = Client::builder()
            .timeout(std::time::Duration::from_secs(300)) // 5分钟超时
            .build()
            .map_err(|e| PatchExecutorError::custom(format!("创建HTTP客户端失败: {e}")))?;

        debug!("创建补丁处理器，临时目录: {:?}", temp_dir.path());

        Ok(Self {
            temp_dir,
            http_client,
        })
    }

    /// 下载补丁包
    pub async fn download_patch(&self, patch_info: &PatchPackageInfo) -> Result<PathBuf> {
        info!("开始下载补丁包: {}", patch_info.url);

        let patch_path = self.temp_dir.path().join("patch.tar.gz");

        // 发起HTTP请求
        let response = self
            .http_client
            .get(&patch_info.url)
            .send()
            .await
            .map_err(|e| PatchExecutorError::download_failed(format!("HTTP请求失败: {e}")))?;

        if !response.status().is_success() {
            return Err(PatchExecutorError::download_failed(format!(
                "HTTP状态码错误: {}",
                response.status()
            )));
        }

        // 获取内容长度用于进度显示
        let total_size = response.content_length().unwrap_or(0);
        debug!("补丁包大小: {} 字节", total_size);

        // 创建文件并写入数据
        let mut file = fs::File::create(&patch_path).await?;
        let mut downloaded = 0u64;

        let mut stream = response.bytes_stream();
        use futures_util::StreamExt;

        while let Some(chunk_result) = stream.next().await {
            let chunk = chunk_result
                .map_err(|e| PatchExecutorError::download_failed(format!("下载数据块失败: {e}")))?;

            file.write_all(&chunk).await?;
            downloaded += chunk.len() as u64;

            if total_size > 0 {
                let progress = (downloaded as f64 / total_size as f64) * 100.0;
                debug!("下载进度: {:.1}%", progress);
            }
        }

        file.flush().await?;
        info!("补丁包下载完成: {:?} ({} 字节)", patch_path, downloaded);

        Ok(patch_path)
    }

    /// 验证补丁完整性
    pub async fn verify_patch_integrity(
        &self,
        patch_path: &Path,
        patch_info: &PatchPackageInfo,
    ) -> Result<()> {
        info!("验证补丁完整性: {:?}", patch_path);

        // 1. 验证文件存在
        if !patch_path.exists() {
            return Err(PatchExecutorError::verification_failed("补丁文件不存在"));
        }

        // 2. 验证哈希值
        if let Some(hash) = &patch_info.hash {
            self.verify_hash(patch_path, hash).await?;
        }

        // 3. 验证数字签名
        if let Some(signature) = &patch_info.signature {
            self.verify_signature(patch_path, signature).await?;
        }

        info!("补丁完整性验证通过");
        Ok(())
    }

    /// 验证文件哈希
    async fn verify_hash(&self, file_path: &Path, expected_hash: &str) -> Result<()> {
        debug!("验证文件哈希: {:?}", file_path);

        // 解析期望的哈希值（格式：sha256:hash_value）
        let expected_hash = if expected_hash.starts_with("sha256:") {
            &expected_hash[7..]
        } else {
            expected_hash
        };

        // 计算文件的SHA256哈希
        let file_content = fs::read(file_path).await?;
        let mut hasher = Sha256::new();
        hasher.update(&file_content);
        let actual_hash = format!("{:x}", hasher.finalize());

        // 比较哈希值
        if actual_hash != expected_hash {
            return Err(PatchExecutorError::hash_mismatch(
                expected_hash.to_string(),
                actual_hash,
            ));
        }

        debug!("哈希验证通过: {}", actual_hash);
        Ok(())
    }

    /// 验证数字签名
    async fn verify_signature(&self, _file_path: &Path, signature: &str) -> Result<()> {
        debug!("验证数字签名: {}", signature);

        // TODO: 这里应该实现真正的数字签名验证
        // 目前只做基本的格式检查
        if signature.is_empty() {
            warn!("数字签名为空，跳过验证");
            return Ok(());
        }

        // 基本的base64格式检查
        use base64::{Engine as _, engine::general_purpose};
        if general_purpose::STANDARD.decode(signature).is_err() {
            return Err(PatchExecutorError::signature_verification_failed(
                "签名不是有效的base64格式",
            ));
        }

        // TODO: 实际项目中需要：
        // 1. 解码签名
        // 2. 使用公钥验证签名
        // 3. 验证证书链

        debug!("数字签名验证通过（简化验证）");
        Ok(())
    }

    /// 解压补丁包
    pub async fn extract_patch(&self, patch_path: &Path) -> Result<PathBuf> {
        info!("解压补丁包: {:?}", patch_path);

        let extract_dir = self.temp_dir.path().join("extracted");
        fs::create_dir_all(&extract_dir).await?;

        // 在阻塞任务中执行解压操作
        let patch_path_clone = patch_path.to_owned();
        let extract_dir_clone = extract_dir.clone();

        tokio::task::spawn_blocking(move || {
            Self::extract_tar_gz(&patch_path_clone, &extract_dir_clone)
        })
        .await
        .map_err(|e| PatchExecutorError::extraction_failed(format!("解压任务失败: {e}")))??;

        info!("补丁包解压完成: {:?}", extract_dir);
        Ok(extract_dir)
    }

    /// 解压tar.gz文件
    fn extract_tar_gz(archive_path: &Path, extract_to: &Path) -> Result<()> {
        let file = std::fs::File::open(archive_path)?;
        let decoder = GzDecoder::new(file);
        let mut archive = Archive::new(decoder);

        // 解压所有文件
        for entry_result in archive.entries()? {
            let mut entry = entry_result
                .map_err(|e| PatchExecutorError::extraction_failed(format!("读取条目失败: {e}")))?;

            // 获取文件路径
            let path = entry.path().map_err(|e| {
                PatchExecutorError::extraction_failed(format!("获取文件路径失败: {e}"))
            })?;

            // 将路径转换为PathBuf以避免借用问题
            let path_buf = path.to_path_buf();

            // 安全检查：防止路径遍历攻击
            if path_buf.is_absolute()
                || path_buf
                    .components()
                    .any(|c| c == std::path::Component::ParentDir)
            {
                return Err(PatchExecutorError::extraction_failed(format!(
                    "不安全的文件路径: {path_buf:?}"
                )));
            }

            let extract_path = extract_to.join(&path_buf);

            // 确保父目录存在
            if let Some(parent) = extract_path.parent() {
                std::fs::create_dir_all(parent)?;
            }

            // 解压文件
            entry.unpack(&extract_path).map_err(|e| {
                PatchExecutorError::extraction_failed(format!("解压文件失败 {path_buf:?}: {e}"))
            })?;

            debug!("解压文件: {:?} -> {:?}", path_buf, extract_path);
        }

        Ok(())
    }

    /// 获取临时目录路径
    pub fn temp_dir(&self) -> &Path {
        self.temp_dir.path()
    }

    /// 获取解压目录中的文件列表
    pub async fn list_extracted_files(&self) -> Result<Vec<PathBuf>> {
        let extract_dir = self.temp_dir.path().join("extracted");

        if !extract_dir.exists() {
            return Ok(Vec::new());
        }

        let mut files = Vec::new();
        let mut read_dir = fs::read_dir(&extract_dir).await?;

        while let Some(entry) = read_dir.next_entry().await? {
            let path = entry.path();
            if path.is_file() {
                if let Ok(relative_path) = path.strip_prefix(&extract_dir) {
                    files.push(relative_path.to_owned());
                }
            }
        }

        Ok(files)
    }

    /// 验证解压后的文件结构
    pub async fn validate_extracted_structure(&self, required_files: &[String]) -> Result<()> {
        let extract_dir = self.temp_dir.path().join("extracted");

        for required_file in required_files {
            let file_path = extract_dir.join(required_file);
            if !file_path.exists() {
                return Err(PatchExecutorError::verification_failed(format!(
                    "必需的文件不存在: {required_file}"
                )));
            }
        }

        debug!("解压后文件结构验证通过");
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use tokio::fs;

    #[tokio::test]
    async fn test_patch_processor_creation() {
        let processor = PatchProcessor::new();
        assert!(processor.is_ok());
    }

    #[tokio::test]
    async fn test_temp_dir_access() {
        let processor = PatchProcessor::new().unwrap();
        let temp_path = processor.temp_dir();
        assert!(temp_path.exists());
        assert!(temp_path.is_dir());
    }

    #[tokio::test]
    async fn test_hash_verification() {
        let processor = PatchProcessor::new().unwrap();

        // 创建测试文件
        let test_file = processor.temp_dir().join("test.txt");
        let content = b"hello world";
        fs::write(&test_file, content).await.unwrap();

        // 计算期望的哈希
        let mut hasher = Sha256::new();
        hasher.update(content);
        let expected_hash = format!("sha256:{:x}", hasher.finalize());

        // 验证哈希
        let result = processor.verify_hash(&test_file, &expected_hash).await;
        assert!(result.is_ok());

        // 测试错误的哈希
        let wrong_hash = "sha256:wronghash";
        let result = processor.verify_hash(&test_file, wrong_hash).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_signature_verification() {
        let processor = PatchProcessor::new().unwrap();
        let test_file = processor.temp_dir().join("test.txt");
        fs::write(&test_file, b"test").await.unwrap();

        // 测试有效的base64签名
        use base64::{Engine as _, engine::general_purpose};
        let valid_signature = general_purpose::STANDARD.encode("test signature");
        let result = processor
            .verify_signature(&test_file, &valid_signature)
            .await;
        assert!(result.is_ok());

        // 测试无效的签名
        let invalid_signature = "invalid!@#$%";
        let result = processor
            .verify_signature(&test_file, invalid_signature)
            .await;
        assert!(result.is_err());

        // 测试空签名
        let result = processor.verify_signature(&test_file, "").await;
        assert!(result.is_ok()); // 空签名会被跳过
    }

    #[tokio::test]
    async fn test_tar_gz_extraction() {
        let processor = PatchProcessor::new().unwrap();

        // 创建简单的tar.gz文件用于测试
        let tar_path = processor.temp_dir().join("test.tar.gz");
        let extract_dir = processor.temp_dir().join("extract_test");
        fs::create_dir_all(&extract_dir).await.unwrap();

        // 创建一个简单的tar.gz文件
        create_test_tar_gz(&tar_path).unwrap();

        // 测试解压
        let result = PatchProcessor::extract_tar_gz(&tar_path, &extract_dir);
        assert!(result.is_ok());

        // 验证文件已被解压
        let extracted_file = extract_dir.join("test.txt");
        assert!(extracted_file.exists());
    }

    #[tokio::test]
    async fn test_list_extracted_files() {
        let processor = PatchProcessor::new().unwrap();
        let extract_dir = processor.temp_dir().join("extracted");
        fs::create_dir_all(&extract_dir).await.unwrap();

        // 创建测试文件
        fs::write(extract_dir.join("file1.txt"), "content1")
            .await
            .unwrap();
        fs::write(extract_dir.join("file2.txt"), "content2")
            .await
            .unwrap();

        let files = processor.list_extracted_files().await.unwrap();
        assert_eq!(files.len(), 2);
        assert!(files.iter().any(|f| f.file_name().unwrap() == "file1.txt"));
        assert!(files.iter().any(|f| f.file_name().unwrap() == "file2.txt"));
    }

    #[tokio::test]
    async fn test_validate_extracted_structure() {
        let processor = PatchProcessor::new().unwrap();
        let extract_dir = processor.temp_dir().join("extracted");
        fs::create_dir_all(&extract_dir).await.unwrap();

        // 创建必需的文件
        fs::write(extract_dir.join("required1.txt"), "content")
            .await
            .unwrap();
        fs::write(extract_dir.join("required2.txt"), "content")
            .await
            .unwrap();

        let required_files = vec!["required1.txt".to_string(), "required2.txt".to_string()];
        let result = processor
            .validate_extracted_structure(&required_files)
            .await;
        assert!(result.is_ok());

        // 测试缺失文件
        let missing_files = vec!["missing.txt".to_string()];
        let result = processor.validate_extracted_structure(&missing_files).await;
        assert!(result.is_err());
    }

    // 辅助函数：创建测试用的tar.gz文件
    fn create_test_tar_gz(output_path: &Path) -> std::io::Result<()> {
        use flate2::Compression;
        use flate2::write::GzEncoder;

        let tar_gz = std::fs::File::create(output_path)?;
        let enc = GzEncoder::new(tar_gz, Compression::default());
        let mut tar = tar::Builder::new(enc);

        // 添加一个测试文件
        let mut header = tar::Header::new_gnu();
        header.set_path("test.txt")?;
        header.set_size(12);
        header.set_cksum();

        tar.append(&header, "hello world\n".as_bytes())?;
        tar.finish()?;

        Ok(())
    }
}
