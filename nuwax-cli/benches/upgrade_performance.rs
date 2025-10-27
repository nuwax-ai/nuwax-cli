use criterion::{
    BenchmarkId, Criterion, SamplingMode, Throughput, criterion_group, criterion_main,
};
use memory_stats::memory_stats;
use pprof::criterion::{Output, PProfProfiler};
use std::time::Duration;
use tempfile::NamedTempFile;
use tokio::runtime::Runtime;

/// 性能测试配置
#[derive(Debug, Clone)]
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

impl Default for PerformanceTestConfig {
    fn default() -> Self {
        Self {
            file_sizes: vec![
                1024 * 1024,        // 1MB
                10 * 1024 * 1024,   // 10MB
                50 * 1024 * 1024,   // 50MB
                100 * 1024 * 1024,  // 100MB
                500 * 1024 * 1024,  // 500MB
                1024 * 1024 * 1024, // 1GB
            ],
            concurrent_downloads: vec![1, 2, 4, 8, 16],
            network_latency_ms: 100,
            network_bandwidth_mbps: 50.0,
        }
    }
}

/// 模拟升级场景
#[derive(Debug, Clone)]
pub enum UpgradeScenario {
    /// 全量升级（完整下载）
    FullUpgrade {
        version_from: String,
        version_to: String,
        package_size: u64,
    },
    /// 增量升级（补丁下载）
    PatchUpgrade {
        version_from: String,
        version_to: String,
        patch_size: u64,
        base_size: u64,
    },
    /// 断点续传升级
    ResumeUpgrade {
        version_from: String,
        version_to: String,
        total_size: u64,
        downloaded_size: u64,
    },
}

impl UpgradeScenario {
    fn describe(&self) -> String {
        match self {
            UpgradeScenario::FullUpgrade {
                version_from,
                version_to,
                package_size,
            } => format!(
                "全量升级: {} → {} ({:.1}MB)",
                version_from,
                version_to,
                *package_size as f64 / 1024.0 / 1024.0
            ),
            UpgradeScenario::PatchUpgrade {
                version_from,
                version_to,
                patch_size,
                base_size,
            } => format!(
                "增量升级: {} → {} (补丁:{:.1}MB, 基础:{:.1}MB)",
                version_from,
                version_to,
                *patch_size as f64 / 1024.0 / 1024.0,
                *base_size as f64 / 1024.0 / 1024.0
            ),
            UpgradeScenario::ResumeUpgrade {
                version_from,
                version_to,
                total_size,
                downloaded_size,
            } => format!(
                "断点续传: {} → {} (已下载:{:.1}MB/{:.1}MB)",
                version_from,
                version_to,
                *downloaded_size as f64 / 1024.0 / 1024.0,
                *total_size as f64 / 1024.0 / 1024.0
            ),
        }
    }

    fn get_effective_size(&self) -> u64 {
        match self {
            UpgradeScenario::FullUpgrade { package_size, .. } => *package_size,
            UpgradeScenario::PatchUpgrade { patch_size, .. } => *patch_size,
            UpgradeScenario::ResumeUpgrade {
                total_size,
                downloaded_size,
                ..
            } => total_size - downloaded_size,
        }
    }
}

/// 性能测试结果
#[derive(Debug)]
pub struct PerformanceResult {
    pub scenario: UpgradeScenario,
    pub duration: Duration,
    pub memory_used: Option<usize>,
    pub throughput_mbps: f64,
    pub cpu_usage_percent: f64,
}

/// 创建测试用的模拟文件
async fn create_test_file(size: u64) -> Result<NamedTempFile, std::io::Error> {
    let file = NamedTempFile::new()?;

    // 创建指定大小的随机数据
    let chunk_size = 8192; // 8KB chunks
    let mut remaining = size;

    while remaining > 0 {
        let current_chunk_size = std::cmp::min(chunk_size, remaining) as usize;
        let data = vec![0u8; current_chunk_size];

        tokio::io::AsyncWriteExt::write_all(&mut tokio::fs::File::from_std(file.reopen()?), &data)
            .await?;
        remaining -= current_chunk_size as u64;
    }

    Ok(file)
}

