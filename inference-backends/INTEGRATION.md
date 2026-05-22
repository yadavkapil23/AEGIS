# Integration Guide: Inference Backends with AEGIS

This guide explains how to integrate the dual-backend inference system (HF API + vLLM) into the AEGIS distributed inference scheduler.

## Architecture Overview

```
┌─────────────────────────────────────┐
│  AEGIS Gateway (Layer 5)            │
│  - API endpoints                    │
│  - Request validation               │
└──────────────┬──────────────────────┘
               │
┌──────────────▼──────────────────────┐
│  AEGIS Scheduler (Layer 4)          │
│  - Consensus coordination           │
│  - Distributed scheduling           │
└──────────────┬──────────────────────┘
               │
┌──────────────▼──────────────────────┐
│  BackendRouter (This Module)        │  ← You are here
│  - Dual backend support            │
│  - Intelligent routing             │
│  - Fallback & health checks        │
└──────────────┬──────────────────────┘
               │
        ┌──────┴─────────┐
        │                │
    ┌───▼───┐         ┌──▼────┐
    │  HF   │         │ vLLM  │
    │  API  │         │Cluster│
    └───────┘         └───────┘
```

## Integration Steps

### 1. Add Dependency to scheduler/Cargo.toml

```toml
[dependencies]
inference-backends = { path = "../inference-backends" }
```

### 2. Create Backend Module in Scheduler

Create `scheduler/src/inference_layer.rs`:

```rust
use inference_backends::prelude::*;
use std::sync::Arc;

/// Inference layer for AEGIS
pub struct InferenceLayer {
    router: Arc<BackendRouter>,
}

impl InferenceLayer {
    /// Initialize the inference layer
    pub async fn new(config_path: Option<&str>) -> anyhow::Result<Self> {
        let config = BackendConfig::load_config(config_path)
            .map_err(|e| anyhow::anyhow!("Failed to load backend config: {}", e))?;

        let router = BackendRouter::new(config).await
            .map_err(|e| anyhow::anyhow!("Failed to initialize router: {}", e))?;

        router.warmup().await.ok();

        Ok(Self {
            router: Arc::new(router),
        })
    }

    /// Execute inference request
    pub async fn infer(&self, request: InferenceRequest) -> anyhow::Result<InferenceResponse> {
        self.router
            .infer(request)
            .await
            .map_err(|e| anyhow::anyhow!("Inference failed: {}", e))
    }

    /// Check backend health
    pub async fn health_check(&self) -> anyhow::Result<Vec<HealthStatus>> {
        self.router
            .health_check()
            .await
            .map_err(|e| anyhow::anyhow!("Health check failed: {}", e))
    }

    /// Get available models
    pub async fn get_models(&self) -> anyhow::Result<Vec<String>> {
        self.router
            .get_available_models()
            .await
            .map_err(|e| anyhow::anyhow!("Failed to get models: {}", e))
    }
}
```

### 3. Integrate into Scheduler State

In `scheduler/src/lib.rs`:

```rust
pub mod inference_layer;

use inference_layer::InferenceLayer;

pub struct SchedulerState {
    // ... existing fields ...
    pub inference_layer: InferenceLayer,
}

impl SchedulerState {
    pub async fn new(config_path: Option<&str>) -> anyhow::Result<Self> {
        // ... existing initialization ...

        let inference_layer = InferenceLayer::new(config_path).await?;

        Ok(Self {
            // ... existing fields ...
            inference_layer,
        })
    }
}
```

### 4. Add Inference Request Handler

In `scheduler/src/handlers.rs`:

```rust
use inference_backends::prelude::*;

/// Handle inference request from client
pub async fn handle_inference_request(
    state: &SchedulerState,
    request: InferenceRequest,
) -> anyhow::Result<InferenceResponse> {
    // 1. Consensus layer routes request
    // (existing AEGIS logic)

    // 2. Route to appropriate backend
    let response = state.inference_layer.infer(request).await?;

    // 3. Update metrics
    // (existing AEGIS logging)

    Ok(response)
}

/// Get inference backend status
pub async fn get_backend_status(
    state: &SchedulerState,
) -> anyhow::Result<Vec<HealthStatus>> {
    state.inference_layer.health_check().await
}

/// Get available models
pub async fn get_available_models(
    state: &SchedulerState,
) -> anyhow::Result<Vec<String>> {
    state.inference_layer.get_models().await
}
```

### 5. Configuration Setup

Create `backends_config.yaml` in project root:

```yaml
default_preference: auto
fallback_order:
  - vllm
  - huggingface
default_timeout_ms: 30000
enable_health_checks: true
health_check_interval_secs: 60

huggingface:
  enabled: true
  api_key: null  # Uses HF_API_KEY env var
  endpoint: https://api-inference.huggingface.co
  models:
    - mistralai/Mistral-7B-Instruct-v0.2
    - meta-llama/Llama-2-7b-hf
  timeout_ms: 30000
  max_concurrent_requests: 100
  enable_cache: true
  cache_ttl_secs: 3600

vllm:
  enabled: true
  endpoints:
    - http://vllm-node-1:8000
    - http://vllm-node-2:8000
    - http://vllm-node-3:8000
  models:
    - mistralai/Mistral-7B-Instruct-v0.2
    - meta-llama/Llama-2-7b-hf
  timeout_ms: 30000
  max_concurrent_requests: 1000
  load_balancing: least_loaded
  enable_connection_pooling: true
  pool_size: 10
```

### 6. Docker Compose Integration

Update `docker-compose.yml`:

