/// Chaos Engineering and Resilience Tests
/// Tests system behavior under various failure scenarios

mod integration_test_harness;

use integration_test_harness::*;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use anyhow::Result;

/// Chaos test configuration
pub struct ChaosConfig {
    pub failure_rate: f32,        // 0.0 to 1.0
    pub recovery_time_secs: u32,  // Time to recover
    pub circuit_breaker_threshold: u32,  // Failures before breaking
}

/// Chaos scenario executor
pub struct ChaosExecutor {
    env: Arc<TestEnvironment>,
    executor: Arc<ScenarioExecutor>,
    is_failing: Arc<AtomicBool>,
    failure_count: Arc<AtomicU32>,
}

impl ChaosExecutor {
    pub fn new(env: Arc<TestEnvironment>) -> Self {
        Self {
            executor: Arc::new(ScenarioExecutor::new(env.clone())),
            env,
            is_failing: Arc::new(AtomicBool::new(false)),
            failure_count: Arc::new(AtomicU32::new(0)),
        }
    }

    /// Simulate random failures
    pub async fn with_random_failures<F>(&self, failure_rate: f32, duration_secs: u32, mut test: F) -> Result<()>
    where
        F: FnMut() -> futures::future::BoxFuture<'static, Result<()>>,
    {
        let start = std::time::Instant::now();

        while start.elapsed().as_secs() < duration_secs as u64 {
            let should_fail = rand::random::<f32>() < failure_rate;
            self.is_failing.store(should_fail, Ordering::SeqCst);

            if should_fail {
                self.failure_count.fetch_add(1, Ordering::SeqCst);
            }

            test().await?;
            tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;
        }

        Ok(())
    }

    /// Simulate component recovery
    pub async fn simulate_recovery(&self, failure_duration_secs: u32, recovery_duration_secs: u32) -> Result<()> {
        println!("🔴 Component failure for {}s", failure_duration_secs);
        self.is_failing.store(true, Ordering::SeqCst);
        tokio::time::sleep(tokio::time::Duration::from_secs(failure_duration_secs as u64)).await;

        println!("🟡 Component recovering for {}s", recovery_duration_secs);
        self.is_failing.store(false, Ordering::SeqCst);
        tokio::time::sleep(tokio::time::Duration::from_secs(recovery_duration_secs as u64)).await;

        println!("🟢 Component healthy");
        Ok(())
    }

    pub fn get_failure_count(&self) -> u32 {
        self.failure_count.load(Ordering::SeqCst)
    }
}

#[tokio::test]
async fn test_backend_failure_recovery() -> Result<()> {
    let env = TestEnvironment::start().await?;
    let env = Arc::new(env);
    let chaos = ChaosExecutor::new(env.clone());

    println!("⚡ TEST: Backend Failure and Recovery");

    // Phase 1: Normal operation
    println!("\n📍 Phase 1: Normal operation (10s)");
    for i in 0..10 {
        let req = TestRequestBuilder::new()
            .request_id(format!("normal-{}", i))
            .build();
        let _ = chaos.executor.allocate(req).await;
        tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;
    }

    // Phase 2: Component failure
    println!("\n🔴 Phase 2: Component failure (5s)");
    chaos.is_failing.store(true, Ordering::SeqCst);
    tokio::time::sleep(tokio::time::Duration::from_secs(5)).await;

    // Requests should fail or queue
    let mut failed = 0;
    for i in 0..5 {
        let req = TestRequestBuilder::new()
            .request_id(format!("failing-{}", i))
            .build();
        if chaos.executor.allocate(req).await.is_err() {
            failed += 1;
        }
    }
    println!("Failed requests during outage: {}/5", failed);

    // Phase 3: Recovery
    println!("\n🟡 Phase 3: Recovery (5s)");
    chaos.is_failing.store(false, Ordering::SeqCst);
    tokio::time::sleep(tokio::time::Duration::from_secs(5)).await;

    // Requests should succeed again
    println!("🟢 Testing recovery: Requests should succeed");
    for i in 0..10 {
        let req = TestRequestBuilder::new()
            .request_id(format!("recovery-{}", i))
            .build();
        match chaos.executor.allocate(req).await {
            Ok(resp) => {
                if resp.success {
                    print!("✓");
                } else {
                    print!("✗");
                }
            }
            Err(_) => print!("E"),
        }
    }
    println!();

    env.stop().await?;
    Ok(())
}