/// 模拟下载过程
async fn simulate_download(
    scenario: &UpgradeScenario,
    network_latency_ms: u64,
    bandwidth_mbps: f64,
) -> PerformanceResult {
    let start_time = std::time::Instant::now();
    let start_memory = memory_stats().map(|stats| stats.physical_mem);

    let effective_size = scenario.get_effective_size();

    // 模拟网络延迟
    tokio::time::sleep(Duration::from_millis(network_latency_ms)).await;

    // 模拟带宽限制下的下载时间
    let download_time_secs = (effective_size as f64 / 1024.0 / 1024.0) / bandwidth_mbps;
    let download_duration = Duration::from_secs_f64(download_time_secs);

    // 模拟下载进度
    let chunks = 100;
    let chunk_duration = download_duration / chunks;

    for _ in 0..chunks {
        tokio::time::sleep(chunk_duration).await;
        // 模拟一些CPU工作（解压、验证等）
        let _dummy_work: u64 = (0..1000).sum();
    }

    let total_duration = start_time.elapsed();
    let end_memory = memory_stats().map(|stats| stats.physical_mem);

    let memory_used = match (start_memory, end_memory) {
        (Some(start), Some(end)) => Some(end.saturating_sub(start)),
        _ => None,
    };

    let throughput_mbps = (effective_size as f64 / 1024.0 / 1024.0) / total_duration.as_secs_f64();

    PerformanceResult {
        scenario: scenario.clone(),
        duration: total_duration,
        memory_used,
        throughput_mbps,
        cpu_usage_percent: 0.0, // 简化版本，实际可以使用系统API获取
    }
}

/// 全量升级 vs 增量升级性能对比基准测试
fn bench_upgrade_strategy_comparison(c: &mut Criterion) {
    let runtime = Runtime::new().unwrap();
    let mut group = c.benchmark_group("upgrade_strategy_comparison");

    // 配置采样模式和测量时间
    group.sampling_mode(SamplingMode::Linear);
    group.measurement_time(Duration::from_secs(10));
    group.sample_size(20);

    let test_config = PerformanceTestConfig::default();

    // 测试不同文件大小的升级策略性能
    for &base_size in &test_config.file_sizes {
        let patch_size = base_size / 10; // 补丁通常是基础包大小的10%

        // 全量升级场景
        let full_scenario = UpgradeScenario::FullUpgrade {
            version_from: "0.0.13.0".to_string(),
            version_to: "0.0.14.0".to_string(),
            package_size: base_size,
        };

        // 增量升级场景
        let patch_scenario = UpgradeScenario::PatchUpgrade {
            version_from: "0.0.13.0".to_string(),
            version_to: "0.0.13.1".to_string(),
            patch_size,
            base_size,
        };

        group.throughput(Throughput::Bytes(base_size));

        // 基准测试全量升级
        group.bench_with_input(
            BenchmarkId::new("full_upgrade", format!("{}MB", base_size / 1024 / 1024)),
            &full_scenario,
            |b, scenario| {
                b.to_async(&runtime).iter(|| async {
                    simulate_download(
                        scenario,
                        test_config.network_latency_ms,
                        test_config.network_bandwidth_mbps,
                    )
                    .await
                });
            },
        );

        // 基准测试增量升级
        group.throughput(Throughput::Bytes(patch_size));
        group.bench_with_input(
            BenchmarkId::new("patch_upgrade", format!("{}MB", base_size / 1024 / 1024)),
            &patch_scenario,
            |b, scenario| {
                b.to_async(&runtime).iter(|| async {
                    simulate_download(
                        scenario,
                        test_config.network_latency_ms,
                        test_config.network_bandwidth_mbps,
                    )
                    .await
                });
            },
        );
    }

    group.finish();
}

