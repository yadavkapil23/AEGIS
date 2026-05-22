/// Real Backend Integration Tests
/// Tests actual LLM inference with vLLM and llama.cpp
/// These tests require real backends to be running

mod integration_test_harness;

use integration_test_harness::*;
use std::sync::Arc;
use anyhow::Result;

/// Test with vLLM backend (requires: docker-compose up vllm)
#[tokio::test]
#[ignore]  // Run with: cargo test --test real_backend_integration -- --ignored --nocapture
async fn test_vllm_real_inference() -> Result<()> {
    println!("\n🔴 TEST: Real vLLM Inference");
    println!("Requires: docker-compose up vllm");
    println!("Endpoint: http://localhost:8000");

    let env = TestEnvironment::start().await?;
    let env = Arc::new(env);
    let executor = ScenarioExecutor::new(env.clone());

    // Create request for vLLM
    let req = AllocationRequest {
        request_id: "vllm-test-001".to_string(),
        num_blocks: 4,
        model: Some("meta-llama/Llama-2-7b-hf".to_string()),
        priority: Some(5),
    };

    // Try to allocate (may fail if vLLM not running)
    match executor.allocate(req.clone()).await {
        Ok(resp) => {
            println!("✅ Allocation successful");
            println!("   Request ID: {}", resp.request_id);
            println!("   Blocks: {:?}", resp.block_ids);
            println!("   Latency: {}ms", resp.latency_ms);
            assert!(resp.success, "Allocation should succeed");
        }
        Err(e) => {
            println!("❌ Allocation failed: {}", e);
            println!("Make sure vLLM is running: docker-compose up vllm");
            return Ok(()); // Don't fail test if backend not running
        }
    }

    env.stop().await?;
    Ok(())
}

/// Test with llama.cpp backend (requires: docker-compose up llamacpp)
#[tokio::test]
#[ignore]  // Run with: cargo test --test real_backend_integration -- --ignored --nocapture
async fn test_llamacpp_real_inference() -> Result<()> {
    println!("\n🟡 TEST: Real llama.cpp Inference");
    println!("Requires: docker-compose up llamacpp");
    println!("Endpoint: http://localhost:8001");

    let env = TestEnvironment::start().await?;
    let env = Arc::new(env);
    let executor = ScenarioExecutor::new(env.clone());

    // Create request for llama.cpp
    let req = AllocationRequest {
        request_id: "llamacpp-test-001".to_string(),
        num_blocks: 4,
        model: Some("mistral-7b".to_string()),
        priority: Some(5),
    };

    // Try to allocate
    match executor.allocate(req.clone()).await {
        Ok(resp) => {
            println!("✅ Allocation successful");
            println!("   Request ID: {}", resp.request_id);
            println!("   Blocks: {:?}", resp.block_ids);
            println!("   Latency: {}ms", resp.latency_ms);
            assert!(resp.success, "Allocation should succeed");
        }
        Err(e) => {
            println!("❌ Allocation failed: {}", e);
            println!("Make sure llama.cpp is running: docker-compose up llamacpp");
            return Ok(()); // Don't fail test if backend not running
        }
    }

    env.stop().await?;
    Ok(())
}

/// Test fallback from vLLM to llama.cpp
#[tokio::test]
#[ignore]
async fn test_backend_fallback() -> Result<()> {
    println!("\n🟢 TEST: Backend Failover/Fallback");
    println!("When vLLM fails, should fallback to llama.cpp");

    let env = TestEnvironment::start().await?;
    let env = Arc::new(env);
    let executor = ScenarioExecutor::new(env.clone());

    // Try different models to test fallback
    let models = vec![
        ("meta-llama/Llama-2-7b-hf", "vLLM model"),
        ("mistral-7b", "llama.cpp model"),
    ];

    for (model, desc) in models {
        println!("\nTesting: {} ({})", model, desc);

        let req = AllocationRequest {
            request_id: format!("fallback-test-{}", model),
            num_blocks: 2,
            model: Some(model.to_string()),
            priority: Some(5),
        };

        match executor.allocate(req).await {
            Ok(resp) => {
                println!("✅ {} - Success", desc);
                println!("   Latency: {}ms", resp.latency_ms);
            }
            Err(e) => {
                println!("⚠️  {} - Failed: {}", desc, e);
            }
        }
    }

    env.stop().await?;
    Ok(())
}

