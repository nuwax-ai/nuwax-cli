# Duck CLI 升级架构增强开发任务

## 📋 项目概述

基于[升级架构增强设计](./upgrade-architecture-enhancement.md)、[增量版本管理设计](./patch-version-management.md)和[文件操作库分析](./file-operations-library-analysis.md)，实现支持架构特定和增量升级的新升级系统。

### 🎯 核心目标
- ✅ 支持 x86_64 和 aarch64 架构特定的升级包
- ✅ 实现增量升级（patch）功能，减少 60-80% 带宽使用
- ✅ 智能升级策略选择
- ✅ 保持完全向后兼容性

### 📊 预估工作量
**总计**: 6-7 周  
**优先级**: 高  
**复杂度**: 中高  

---

## 🚀 Phase 1: 基础架构 (1-2周)

### Task 1.1: 扩展数据结构定义 ✅
**文件**: `client-core/src/api.rs`  
**工作量**: 2-3天  
**依赖**: 无  

#### 子任务:
- [x] 定义 `EnhancedServiceManifest` 结构体
- [x] 定义 `PlatformPackages` 和 `PlatformPackageInfo` 结构体  
- [x] 定义 `PatchInfo` 和 `PatchPackageInfo` 结构体
- [x] 定义 `PatchOperations` 和 `ReplaceOperations` 结构体
- [x] 添加 JSON 序列化/反序列化支持
- [x] 添加数据验证逻辑

#### 验收标准:
```rust
// 能够成功解析新的JSON格式
let manifest: EnhancedServiceManifest = serde_json::from_str(json_str)?;
assert!(manifest.platforms.is_some());
assert!(manifest.patch.is_some());
```

### Task 1.2: 版本管理系统重构 ✅
**文件**: `client-core/src/version.rs` (新建)  
**工作量**: 2-3天  
**依赖**: 无  

#### 子任务:
- [x] 创建 `Version` 结构体，支持四段式版本号 (major.minor.patch.build)
- [x] 实现版本解析 `from_str()` 方法
- [x] 实现版本比较逻辑 (PartialOrd, Ord)
- [x] 实现 `base_version()` 方法
- [x] 实现 `can_apply_patch()` 方法
- [x] 添加版本格式验证

#### 验收标准:
```rust
let v1 = Version::from_str("0.0.13.5")?;
let v2 = Version::from_str("0.0.13.2")?;
assert!(v1 > v2);
assert!(v1.base_version() == v2.base_version());
assert!(v1.can_apply_patch(&v2));
```

### Task 1.3: 配置文件结构扩展 ✅
**文件**: `client-core/src/config.rs`  
**工作量**: 1-2天  
**依赖**: Task 1.2  

#### 子任务:
- [x] 扩展 `VersionConfig` 结构体
- [x] 添加 `patch_version` 字段
- [x] 添加 `local_patch_level` 字段  
- [x] 添加 `full_version_with_patches` 字段
- [x] 添加 `applied_patches` 历史记录
- [x] 实现 `update_full_version()` 方法
- [x] 实现 `apply_patch()` 方法
- [x] 实现 `get_current_version()` 方法
- [x] 添加配置迁移逻辑（向后兼容）

#### 验收标准:
```rust
let mut config = VersionConfig::new();
config.update_full_version("0.0.14".to_string());
assert_eq!(config.full_version_with_patches, "0.0.14.0");

config.apply_patch("0.0.1".to_string());
assert_eq!(config.full_version_with_patches, "0.0.14.1");
```

### Task 1.4: 架构检测模块 ✅
**文件**: `client-core/src/architecture.rs` (新建)  
**工作量**: 1天  
**依赖**: 无  

#### 子任务:
- [x] 定义 `Architecture` 枚举
- [x] 实现 `detect()` 方法，使用 `std::env::consts::ARCH`
- [x] 实现 `as_str()` 方法
- [x] 实现 `from_str()` 方法
- [x] 添加单元测试

#### 验收标准:
```rust
let arch = Architecture::detect();
assert!(matches!(arch, Architecture::X86_64 | Architecture::Aarch64));
assert_eq!(arch.as_str(), "x86_64"); // 或 "aarch64"
```

### Task 1.5: API 客户端扩展 ✅
**文件**: `client-core/src/api.rs`  
**工作量**: 1-2天  
**依赖**: Task 1.1  

#### 子任务:
- [x] 添加 `get_enhanced_service_manifest()` 方法
- [x] 保持现有 `check_docker_version()` 方法不变（向后兼容）
- [x] 添加错误处理，支持旧格式降级
- [x] 添加超时和重试机制
- [x] 添加单元测试

