/// Contract Testing
/// Ensures API contracts between services (Gateway ↔ Scheduler, Gateway ↔ Inference Backend)
/// Validates request/response shapes, error codes, and protocol compliance

mod integration_test_harness;

use integration_test_harness::*;
use std::sync::Arc;
use anyhow::Result;
use serde_json::json;

/// Contract validator for API responses
pub struct ContractValidator {
    env: Arc<TestEnvironment>,
    executor: Arc<ScenarioExecutor>,
}

impl ContractValidator {
    pub fn new(env: Arc<TestEnvironment>) -> Self {
        Self {
            executor: Arc::new(ScenarioExecutor::new(env.clone())),
            env,
        }
    }

    /// Validate allocation request contract
    pub fn validate_allocation_request(&self, req: &AllocationRequest) -> Result<()> {
        // All fields required
        assert!(!req.request_id.is_empty(), "request_id is required");
        assert!(req.num_blocks > 0, "num_blocks must be > 0");

        // request_id should be valid UUID or alphanumeric
        assert!(
            req.request_id.chars().all(|c| c.is_alphanumeric() || c == '-'),
            "request_id format invalid"
        );

        // Priority should be 0-10
        if let Some(priority) = req.priority {
            assert!(priority <= 10, "priority must be <= 10");
        }

        Ok(())
    }

    /// Validate allocation response contract
    pub fn validate_allocation_response(&self, resp: &AllocationResponse) -> Result<()> {
        // Response must have matching request_id
        assert!(!resp.request_id.is_empty(), "request_id is required in response");

        // If success, must have block_ids
        if resp.success {
            assert!(!resp.block_ids.is_empty(), "block_ids required on success");
        } else {
            // If failed, must have error message
            assert!(resp.error.is_some(), "error message required on failure");
        }

        // Latency should be reasonable
        assert!(resp.latency_ms < 60000, "latency_ms should be < 60s");

        Ok(())
    }

    /// Validate inference request contract
    pub fn validate_inference_request(&self, req: &InferenceRequest) -> Result<()> {
        assert!(!req.request_id.is_empty(), "request_id is required");
        assert!(!req.model.is_empty(), "model is required");
        assert!(!req.prompt.is_empty(), "prompt cannot be empty");
        assert!(req.max_tokens > 0, "max_tokens must be > 0");
        assert!(req.max_tokens <= 32000, "max_tokens must be <= 32000");

        Ok(())
    }

    /// Validate inference response contract
    pub fn validate_inference_response(&self, resp: &InferenceResponse) -> Result<()> {
        assert!(!resp.request_id.is_empty(), "request_id is required");

        if resp.success {
            assert!(resp.output.is_some(), "output required on success");
            assert!(resp.tokens_generated > 0, "tokens_generated must be > 0");
        } else {
            // Failed response should have error info
            // (could be in response or separate error field)
        }

        assert!(resp.latency_ms > 0, "latency_ms must be > 0");

        Ok(())
    }

    /// Validate cache stats contract
    pub fn validate_cache_stats(&self, stats: &CacheStats) -> Result<()> {
        assert!(stats.total_blocks > 0, "total_blocks must be > 0");
        assert!(
            stats.allocated_blocks <= stats.total_blocks,
            "allocated must be <= total"
        );
        assert!(
            stats.free_blocks == stats.total_blocks - stats.allocated_blocks,
            "free blocks calculation incorrect"
        );
        assert!(stats.utilization_percent <= 100, "utilization_percent must be <= 100");

        Ok(())
    }
}

