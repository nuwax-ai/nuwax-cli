# Duck CLI 增量升级文件操作库选择分析

## 🎯 操作需求分析

基于 `docker-update.json` 中的 operations 结构：

```json
"operations": {
    "replace": {
        "files": ["app/app.jar", "config/application.yml"],
        "directories": ["front/", "plugins/", "templates/"]
    },
    "delete": [
        "front/old-assets/",
        "plugins/deprecated/", 
        "config/old.conf"
    ]
}
```

### 核心功能需求

1. **文件替换**：安全地替换单个文件
2. **目录替换**：递归替换整个目录树
3. **文件删除**：删除指定文件
4. **目录删除**：递归删除目录及其内容
5. **原子性操作**：操作失败时能够回滚
6. **权限处理**：保持文件权限和所有权
7. **错误处理**：详细的错误信息和恢复机制

## 📚 Rust 生态库分析

### 1. fs_extra 📦

**优势**：
- ✅ 专门为扩展文件操作设计
- ✅ 支持目录复制、移动、删除
- ✅ 内置进度回调
- ✅ 跨平台兼容性好
- ✅ 处理复杂的目录操作

```rust
// Cargo.toml
fs_extra = "1.3"

// 使用示例
use fs_extra::dir::{copy, remove, CopyOptions};
use fs_extra::file;

// 目录复制
let options = CopyOptions::new();
fs_extra::dir::copy("source/", "dest/", &options)?;

// 目录删除
fs_extra::dir::remove("target_dir")?;

// 文件操作
fs_extra::file::copy("source.txt", "dest.txt", &options)?;
fs_extra::file::remove("file.txt")?;
```

**劣势**：
- ❌ 不是异步的
- ❌ 原子性支持有限

### 2. walkdir + std::fs 📦

**优势**：
- ✅ 轻量级，标准库为主
- ✅ 精确控制
- ✅ 异步友好（配合tokio::fs）
- ✅ 无额外依赖

```rust
// Cargo.toml  
walkdir = "2.3"

// 使用示例
use walkdir::WalkDir;
use std::fs;

// 递归删除目录
fn remove_dir_recursive(path: &Path) -> Result<()> {
    if path.is_dir() {
        for entry in WalkDir::new(path).contents_first(true) {
            let entry = entry?;
            if entry.file_type().is_dir() {
                fs::remove_dir(entry.path())?;
            } else {
                fs::remove_file(entry.path())?;
            }
        }
    }
    Ok(())
}
```

**劣势**：
- ❌ 需要更多手动实现
- ❌ 错误处理复杂

### 3. remove_dir_all 📦

**优势**：
- ✅ 专门解决 Windows 上的目录删除问题
- ✅ 比 std::fs::remove_dir_all 更可靠
- ✅ 轻量级

```rust
// Cargo.toml
remove_dir_all = "0.8"

// 使用示例
remove_dir_all::remove_dir_all("target_dir")?;
```

### 4. tempfile 📦

**优势**：
- ✅ 原子性操作支持
- ✅ 临时文件管理
- ✅ 自动清理

```rust
// Cargo.toml
tempfile = "3.8"

// 原子性文件替换
use tempfile::NamedTempFile;

fn atomic_replace_file(target: &Path, content: &[u8]) -> Result<()> {
    let temp_file = NamedTempFile::new_in(target.parent().unwrap())?;
    temp_file.as_file().write_all(content)?;
    temp_file.persist(target)?;
    Ok(())
}
```

## 🏆 推荐方案：混合使用

### 依赖选择

```toml
# Cargo.toml
[dependencies]
fs_extra = "1.3"           # 主要文件操作
remove_dir_all = "0.8"     # Windows兼容的目录删除
tempfile = "3.8"           # 原子性操作
walkdir = "2.3"            # 精确的目录遍历
tokio = { version = "1.0", features = ["fs"] }  # 异步文件操作
```

### 实现架构

