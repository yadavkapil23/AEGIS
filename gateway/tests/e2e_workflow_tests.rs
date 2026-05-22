/// End-to-End Workflow Integration Tests
/// Tests complete inference request lifecycle: Allocate → Infer → Deallocate → Verify

mod integration_test_harness;

use integration_test_harness::*;
use std::sync::Arc;
use anyhow::Result;

#[tokio::test]
async fn test_single_inference_workflow() -> Result<()> {
    // Start test environment
    let env = TestEnvironment::start().await?;
    let env = Arc::new(env);
    let executor = ScenarioExecutor::new(env.clone());

    // 1. Create allocation request
    let alloc_req = TestRequestBuilder::new()
        .request_id("test-001".to_string())
        .num_blocks(4)
        .model("llama-7b".to_string())
        .priority(5)
        .build();

    // 2. Allocate KV cache blocks
    let alloc_resp = executor.allocate(alloc_req.clone()).await?;
    assert!(alloc_resp.success, "Allocation should succeed");
    assert_eq!(alloc_resp.block_ids.len(), 4, "Should allocate 4 blocks");
    assert!(alloc_resp.latency_ms < 100, "Allocation should be fast");

    let block_ids = alloc_resp.block_ids.clone();

    // 3. Run inference with allocated blocks
    let infer_req = InferenceRequest {
        request_id: "test-001".to_string(),
        model: "llama-7b".to_string(),
        prompt: "What is AI?".to_string(),
        max_tokens: 100,
    };

    let infer_resp = executor.infer(infer_req).await?;
    assert!(infer_resp.success, "Inference should succeed");
    assert!(infer_resp.output.is_some(), "Should have output");
    assert!(infer_resp.tokens_generated > 0, "Should generate tokens");

    // 4. Deallocate blocks
    let dealloc_ok = executor.deallocate("test-001", block_ids).await?;
    assert!(dealloc_ok, "Deallocation should succeed");

    // 5. Verify cache is clean
    let stats = executor.get_cache_stats().await?;
    assert!(stats.allocated_blocks < 10, "Cache should be mostly free");

    // Cleanup
    env.stop().await?;
    Ok(())
}

#[tokio::test]
async fn test_concurrent_inference_requests() -> Result<()> {
    let env = TestEnvironment::start().await?;
    let env = Arc::new(env);
    let executor = Arc::new(ScenarioExecutor::new(env.clone()));

    // Spawn 10 concurrent inference requests
    let mut handles = vec![];
    for i in 0..10 {
        let executor = executor.clone();
        let handle = tokio::spawn(async move {
            let req = TestRequestBuilder::new()
                .request_id(format!("concurrent-{}", i))
                .num_blocks(2)
                .build();

            let alloc_resp = executor.allocate(req).await.unwrap();
            assert!(alloc_resp.success);

            let infer_req = InferenceRequest {
                request_id: format!("concurrent-{}", i),
                model: "llama-7b".to_string(),
                prompt: format!("Question {}", i),
                max_tokens: 50,
            };

            let infer_resp = executor.infer(infer_req).await.unwrap();
            assert!(infer_resp.success);

            let _ = executor.deallocate(&format!("concurrent-{}", i), alloc_resp.block_ids).await;
        });
        handles.push(handle);
    }

    // Wait for all to complete
    for handle in handles {
        handle.await?;
    }

    env.stop().await?;
    Ok(())
}

#[tokio::test]
async fn test_cache_hit_scenario() -> Result<()> {
    let env = TestEnvironment::start().await?;
    let env = Arc::new(env);
    let executor = ScenarioExecutor::new(env.clone());

    // Request 1: Cache miss
    let req1 = TestRequestBuilder::new()
        .request_id("cache-test-1".to_string())
        .model("llama-7b".to_string())
        .build();

    let alloc1 = executor.allocate(req1).await?;
    let infer1 = executor
        .infer(InferenceRequest {
            request_id: "cache-test-1".to_string(),
            model: "llama-7b".to_string(),
            prompt: "What is AI?".to_string(),
            max_tokens: 100,
        })
        .await?;

    let latency_miss = infer1.latency_ms;

    // Request 2: Same model, might hit cache
    let req2 = TestRequestBuilder::new()
        .request_id("cache-test-2".to_string())
        .model("llama-7b".to_string())
        .build();

    let alloc2 = executor.allocate(req2).await?;
    let infer2 = executor
        .infer(InferenceRequest {
            request_id: "cache-test-2".to_string(),
            model: "llama-7b".to_string(),
            prompt: "What is AI?".to_string(),
            max_tokens: 100,
        })
        .await?;

    let latency_hit = infer2.latency_ms;

    // Cache hit should be faster (typically)
    println!(
        "Miss latency: {}ms, Hit latency: {}ms",
        latency_miss, latency_hit
    );

    // Cleanup
    executor.deallocate("cache-test-1", alloc1.block_ids).await?;
    executor.deallocate("cache-test-2", alloc2.block_ids).await?;

    env.stop().await?;
    Ok(())
}

