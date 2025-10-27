use crate::docker_service::architecture::{Architecture, detect_architecture};
use crate::docker_service::error::{DockerServiceError, DockerServiceResult};
use client_core::constants::docker::DOCKER_SOCKET_PATH;
use client_core::container::DockerManager;
// use client_core::{DuckError, Result};
use ducker::docker::{image::DockerImage, util::new_local_docker_connection};
use std::path::PathBuf;
use std::sync::Arc;
use tracing::{error, info, warn};

/// 镜像类型
#[derive(Debug, Clone, PartialEq)]
pub enum ImageType {
    /// 业务镜像
    Business,
    /// 基础组件镜像
    Infrastructure,
}

/// 镜像信息
#[derive(Debug, Clone)]
pub struct ImageInfo {
    /// 镜像文件路径
    pub file_path: PathBuf,
    /// 镜像类型
    #[allow(dead_code)]
    pub image_type: ImageType,
    /// 原始标签（带架构后缀）
    pub original_tag: String,
    /// 目标标签（去除架构后缀）
    pub target_tag: String,
    /// 镜像架构
    #[allow(dead_code)]
    pub architecture: Architecture,
    /// 文件大小
    pub file_size: u64,
}

impl ImageInfo {
    /// 从文件路径推断镜像信息
    pub fn from_file_path(
        file_path: PathBuf,
        architecture: Architecture,
    ) -> DockerServiceResult<Self> {
        let file_name = file_path
            .file_name()
            .and_then(|n| n.to_str())
            .ok_or_else(|| DockerServiceError::ImageLoading("无效的文件名".to_string()))?;

        // 推断镜像类型
        let image_type = if file_name.contains("mysql")
            || file_name.contains("redis")
            || file_name.contains("nginx")
            || file_name.contains("postgres")
        {
            ImageType::Infrastructure
        } else {
            ImageType::Business
        };

        // 提取原始标签和目标标签
        let arch_suffix = format!("-{}", architecture.as_str());
        let (original_tag, target_tag) =
            if let Some(name_without_ext) = file_name.strip_suffix(".tar") {
                if name_without_ext.ends_with(&arch_suffix) {
                    let target = &name_without_ext[..name_without_ext.len() - arch_suffix.len()];
                    (name_without_ext.to_string(), target.to_string())
                } else {
                    (name_without_ext.to_string(), name_without_ext.to_string())
                }
            } else {
                (file_name.to_string(), file_name.to_string())
            };

        // 获取文件大小
        let file_size = std::fs::metadata(&file_path)
            .map_err(|e| DockerServiceError::FileSystem(e.to_string()))?
            .len();

        Ok(Self {
            file_path,
            image_type,
            original_tag,
            target_tag,
            architecture,
            file_size,
        })
    }
}

/// 镜像加载结果
#[derive(Debug, Clone)]
pub struct LoadResult {
    pub success_count: usize,
    pub failure_count: usize,
    pub loaded_images: Vec<String>,
    pub failed_images: Vec<(String, String)>, // (文件名, 错误信息)
    pub image_mappings: Vec<(String, String)>, // (文件名, 实际镜像名称)
}

impl LoadResult {
    pub fn new() -> Self {
        Self {
            success_count: 0,
            failure_count: 0,
            loaded_images: Vec::new(),
            failed_images: Vec::new(),
            image_mappings: Vec::new(),
        }
    }

    pub fn add_success(&mut self, image_name: String) {
        self.success_count += 1;
        self.loaded_images.push(image_name);
    }

    pub fn add_success_with_mapping(&mut self, file_name: String, actual_image_name: String) {
        self.success_count += 1;
        self.loaded_images.push(file_name.clone());
        self.image_mappings.push((file_name, actual_image_name));
    }

    pub fn add_failure(&mut self, image_name: String, error: String) {
        self.failure_count += 1;
        self.failed_images.push((image_name, error));
    }

    pub fn is_all_successful(&self) -> bool {
        self.failure_count == 0
    }

    pub fn success_count(&self) -> usize {
        self.success_count
    }

    pub fn failure_count(&self) -> usize {
        self.failure_count
    }
}

impl Default for LoadResult {
    fn default() -> Self {
        Self::new()
    }
}

/// 标签设置结果
#[derive(Debug, Clone)]
pub struct TagResult {
    pub success_count: usize,
    pub failure_count: usize,
    pub tagged_images: Vec<(String, String)>, // (原始标签, 目标标签)
    pub failed_tags: Vec<(String, String, String)>, // (原始标签, 目标标签, 错误信息)
}

impl TagResult {
    pub fn new() -> Self {
        Self {
            success_count: 0,
            failure_count: 0,
            tagged_images: Vec::new(),
            failed_tags: Vec::new(),
        }
    }