/// 断点续传性能测试
fn bench_resume_download_performance(c: &mut Criterion) {
    let runtime = Runtime::new().unwrap();
    let mut group = c.benchmark_group("resume_download_performance");

    group.sampling_mode(SamplingMode::Linear);
    group.measurement_time(Duration::from_secs(10));

    let test_config = PerformanceTestConfig::default();
    let total_size = 100 * 1024 * 1024; // 100MB

    // 测试不同续传点的性能
    let resume_percentages = vec![10, 25, 50, 75, 90]; // 已下载的百分比

    for percentage in resume_percentages {
        let downloaded_size = total_size * percentage / 100;

        let resume_scenario = UpgradeScenario::ResumeUpgrade {
            version_from: "0.0.13.0".to_string(),
            version_to: "0.0.13.1".to_string(),
            total_size,
            downloaded_size,
        };

        group.throughput(Throughput::Bytes(total_size - downloaded_size));

        group.bench_with_input(
            BenchmarkId::new("resume_download", format!("{percentage}%_completed")),
            &resume_scenario,
            |b, scenario| {
                b.to_async(&runtime).iter(|| async {
                    simulate_download(
                        scenario,
                        test_config.network_latency_ms,
                        test_config.network_bandwidth_mbps,
                    )
                    .await
                });
            },
        );
    }

    group.finish();
}

/// 内存使用量基准测试
fn bench_memory_usage(c: &mut Criterion) {
    let runtime = Runtime::new().unwrap();
    let mut group = c.benchmark_group("memory_usage");

    group.sampling_mode(SamplingMode::Linear);
    group.measurement_time(Duration::from_secs(15));

    let test_config = PerformanceTestConfig::default();

    for &file_size in &test_config.file_sizes {
        let scenario = UpgradeScenario::FullUpgrade {
            version_from: "0.0.13.0".to_string(),
            version_to: "0.0.14.0".to_string(),
            package_size: file_size,
        };

        group.bench_with_input(
            BenchmarkId::new(
                "memory_consumption",
                format!("{}MB", file_size / 1024 / 1024),
            ),
            &scenario,
            |b, scenario| {
                b.to_async(&runtime).iter(|| async {
                    // 记录内存使用情况
                    let initial_memory =
                        memory_stats().map(|stats| stats.physical_mem).unwrap_or(0);

                    let result = simulate_download(
                        scenario,
                        test_config.network_latency_ms,
                        test_config.network_bandwidth_mbps,
                    )
                    .await;

                    let final_memory = memory_stats().map(|stats| stats.physical_mem).unwrap_or(0);
                    let memory_delta = final_memory.saturating_sub(initial_memory);

                    // 在详细模式下记录内存使用情况
                    if memory_delta > 0 {
                        eprintln!("内存使用: {:.2} MB", memory_delta as f64 / 1024.0 / 1024.0);
                    }

                    result
                });
            },
        );
    }

    group.finish();
}

/// 并发下载性能测试
fn bench_concurrent_downloads(c: &mut Criterion) {
    let runtime = Runtime::new().unwrap();
    let mut group = c.benchmark_group("concurrent_downloads");

    group.sampling_mode(SamplingMode::Linear);
    group.measurement_time(Duration::from_secs(20));

    let test_config = PerformanceTestConfig::default();
    let file_size = 50 * 1024 * 1024; // 50MB per file

    for &concurrent_count in &test_config.concurrent_downloads {
        let scenarios: Vec<_> = (0..concurrent_count)
            .map(|i| UpgradeScenario::FullUpgrade {
                version_from: format!("0.0.{}.0", 13 + i),
                version_to: format!("0.0.{}.0", 14 + i),
                package_size: file_size,
            })
            .collect();

        group.throughput(Throughput::Bytes(file_size * concurrent_count as u64));

        group.bench_with_input(
            BenchmarkId::new("concurrent_downloads", concurrent_count),
            &scenarios,
            |b, scenarios| {
                b.to_async(&runtime).iter(|| async {
                    // 并发执行多个下载任务
                    let mut futures = Vec::new();
                    for scenario in scenarios {
                        let future = simulate_download(
                            scenario,
                            test_config.network_latency_ms / concurrent_count as u64,
                            test_config.network_bandwidth_mbps,
                        );
                        futures.push(future);
                    }

                    // 等待所有下载完成

                    futures::future::join_all(futures).await
                });
            },
        );
    }

    group.finish();
}

