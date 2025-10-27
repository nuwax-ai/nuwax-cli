# 智能 Upload 目录保护功能

## 功能概述

实现了智能的 upload 目录处理逻辑，能够根据不同场景自动选择最佳策略：

- **保护现有数据**：如果 upload 目录已存在，跳过解压以保护用户数据
- **创建新结构**：如果 upload 目录不存在，正常解压以创建目录结构

## 智能逻辑

### 处理策略

```rust
// 检查是否为 upload 目录路径
if is_upload_directory_path(&target_path) {
    // 如果 upload 目录已存在，跳过解压以保护用户数据
    // 如果 upload 目录不存在，正常解压以创建目录结构
    if target_path.exists() {
        info!("🛡️ 保护现有 upload 目录，跳过解压: {}", target_path.display());
        continue;
    } else {
        info!("📁 创建新的 upload 目录结构: {}", target_path.display());
    }
}
```

### 场景分析

| 场景 | 已存在 upload 目录 | 处理方式 | 目的 |
|------|-------------------|----------|------|
| **升级部署** | ✅ 有 | 跳过解压 | 保护用户现有数据 |
| **首次部署** | ❌ 无 | 正常解压 | 创建 upload 目录结构 |
| **修复部署** | ✅ 有 | 跳过解压 | 防止覆盖用户文件 |
| **迁移部署** | ❌ 无 | 正常解压 | 初始化 upload 目录 |

## 技术实现

### 1. 安全删除函数

保留原有的优化删除逻辑，只删除非 upload 内容：

```rust
fn safe_remove_docker_directory(output_dir: &std::path::Path) -> Result<()> {
    // 遍历 docker 目录，删除除了 upload 之外的所有内容
    for entry in std::fs::read_dir(output_dir)? {
        let path = entry.path();
        let file_name = entry.file_name();

        // 跳过 upload 目录
        if file_name == "upload" {
            info!("🛡️ 保留 upload 目录: {}", path.display());
            continue;
        }

        // 删除其他文件或目录
        if path.is_dir() {
            std::fs::remove_dir_all(&path)?;
        } else {
            std::fs::remove_file(&path)?;
        }
    }
    Ok(())
}
```

### 2. 智能解压逻辑

在 FullUpgrade 和 PatchUpgrade 分支中都实现了相同的智能逻辑：

```rust
// 检查是否为 upload 目录路径
if is_upload_directory_path(&dst) {
    // 如果 upload 目录已存在，跳过解压以保护用户数据
    // 如果 upload 目录不存在，正常解压以创建目录结构
    if dst.exists() {
        info!("🛡️ 保护现有 upload 目录，跳过替换: {}", dst.display());
        continue;
    } else {
        info!("📁 创建新的 upload 目录结构: {}", dst.display());
    }
}
```

## 功能优势

### 1. 数据安全
- ✅ **零数据丢失**：现有 upload 数据得到完全保护
- ✅ **无覆盖风险**：绝对不会覆盖用户文件
- ✅ **完整性保证**：目录结构和文件都得到保护

### 2. 智能适应
- ✅ **首次部署**：自动创建 upload 目录结构
- ✅ **升级部署**：智能保护现有数据
- ✅ **灵活配置**：适应不同的使用场景

### 3. 性能优化
- ✅ **最小 I/O**：只删除必要的内容
- ✅ **无备份开销**：避免复制大文件
- ✅ **快速执行**：直接操作，效率高

### 4. 用户体验
- ✅ **透明操作**：用户无需关心底层逻辑
- ✅ **向后兼容**：不影响现有使用方式
- ✅ **清晰日志**：详细的操作记录

## 使用场景

### 1. 生产环境升级
```
现有系统: docker/upload/ (包含用户数据)
升级过程: 跳过 upload 目录，保护用户数据
结果: 用户数据完整保留，其他组件更新
```

### 2. 新系统部署
```
现有系统: docker/ (无 upload 目录)
部署过程: 正常解压，创建 upload 目录结构
结果: 系统包含完整的 upload 目录
```

### 3. 系统修复
```
现有系统: docker/upload/ (用户数据损坏但目录存在)
修复过程: 跳过 upload 目录，防止二次损坏
结果: 保护现有目录结构，其他组件修复
```

### 4. 数据迁移
```
现有系统: docker/ (旧版本，无 upload 目录)
迁移过程: 正常解压，创建新的 upload 目录
结果: 新系统包含完整的 upload 功能
```

## 测试验证

通过完整的测试用例验证了所有场景：

1. **保护现有数据**：已存在的 upload 目录被正确跳过
2. **创建新结构**：不存在的 upload 目录被正确创建
3. **文件级别保护**：upload 目录中的所有文件都得到保护
4. **目录结构完整**：包括子目录的完整结构都得到处理

## 总结

这个智能的 upload 目录保护功能实现了：

- **智能判断**：根据目录存在状态自动选择处理方式
- **数据安全**：确保用户上传数据永不丢失
- **性能优化**：采用高效的直接操作方式
- **用户友好**：透明的工作方式，无需用户干预

是一个既安全又高效的解决方案，完美平衡了数据保护和使用便利性。