# Inference Backends Module

Dual-backend inference system with support for **Hugging Face API** and **vLLM**, with automatic routing, health checking, and intelligent fallback.

## Features

- ✅ **Dual Backend Support**: Hugging Face API (cloud) + vLLM (self-hosted)
- ✅ **Intelligent Routing**: Auto-select best backend based on heuristics
- ✅ **Fallback Logic**: Automatic failover between backends
- ✅ **Health Checking**: Continuous monitoring of backend availability
- ✅ **Load Balancing**: Round-robin, least-loaded, or random distribution
- ✅ **Response Caching**: HF API response caching with TTL
- ✅ **Performance Metrics**: Track latency, throughput, and errors

## Architecture

```
┌────────────────────────┐
│  BackendRouter         │  Main coordinator
└─────────┬──────────────┘
          │
    ┌─────┴─────┐
    │           │
    ▼           ▼
┌─────────────┐ ┌──────────────┐
│ HFBackend   │ │ VLLMBackend  │
│ (Cloud API) │ │ (Cluster)    │
└─────────────┘ └──────────────┘
```

## Usage

### Basic Setup

```rust
use inference_backends::prelude::*;

#[tokio::main]
async fn main() -> Result<()> {
    // Load configuration
    let config = BackendConfig::load_config(Some("config.yaml"))?;

    // Create router with all backends
    let router = BackendRouter::new(config).await?;

    // Warmup backends
    router.warmup().await?;

    // Create inference request
    let request = InferenceRequest::new(
        "mistralai/Mistral-7B-Instruct-v0.2",
        "Hello, how are you?"
    )
    .with_max_tokens(100)
    .with_backend(BackendPreference::Auto);

    // Execute inference (auto-routes to best backend)
    let response = router.infer(request).await?;

    println!("Response: {}", response.text);
    println!("Backend used: {}", response.backend_used);
    println!("Latency: {}ms", response.processing_time_ms);

    Ok(())
}
```

### Configuration

Create `backends_config.yaml`:

```yaml
default_preference: auto
fallback_order:
  - vllm      # Try vLLM first (faster, self-hosted)
  - huggingface  # Fall back to HF API

huggingface:
  enabled: true
  api_key: ${HF_API_KEY}  # From environment
  models:
    - mistralai/Mistral-7B-Instruct-v0.2

vllm:
  enabled: true
  endpoints:
    - http://vllm-node-1:8000
    - http://vllm-node-2:8000
  load_balancing: least_loaded
```

### Routing Strategies

#### 1. Auto Mode (Recommended)
```rust
request.with_backend(BackendPreference::Auto)
```
The router decides:
- Prefers vLLM for latency-sensitive requests (timeout < 5s)
- Falls back to HF API if vLLM unavailable
- Respects configured fallback order

#### 2. Backend-Specific
```rust
// Always use vLLM
request.with_backend(BackendPreference::VLLm)

// Always use Hugging Face
request.with_backend(BackendPreference::HuggingFace)
```

### Load Balancing

vLLM supports three strategies:

```yaml
vllm:
  load_balancing: round_robin    # Cycle through endpoints
  # or
  load_balancing: least_loaded   # Send to least busy endpoint
  # or
  load_balancing: random         # Random endpoint selection
```

### Health Monitoring

```rust
// Check health of all backends
let statuses = router.health_check().await?;

for status in statuses {
    println!("Backend: {}", status.backend);
    println!("Healthy: {}", status.healthy);
    println!("Latency: {:.2}ms", status.latency_ms);
    println!("Requests: {}", status.request_count);
}
```

### Get Available Models

```rust
// Get all supported models
let models = router.get_available_models().await?;

// Check if specific model is supported
let supported = router.supports_model("meta-llama/Llama-2-7b-hf").await?;
```

## Deployment

### With Docker Compose