    pub fn add_success(&mut self, original: String, target: String) {
        self.success_count += 1;
        self.tagged_images.push((original, target));
    }

    pub fn add_failure(&mut self, original: String, target: String, error: String) {
        self.failure_count += 1;
        self.failed_tags.push((original, target, error));
    }

    pub fn is_all_successful(&self) -> bool {
        self.failure_count == 0
    }

    pub fn success_count(&self) -> usize {
        self.success_count
    }

    pub fn failure_count(&self) -> usize {
        self.failure_count
    }
}

impl Default for TagResult {
    fn default() -> Self {
        Self::new()
    }
}

/// 镜像加载器
pub struct ImageLoader {
    docker_manager: Arc<DockerManager>,
    #[allow(dead_code)]
    work_dir: PathBuf,
    architecture: Architecture,
    images_dir: PathBuf,
}

impl ImageLoader {
    /// 创建新的镜像加载器
    pub fn new(docker_manager: Arc<DockerManager>, work_dir: PathBuf) -> DockerServiceResult<Self> {
        let architecture = detect_architecture();
        let images_dir = work_dir.join(client_core::constants::docker::IMAGES_DIR_NAME);

        Ok(Self {
            docker_manager,
            work_dir,
            architecture,
            images_dir,
        })
    }

    /// 扫描并获取当前架构的镜像列表
    pub fn scan_architecture_images(&self) -> DockerServiceResult<Vec<ImageInfo>> {
        if !self.images_dir.exists() {
            return Err(DockerServiceError::ImageLoading(format!(
                "镜像目录不存在: {}",
                self.images_dir.display()
            )));
        }

        let arch_suffix = format!("-{}.tar", self.architecture.as_str());
        let mut images = Vec::new();

        for entry in std::fs::read_dir(&self.images_dir)
            .map_err(|e| DockerServiceError::ImageLoading(e.to_string()))?
        {
            let entry = entry.map_err(|e| DockerServiceError::ImageLoading(e.to_string()))?;
            let path = entry.path();

            if path.is_file() {
                if let Some(file_name) = path.file_name().and_then(|n| n.to_str()) {
                    if file_name.ends_with(&arch_suffix) {
                        match ImageInfo::from_file_path(path.clone(), self.architecture) {
                            Ok(image_info) => {
                                info!(
                                    "发现镜像文件: {} ({})",
                                    file_name,
                                    format_file_size(image_info.file_size)
                                );
                                images.push(image_info);
                            }
                            Err(e) => {
                                warn!("解析镜像文件失败: {} - {}", file_name, e);
                            }
                        }
                    }
                }
            }
        }

        if images.is_empty() {
            return Err(DockerServiceError::ImageLoading(format!(
                "未找到 {} 架构的镜像文件",
                self.architecture.as_str()
            )));
        }

        info!(
            "共发现 {} 个 {} 架构的镜像文件",
            images.len(),
            self.architecture.as_str()
        );
        Ok(images)
    }

    /// 加载所有镜像
    pub async fn load_all_images(&self) -> DockerServiceResult<LoadResult> {
        let images = self.scan_architecture_images()?;
        let mut result = LoadResult::new();

        info!("开始加载 {} 个镜像文件...", images.len());

        for (index, image) in images.iter().enumerate() {
            let progress = format!("[{}/{}]", index + 1, images.len());
            let file_name = image
                .file_path
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or("unknown");

            info!(
                "{} 加载镜像: {} ({})",
                progress,
                file_name,
                format_file_size(image.file_size)
            );

            match self.docker_manager.load_image(&image.file_path).await {
                Ok(actual_image_name) => {
                    info!(
                        "{} ✓ 镜像加载成功: {} -> {}",
                        progress, file_name, actual_image_name
                    );
                    result.add_success_with_mapping(file_name.to_string(), actual_image_name);
                }
                Err(e) => {
                    error!("{} ✗ 镜像加载失败: {} - {}", progress, file_name, e);
                    result.add_failure(file_name.to_string(), e.to_string());
                }
            }
        }

        info!(
            "镜像加载完成: 成功 {}, 失败 {}",
            result.success_count, result.failure_count
        );
        Ok(result)
    }