#### 验收标准:
```rust
let manifest = api_client.get_enhanced_service_manifest().await?;
// 新格式解析成功
assert!(manifest.platforms.is_some());

// 旧格式兼容性
let old_response = api_client.check_docker_version("0.0.12").await?;
assert!(old_response.has_update);
```

---

## 🧠 Phase 2: 升级策略 (1周)

### Task 2.1: 升级策略管理器 ✅
**文件**: `client-core/src/upgrade_strategy.rs` (新建)  
**工作量**: 3-4天  
**依赖**: Task 1.1, 1.2, 1.4  

#### 子任务:
- [x] 定义 `UpgradeStrategy` 枚举
- [x] 创建 `UpgradeStrategyManager` 结构体
- [x] 实现 `determine_strategy()` 方法
- [x] 实现 `select_full_upgrade_strategy()` 方法
- [x] 实现 `select_patch_upgrade_strategy()` 方法
- [x] 实现 `is_patch_applicable()` 方法
- [x] 添加策略决策日志
- [x] 添加单元测试覆盖所有场景

#### 验收标准:
```rust
// 全量升级场景
let strategy = UpgradeStrategyManager::determine_strategy(
    &manifest, "0.0.12", false, Architecture::X86_64
)?;
assert!(matches!(strategy, UpgradeStrategy::FullUpgrade { .. }));

// 增量升级场景
let strategy = UpgradeStrategyManager::determine_strategy(
    &manifest, "0.0.13.0", false, Architecture::X86_64
)?;
assert!(matches!(strategy, UpgradeStrategy::PatchUpgrade { .. }));

// 无需升级场景
let strategy = UpgradeStrategyManager::determine_strategy(
    &manifest, "0.0.13.5", false, Architecture::X86_64
)?;
assert!(matches!(strategy, UpgradeStrategy::NoUpgrade));
```

### Task 2.2: 策略决策逻辑优化 ✅
**文件**: `client-core/src/upgrade_strategy.rs`  
**工作量**: 1-2天  
**依赖**: Task 2.1  

#### 子任务:
- [x] 添加强制升级选项处理
- [x] 添加网络状况考虑（优先patch包）
- [x] 添加磁盘空间检查
- [x] 添加升级风险评估
- [x] 实现策略推荐算法
- [x] 添加性能测试

#### 验收标准:
```rust
// 强制全量升级
let strategy = UpgradeStrategyManager::determine_strategy(
    &manifest, "0.0.13.2", true, Architecture::X86_64
)?;
assert!(matches!(strategy, UpgradeStrategy::FullUpgrade { .. }));

// 磁盘空间不足时的处理
let strategy = UpgradeStrategyManager::determine_strategy_with_constraints(
    &manifest, "0.0.13.0", false, Architecture::X86_64, &constraints
)?;
```

---

## 🔧 Phase 3: 增量升级 (2周)

### Task 3.1: 文件操作库集成 ✅
**文件**: `client-core/Cargo.toml`, `client-core/src/patch_executor/mod.rs` (新建)  
**工作量**: 1天  
**依赖**: 无  

#### 子任务:
- [x] 添加依赖项到 Cargo.toml:
  - `fs_extra = "1.3"`
  - `remove_dir_all = "0.8"`
  - `tempfile = "3.8"`
  - `walkdir = "2.3"`
- [x] 创建 `patch_executor` 模块
- [x] 设置模块结构

#### 验收标准:
```toml
# Cargo.toml 包含必要依赖
[dependencies]
fs_extra = "1.3"
remove_dir_all = "0.8"
tempfile = "3.8"
walkdir = "2.3"
```

### Task 3.2: 文件操作执行器核心 ✅
**文件**: `client-core/src/patch_executor/file_operations.rs` (新建)  
**工作量**: 4-5天  
**依赖**: Task 3.1  

#### 子任务:
- [x] 创建 `FileOperationExecutor` 结构体
- [x] 实现备份系统 `enable_backup()`
- [x] 实现文件替换 `replace_files()`
- [x] 实现目录替换 `replace_directories()`
- [x] 实现删除操作 `delete_items()`
- [x] 实现原子性文件替换 `atomic_file_replace()`
- [x] 实现跨平台目录删除 `safe_remove_directory()`
- [x] 实现回滚功能 `rollback()`
- [x] 添加详细日志记录

#### 验收标准:
```rust
let mut executor = FileOperationExecutor::new(work_dir)?;
executor.enable_backup()?;

// 文件替换
executor.replace_files(&["app/app.jar", "config/app.yml"]).await?;

// 目录替换
executor.replace_directories(&["front/", "plugins/"]).await?;

// 删除操作
executor.delete_items(&["old-files/", "deprecated.conf"]).await?;

// 回滚测试
executor.rollback().await?;
```