```yaml
version: '3.8'

services:
  vllm-1:
    image: vllm/vllm-openai:latest
    environment:
      MODEL: mistralai/Mistral-7B-Instruct-v0.2
    ports:
      - "8001:8000"

  vllm-2:
    image: vllm/vllm-openai:latest
    environment:
      MODEL: mistralai/Mistral-7B-Instruct-v0.2
    ports:
      - "8002:8000"

  aegis:
    build: ..
    environment:
      HF_API_KEY: ${HF_API_KEY}
      VLLM_ENDPOINTS: "http://vllm-1:8000,http://vllm-2:8000"
    volumes:
      - ./backends_config.yaml:/app/config.yaml
```

### Environment Variables

```bash
# Hugging Face API key
export HF_API_KEY=hf_xxxxx

# vLLM endpoints (optional, overrides config)
export VLLM_ENDPOINTS=http://node1:8000,http://node2:8000,http://node3:8000

# Default backend preference
export DEFAULT_BACKEND=auto
```

## Error Handling

```rust
match router.infer(request).await {
    Ok(response) => {
        // Use response
    }
    Err(BackendError::ModelNotFound(model)) => {
        eprintln!("Model not available: {}", model);
    }
    Err(BackendError::AllBackendsUnavailable) => {
        eprintln!("All backends down!");
    }
    Err(e) => {
        eprintln!("Inference failed: {}", e);
    }
}
```

## Performance Characteristics

### HuggingFace API
- **Latency**: 1-5 seconds (network + inference)
- **Throughput**: Limited by API rate limits
- **Cost**: Pay per request
- **Setup**: Zero - just needs API key
- **Best for**: Development, testing, bursty loads

### vLLM
- **Latency**: 100-500ms (local cluster)
- **Throughput**: 10,000+ req/s (3-node cluster)
- **Cost**: One-time hardware investment
- **Setup**: Docker containers + config
- **Best for**: Production, high throughput, low latency

## Testing

```bash
# Run all tests
cargo test -p inference-backends

# Run with output
cargo test -p inference-backends -- --nocapture

# Test specific backend
cargo test -p inference-backends huggingface::tests::
```

## Monitoring & Metrics

The router exposes:
- Request count per backend
- Error count per backend
- Average latency per endpoint
- Active request count

Use with Prometheus for monitoring:

```prometheus
# Queries
rate(aegis_inference_requests_total[5m])
histogram_quantile(0.99, aegis_inference_latency_ms)
aegis_backend_healthy
```

## Architecture Details

### InferenceBackend Trait

All backends implement:
- `infer()` - Execute inference
- `health_check()` - Check availability
- `supports_model()` - Model support check
- `get_models()` - List available models
- `warmup()` - Optional pre-loading

### Router Decision Logic

1. **Parse request** - Extract backend preference
2. **Get backends to try** - Based on preference and health
3. **Attempt inference** - Try each backend in order
4. **Return or fallback** - Success or next backend
5. **Update health status** - Track failures

## Integration with AEGIS

The BackendRouter is the inference layer for AEGIS:

```
AEGIS Gateway
    ↓
AEGIS Scheduler (consensus layer)
    ↓
BackendRouter (this module)
    ↓
vLLM Cluster OR HF API
```

## Troubleshooting

### "All backends unavailable"
- Check HF_API_KEY is set
- Verify vLLM endpoints are reachable
- Run `router.health_check().await`

### High latency
- Use vLLM backend for low-latency needs
- Check endpoint load: `least_loaded` strategy
- Monitor network latency between nodes

### Model not found
- Check `config.yaml` includes your model
- Verify model name matches exactly
- Run `router.get_available_models().await`

## Future Enhancements

- [ ] OpenAI API backend
- [ ] LocalAI support
- [ ] Ollama integration
- [ ] Response streaming
- [ ] Advanced caching strategies
- [ ] Cost optimization (HF vs vLLM)

---

**Status**: Production Ready  
**Version**: 1.0.0  
**Maintenance**: Active