/// 网络条件影响测试
fn bench_network_conditions(c: &mut Criterion) {
    let runtime = Runtime::new().unwrap();
    let mut group = c.benchmark_group("network_conditions");

    group.sampling_mode(SamplingMode::Linear);
    group.measurement_time(Duration::from_secs(10));

    let file_size = 50 * 1024 * 1024; // 50MB
    let scenario = UpgradeScenario::FullUpgrade {
        version_from: "0.0.13.0".to_string(),
        version_to: "0.0.14.0".to_string(),
        package_size: file_size,
    };

    // 测试不同网络条件
    let network_conditions = vec![
        ("fiber_1gbps", 0, 1000.0),      // 千兆光纤
        ("broadband_100mbps", 5, 100.0), // 百兆宽带
        ("adsl_20mbps", 20, 20.0),       // ADSL
        ("mobile_4g", 50, 10.0),         // 4G移动网络
        ("mobile_3g", 100, 2.0),         // 3G移动网络
        ("slow_connection", 200, 0.5),   // 慢速连接
    ];

    for (name, latency_ms, bandwidth_mbps) in network_conditions {
        group.throughput(Throughput::Bytes(file_size));

        group.bench_with_input(
            BenchmarkId::new("network_condition", name),
            &(latency_ms, bandwidth_mbps),
            |b, &(latency, bandwidth)| {
                b.to_async(&runtime)
                    .iter(|| async { simulate_download(&scenario, latency, bandwidth).await });
            },
        );
    }

    group.finish();
}

/// 版本检查和策略选择性能测试
fn bench_version_strategy_selection(c: &mut Criterion) {
    let runtime = Runtime::new().unwrap();
    let mut group = c.benchmark_group("version_strategy_selection");

    group.sampling_mode(SamplingMode::Linear);
    group.measurement_time(Duration::from_secs(5));

    // 模拟版本比较和策略选择
    group.bench_function("version_comparison", |b| {
        b.to_async(&runtime).iter(|| async {
            // 模拟版本解析和比较
            let current_version = "0.0.13.5";
            let server_version = "0.0.14.0";
            let patch_version = "0.0.13.6";

            // 模拟解析时间
            tokio::time::sleep(Duration::from_micros(100)).await;

            // 模拟策略选择逻辑
            let needs_full_upgrade = server_version > current_version;
            let can_patch_upgrade = patch_version > current_version;

            (needs_full_upgrade, can_patch_upgrade)
        });
    });

    group.finish();
}

// 创建 Criterion 配置，启用内存分析
fn create_criterion() -> Criterion {
    Criterion::default()
        .with_profiler(PProfProfiler::new(100, Output::Flamegraph(None)))
        .warm_up_time(Duration::from_secs(3))
        .measurement_time(Duration::from_secs(10))
        .sample_size(50)
}

// 定义基准测试组
criterion_group! {
    name = upgrade_benches;
    config = create_criterion();
    targets =
        bench_upgrade_strategy_comparison,
        bench_resume_download_performance,
        bench_memory_usage,
        bench_concurrent_downloads,
        bench_network_conditions,
        bench_version_strategy_selection
}

criterion_main!(upgrade_benches);