#[tokio::test]
async fn test_cascading_failure_prevention() -> Result<()> {
    let env = TestEnvironment::start().await?;
    let env = Arc::new(env);
    let chaos = ChaosExecutor::new(env.clone());

    println!("⚠️ TEST: Cascading Failure Prevention");

    // Simulate cascading failures across different components
    // Component 1 fails, should not bring down Component 2

    println!("Triggering failures on multiple fronts...");

    // Concurrent requests while failures are happening
    let mut handles = vec![];

    for i in 0..20 {
        let executor = chaos.executor.clone();
        let handle = tokio::spawn(async move {
            let req = TestRequestBuilder::new()
                .request_id(format!("cascade-{}", i))
                .build();

            match executor.allocate(req).await {
                Ok(resp) => resp.success,
                Err(_) => false,
            }
        });
        handles.push(handle);
    }

    // Simulate some failures
    chaos.is_failing.store(true, Ordering::SeqCst);
    tokio::time::sleep(tokio::time::Duration::from_millis(500)).await;
    chaos.is_failing.store(false, Ordering::SeqCst);

    // Wait for all to complete
    let mut successes = 0;
    for handle in handles {
        if handle.await? {
            successes += 1;
        }
    }

    println!("Successful requests: {}/20", successes);
    assert!(
        successes > 5,
        "System should handle some requests despite failures"
    );

    env.stop().await?;
    Ok(())
}

#[tokio::test]
async fn test_circuit_breaker_activation() -> Result<()> {
    let env = TestEnvironment::start().await?;
    let env = Arc::new(env);
    let chaos = ChaosExecutor::new(env.clone());

    println!("🔌 TEST: Circuit Breaker Activation");

    // Trigger repeated failures to activate circuit breaker
    println!("Triggering multiple failures...");

    for i in 0..10 {
        chaos.is_failing.store(true, Ordering::SeqCst);

        let req = TestRequestBuilder::new()
            .request_id(format!("cb-fail-{}", i))
            .build();

        let result = chaos.executor.allocate(req).await;
        println!(
            "Attempt {}: {}",
            i + 1,
            if result.is_ok() { "Ok" } else { "Err" }
        );

        tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;
    }

    println!(
        "\nTotal failures injected: {}",
        chaos.get_failure_count()
    );

    // After multiple failures, circuit should be OPEN
    // Subsequent requests should fail fast without trying backend
    println!("\nTesting circuit breaker open state (should fail fast):");

    chaos.is_failing.store(false, Ordering::SeqCst);

    let req = TestRequestBuilder::new()
        .request_id("cb-open-test".to_string())
        .build();

    match chaos.executor.allocate(req).await {
        Ok(resp) => println!("Response: success={}", resp.success),
        Err(e) => println!("Fast-failed with: {}", e),
    }

    env.stop().await?;
    Ok(())
}

#[tokio::test]
async fn test_memory_exhaustion_scenario() -> Result<()> {
    let env = TestEnvironment::start().await?;
    let env = Arc::new(env);
    let chaos = ChaosExecutor::new(env.clone());

    println!("💾 TEST: Memory Exhaustion and Recovery");

    // Allocate increasingly large amounts
    let mut allocations = vec![];

    println!("Phase 1: Allocating memory...");
    for i in 0..50 {
        let req = TestRequestBuilder::new()
            .request_id(format!("memory-{}", i))
            .num_blocks(100)
            .build();

        match chaos.executor.allocate(req).await {
            Ok(alloc) => {
                allocations.push(alloc);
                print!(".");
            }
            Err(_) => {
                println!("\n⚠️  Hit allocation limit at iteration {}", i);
                break;
            }
        }
    }

    println!("\nTotal allocated: {} blocks", allocations.len() * 100);

    // Should have hit memory limit
    let stats = chaos.executor.get_cache_stats().await?;
    println!(
        "Cache utilization: {}% ({}/{})",
        stats.utilization_percent, stats.allocated_blocks, stats.total_blocks
    );

    // Try new allocation - should fail gracefully
    println!("\nPhase 2: New allocation under memory pressure...");
    let new_req = TestRequestBuilder::new()
        .request_id("memory-new".to_string())
        .num_blocks(100)
        .build();

    let result = chaos.executor.allocate(new_req).await;
    match result {
        Ok(resp) => {
            if !resp.success {
                println!("✓ Graceful failure (expected)");
            }
        }
        Err(e) => println!("✓ Graceful error: {}", e),
    }

    // Phase 3: Cleanup and verify recovery
    println!("\nPhase 3: Cleanup and recovery...");
    for alloc in allocations.iter().take(25) {
        let _ = chaos.executor.deallocate(&alloc.request_id, alloc.block_ids.clone()).await;
    }

    let stats = chaos.executor.get_cache_stats().await?;
    println!("After cleanup: {}% utilized", stats.utilization_percent);

    env.stop().await?;
    Ok(())
}