    /// 基于实际加载的镜像设置标签
    pub async fn setup_image_tags_with_mappings(
        &self,
        image_mappings: &[(String, String)],
    ) -> DockerServiceResult<TagResult> {
        use tracing::{debug, info, warn};

        let mut result = TagResult::new();

        info!("开始设置镜像标签...");

        for (file_name, actual_image_name) in image_mappings {
            debug!("处理镜像映射: {} -> {}", file_name, actual_image_name);

            // 基于实际镜像名称创建目标标签（去除架构后缀）
            let target_tag = self.remove_architecture_suffix(actual_image_name);

            info!("设置标签: {} -> {}", actual_image_name, target_tag);

            // 使用实际的镜像名称设置标签
            match self.tag_image(actual_image_name, &target_tag).await {
                Ok(_) => {
                    info!("✓ 标签设置成功: {}", target_tag);
                    result.add_success(actual_image_name.clone(), target_tag);
                }
                Err(e) => {
                    warn!("✗ 标签设置失败: {} - {}", target_tag, e);
                    result.add_failure(actual_image_name.clone(), target_tag, e.to_string());
                }
            }
        }

        info!(
            "标签设置完成: 成功 {}, 失败 {}",
            result.success_count, result.failure_count
        );
        Ok(result)
    }

    /// 从镜像名称中移除架构后缀
    fn remove_architecture_suffix(&self, image_name: &str) -> String {
        use tracing::debug;

        debug!("处理镜像名称: {}", image_name);

        // 检查是否有标签中的架构后缀：:latest-arm64, :latest-amd64 等
        if let Some((name_part, tag_part)) = image_name.rsplit_once(':') {
            debug!("分离后 - 名称: {}, 标签: {}", name_part, tag_part);

            // 检查标签中是否包含架构后缀
            if tag_part.ends_with("-arm64")
                || tag_part.ends_with("-amd64")
                || tag_part.ends_with("-x86_64")
                || tag_part.ends_with("-aarch64")
            {
                // 移除标签中的架构后缀
                let clean_tag = tag_part
                    .replace("-arm64", "")
                    .replace("-amd64", "")
                    .replace("-x86_64", "")
                    .replace("-aarch64", "");

                let result = format!(
                    "{}:{}",
                    name_part,
                    if clean_tag.is_empty() {
                        "latest"
                    } else {
                        &clean_tag
                    }
                );
                debug!("移除标签中的架构后缀: {} -> {}", image_name, result);
                return result;
            }
        }

        // 如果没有找到架构后缀，对于基础镜像（如mysql:8.0）直接返回
        debug!("未找到架构后缀，返回原镜像名称: {}", image_name);
        image_name.to_string()
    }

    /// 设置镜像标签（传统方法，保持向后兼容）
    pub async fn setup_image_tags(&self) -> DockerServiceResult<TagResult> {
        let images = self.scan_architecture_images()?;
        let mut result = TagResult::new();

        info!("开始设置镜像标签...");

        for image in images {
            if image.original_tag != image.target_tag {
                let original_with_latest =
                    format!("{}:latest-{}", image.target_tag, self.architecture.as_str());
                let target_with_latest = format!("{}:latest", image.target_tag);

                info!(
                    "设置标签: {} -> {}",
                    original_with_latest, target_with_latest
                );

                // 使用 docker tag 命令设置标签
                match self
                    .tag_image(&original_with_latest, &target_with_latest)
                    .await
                {
                    Ok(_) => {
                        info!("✓ 标签设置成功: {}", target_with_latest);
                        result.add_success(original_with_latest, target_with_latest);
                    }
                    Err(e) => {
                        error!("✗ 标签设置失败: {} - {}", target_with_latest, e);
                        result.add_failure(original_with_latest, target_with_latest, e.to_string());
                    }
                }
            }
        }

        info!(
            "标签设置完成: 成功 {}, 失败 {}",
            result.success_count, result.failure_count
        );
        Ok(result)
    }

    /// 为单个镜像设置标签
    async fn tag_image(&self, source_tag: &str, target_tag: &str) -> DockerServiceResult<()> {
        use tokio::process::Command;

        let output = Command::new("docker")
            .args(["tag", source_tag, target_tag])
            .output()
            .await
            .map_err(|e| DockerServiceError::DockerCommand(e.to_string()))?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(DockerServiceError::DockerCommand(format!(
                "设置镜像标签失败: {stderr}"
            )));
        }

