/// Load and Performance Testing Suite
/// Measures throughput, latency, and resource utilization under various loads

mod integration_test_harness;

use integration_test_harness::*;
use std::sync::Arc;
use std::time::Instant;
use anyhow::Result;

/// Load test metrics
#[derive(Debug, Clone)]
pub struct LoadMetrics {
    pub total_requests: u32,
    pub successful_requests: u32,
    pub failed_requests: u32,
    pub total_duration_secs: f64,
    pub throughput_rps: f64,
    pub avg_latency_ms: f64,
    pub min_latency_ms: u32,
    pub max_latency_ms: u32,
    pub p95_latency_ms: u32,
    pub p99_latency_ms: u32,
}

impl LoadMetrics {
    pub fn report(&self) {
        println!("╔════════════════════════════════════════╗");
        println!("║     LOAD TEST RESULTS                  ║");
        println!("╠════════════════════════════════════════╣");
        println!("║ Total Requests:    {:>20} ║", self.total_requests);
        println!("║ Successful:        {:>20} ║", self.successful_requests);
        println!("║ Failed:            {:>20} ║", self.failed_requests);
        println!("║ Duration (sec):    {:>20.2} ║", self.total_duration_secs);
        println!("├────────────────────────────────────────┤");
        println!("║ Throughput (RPS):  {:>20.2} ║", self.throughput_rps);
        println!("║ Avg Latency (ms):  {:>20.2} ║", self.avg_latency_ms);
        println!("║ Min Latency (ms):  {:>20} ║", self.min_latency_ms);
        println!("║ Max Latency (ms):  {:>20} ║", self.max_latency_ms);
        println!("║ P95 Latency (ms):  {:>20} ║", self.p95_latency_ms);
        println!("║ P99 Latency (ms):  {:>20} ║", self.p99_latency_ms);
        println!("╚════════════════════════════════════════╝");
    }
}

/// Load test executor
pub struct LoadTestRunner {
    env: Arc<TestEnvironment>,
    executor: Arc<ScenarioExecutor>,
}

impl LoadTestRunner {
    pub fn new(env: Arc<TestEnvironment>) -> Self {
        let executor = Arc::new(ScenarioExecutor::new(env.clone()));
        Self { env, executor }
    }

    /// Run constant rate load test
    pub async fn run_constant_load(
        &self,
        rps: u32,
        duration_secs: u32,
    ) -> Result<LoadMetrics> {
        let total_requests = rps * duration_secs;
        let delay_between_requests = std::time::Duration::from_millis(1000 / rps as u64);

        let mut latencies = vec![];
        let mut successful = 0u32;
        let mut failed = 0u32;

        let start = Instant::now();

        for i in 0..total_requests {
            let req_start = Instant::now();

            let req = TestRequestBuilder::new()
                .request_id(format!("load-test-{}", i))
                .build();

            match self.executor.allocate(req).await {
                Ok(alloc) => {
                    if alloc.success {
                        successful += 1;
                        latencies.push(req_start.elapsed().as_millis() as u32);

                        // Deallocate
                        let _ = self
                            .executor
                            .deallocate(&alloc.request_id, alloc.block_ids)
                            .await;
                    } else {
                        failed += 1;
                    }
                }
                Err(_) => failed += 1,
            }

            // Rate limiting
            tokio::time::sleep(delay_between_requests).await;

            if start.elapsed().as_secs() >= duration_secs as u64 {
                break;
            }
        }

        let total_duration = start.elapsed().as_secs_f64();

        // Calculate percentiles
        latencies.sort_unstable();
        let p95_idx = (latencies.len() as f64 * 0.95) as usize;
        let p99_idx = (latencies.len() as f64 * 0.99) as usize;

        let metrics = LoadMetrics {
            total_requests,
            successful_requests: successful,
            failed_requests: failed,
            total_duration_secs: total_duration,
            throughput_rps: successful as f64 / total_duration,
            avg_latency_ms: latencies.iter().map(|l| *l as f64).sum::<f64>()
                / latencies.len() as f64,
            min_latency_ms: latencies.first().copied().unwrap_or(0),
            max_latency_ms: latencies.last().copied().unwrap_or(0),
            p95_latency_ms: latencies.get(p95_idx).copied().unwrap_or(0),
            p99_latency_ms: latencies.get(p99_idx).copied().unwrap_or(0),
        };

        Ok(metrics)
    }

    /// Run ramp-up load test (gradually increase load)
    pub async fn run_ramp_load(
        &self,
        start_rps: u32,
        end_rps: u32,
        ramp_duration_secs: u32,
    ) -> Result<Vec<LoadMetrics>> {
        let mut results = vec![];
        let steps = 5;
        let step_duration = ramp_duration_secs / steps;

        for step in 0..steps {
            let current_rps = start_rps
                + ((end_rps - start_rps) as f64 * step as f64 / steps as f64) as u32;
            println!("Ramp step {}: {} RPS", step + 1, current_rps);

            let metrics = self.run_constant_load(current_rps, step_duration).await?;
            results.push(metrics);
        }

        Ok(results)
    }