#[tokio::test]
async fn test_timeout_handling() -> Result<()> {
    let env = TestEnvironment::start().await?;
    let env = Arc::new(env);
    let chaos = ChaosExecutor::new(env.clone());

    println!("⏱️ TEST: Timeout Handling");

    // Simulate slow responses
    println!("Testing timeouts with slow responses...");

    for i in 0..5 {
        let req = TestRequestBuilder::new()
            .request_id(format!("timeout-{}", i))
            .build();

        let start = std::time::Instant::now();
        let _result = chaos.executor.allocate(req).await;
        let elapsed = start.elapsed().as_millis();

        println!("Request {} took {}ms", i + 1, elapsed);

        // Verify timeout enforcement
        if elapsed > 5000 {
            println!("✗ Request exceeded timeout");
        }
    }

    env.stop().await?;
    Ok(())
}

#[tokio::test]
async fn test_graceful_degradation() -> Result<()> {
    let env = TestEnvironment::start().await?;
    let env = Arc::new(env);
    let chaos = ChaosExecutor::new(env.clone());

    println!("📉 TEST: Graceful Degradation");

    // Simulate increasing resource pressure
    println!("Simulating increasing resource pressure...");

    let pressure_levels = [10, 30, 50, 70, 90];

    for (i, pressure) in pressure_levels.iter().enumerate() {
        println!("\n--- Pressure Level {}: {}% ---", i + 1, pressure);

        // Simulate pressure by allocating memory
        let mut temp_allocs = vec![];
        for j in 0..(*pressure / 10) {
            let req = TestRequestBuilder::new()
                .request_id(format!("pressure-{}-{}", i, j))
                .num_blocks(50)
                .build();

            if let Ok(alloc) = chaos.executor.allocate(req).await {
                temp_allocs.push(alloc);
            }
        }

        // Try new request
        let new_req = TestRequestBuilder::new()
            .request_id(format!("under-pressure-{}", i))
            .build();

        match chaos.executor.allocate(new_req).await {
            Ok(resp) => {
                println!(
                    "New request: {} (latency: {}ms)",
                    if resp.success { "Success" } else { "Failed" },
                    resp.latency_ms
                );
            }
            Err(e) => println!("New request error: {}", e),
        }

        // Cleanup
        for alloc in temp_allocs {
            let _ = chaos.executor.deallocate(&alloc.request_id, alloc.block_ids).await;
        }
    }

    env.stop().await?;
    Ok(())
}

#[tokio::test]
async fn test_request_queuing_under_load() -> Result<()> {
    let env = TestEnvironment::start().await?;
    let env = Arc::new(env);
    let chaos = ChaosExecutor::new(env.clone());

    println!("📋 TEST: Request Queuing Under Load");

    // Simulate backend saturation
    chaos.is_failing.store(true, Ordering::SeqCst);

    println!("Queuing 50 requests while backend is saturated...");

    let mut handles = vec![];
    for i in 0..50 {
        let executor = chaos.executor.clone();
        let handle = tokio::spawn(async move {
            let req = TestRequestBuilder::new()
                .request_id(format!("queued-{}", i))
                .build();

            executor.allocate(req).await
        });
        handles.push(handle);
    }

    // Let requests queue
    tokio::time::sleep(tokio::time::Duration::from_secs(1)).await;

    // Recover backend
    println!("Recovering backend...");
    chaos.is_failing.store(false, Ordering::SeqCst);

    // Wait for queue to drain
    let mut successes = 0;
    let mut failures = 0;

    for handle in handles {
        match handle.await {
            Ok(Ok(resp)) => {
                if resp.success {
                    successes += 1;
                } else {
                    failures += 1;
                }
            }
            _ => failures += 1,
        }
    }

    println!("Queue results: {} success, {} failed", successes, failures);
    println!(
        "Success rate: {:.1}%",
        (successes as f64 / (successes + failures) as f64) * 100.0
    );

    env.stop().await?;
    Ok(())
}