        Ok(())
    }

    /// 使用 ducker 检查镜像是否存在
    pub async fn check_image_exists_with_ducker(
        &self,
        image_name: &str,
    ) -> DockerServiceResult<bool> {
        use tracing::{debug, warn};

        debug!("使用 ducker 检查镜像是否存在: {}", image_name);

        // 创建 Docker 连接
        let docker = match new_local_docker_connection(DOCKER_SOCKET_PATH, None).await {
            Ok(d) => d,
            Err(e) => {
                warn!("无法连接到 Docker: {}", e);
                return Ok(false);
            }
        };

        // 获取所有镜像列表
        let images = match DockerImage::list(&docker, false).await {
            Ok(imgs) => imgs,
            Err(e) => {
                warn!("无法获取镜像列表: {}", e);
                return Ok(false);
            }
        };

        // 检查目标镜像是否存在
        let exists = images.iter().any(|img| img.get_full_name() == image_name);

        debug!("镜像 {} 存在检查结果: {}", image_name, exists);
        Ok(exists)
    }

    /// 使用 ducker 列出所有镜像
    pub async fn list_images_with_ducker(&self) -> DockerServiceResult<Vec<String>> {
        use tracing::{debug, warn};

        debug!("使用 ducker 获取镜像列表");

        // 创建 Docker 连接
        let docker = match new_local_docker_connection(DOCKER_SOCKET_PATH, None).await {
            Ok(d) => d,
            Err(e) => {
                warn!("无法连接到 Docker: {}", e);
                return Ok(vec![]);
            }
        };

        // 获取所有镜像列表
        let images = match DockerImage::list(&docker, false).await {
            Ok(imgs) => imgs,
            Err(e) => {
                warn!("无法获取镜像列表: {}", e);
                return Ok(vec![]);
            }
        };

        let image_names: Vec<String> = images.iter().map(|img| img.get_full_name()).collect();

        debug!("找到 {} 个镜像", image_names.len());
        Ok(image_names)
    }

    /// 基于 ducker 验证镜像后再设置标签
    pub async fn setup_image_tags_with_validation(
        &self,
        image_mappings: &[(String, String)],
    ) -> DockerServiceResult<TagResult> {
        use tracing::{debug, info, warn};

        let mut result = TagResult::new();

        info!("开始验证并设置镜像标签...");

        for (file_name, actual_image_name) in image_mappings {
            debug!("处理镜像映射: {} -> {}", file_name, actual_image_name);

            // 使用 ducker 检查源镜像是否存在
            match self.check_image_exists_with_ducker(actual_image_name).await {
                Ok(true) => {
                    debug!("源镜像存在: {}", actual_image_name);
                }
                Ok(false) => {
                    warn!("源镜像不存在，跳过标签设置: {}", actual_image_name);
                    result.add_failure(
                        actual_image_name.clone(),
                        "源镜像不存在".to_string(),
                        "镜像未找到".to_string(),
                    );
                    continue;
                }
                Err(e) => {
                    warn!("检查镜像存在性失败: {} - {}", actual_image_name, e);
                    // 继续尝试设置标签，因为可能是 ducker 连接问题
                }
            }

            // 基于实际镜像名称创建目标标签（去除架构后缀）
            let target_tag = self.remove_architecture_suffix(actual_image_name);

            // 如果源镜像和目标标签相同，跳过
            if actual_image_name == &target_tag {
                debug!("源镜像和目标标签相同，跳过: {}", actual_image_name);
                result.add_success(actual_image_name.clone(), target_tag);
                continue;
            }

            info!("设置标签: {} -> {}", actual_image_name, target_tag);

            // 使用实际的镜像名称设置标签
            match self.tag_image(actual_image_name, &target_tag).await {
                Ok(_) => {
                    info!("✓ 标签设置成功: {}", target_tag);
                    result.add_success(actual_image_name.clone(), target_tag);
                }
                Err(e) => {
                    warn!("✗ 标签设置失败: {} - {}", target_tag, e);
                    result.add_failure(actual_image_name.clone(), target_tag, e.to_string());
                }
            }
        }

        info!(
            "标签设置完成: 成功 {}, 失败 {}",
            result.success_count, result.failure_count
        );
        Ok(result)
    }
}

/// 格式化文件大小显示
fn format_file_size(size: u64) -> String {
    const UNITS: &[&str] = &["B", "KB", "MB", "GB"];
    let mut size = size as f64;
    let mut unit_index = 0;

    while size >= 1024.0 && unit_index < UNITS.len() - 1 {
        size /= 1024.0;
        unit_index += 1;
    }

    if unit_index == 0 {
        format!("{} {}", size as u64, UNITS[unit_index])
    } else {
        format!("{:.1} {}", size, UNITS[unit_index])
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn test_image_info_from_file_path() {
        let temp_dir = tempdir().unwrap();
        let file_path = temp_dir.path().join("agent-platform-front-amd64.tar");
        std::fs::write(&file_path, b"fake image data").unwrap();

        let image_info = ImageInfo::from_file_path(file_path, Architecture::Amd64).unwrap();

        assert_eq!(image_info.image_type, ImageType::Business);
        assert_eq!(image_info.architecture, Architecture::Amd64);
        assert!(image_info.original_tag.contains("agent-platform-front"));
        assert!(image_info.original_tag.contains("amd64"));
        assert!(!image_info.target_tag.contains("amd64"));
    }
}
