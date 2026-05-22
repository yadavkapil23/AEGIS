# Mock Backend Guide

## ⚠️ WARNING: TEST ONLY

The **mock backend** generates **FAKE tokens** and is for testing purposes only.

**DO NOT USE IN PRODUCTION**

## When to Use Mock Backend

### ✅ USE FOR:
- Unit testing
- Integration testing
- Development and debugging
- CI/CD pipelines (no real model needed)
- Load testing without infrastructure
- Demonstration/proof-of-concept
- Teaching/training

### ❌ DO NOT USE FOR:
- Production inference
- Real customer queries
- Benchmarking actual performance
- Expecting real AI responses

## Configuration

```yaml
mock:
  enabled: true  # Set to false for production
  models:
    - mock-model-7b
    - mock-model-13b
  simulated_latency_ms: 100  # Simulate network delay
  simulate_failures: false
  failure_rate: 0.0
```

## Usage Examples

### Basic Usage

```rust
use inference_backends::prelude::*;
use inference_backends::mock::MockConfig;

// Create config
let mock_config = MockConfig {
    enabled: true,
    simulated_latency_ms: 100,
    ..Default::default()
};

// Create backend
let mock = MockBackend::new(mock_config);

// Execute (generates fake tokens)
let request = InferenceRequest::new("mock-model", "Hello world");
let response = mock.infer(request).await?;

println!("Fake response: {}", response.text);
println!("Backend: {}", response.backend_used); // "mock (test only)"
```

### Testing with Fallback

```rust
// Use mock in tests, real backends in production
let config = if cfg!(test) {
    BackendConfig {
        mock: Some(MockConfig::default()),
        ..Default::default()
    }
} else {
    // Real backends
    BackendConfig {
        vllm: Some(VLLMConfig::default()),
        huggingface: Some(HuggingFaceConfig::default()),
        ..Default::default()
    }
};

let router = BackendRouter::new(config).await?;
```

### Simulating Failures

```rust
let config = MockConfig {
    enabled: true,
    simulate_failures: true,
    failure_rate: 0.1, // 10% of requests fail
    ..Default::default()
};

// Use for testing retry logic and resilience patterns
let mock = MockBackend::new(config);
```

### Load Testing

```rust
// Generate load without needing real infrastructure
let config = MockConfig {
    enabled: true,
    simulated_latency_ms: 50, // Simulate 50ms latency
    ..Default::default()
};

let mock = MockBackend::new(config);

// Spawn 1000 concurrent requests
let mut handles = vec![];
for _ in 0..1000 {
    let mock = Arc::new(mock);
    let handle = tokio::spawn(async move {
        let request = InferenceRequest::new("mock-model", "test");
        let _ = mock.infer(request).await;
    });
    handles.push(handle);
}

// Wait for all to complete
for handle in handles {
    let _ = handle.await;
}
```

## Features

### Configurable Latency

Simulate network/processing delays:

```rust
let config = MockConfig {
    simulated_latency_ms: 1000, // 1 second
    ..Default::default()
};
```

### Failure Simulation

Test error handling and resilience:

```rust
let config = MockConfig {
    simulate_failures: true,
    failure_rate: 0.2, // 20% fail
    ..Default::default()
};
```

### Health Checks

Mock backend reports healthy status:

```rust
let health = mock.health_check().await?;
assert!(health.healthy);
assert_eq!(health.backend, "mock (TEST ONLY)");
```

### Model Support

Mock "supports" configured models:

```rust
let config = MockConfig {
    models: vec![
        "mock-model-7b".to_string(),
        "mock-model-13b".to_string(),
    ],
    ..Default::default()
};

assert!(mock.supports_model("mock-model").await?);
```

## Test Examples

### Unit Test

```rust
#[tokio::test]
async fn test_with_mock_backend() {
    let config = MockConfig::default();
    let mock = MockBackend::new(config);
    
    let request = InferenceRequest::new("mock-model", "test");
    let response = mock.infer(request).await.unwrap();
    
    assert!(!response.text.is_empty());
    assert!(response.backend_used.contains("mock"));
}
```

