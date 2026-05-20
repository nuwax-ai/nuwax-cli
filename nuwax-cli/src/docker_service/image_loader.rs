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
            .ok_or_else(|| {
                DockerServiceError::ImageLoading(format!("{}", t!("image_loader.invalid_file_name")))
            })?;

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

        // 获取文件大小
        let file_size = std::fs::metadata(&file_path)
            .map_err(|e| DockerServiceError::FileSystem(e.to_string()))?
            .len();

        Ok(Self {
            file_path,
            image_type,
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
                "{}",
                t!("image_loader.images_dir_not_exists", path = self.images_dir.display())
            )));
        }

        let arch_suffix = format!("-{}.tar", self.architecture.as_str());
        let mut images = Vec::new();

        for entry in std::fs::read_dir(&self.images_dir)
            .map_err(|e| DockerServiceError::ImageLoading(e.to_string()))?
        {
            let entry = entry.map_err(|e| DockerServiceError::ImageLoading(e.to_string()))?;
            let path = entry.path();

            if path.is_file()
                && let Some(file_name) = path.file_name().and_then(|n| n.to_str())
                    && file_name.ends_with(&arch_suffix) {
                        match ImageInfo::from_file_path(path.clone(), self.architecture) {
                            Ok(image_info) => {
                                info!("Image file found: {name} ({size})", name = file_name, size = format_file_size(image_info.file_size));
                                images.push(image_info);
                            }
                            Err(e) => {
                                warn!("Failed to parse image file: {name} - {error}", name = file_name, error = e.to_string());
                            }
                        }
                    }
        }

        if images.is_empty() {
            return Err(DockerServiceError::ImageLoading(format!(
                "{}",
                t!(
                    "image_loader.no_arch_images_found",
                    arch = self.architecture.as_str()
                )
            )));
        }

        info!("Found {count} image files for architecture {arch}", count = images.len(), arch = self.architecture.as_str());
        Ok(images)
    }

    /// 加载所有镜像
    pub async fn load_all_images(&self) -> DockerServiceResult<LoadResult> {
        let images = self.scan_architecture_images()?;
        let mut result = LoadResult::new();

info!("Starting to load {count} image files...", count = images.len());

        for (index, image) in images.iter().enumerate() {
            let progress = format!("[{}/{}]", index + 1, images.len());
            let file_name = image
                .file_path
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or("unknown");

            info!("{progress} Loading image: {name} ({size})", progress = progress, name = file_name, size = format_file_size(image.file_size));

            match self.docker_manager.load_image(&image.file_path).await {
                Ok(actual_image_name) => {
                    info!("{progress} ✓ Image loaded successfully: {name} -> {actual}",
                            progress = progress,
                            name = file_name,
                            actual = actual_image_name
                        );
                    result.add_success_with_mapping(file_name.to_string(), actual_image_name);
                }
                Err(e) => {
                    error!("{progress} ✗ Image load failed: {name} - {error}", progress = progress, name = file_name, error = e.to_string());

                    // 立即返回错误，停止后续镜像加载
                    return Err(DockerServiceError::ImageLoading(format!(
                        "{}",
                        t!(
                            "image_loader.load_image_failed_summary",
                            name = file_name,
                            error = e.to_string(),
                            loaded = result.success_count,
                            remaining = images.len() - index
                        )
                    )));
                }
            }
        }

        info!("Image loading completed: success {success}, failed {failed}",
                success = result.success_count,
                failed = result.failure_count
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

        info!("Starting image tag setup...");

        for (file_name, actual_image_name) in image_mappings {
            debug!("Processing image mapping: {file} -> {actual}",
                    file = file_name,
                    actual = actual_image_name
                );

            // 基于实际镜像名称创建目标标签（去除架构后缀）
            let target_tag = self.remove_architecture_suffix(actual_image_name);

            info!("Setting tag: {source} -> {target}",
                    source = actual_image_name,
                    target = target_tag
                );

            // 使用实际的镜像名称设置标签
            match self.tag_image(actual_image_name, &target_tag).await {
                Ok(_) => {
                    info!("✓ Tag set successfully: {target}", target = target_tag);
                    result.add_success(actual_image_name.clone(), target_tag);
                }
                Err(e) => {
                    warn!("✗ Tag set failed: {target} - {error}", target = target_tag, error = e.to_string());
                    result.add_failure(actual_image_name.clone(), target_tag, e.to_string());
                }
            }
        }

        info!("Tag setup completed: success {success}, failed {failed}",
                success = result.success_count,
                failed = result.failure_count
            );
        Ok(result)
    }

    /// 从镜像名称中移除架构后缀
    fn remove_architecture_suffix(&self, image_name: &str) -> String {
        use tracing::debug;

        debug!("Processing image name: {name}", name = image_name);

        // 检查是否有标签中的架构后缀：:latest-arm64, :latest-amd64 等
        if let Some((name_part, tag_part)) = image_name.rsplit_once(':') {
            debug!("Split result - name: {name}, tag: {tag}",
                    name = name_part,
                    tag = tag_part
                );

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
                debug!("Removed architecture suffix from tag: {source} -> {target}",
                        source = image_name,
                        target = result
                    );
                return result;
            }
        }

        // 如果没有找到架构后缀，对于基础镜像（如mysql:8.0）直接返回
        debug!("No architecture suffix found, returning original image name: {name}", name = image_name);
        image_name.to_string()
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
                "{}",
                t!("image_loader.set_image_tag_failed", error = stderr)
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

        debug!("Checking image existence via ducker: {name}", name = image_name);

        // 创建 Docker 连接
        let docker = match new_local_docker_connection(DOCKER_SOCKET_PATH, None).await {
            Ok(d) => d,
            Err(e) => {
warn!("Failed to connect Docker: {error}", error = e.to_string());
                return Ok(false);
            }
        };

        // 获取所有镜像列表
        let images = match DockerImage::list(&docker, false).await {
            Ok(imgs) => imgs,
            Err(e) => {
warn!("Failed to list images: {error}", error = e.to_string());
                return Ok(false);
            }
        };

        // 检查目标镜像是否存在
        let exists = images.iter().any(|img| img.get_full_name() == image_name);

        debug!("Image existence check result {name}: {exists}",
                name = image_name,
                exists = exists
            );
        Ok(exists)
    }

    /// 使用 ducker 列出所有镜像
    pub async fn list_images_with_ducker(&self) -> DockerServiceResult<Vec<String>> {
        use tracing::{debug, warn};

        debug!("Listing images via ducker");

        // 创建 Docker 连接
        let docker = match new_local_docker_connection(DOCKER_SOCKET_PATH, None).await {
            Ok(d) => d,
            Err(e) => {
warn!("Failed to connect Docker: {error}", error = e.to_string());
                return Ok(vec![]);
            }
        };

        // 获取所有镜像列表
        let images = match DockerImage::list(&docker, false).await {
            Ok(imgs) => imgs,
            Err(e) => {
warn!("Failed to list images: {error}", error = e.to_string());
                return Ok(vec![]);
            }
        };

        let image_names: Vec<String> = images.iter().map(|img| img.get_full_name()).collect();

        debug!("Found {count} images", count = image_names.len());
        Ok(image_names)
    }

    /// 基于 ducker 验证镜像后再设置标签
    pub async fn setup_image_tags_with_validation(
        &self,
        image_mappings: &[(String, String)],
    ) -> DockerServiceResult<TagResult> {
        use tracing::{debug, info, warn};

        let mut result = TagResult::new();

        info!("Starting validated image tag setup...");

        for (file_name, actual_image_name) in image_mappings {
            debug!("Processing image mapping: {file} -> {actual}",
                    file = file_name,
                    actual = actual_image_name
                );

            // 使用 ducker 检查源镜像是否存在
            match self.check_image_exists_with_ducker(actual_image_name).await {
                Ok(true) => {
                    debug!("Source image exists: {name}", name = actual_image_name);
                }
                Ok(false) => {
                    warn!("Source image not found, skip tag setup: {source}",
                            source = actual_image_name
                        );
                    result.add_failure(
                        actual_image_name.clone(),
                        t!("image_loader.source_image_not_found").to_string(),
                        t!("image_loader.image_not_found").to_string(),
                    );
                    continue;
                }
                Err(e) => {
                    warn!("Failed to check image existence: {source} - {error}", source = actual_image_name, error = e.to_string());
                    // 继续尝试设置标签，因为可能是 ducker 连接问题
                }
            }

            // 基于实际镜像名称创建目标标签（去除架构后缀）
            let target_tag = self.remove_architecture_suffix(actual_image_name);

            // 如果源镜像和目标标签相同，跳过
            if actual_image_name == &target_tag {
                debug!("Source image and target tag are the same, skip: {name}", name = actual_image_name);
                result.add_success(actual_image_name.clone(), target_tag);
                continue;
            }

            info!("Setting tag: {source} -> {target}",
                    source = actual_image_name,
                    target = target_tag
                );

            // 使用实际的镜像名称设置标签
            match self.tag_image(actual_image_name, &target_tag).await {
                Ok(_) => {
                    info!("✓ Tag set successfully: {target}", target = target_tag);
                    result.add_success(actual_image_name.clone(), target_tag);
                }
                Err(e) => {
                    warn!("✗ Tag set failed: {target} - {error}", target = target_tag, error = e.to_string());
                    result.add_failure(actual_image_name.clone(), target_tag, e.to_string());
                }
            }
        }

        info!("Tag setup completed: success {success}, failed {failed}",
                success = result.success_count,
                failed = result.failure_count
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
    }
}