```yaml
version: '3.8'

services:
  # vLLM inference nodes
  vllm-node-1:
    image: vllm/vllm-openai:latest
    environment:
      MODEL: mistralai/Mistral-7B-Instruct-v0.2
      CUDA_VISIBLE_DEVICES: 0
    ports:
      - "8001:8000"
    volumes:
      - vllm-cache:/root/.cache
    deploy:
      resources:
        reservations:
          devices:
            - driver: nvidia
              count: 1
              capabilities: [gpu]

  vllm-node-2:
    image: vllm/vllm-openai:latest
    environment:
      MODEL: mistralai/Mistral-7B-Instruct-v0.2
      CUDA_VISIBLE_DEVICES: 1
    ports:
      - "8002:8000"
    volumes:
      - vllm-cache:/root/.cache
    deploy:
      resources:
        reservations:
          devices:
            - driver: nvidia
              count: 1
              capabilities: [gpu]

  # AEGIS scheduler with backends configured
  aegis-scheduler:
    build:
      context: .
      dockerfile: Dockerfile
    environment:
      HF_API_KEY: ${HF_API_KEY}
      RUST_LOG: info
    ports:
      - "6000:6000"
      - "8000:8000"
    volumes:
      - ./backends_config.yaml:/app/config.yaml
    depends_on:
      - vllm-node-1
      - vllm-node-2
    command: /app/aegis-scheduler --config /app/config.yaml

volumes:
  vllm-cache:
```

### 7. Environment Setup

Create `.env`:

```bash
# Hugging Face API key
HF_API_KEY=hf_your_token_here

# vLLM endpoints
VLLM_NODE_1=http://vllm-node-1:8000
VLLM_NODE_2=http://vllm-node-2:8000
VLLM_NODE_3=http://vllm-node-3:8000

# Logging
RUST_LOG=debug
```

## Usage in AEGIS

### Client Request Flow

```rust
// 1. Client sends inference request
let request = InferenceRequest::new(
    "mistralai/Mistral-7B-Instruct-v0.2",
    "Explain quantum computing"
)
.with_backend(BackendPreference::Auto)
.with_max_tokens(500);

// 2. AEGIS Gateway validates & routes
// 3. Scheduler reaches consensus on processing
// 4. Backend Router selects best backend
// 5. Inference executed on vLLM or HF API
// 6. Response returned through consensus layer
// 7. Client receives result
```

### Health Monitoring

The inference layer integrates with AEGIS health checks:

```rust
// In existing health check endpoint
pub async fn health_handler(state: &SchedulerState) -> Json<HealthResponse> {
    let mut response = HealthResponse::default();

    // Add backend health
    if let Ok(statuses) = state.inference_layer.health_check().await {
        for status in statuses {
            response.backends.push(status);
        }
    }

    Json(response)
}
```

### Metrics Integration

Add to existing Prometheus metrics:

```rust
// In metrics collection
pub fn collect_inference_metrics(layer: &InferenceLayer) {
    // This would integrate with existing AEGIS metrics
    // Track per-backend performance
    // Monitor fallback frequency
}
```

## Deployment Scenarios

### Scenario 1: Development (HF API Only)

```yaml
vllm:
  enabled: false

huggingface:
  enabled: true
  api_key: ${HF_API_KEY}
  # Quick testing with managed service
```

### Scenario 2: Production (Balanced)

```yaml
vllm:
  enabled: true
  endpoints:
    - http://vllm-node-1:8000
    - http://vllm-node-2:8000
    - http://vllm-node-3:8000
  load_balancing: least_loaded

huggingface:
  enabled: true
  # Fallback for high load or node failures
```

### Scenario 3: Cost Optimized (vLLM Primary)

```yaml
default_preference: auto
fallback_order:
  - vllm  # Primary
  - huggingface  # Fallback only

vllm:
  enabled: true
  endpoints: [...]

huggingface:
  enabled: true
  # Only used during vLLM outages
```

## Monitoring Dashboard

Track these metrics in Grafana:

```
- aegis_inference_requests_total (by backend)
- aegis_inference_latency_ms (p50, p95, p99)
- aegis_backend_healthy (0/1 for each backend)
- aegis_fallback_count (how often fallback used)
- aegis_vllm_active_requests
- aegis_hf_cache_hit_rate
```

## Troubleshooting Integration

### vLLM endpoints unreachable

```bash
# Check endpoint health
curl http://vllm-node-1:8000/health

# Check AEGIS logs
docker logs aegis-scheduler | grep vllm
```

### HF API failures

```bash
# Verify API key
echo $HF_API_KEY

# Test HF API manually
curl -H "Authorization: Bearer $HF_API_KEY" \
  https://api-inference.huggingface.co/models/mistralai/Mistral-7B-Instruct-v0.2
```

### Routing not working

```bash
# Check backend router logs
RUST_LOG=debug cargo run

# Verify backends are registered
curl http://localhost:8000/health
```

## Performance Tuning

### vLLM Settings

```yaml
vllm:
  load_balancing: least_loaded  # Better than round_robin
  pool_size: 20  # More connections for high throughput
  max_concurrent_requests: 5000  # Increase for bursty loads
```

### HF API Settings

```yaml
huggingface:
  cache_ttl_secs: 7200  # Longer cache for repeated prompts
  enable_cache: true
  max_concurrent_requests: 200
```

## Next Steps

1. Configure `backends_config.yaml` for your environment
2. Deploy vLLM nodes (if using self-hosted)
3. Set `HF_API_KEY` environment variable
4. Run integration tests
5. Monitor metrics in production
6. Adjust load balancing based on performance

---

**Integration Status**: Ready for Production  
**Last Updated**: 2026-05-22