### Integration Test

```rust
#[tokio::test]
async fn test_router_with_mock() {
    let config = BackendConfig {
        mock: Some(MockConfig::default()),
        ..Default::default()
    };
    
    let router = BackendRouter::new(config).await.unwrap();
    
    let request = InferenceRequest::new("mock-model", "test");
    let response = router.infer(request).await.unwrap();
    
    assert!(response.text.contains("Mock response"));
}
```

### Resilience Testing

```rust
#[tokio::test]
async fn test_retry_with_mock_failures() {
    use resilience::prelude::*;
    
    let mock_config = MockConfig {
        enabled: true,
        simulate_failures: true,
        failure_rate: 0.5, // 50% fail
        ..Default::default()
    };
    
    let mock = MockBackend::new(mock_config);
    let retry = RetryHandler::new(RetryConfig::default());
    
    // Retries should eventually succeed
    let result = retry.execute(|| async {
        mock.infer(InferenceRequest::new("mock-model", "test")).await
    }).await;
    
    assert!(result.is_ok());
}
```

## CI/CD Usage

### GitHub Actions Example

```yaml
name: Tests
on: [push]

jobs:
  test:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v2
      - uses: actions-rs/toolchain@v1
      - run: cargo test -p inference-backends
        env:
          # Tests automatically use mock backend
          MOCK_BACKEND_ENABLED: "true"
```

No need to configure real backends in CI!

## Performance Characteristics

- **Latency**: Configurable (default 100ms)
- **Throughput**: Unlimited (no I/O)
- **CPU**: Minimal
- **Memory**: Minimal
- **Network**: None

## Output Format

Mock responses look like:

```
Mock response to: 'Hello world'

The quick brown fox jumps over the lazy dog
Rust is a systems programming language that runs
blazingly fast and prevents segfaults Machine learning
inference is the process of using a trained model
to make predictions...
```

⚠️ **These are NOT real AI responses!**

## Switching to Real Backends

When ready for production, switch config:

```rust
// Development
let config = BackendConfig {
    mock: Some(MockConfig::default()),
    ..Default::default()
};

// Production (same code, different config)
let config = BackendConfig {
    vllm: Some(VLLMConfig::default()),
    huggingface: Some(HuggingFaceConfig::default()),
    ..Default::default()
};
```

## Important Notes

1. **Responses are fake** - Don't use for real use cases
2. **Log warnings** - Look for "⚠️" in logs when mock is used
3. **Health status says "TEST ONLY"** - Makes it obvious
4. **No API calls** - Completely local, no external dependencies
5. **Deterministic** - Same input generates similar (not identical) output

## Troubleshooting

### Mock responses are too short
Increase `max_tokens` in request:
```rust
request.with_max_tokens(500)
```

### Want different latency
Configure `simulated_latency_ms`:
```yaml
mock:
  simulated_latency_ms: 500  # 500ms instead of 100ms
```

### Want to test failures
Enable failure simulation:
```yaml
mock:
  simulate_failures: true
  failure_rate: 0.2  # 20% fail
```

## Best Practices

1. **Use only in test code** - Never in production
2. **Set realistic latency** - Match your production expectations
3. **Test failure paths** - Use failure simulation
4. **Clear documentation** - Mark test code that uses mock
5. **Switch configs by environment** - Dev uses mock, prod uses real

## Summary

**Mock backend is perfect for:**
- ✅ Testing without infrastructure
- ✅ CI/CD pipelines
- ✅ Load testing the system
- ✅ Verifying resilience patterns
- ✅ Development and debugging

**But remember:**
- ⚠️ Generates FAKE tokens
- ⚠️ Never use in production
- ⚠️ Responses are not real AI
- ⚠️ Only for testing!

---

**Status**: ✅ Ready for Testing  
**Use Case**: Development, Testing, CI/CD only  
**Production**: ❌ DO NOT USE