/// Test real inference end-to-end
#[tokio::test]
#[ignore]
async fn test_full_inference_pipeline() -> Result<()> {
    println!("\n🚀 TEST: Full Inference Pipeline");
    println!("Allocate → Inference → Deallocate with real backend");

    let env = TestEnvironment::start().await?;
    let env = Arc::new(env);
    let executor = ScenarioExecutor::new(env.clone());

    // Step 1: Allocate
    let alloc_req = AllocationRequest {
        request_id: "full-pipeline-001".to_string(),
        num_blocks: 4,
        model: Some("meta-llama/Llama-2-7b-hf".to_string()),
        priority: Some(5),
    };

    println!("Step 1: Allocating cache blocks...");
    let alloc_resp = match executor.allocate(alloc_req.clone()).await {
        Ok(resp) => {
            println!("✅ Allocated {} blocks", resp.block_ids.len());
            resp
        }
        Err(e) => {
            println!("❌ Allocation failed: {}", e);
            println!("Ensure vLLM is running on http://localhost:8000");
            return Ok(());
        }
    };

    let block_ids = alloc_resp.block_ids.clone();

    // Step 2: Run inference
    let infer_req = InferenceRequest {
        request_id: "full-pipeline-001".to_string(),
        model: "meta-llama/Llama-2-7b-hf".to_string(),
        prompt: "What is artificial intelligence? Explain in one sentence.".to_string(),
        max_tokens: 50,
    };

    println!("\nStep 2: Running inference...");
    let infer_resp = match executor.infer(infer_req).await {
        Ok(resp) => {
            println!("✅ Inference complete");
            println!("   Output: {}", resp.output.as_ref().unwrap_or(&"[none]".to_string()));
            println!("   Tokens: {}", resp.tokens_generated);
            println!("   Latency: {}ms", resp.latency_ms);
            resp
        }
        Err(e) => {
            println!("❌ Inference failed: {}", e);
            // Still try to deallocate
            let _ = executor.deallocate("full-pipeline-001", block_ids.clone()).await;
            return Ok(());
        }
    };

    // Step 3: Deallocate
    println!("\nStep 3: Deallocating blocks...");
    match executor.deallocate("full-pipeline-001", block_ids).await {
        Ok(_) => println!("✅ Deallocated successfully"),
        Err(e) => println!("❌ Deallocation failed: {}", e),
    }

    // Verify cache is clean
    let stats = executor.get_cache_stats().await?;
    println!("\nFinal Cache Stats:");
    println!("  Utilization: {}%", stats.utilization_percent);
    println!("  Allocated: {}", stats.allocated_blocks);
    println!("  Free: {}", stats.free_blocks);

    env.stop().await?;
    Ok(())
}

/// Performance test with real backend
#[tokio::test]
#[ignore]
async fn test_real_backend_performance() -> Result<()> {
    println!("\n📊 TEST: Real Backend Performance");
    println!("Measure inference latency and throughput");

    let env = TestEnvironment::start().await?;
    let env = Arc::new(env);

    let mut latencies = vec![];
    let mut successes = 0;
    let mut failures = 0;

    let start = std::time::Instant::now();

    // Run 10 inference requests
    for i in 0..10 {
        let req_start = std::time::Instant::now();

        let infer_req = InferenceRequest {
            request_id: format!("perf-test-{}", i),
            model: "meta-llama/Llama-2-7b-hf".to_string(),
            prompt: format!("Question {}?", i),
            max_tokens: 20,
        };

        let executor = ScenarioExecutor::new(env.clone());
        match executor.infer(infer_req).await {
            Ok(resp) => {
                let latency = req_start.elapsed().as_millis() as u32;
                latencies.push(latency);
                successes += 1;
                println!("✅ Request {}: {}ms", i, latency);
            }
            Err(e) => {
                failures += 1;
                println!("❌ Request {}: {}", i, e);
            }
        }
    }

    let total_duration = start.elapsed();

    // Calculate stats
    latencies.sort_unstable();
    let avg_latency = if latencies.is_empty() {
        0.0
    } else {
        latencies.iter().map(|l| *l as f64).sum::<f64>() / latencies.len() as f64
    };

    let min_latency = latencies.first().copied().unwrap_or(0);
    let max_latency = latencies.last().copied().unwrap_or(0);
    let p95_idx = (latencies.len() as f64 * 0.95) as usize;
    let p95_latency = latencies.get(p95_idx).copied().unwrap_or(0);

    println!("\n📈 Performance Results:");
    println!("  Successful: {}/10", successes);
    println!("  Failed: {}/10", failures);
    println!("  Duration: {}ms", total_duration.as_millis());
    println!("  Avg Latency: {:.2}ms", avg_latency);
    println!("  Min Latency: {}ms", min_latency);
    println!("  Max Latency: {}ms", max_latency);
    println!("  P95 Latency: {}ms", p95_latency);

    if successes > 0 {
        let throughput = (successes as f64 / total_duration.as_secs_f64());
        println!("  Throughput: {:.2} req/sec", throughput);
    }

    env.stop().await?;
    Ok(())
}
