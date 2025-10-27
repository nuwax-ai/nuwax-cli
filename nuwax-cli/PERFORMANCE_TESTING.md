# Duck CLI 性能测试指南

## 概述

Duck CLI 使用 [Criterion.rs](https://github.com/bheisler/criterion.rs) 进行全面的性能基准测试，涵盖升级系统的各个方面。

## 🚀 快速开始

### 运行所有性能测试

```bash
cd duck-cli
cargo bench
```

### 运行特定测试组

```bash
# 升级策略性能对比
cargo bench upgrade_strategy_comparison

# 断点续传性能测试
cargo bench resume_download_performance

# 内存使用量测试
cargo bench memory_usage

# 并发下载性能测试
cargo bench concurrent_downloads

# 网络条件影响测试
cargo bench network_conditions

# 版本策略选择性能测试
cargo bench version_strategy_selection
```

## 📊 测试覆盖范围

### 1. 升级策略性能对比
**测试内容**：
- 全量升级 vs 增量升级
- 不同文件大小的性能影响 (1MB - 1GB)
- 补丁包大小对升级时间的影响

**关键指标**：
- 下载时间
- 内存使用
- 吞吐量 (MB/s)

### 2. 断点续传性能测试
**测试内容**：
- 不同续传点的性能 (10%, 25%, 50%, 75%, 90%)
- 续传效率分析
- 重新下载 vs 续传的性能对比

**关键指标**：
- 续传开销
- 完成时间
- 网络效率

### 3. 内存使用量测试
**测试内容**：
- 不同文件大小的内存消耗
- 内存峰值分析
- 内存泄漏检测

**关键指标**：
- 峰值内存使用
- 内存增长率
- 垃圾回收影响

### 4. 并发下载性能测试
**测试内容**：
- 1-16 个并发下载
- 并发效率分析
- 资源竞争检测

**关键指标**：
- 总体吞吐量
- 单个任务延迟
- CPU 使用率

### 5. 网络条件影响测试
**测试内容**：
- 不同网络环境：千兆光纤、百兆宽带、ADSL、4G、3G、慢速连接
- 网络延迟和带宽对性能的影响
- 适应性分析

**关键指标**：
- 适应性评分
- 超时率
- 重试效率

### 6. 版本策略选择性能测试
**测试内容**：
- 版本解析性能
- 策略选择算法效率
- 决策时间分析

**关键指标**：
- 解析速度
- 决策时间
- 算法复杂度

## 📈 性能报告

### HTML 报告

Criterion 会自动生成详细的 HTML 报告：

```bash
# 报告位置
target/criterion/
├── upgrade_strategy_comparison/
├── resume_download_performance/
├── memory_usage/
├── concurrent_downloads/
├── network_conditions/
└── version_strategy_selection/
```

打开 `target/criterion/index.html` 查看汇总报告。

### 火焰图（性能分析）

启用 `pprof` 特性后，会生成火焰图用于深度性能分析：

```bash
# 生成火焰图
cargo bench --features=pprof

# 查看火焰图
target/criterion/*/profile/flamegraph.svg
```

## 🔧 配置选项

### 修改测试参数

编辑 `benches/upgrade_performance.rs` 中的 `PerformanceTestConfig`：

```rust
pub struct PerformanceTestConfig {
    /// 测试文件大小（字节）
    pub file_sizes: Vec<u64>,
    /// 并发下载数量
    pub concurrent_downloads: Vec<usize>,
    /// 模拟网络延迟（毫秒）
    pub network_latency_ms: u64,
    /// 模拟网络带宽（MB/s）
    pub network_bandwidth_mbps: f64,
}
```

### 自定义网络条件

在 `bench_network_conditions` 函数中添加新的网络环境：

```rust
let network_conditions = vec![
    ("custom_network", 延迟ms, 带宽mbps),
    // ... 其他配置
];
```

## 📝 性能基准

### 预期性能指标

| 测试场景 | 目标性能 | 备注 |
|---------|---------|------|
| 全量升级 (100MB) | < 120秒 | 50Mbps 网络 |
| 增量升级 (10MB) | < 15秒 | 50Mbps 网络 |
| 断点续传开销 | < 5% | 相对于重新下载 |
| 内存使用 | < 200MB | 峰值内存 |
| 并发效率 | > 80% | 4并发时的效率 |
| 版本解析 | < 10ms | 单次解析时间 |

### 性能回归检测

Criterion 会自动检测性能回归：

- **改进** ✅：性能提升 > 5%
- **稳定** ➖：性能变化 < 5%
- **回归** ❌：性能下降 > 5%

## 🛠️ 故障排除

### 常见问题

1. **编译错误**
   ```bash
   # 更新依赖
   cargo update
   
   # 清理重建
   cargo clean && cargo build
   ```

2. **内存不足**
   ```bash
   # 减少测试文件大小
   # 编辑 file_sizes 配置
   ```

3. **测试超时**
   ```bash
   # 增加测试时间
   # 编辑 measurement_time 配置
   ```

### 调试模式

运行详细的调试测试：

```bash
# 启用详细输出
RUST_LOG=debug cargo bench

# 单个测试调试
cargo bench upgrade_strategy_comparison -- --verbose
```

## 🎯 最佳实践

### 测试环境准备

1. **稳定的测试环境**
   - 关闭其他网络密集型程序
   - 确保系统负载较低
   - 使用固定的硬件配置

2. **基准数据收集**
   - 至少运行 3 次测试
   - 记录环境变量（CPU、内存、网络）
   - 使用版本控制跟踪性能变化

3. **持续监控**
   - 集成到 CI/CD 流水线
   - 设置性能阈值告警
   - 定期更新基准数据

### 性能优化流程

1. **识别瓶颈**
   ```bash
   cargo bench  # 获取基准
   # 分析火焰图和报告
   ```

2. **实施优化**
   ```bash
   # 修改代码
   cargo bench  # 验证改进
   ```

3. **验证结果**
   ```bash
   # 对比前后性能数据
   # 确保没有性能回归
   ```

## 📊 CI/CD 集成

### GitHub Actions 示例

```yaml
name: Performance Tests

on:
  pull_request:
    branches: [main]
  schedule:
    - cron: '0 2 * * *'  # 每日凌晨2点

jobs:
  performance:
    runs-on: ubuntu-latest
    
    steps:
    - uses: actions/checkout@v4
    
    - name: Install Rust
      uses: actions-rs/toolchain@v1
      with:
        toolchain: stable
        profile: minimal
        override: true
    
    - name: Run benchmarks
      run: |
        cd duck-cli
        cargo bench --message-format=json > bench-results.json
    
    - name: Upload results
      uses: actions/upload-artifact@v3
      with:
        name: benchmark-results
        path: duck-cli/target/criterion/
```

## 🔗 相关资源

- [Criterion.rs 官方文档](https://bheisler.github.io/criterion.rs/book/)
- [Rust 性能优化指南](https://nnethercote.github.io/perf-book/)
- [火焰图分析教程](https://www.brendangregg.com/flamegraphs.html)
- [内存分析工具](https://github.com/koute/memory-profiler) 