```rust
// client-core/src/patch_executor/file_operations.rs

use fs_extra::{dir, file, copy_items, CopyOptions};
use remove_dir_all::remove_dir_all;
use tempfile::{TempDir, NamedTempFile};
use walkdir::WalkDir;
use std::path::{Path, PathBuf};
use tokio::fs;
use tracing::{info, warn, error};

/// 文件操作执行器
pub struct FileOperationExecutor {
    /// 工作目录
    work_dir: PathBuf,
    /// 备份目录（用于回滚）
    backup_dir: Option<TempDir>,
}

impl FileOperationExecutor {
    pub fn new(work_dir: PathBuf) -> Result<Self> {
        Ok(Self {
            work_dir,
            backup_dir: None,
        })
    }
    
    /// 启用备份模式（支持回滚）
    pub fn enable_backup(&mut self) -> Result<()> {
        self.backup_dir = Some(TempDir::new()?);
        info!("📦 已启用操作备份模式");
        Ok(())
    }
    
    /// 执行替换操作
    pub async fn replace_files(&self, files: &[String]) -> Result<()> {
        info!("🔄 开始替换 {} 个文件", files.len());
        
        for file_path in files {
            self.replace_single_file(file_path).await?;
        }
        
        info!("✅ 文件替换完成");
        Ok(())
    }
    
    /// 执行目录替换操作
    pub async fn replace_directories(&self, directories: &[String]) -> Result<()> {
        info!("🔄 开始替换 {} 个目录", directories.len());
        
        for dir_path in directories {
            self.replace_single_directory(dir_path).await?;
        }
        
        info!("✅ 目录替换完成");
        Ok(())
    }
    
    /// 执行删除操作
    pub async fn delete_items(&self, items: &[String]) -> Result<()> {
        info!("🗑️ 开始删除 {} 个项目", items.len());
        
        for item_path in items {
            self.delete_single_item(item_path).await?;
        }
        
        info!("✅ 删除操作完成");
        Ok(())
    }
    
    /// 替换单个文件
    async fn replace_single_file(&self, file_path: &str) -> Result<()> {
        let target_path = self.work_dir.join(file_path);
        let source_path = Path::new("patch_extracted").join(file_path);
        
        // 创建备份
        if let Some(backup_dir) = &self.backup_dir {
            if target_path.exists() {
                let backup_path = backup_dir.path().join(file_path);
                if let Some(parent) = backup_path.parent() {
                    fs::create_dir_all(parent).await?;
                }
                fs::copy(&target_path, &backup_path).await?;
            }
        }
        
        // 原子性替换
        self.atomic_file_replace(&source_path, &target_path).await?;
        
        info!("📄 已替换文件: {}", file_path);
        Ok(())
    }
    
    /// 替换单个目录
    async fn replace_single_directory(&self, dir_path: &str) -> Result<()> {
        let target_path = self.work_dir.join(dir_path);
        let source_path = Path::new("patch_extracted").join(dir_path);
        
        // 创建备份
        if let Some(backup_dir) = &self.backup_dir {
            if target_path.exists() {
                let backup_path = backup_dir.path().join(dir_path);
                self.backup_directory(&target_path, &backup_path).await?;
            }
        }
        
        // 删除目标目录
        if target_path.exists() {
            self.safe_remove_directory(&target_path).await?;
        }
        
        // 复制新目录
        self.copy_directory(&source_path, &target_path).await?;
        
        info!("📁 已替换目录: {}", dir_path);
        Ok(())
    }
    
    /// 删除单个项目
    async fn delete_single_item(&self, item_path: &str) -> Result<()> {
        let target_path = self.work_dir.join(item_path);
        
        if !target_path.exists() {
            warn!("⚠️ 删除目标不存在，跳过: {}", item_path);
            return Ok(());
        }
        
        // 创建备份
        if let Some(backup_dir) = &self.backup_dir {
            let backup_path = backup_dir.path().join(item_path);
            if target_path.is_dir() {
                self.backup_directory(&target_path, &backup_path).await?;
            } else {
                if let Some(parent) = backup_path.parent() {
                    fs::create_dir_all(parent).await?;
                }
                fs::copy(&target_path, &backup_path).await?;
            }
        }
        
        // 执行删除
        if target_path.is_dir() {
            self.safe_remove_directory(&target_path).await?;
        } else {
            fs::remove_file(&target_path).await?;
        }
        
        info!("🗑️ 已删除: {}", item_path);
        Ok(())
    }
    
    /// 原子性文件替换
    async fn atomic_file_replace(&self, source: &Path, target: &Path) -> Result<()> {
        // 确保目标目录存在
        if let Some(parent) = target.parent() {
            fs::create_dir_all(parent).await?;
        }
        
        // 使用临时文件实现原子性替换
        let temp_file = NamedTempFile::new_in(target.parent().unwrap())?;
        
        // 复制内容
        let source_content = fs::read(source).await?;
        fs::write(temp_file.path(), source_content).await?;
        
        // 原子性移动
        temp_file.persist(target)?;
        
        Ok(())
    }
    
    /// 安全删除目录（跨平台兼容）
    async fn safe_remove_directory(&self, path: &Path) -> Result<()> {
        tokio::task::spawn_blocking({
            let path = path.to_owned();
            move || remove_dir_all(&path)
        }).await??;
        Ok(())
    }
    
    /// 复制目录
    async fn copy_directory(&self, source: &Path, target: &Path) -> Result<()> {
        tokio::task::spawn_blocking({
            let source = source.to_owned();
            let target = target.to_owned();
            move || {
                let options = CopyOptions::new().overwrite(true);
                dir::copy(&source, &target, &options)?;
                Ok::<(), Box<dyn std::error::Error + Send + Sync>>(())
            }
        }).await??;
        Ok(())
    }
    
    /// 备份目录
    async fn backup_directory(&self, source: &Path, backup: &Path) -> Result<()> {
        if let Some(parent) = backup.parent() {
            fs::create_dir_all(parent).await?;
        }
        self.copy_directory(source, backup).await
    }
    
    /// 回滚操作
    pub async fn rollback(&self) -> Result<()> {
        if let Some(backup_dir) = &self.backup_dir {
            info!("🔄 开始回滚操作...");
            
            // 遍历备份目录，恢复所有文件
            for entry in WalkDir::new(backup_dir.path()) {
                let entry = entry?;
                let backup_path = entry.path();
                let relative_path = backup_path.strip_prefix(backup_dir.path())?;
                let target_path = self.work_dir.join(relative_path);
                
                if backup_path.is_file() {
                    if let Some(parent) = target_path.parent() {
                        fs::create_dir_all(parent).await?;
                    }
                    fs::copy(backup_path, &target_path).await?;
                }
            }
            
            info!("✅ 回滚操作完成");
        } else {
            warn!("⚠️ 未启用备份模式，无法回滚");
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;
    
    #[tokio::test]
    async fn test_file_operations() {
        let temp_dir = TempDir::new().unwrap();
        let mut executor = FileOperationExecutor::new(temp_dir.path().to_owned()).unwrap();
        executor.enable_backup().unwrap();
        
        // 测试文件替换、目录操作等
        // ...
    }
}
```