    /// Run spike load test (sudden increase)
    pub async fn run_spike_load(
        &self,
        baseline_rps: u32,
        spike_rps: u32,
        baseline_duration_secs: u32,
        spike_duration_secs: u32,
    ) -> Result<(LoadMetrics, LoadMetrics)> {
        println!("Baseline phase: {} RPS for {}s", baseline_rps, baseline_duration_secs);
        let baseline = self.run_constant_load(baseline_rps, baseline_duration_secs).await?;
        baseline.report();

        println!("\nSpike phase: {} RPS for {}s", spike_rps, spike_duration_secs);
        let spike = self.run_constant_load(spike_rps, spike_duration_secs).await?;
        spike.report();

        Ok((baseline, spike))
    }

    /// Run sustained load test with periodic cache stats
    pub async fn run_sustained_load(
        &self,
        rps: u32,
        duration_secs: u32,
    ) -> Result<LoadMetrics> {
        let metrics = self.run_constant_load(rps, duration_secs).await?;

        let stats = self.executor.get_cache_stats().await?;
        println!("\nCache Statistics:");
        println!("  Total Blocks: {}", stats.total_blocks);
        println!("  Allocated: {} ({}%)", stats.allocated_blocks, stats.utilization_percent);
        println!("  Free: {}", stats.free_blocks);

        Ok(metrics)
    }
}

#[tokio::test]
async fn test_low_load() -> Result<()> {
    let env = TestEnvironment::start().await?;
    let env = Arc::new(env);
    let runner = LoadTestRunner::new(env.clone());

    println!("🔵 LOW LOAD TEST: 10 RPS for 10 seconds");
    let metrics = runner.run_constant_load(10, 10).await?;
    metrics.report();

    assert!(
        metrics.successful_requests > 0,
        "Should have successful requests"
    );
    assert!(metrics.throughput_rps > 0.0, "Should have positive throughput");

    env.stop().await?;
    Ok(())
}

#[tokio::test]
async fn test_medium_load() -> Result<()> {
    let env = TestEnvironment::start().await?;
    let env = Arc::new(env);
    let runner = LoadTestRunner::new(env.clone());

    println!("🟡 MEDIUM LOAD TEST: 50 RPS for 20 seconds");
    let metrics = runner.run_constant_load(50, 20).await?;
    metrics.report();

    assert!(
        metrics.successful_requests > 0,
        "Should have successful requests"
    );

    env.stop().await?;
    Ok(())
}

#[tokio::test]
async fn test_high_load() -> Result<()> {
    let env = TestEnvironment::start().await?;
    let env = Arc::new(env);
    let runner = LoadTestRunner::new(env.clone());

    println!("🔴 HIGH LOAD TEST: 100 RPS for 30 seconds");
    let metrics = runner.run_constant_load(100, 30).await?;
    metrics.report();

    // Under high load, some failures may be expected
    println!(
        "Success rate: {:.2}%",
        (metrics.successful_requests as f64 / metrics.total_requests as f64) * 100.0
    );

    env.stop().await?;
    Ok(())
}

#[tokio::test]
async fn test_ramp_load() -> Result<()> {
    let env = TestEnvironment::start().await?;
    let env = Arc::new(env);
    let runner = LoadTestRunner::new(env.clone());

    println!("📈 RAMP LOAD TEST: 10 RPS → 100 RPS");
    let results = runner.run_ramp_load(10, 100, 25).await?;

    for (i, metrics) in results.iter().enumerate() {
        println!("\nStep {}:", i + 1);
        metrics.report();
    }

    env.stop().await?;
    Ok(())
}

#[tokio::test]
async fn test_spike_load() -> Result<()> {
    let env = TestEnvironment::start().await?;
    let env = Arc::new(env);
    let runner = LoadTestRunner::new(env.clone());

    println!("⚡ SPIKE LOAD TEST");
    runner
        .run_spike_load(20, 200, 10, 10)
        .await?;

    env.stop().await?;
    Ok(())
}

#[tokio::test]
async fn test_sustained_load() -> Result<()> {
    let env = TestEnvironment::start().await?;
    let env = Arc::new(env);
    let runner = LoadTestRunner::new(env.clone());

    println!("🔁 SUSTAINED LOAD TEST: 50 RPS for 60 seconds");
    let metrics = runner.run_sustained_load(50, 60).await?;
    metrics.report();

    env.stop().await?;
    Ok(())
}

#[tokio::test]
async fn test_throughput_benchmark() -> Result<()> {
    let env = TestEnvironment::start().await?;
    let env = Arc::new(env);
    let runner = LoadTestRunner::new(env.clone());

    println!("📊 THROUGHPUT BENCHMARK: Finding max sustainable RPS");

    let mut results = vec![];
    for rps in [10, 25, 50, 75, 100, 150, 200].iter() {
        let metrics = runner.run_constant_load(*rps, 10).await?;
        let success_rate =
            (metrics.successful_requests as f64 / metrics.total_requests as f64) * 100.0;

        println!(
            "Target: {} RPS | Achieved: {:.2} RPS | Success: {:.1}%",
            rps, metrics.throughput_rps, success_rate
        );

        results.push((*rps, metrics.throughput_rps, success_rate));

        if success_rate < 80.0 {
            println!("⚠️  Success rate below 80%, stopping ramp");
            break;
        }
    }

    env.stop().await?;
    Ok(())
}