### Task 3.3: 补丁包处理器 ✅
**文件**: `client-core/src/patch_executor/patch_processor.rs` (新建)  
**工作量**: 2-3天  
**依赖**: Task 3.2  

#### 子任务:
- [x] 创建 `PatchProcessor` 结构体
- [x] 实现补丁包下载 `download_patch()`
- [x] 实现补丁包解压 `extract_patch()`
- [x] 实现补丁验证 `verify_patch_integrity()`
- [x] 实现数字签名验证 `verify_signature()`
- [x] 添加进度回调支持
- [x] 添加错误恢复机制

#### 验收标准:
```rust
let processor = PatchProcessor::new(temp_dir)?;

// 下载和验证
processor.download_patch(&patch_info).await?;
processor.verify_patch_integrity(&patch_info).await?;

// 解压
let extracted_path = processor.extract_patch().await?;
assert!(extracted_path.exists());
```

### Task 3.4: 主补丁执行器 ✅
**文件**: `client-core/src/patch_executor/mod.rs`  
**工作量**: 2-3天  
**依赖**: Task 3.2, 3.3  

#### 子任务:
- [x] 创建 `PatchExecutor` 主结构体
- [x] 实现 `apply_patch()` 方法，协调整个流程
- [x] 实现进度报告机制
- [x] 实现错误处理和回滚逻辑
- [x] 添加操作日志记录
- [x] 集成文件操作和补丁处理
- [x] 添加集成测试

#### 验收标准:
```rust
let executor = PatchExecutor::new(work_dir)?;

let result = executor.apply_patch(
    &patch_info, 
    &patch_operations,
    |progress| println!("进度: {:.1}%", progress * 100.0)
).await;

assert!(result.is_ok());
// 验证文件已正确更新
// 验证旧文件已备份
```

---

## 🖥️ Phase 4: CLI 集成 (1周)

### Task 4.1: 升级命令重构 ✅
**文件**: `duck-cli/src/cli.rs`  
**工作量**: 1-2天  
**依赖**: Phase 1, 2, 3  

#### 子任务:
- [x] 扩展 `UpgradeArgs` 结构体
- [x] 添加 `--patch` 参数（优先增量升级）
- [x] 添加 `--arch` 参数（指定架构）
- [x] 添加 `--strategy` 参数（显示升级策略）
- [x] 保持现有参数向后兼容
- [x] 添加参数验证逻辑

#### 验收标准:
```bash
# 新功能
duck-cli upgrade --patch          # 优先增量升级
duck-cli upgrade --strategy       # 显示升级策略
duck-cli upgrade --arch aarch64   # 指定架构

# 现有功能保持不变
duck-cli upgrade --full
duck-cli upgrade --check
```

### Task 4.2: 升级流程重构 ✅
**文件**: `duck-cli/src/commands/update.rs`  
**工作量**: 3-4天  
**依赖**: Task 4.1  

#### 子任务:
- [x] 创建 `run_enhanced_upgrade()` 函数
- [x] 集成架构检测
- [x] 集成升级策略管理器
- [x] 实现 `execute_full_upgrade()` 函数
- [x] 实现 `execute_patch_upgrade()` 函数
- [x] 保持 `execute_legacy_upgrade()` 兼容性
- [x] 添加详细的用户反馈和进度显示
- [x] 实现错误处理和回滚

#### 验收标准:
```rust
// 升级流程测试
let result = run_enhanced_upgrade(&mut app, upgrade_args).await;
assert!(result.is_ok());

// 策略显示测试
let args = UpgradeArgs { strategy: true, ..Default::default() };
run_enhanced_upgrade(&mut app, args).await?;
// 应该显示策略信息而不执行升级
```

### Task 4.3: 用户界面优化 ✅
**文件**: `duck-cli/src/commands/update.rs`  
**工作量**: 1-2天  
**依赖**: Task 4.2  

#### 子任务:
- [x] 优化升级进度显示
- [x] 添加预计时间和带宽节省信息
- [x] 实现彩色输出和图标
- [x] 添加确认提示（危险操作）
- [x] 实现详细模式和静默模式
- [x] 添加升级后的验证报告

#### 验收标准:
```bash
# 进度显示示例
🔍 检测架构: x86_64
📥 下载策略: 增量升级 (节省带宽 75%)
⏱️ 预计时间: 2分钟
🔄 应用补丁: ████████░░ 80%
✅ 升级完成: 0.0.13.0 → 0.0.13.2
```