### 错误处理和日志

```rust
// client-core/src/patch_executor/error.rs

#[derive(Debug, thiserror::Error)]
pub enum FileOperationError {
    #[error("文件操作失败: {0}")]
    IoError(#[from] std::io::Error),
    
    #[error("路径错误: {path}")]
    PathError { path: String },
    
    #[error("权限错误: {path}")]
    PermissionError { path: String },
    
    #[error("原子操作失败: {reason}")]
    AtomicOperationFailed { reason: String },
    
    #[error("回滚失败: {reason}")]
    RollbackFailed { reason: String },
}
```

## 🎯 总结建议

### 最终选择：混合方案

1. **fs_extra**: 主要的文件/目录操作
2. **remove_dir_all**: Windows兼容的目录删除  
3. **tempfile**: 原子性操作和备份管理
4. **walkdir**: 精确的目录遍历控制
5. **tokio::fs**: 异步文件操作

### 优势

✅ **可靠性高**: 多个成熟库组合，覆盖各种边界情况  
✅ **跨平台**: 特别处理Windows文件系统的特殊性  
✅ **原子性**: 支持操作失败时的完整回滚  
✅ **性能好**: 异步操作，不阻塞主线程  
✅ **可观测**: 详细的日志和进度反馈  

### 实现成本

📊 **开发工作量**: 中等（约1-2周）  
📊 **依赖数量**: 4个外部crate，都是轻量级  
📊 **维护成本**: 低，都是成熟稳定的库  

这个方案既利用了成熟库的优势，又保持了足够的控制力和扩展性。比完全自实现更可靠，比单一库更灵活。 