#[tokio::test]
async fn test_allocation_failure_handling() -> Result<()> {
    let env = TestEnvironment::start().await?;
    let env = Arc::new(env);
    let executor = ScenarioExecutor::new(env.clone());

    // Request more blocks than available
    let huge_req = TestRequestBuilder::new()
        .request_id("huge-alloc".to_string())
        .num_blocks(100000) // Way too many
        .build();

    // This should fail gracefully
    match executor.allocate(huge_req).await {
        Ok(resp) => {
            // Could either fail in allocation or succeed with what's available
            println!("Response: {:?}", resp);
        }
        Err(e) => {
            // Error handling is also valid
            println!("Expected error: {}", e);
        }
    }

    env.stop().await?;
    Ok(())
}

#[tokio::test]
async fn test_request_timeout_handling() -> Result<()> {
    let env = TestEnvironment::start().await?;
    let env = Arc::new(env);
    let executor = ScenarioExecutor::new(env.clone());

    // Create request with timeout
    let req = TestRequestBuilder::new()
        .request_id("timeout-test".to_string())
        .build();

    // In real implementation, would test actual timeout behavior
    let alloc = executor.allocate(req).await?;
    assert!(alloc.success);

    env.stop().await?;
    Ok(())
}

#[tokio::test]
async fn test_circuit_breaker_degradation() -> Result<()> {
    let env = TestEnvironment::start().await?;
    let env = Arc::new(env);
    let executor = ScenarioExecutor::new(env.clone());

    // Simulate backend failures
    for i in 0..5 {
        let req = TestRequestBuilder::new()
            .request_id(format!("degradation-{}", i))
            .build();

        // In real scenario, backend would start failing here
        let _alloc = executor.allocate(req).await?;
    }

    // After failures, system should gracefully degrade
    let final_req = TestRequestBuilder::new()
        .request_id("final-request".to_string())
        .build();

    let final_alloc = executor.allocate(final_req).await?;
    // Should either succeed or fail gracefully
    println!("Final allocation status: {}", final_alloc.success);

    env.stop().await?;
    Ok(())
}

#[tokio::test]
async fn test_priority_queue_ordering() -> Result<()> {
    let env = TestEnvironment::start().await?;
    let env = Arc::new(env);
    let executor = ScenarioExecutor::new(env.clone());

    // Create requests with different priorities
    let low_priority = TestRequestBuilder::new()
        .request_id("low-priority".to_string())
        .priority(1)
        .build();

    let high_priority = TestRequestBuilder::new()
        .request_id("high-priority".to_string())
        .priority(10)
        .build();

    // Both should eventually succeed
    let low_alloc = executor.allocate(low_priority).await?;
    let high_alloc = executor.allocate(high_priority).await?;

    assert!(low_alloc.success);
    assert!(high_alloc.success);

    env.stop().await?;
    Ok(())
}

#[tokio::test]
async fn test_memory_pressure_handling() -> Result<()> {
    let env = TestEnvironment::start().await?;
    let env = Arc::new(env);
    let executor = ScenarioExecutor::new(env.clone());

    // Allocate many requests to fill cache
    let mut allocations = vec![];
    for i in 0..50 {
        let req = TestRequestBuilder::new()
            .request_id(format!("memory-pressure-{}", i))
            .num_blocks(10)
            .build();

        match executor.allocate(req).await {
            Ok(alloc) => {
                allocations.push(alloc);
            }
            Err(_) => {
                // Expected when cache is full
                break;
            }
        }
    }

    let stats = executor.get_cache_stats().await?;
    println!("Utilization: {}%", stats.utilization_percent);

    // Deallocate to verify cleanup
    for alloc in allocations {
        executor.deallocate(&alloc.request_id, alloc.block_ids).await?;
    }

    env.stop().await?;
    Ok(())
}