---

## 🧪 Phase 5: 测试和文档 (1周)

### Task 5.1: 单元测试 ✅
**文件**: 各模块的 `tests.rs`  
**工作量**: 2-3天  
**依赖**: Phase 1-4  

#### 子任务:
- [x] `version.rs` 单元测试 - 版本解析和比较
- [x] `architecture.rs` 单元测试 - 架构检测
- [x] `upgrade_strategy.rs` 单元测试 - 策略选择逻辑
- [x] `file_operations.rs` 单元测试 - 文件操作
- [x] `patch_executor.rs` 单元测试 - 补丁应用
- [x] `api.rs` 单元测试 - API 客户端
- [x] 达到 90% 代码覆盖率

#### 验收标准:
```bash
cargo test --package client-core
cargo test --package duck-cli

# 所有测试通过，覆盖率 ≥ 90%
```

### Task 5.2: 集成测试 ✅
**文件**: `duck-cli/tests/integration_upgrade.rs` (新建)  
**工作量**: 2天  
**依赖**: Task 5.1  

#### 子任务:
- [x] 端到端升级流程测试
- [x] 跨架构兼容性测试
- [x] 网络异常处理测试
- [x] 磁盘空间不足测试
- [x] 回滚功能测试
- [x] 向后兼容性测试

#### 验收标准:
```rust
#[tokio::test]
async fn test_end_to_end_patch_upgrade() {
    // 模拟完整的增量升级流程
    // 验证文件正确更新
    // 验证配置正确保存
}

#[tokio::test]
async fn test_rollback_on_failure() {
    // 模拟升级失败场景
    // 验证回滚功能正常工作
}
```

### Task 5.3: 性能测试
**文件**: `duck-cli/benches/upgrade_performance.rs` (新建)  
**工作量**: 1天  
**依赖**: Task 5.2  

#### 子任务:
- [ ] 全量升级 vs 增量升级性能对比
- [ ] 不同文件大小的升级时间测试
- [ ] 内存使用量测试
- [ ] 并发下载性能测试
- [ ] 生成性能报告

#### 验收标准:
```bash
cargo bench

# 验证性能指标:
# - 增量升级时间 < 全量升级时间的 30%
# - 内存使用量 < 200MB
# - 下载带宽节省 > 60%
```

### Task 5.4: 文档更新
**文件**: `README.md`, `CLI_USAGE.md`  
**工作量**: 1天  
**依赖**: Task 5.3  

#### 子任务:
- [ ] 更新 CLI 使用文档
- [ ] 添加新功能说明
- [ ] 更新架构图和示例
- [ ] 添加故障排除指南
- [ ] 创建迁移指南

#### 验收标准:
```markdown
# 新增文档内容
## 增量升级功能
## 架构特定升级
## 升级策略选择
## 故障排除
```

---

## 🔄 验收标准总览

### 🎯 功能验收
- [ ] 支持 x86_64 和 aarch64 架构特定升级
- [ ] 实现增量升级，带宽节省 ≥ 60%
- [ ] 智能升级策略选择
- [ ] 完整的回滚功能
- [ ] 向后兼容性保持 100%

### 📊 性能验收
- [ ] 增量升级时间 ≤ 全量升级时间的 30%
- [ ] 内存使用量 ≤ 200MB
- [ ] 升级成功率 ≥ 99%
- [ ] 回滚成功率 ≥ 99%

### 🧪 质量验收
- [ ] 单元测试覆盖率 ≥ 90%
- [ ] 所有集成测试通过
- [ ] 无内存泄漏
- [ ] 跨平台兼容性验证

### 📚 文档验收
- [ ] API 文档完整
- [ ] 用户指南更新
- [ ] 故障排除文档
- [ ] 代码注释覆盖率 ≥ 80%

---

## 🚨 风险评估

### 高风险任务
1. **Task 3.2** - 文件操作执行器：涉及复杂的跨平台文件操作
2. **Task 4.2** - 升级流程重构：核心业务逻辑，影响面大
3. **Task 5.2** - 集成测试：可能发现架构设计问题

### 风险缓解
- 提前进行原型验证
- 分阶段部署和测试
- 保持详细的回滚方案
- 充分的错误处理和日志记录

### 依赖风险
- 外部 API 格式变更：通过向后兼容设计缓解
- 第三方库版本冲突：选择稳定版本，做好测试

---

**文档版本**: v1.0  
**创建日期**: 2025-01-12  
**预计完成时间**: 2025-03-02  
**负责人**: Duck CLI 开发团队 