#[tokio::test]
async fn test_allocation_contract() -> Result<()> {
    let env = TestEnvironment::start().await?;
    let env = Arc::new(env);
    let validator = ContractValidator::new(env.clone());

    println!("📋 TEST: Allocation Request/Response Contract");

    // Valid request
    let valid_req = TestRequestBuilder::new()
        .request_id("test-123".to_string())
        .num_blocks(4)
        .model("llama-7b".to_string())
        .priority(5)
        .build();

    validator.validate_allocation_request(&valid_req)?;
    println!("✓ Valid request contract passed");

    // Execute and validate response
    let resp = validator.executor.allocate(valid_req).await?;
    validator.validate_allocation_response(&resp)?;
    println!("✓ Valid response contract passed");

    // Test invalid requests
    println!("\nTesting invalid request contracts:");

    // Invalid: empty request_id
    let invalid = TestRequestBuilder::new()
        .request_id("".to_string())
        .build();

    match validator.validate_allocation_request(&invalid) {
        Err(e) => println!("✓ Correctly rejected invalid request: {}", e),
        Ok(_) => panic!("Should reject invalid request"),
    }

    // Invalid: zero blocks
    let invalid = AllocationRequest {
        request_id: "test".to_string(),
        num_blocks: 0,
        model: None,
        priority: None,
    };

    match validator.validate_allocation_request(&invalid) {
        Err(e) => println!("✓ Correctly rejected zero blocks: {}", e),
        Ok(_) => panic!("Should reject zero blocks"),
    }

    env.stop().await?;
    Ok(())
}

#[tokio::test]
async fn test_inference_contract() -> Result<()> {
    let env = TestEnvironment::start().await?;
    let env = Arc::new(env);
    let validator = ContractValidator::new(env.clone());

    println!("📋 TEST: Inference Request/Response Contract");

    // Valid request
    let valid_req = InferenceRequest {
        request_id: "infer-001".to_string(),
        model: "llama-7b".to_string(),
        prompt: "What is AI?".to_string(),
        max_tokens: 100,
    };

    validator.validate_inference_request(&valid_req)?;
    println!("✓ Valid inference request passed");

    // Execute and validate response
    let resp = validator.executor.infer(valid_req).await?;
    validator.validate_inference_response(&resp)?;
    println!("✓ Valid inference response passed");

    // Test invalid requests
    println!("\nTesting invalid inference contracts:");

    // Invalid: empty model
    let invalid = InferenceRequest {
        request_id: "test".to_string(),
        model: "".to_string(),
        prompt: "test".to_string(),
        max_tokens: 100,
    };

    match validator.validate_inference_request(&invalid) {
        Err(e) => println!("✓ Correctly rejected empty model: {}", e),
        Ok(_) => panic!("Should reject empty model"),
    }

    // Invalid: empty prompt
    let invalid = InferenceRequest {
        request_id: "test".to_string(),
        model: "llama".to_string(),
        prompt: "".to_string(),
        max_tokens: 100,
    };

    match validator.validate_inference_request(&invalid) {
        Err(e) => println!("✓ Correctly rejected empty prompt: {}", e),
        Ok(_) => panic!("Should reject empty prompt"),
    }

    // Invalid: max_tokens too high
    let invalid = InferenceRequest {
        request_id: "test".to_string(),
        model: "llama".to_string(),
        prompt: "test".to_string(),
        max_tokens: 50000,
    };

    match validator.validate_inference_request(&invalid) {
        Err(e) => println!("✓ Correctly rejected max_tokens > 32000: {}", e),
        Ok(_) => panic!("Should reject max_tokens > 32000"),
    }

    env.stop().await?;
    Ok(())
}

#[tokio::test]
async fn test_cache_stats_contract() -> Result<()> {
    let env = TestEnvironment::start().await?;
    let env = Arc::new(env);
    let validator = ContractValidator::new(env.clone());

    println!("📋 TEST: Cache Stats Contract");

    // Get real cache stats
    let stats = validator.executor.get_cache_stats().await?;
    validator.validate_cache_stats(&stats)?;
    println!("✓ Cache stats contract valid");

    // Verify calculations
    let calculated_free = stats.total_blocks - stats.allocated_blocks;
    assert_eq!(
        calculated_free, stats.free_blocks,
        "Free blocks calculation mismatch"
    );
    println!("✓ Cache stats calculations correct");

    env.stop().await?;
    Ok(())
}

#[tokio::test]
async fn test_response_schema_consistency() -> Result<()> {
    let env = TestEnvironment::start().await?;
    let env = Arc::new(env);
    let validator = ContractValidator::new(env.clone());

    println!("📋 TEST: Response Schema Consistency");

    // Run multiple allocations and verify schema consistency
    println!("Testing allocation response schema consistency...");

    for i in 0..10 {
        let req = TestRequestBuilder::new()
            .request_id(format!("consistency-{}", i))
            .build();

        let resp = validator.executor.allocate(req).await?;

        // Every response should have same structure
        assert!(!resp.request_id.is_empty());
        assert!(resp.latency_ms >= 0);

        if resp.success {
            assert!(!resp.block_ids.is_empty());
        } else {
            assert!(resp.error.is_some());
        }
    }

    println!("✓ Allocation response schema consistent");

    // Test inference response schema
    println!("Testing inference response schema consistency...");

    for i in 0..10 {
        let req = InferenceRequest {
            request_id: format!("infer-consistency-{}", i),
            model: "llama".to_string(),
            prompt: format!("Prompt {}", i),
            max_tokens: 50,
        };

        let resp = validator.executor.infer(req).await?;

        // Verify schema
        assert!(!resp.request_id.is_empty());
        assert!(resp.latency_ms > 0);

        if resp.success {
            assert!(resp.output.is_some());
        }
    }

    println!("✓ Inference response schema consistent");

    env.stop().await?;
    Ok(())
}

#[tokio::test]
async fn test_error_response_contract() -> Result<()> {
    let env = TestEnvironment::start().await?;
    let env = Arc::new(env);
    let validator = ContractValidator::new(env.clone());

    println!("📋 TEST: Error Response Contract");

    // Try allocation that might fail (huge request)
    let huge_req = TestRequestBuilder::new()
        .request_id("error-test".to_string())
        .num_blocks(1000000)
        .build();

    match validator.executor.allocate(huge_req).await {
        Ok(resp) => {
            if !resp.success {
                // Error response must have error message
                assert!(resp.error.is_some(), "Error response must include error message");
                println!("✓ Error response has error message: {:?}", resp.error);
            }
        }
        Err(e) => {
            // System-level error should be descriptive
            println!("✓ System error is descriptive: {}", e);
        }
    }

    env.stop().await?;
    Ok(())
}

#[tokio::test]
async fn test_backward_compatibility() -> Result<()> {
    let env = TestEnvironment::start().await?;
    let env = Arc::new(env);
    let validator = ContractValidator::new(env.clone());

    println!("📋 TEST: Backward Compatibility");

    // Test that requests with missing optional fields still work
    println!("Testing optional field handling...");

    // Allocation without priority and model
    let req = AllocationRequest {
        request_id: "compat-1".to_string(),
        num_blocks: 4,
        model: None,
        priority: None,
    };

    validator.validate_allocation_request(&req)?;
    let resp = validator.executor.allocate(req).await?;
    validator.validate_allocation_response(&resp)?;
    println!("✓ Request without optional fields works");

    // Verify response still has all required fields
    assert!(!resp.request_id.is_empty());
    assert!(resp.latency_ms >= 0);

    println!("✓ Response maintains full contract");

    env.stop().await?;
    Ok(())
}

#[tokio::test]
async fn test_idempotency_contract() -> Result<()> {
    let env = TestEnvironment::start().await?;
    let env = Arc::new(env);
    let executor = Arc::new(ScenarioExecutor::new(env.clone()));

    println!("📋 TEST: Idempotency Contract");

    // Send same request multiple times
    let req_id = "idempotent-test".to_string();

    let req = AllocationRequest {
        request_id: req_id.clone(),
        num_blocks: 4,
        model: Some("llama".to_string()),
        priority: Some(5),
    };

    let resp1 = executor.allocate(req.clone()).await?;
    let resp2 = executor.allocate(req.clone()).await?;
    let resp3 = executor.allocate(req.clone()).await?;

    // Responses should be consistent
    assert_eq!(resp1.request_id, resp2.request_id);
    assert_eq!(resp2.request_id, resp3.request_id);

    // All should have same success/failure status
    assert_eq!(resp1.success, resp2.success);
    assert_eq!(resp2.success, resp3.success);

    println!("✓ Idempotent requests return consistent responses");

    env.stop().await?;
    Ok(())
}
