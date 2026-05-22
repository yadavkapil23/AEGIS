# AEGIS Observability Integration Guide

Step-by-step guide for integrating observability (metrics, tracing, health probes) into inference-backends and resilience modules.

## Table of Contents

1. [Setup](#setup)
2. [Backend Integration](#backend-integration)
3. [Resilience Integration](#resilience-integration)
4. [API Gateway Integration](#api-gateway-integration)
5. [Health Probes](#health-probes)
6. [Complete Example](#complete-example)

## Setup

### Step 1: Add observability dependency

```toml
# inference-backends/Cargo.toml and resilience/Cargo.toml
[dependencies]
observability = { path = "../observability" }
```

### Step 2: Initialize observability at startup

```rust
// In main.rs or application startup
use observability::{init_tracing, HealthManager, TracingConfig, METRICS};

#[tokio::main]
async fn main() -> Result<()> {
    // Initialize tracing
    let tracing_config = TracingConfig {
        log_level: "info".to_string(),
        json_format: true,
        jaeger_endpoint: Some("http://localhost:6831".to_string()),
    };
    init_tracing(&tracing_config);

    // Initialize health manager
    let health = HealthManager::new();
    
    // Start background services (backends, resilience, etc.)
    // ...
    
    // Mark as ready
    health.mark_backends_ready();
    health.mark_inference_ready();

    // Expose health endpoints
    expose_health_endpoints(&health).await?;
    
    // Expose metrics endpoint
    expose_metrics_endpoint().await?;
    
    Ok(())
}
```

## Backend Integration

### HuggingFace Backend

Add metrics recording to src/huggingface.rs:

```rust
use observability::METRICS;
use std::time::Instant;

impl InferenceBackend for HuggingFaceBackend {
    async fn infer(&self, request: InferenceRequest) -> Result<InferenceResponse> {
        let start = Instant::now();
        
        tracing::debug!(
            model = %request.model,
            backend = "hf-api",
            "Starting HuggingFace inference"
        );

        match self.execute_inference(&request).await {
            Ok(response) => {
                let latency_ms = start.elapsed().as_millis() as f64;
                
                METRICS.record_inference_request("hf-api", latency_ms);
                METRICS.record_backend_health("hf-api", 1.0);

                tracing::info!(
                    backend = "hf-api",
                    model = %request.model,
                    latency_ms = latency_ms,
                    tokens = response.tokens.len(),
                    "Inference completed"
                );

                Ok(response)
            }
            Err(e) => {
                METRICS.record_inference_error("hf-api");
                METRICS.record_backend_health("hf-api", 0.0);

                tracing::error!(
                    backend = "hf-api",
                    error = %e,
                    "Inference failed"
                );

                Err(e)
            }
        }
    }

    async fn health_check(&self) -> Result<bool> {
        let start = Instant::now();
        
        let healthy = self.check_health().await?;
        let latency_ms = start.elapsed().as_millis() as f64;

        METRICS.record_backend_health("hf-api", if healthy { 1.0 } else { 0.0 });

        Ok(healthy)
    }
}
```

### vLLM Backend

Add metrics recording to src/vllm.rs:

```rust
use observability::METRICS;
use std::time::Instant;

impl InferenceBackend for VLLMBackend {
    async fn infer(&self, request: InferenceRequest) -> Result<InferenceResponse> {
        let start = Instant::now();
        let endpoint = self.select_endpoint()?; // Round-robin/least-loaded
        
        tracing::debug!(
            model = %request.model,
            backend = "vllm",
            endpoint = %endpoint,
            "Starting vLLM inference"
        );

        match self.execute_inference(&request, &endpoint).await {
            Ok(response) => {
                let latency_ms = start.elapsed().as_millis() as f64;
                
                METRICS.record_inference_request("vllm", latency_ms);
                METRICS.record_backend_health("vllm", 1.0);

                tracing::info!(
                    backend = "vllm",
                    endpoint = %endpoint,
                    latency_ms = latency_ms,
                    tokens = response.tokens.len(),
                    "Inference completed"
                );

                Ok(response)
            }
            Err(e) => {
                METRICS.record_inference_error("vllm");
                METRICS.record_backend_health("vllm", 0.0);

                tracing::error!(
                    backend = "vllm",
                    endpoint = %endpoint,
                    error = %e,
                    "Inference failed"
                );

                Err(e)
            }
        }
    }
}
```

### Llama.cpp Backend

Add metrics recording to src/llamacpp.rs:

```rust
use observability::METRICS;
use std::time::Instant;

impl InferenceBackend for LlamaCppBackend {
    async fn infer(&self, request: InferenceRequest) -> Result<InferenceResponse> {
        let start = Instant::now();
        
        tracing::debug!(
            model = %request.model,
            backend = "llamacpp",
            "Starting llama.cpp inference"
        );

        match self.execute_inference(&request).await {
            Ok(response) => {
                let latency_ms = start.elapsed().as_millis() as f64;
                
                METRICS.record_inference_request("llamacpp", latency_ms);
                METRICS.record_backend_health("llamacpp", 1.0);

                tracing::info!(
                    backend = "llamacpp",
                    latency_ms = latency_ms,
                    tokens = response.tokens.len(),
                    "Inference completed"
                );

                Ok(response)
            }
            Err(e) => {
                METRICS.record_inference_error("llamacpp");
                METRICS.record_backend_health("llamacpp", 0.0);

                tracing::error!(
                    backend = "llamacpp",
                    error = %e,
                    "Inference failed"
                );

                Err(e)
            }
        }
    }
}
```

## Resilience Integration

### Circuit Breaker

Add metrics recording to resilience/src/circuit_breaker.rs:

```rust
use observability::METRICS;

impl CircuitBreaker {
    async fn call<F, T>(&self, f: F) -> Result<T>
    where
        F: FnOnce() -> BoxFuture<'static, Result<T>>,
    {
        match self.state {
            State::Closed => {
                match f().await {
                    Ok(result) => {
                        // Reset failure count on success
                        self.consecutive_failures.store(0, Ordering::SeqCst);
                        Ok(result)
                    }
                    Err(e) => {
                        let failures = self.consecutive_failures.fetch_add(1, Ordering::SeqCst) + 1;
                        
                        METRICS.record_circuit_breaker_failure(&self.name);
                        
                        tracing::warn!(
                            circuit = %self.name,
                            failures = failures,
                            threshold = self.failure_threshold,
                            "Circuit breaker failure"
                        );

                        if failures >= self.failure_threshold {
                            self.state.store(State::Open, Ordering::SeqCst);
                            METRICS.record_circuit_breaker_state(&self.name, 1); // 1 = Open
                            
                            tracing::error!(
                                circuit = %self.name,
                                "Circuit breaker opened"
                            );
                        }

                        Err(e)
                    }
                }
            }
            State::Open => {
                // Check timeout for HalfOpen transition
                let elapsed = self.opened_at.elapsed();
                if elapsed >= self.timeout {
                    self.state.store(State::HalfOpen, Ordering::SeqCst);
                    METRICS.record_circuit_breaker_state(&self.name, 2); // 2 = HalfOpen
                    
                    tracing::info!(
                        circuit = %self.name,
                        "Circuit breaker half-open"
                    );
                    
                    f().await // Retry in HalfOpen state
                } else {
                    tracing::warn!(
                        circuit = %self.name,
                        "Circuit breaker open, rejecting request"
                    );
                    Err(anyhow!("Circuit breaker is open"))
                }
            }
            State::HalfOpen => {
                match f().await {
                    Ok(result) => {
                        self.state.store(State::Closed, Ordering::SeqCst);
                        self.consecutive_failures.store(0, Ordering::SeqCst);
                        METRICS.record_circuit_breaker_state(&self.name, 0); // 0 = Closed
                        
                        tracing::info!(
                            circuit = %self.name,
                            "Circuit breaker closed"
                        );
                        
                        Ok(result)
                    }
                    Err(e) => {
                        self.state.store(State::Open, Ordering::SeqCst);
                        METRICS.record_circuit_breaker_state(&self.name, 1); // 1 = Open
                        
                        tracing::error!(
                            circuit = %self.name,
                            "Circuit breaker reopened after half-open failure"
                        );
                        
                        Err(e)
                    }
                }
            }
        }
    }
}
```

### Retry Handler

Add metrics recording to resilience/src/retry.rs:

```rust
use observability::METRICS;

impl RetryHandler {
    async fn execute_with_retries<F, T>(&self, mut f: F) -> Result<T>
    where
        F: FnMut() -> BoxFuture<'static, Result<T>>,
    {
        let mut attempt = 0;

        loop {
            attempt += 1;
            
            tracing::debug!(
                max_retries = self.max_retries,
                attempt = attempt,
                "Attempting request"
            );

            match f().await {
                Ok(result) => {
                    if attempt > 1 {
                        METRICS.record_retry_success(&self.name);
                        tracing::info!(
                            retries = attempt - 1,
                            "Request succeeded after retries"
                        );
                    }
                    return Ok(result);
                }
                Err(e) if attempt < self.max_retries => {
                    let backoff = self.calculate_backoff(attempt);
                    
                    METRICS.record_retry_attempt(&self.name);
                    
                    tracing::warn!(
                        attempt = attempt,
                        backoff_ms = backoff.as_millis(),
                        error = %e,
                        "Retrying after backoff"
                    );

                    tokio::time::sleep(backoff).await;
                }
                Err(e) => {
                    METRICS.record_retry_attempt(&self.name);
                    
                    tracing::error!(
                        attempts = attempt,
                        "Max retries exceeded"
                    );
                    
                    return Err(e);
                }
            }
        }
    }
}
```

### Timeout Handler

Add metrics recording to resilience/src/timeout.rs:

```rust
use observability::METRICS;

impl TimeoutHandler {
    async fn execute_with_timeout<F, T>(&self, future: F) -> Result<T>
    where
        F: Future<Output = Result<T>>,
    {
        tracing::debug!(
            timeout_ms = self.timeout.as_millis(),
            "Executing with timeout"
        );

        match tokio::time::timeout(self.timeout, future).await {
            Ok(Ok(result)) => Ok(result),
            Ok(Err(e)) => {
                tracing::error!(error = %e, "Request error within timeout");
                Err(e)
            }
            Err(_) => {
                METRICS.record_timeout_error();
                
                tracing::error!(
                    timeout_ms = self.timeout.as_millis(),
                    "Request timeout"
                );
                
                Err(anyhow!("Request timeout after {:?}", self.timeout))
            }
        }
    }
}
```

### Graceful Degradation

Add metrics recording to resilience/src/graceful_degradation.rs:

```rust
use observability::METRICS;

impl GracefulDegradation {
    pub fn update_health(&self, healthy: bool) {
        let new_level = if healthy {
            DegradationLevel::Healthy
        } else {
            let failures = self.failure_count.fetch_add(1, Ordering::SeqCst) + 1;
            
            if failures < self.degradation_threshold {
                DegradationLevel::Degraded
            } else {
                DegradationLevel::Critical
            }
        };

        self.level.store(new_level as u32, Ordering::SeqCst);
        
        let level_value = match new_level {
            DegradationLevel::Healthy => 0.0,
            DegradationLevel::Degraded => 1.0,
            DegradationLevel::Critical => 2.0,
        };

        METRICS.record_degradation_level(level_value);

        tracing::info!(
            level = ?new_level,
            failures = failures,
            "Service degradation level updated"
        );
    }

    pub async fn execute_with_fallback<F, T>(&self, primary: F, fallback: F) -> Result<T>
    where
        F: Fn() -> BoxFuture<'static, Result<T>>,
    {
        match self.level.load(Ordering::SeqCst) {
            level if level < 2 => {
                // Healthy or Degraded - try primary
                match primary().await {
                    Ok(result) => Ok(result),
                    Err(e) if self.level.load(Ordering::SeqCst) >= 2 => {
                        // Degraded to Critical - use fallback
                        METRICS.record_degradation_fallback_use();
                        
                        tracing::warn!(
                            "Primary failed, using fallback due to degradation"
                        );
                        
                        fallback().await
                    }
                    Err(e) => Err(e),
                }
            }
            _ => {
                // Critical - use fallback directly
                METRICS.record_degradation_fallback_use();
                
                tracing::info!("Service critical, using fallback");
                
                fallback().await
            }
        }
    }
}
```

## API Gateway Integration

### Health Check Endpoints

Add to gateway/src/routes.rs:

```rust
use observability::{HealthManager, METRICS};
use axum::{Router, routing::get, Json};

pub fn create_health_routes(health: Arc<HealthManager>) -> Router {
    Router::new()
        .route("/health/live", get({
            let h = health.clone();
            move || {
                let h = h.clone();
                async move {
                    let probe = h.get_liveness();
                    Json(probe)
                }
            }
        }))
        .route("/health/ready", get({
            let h = health.clone();
            move || {
                let h = h.clone();
                async move {
                    let probe = h.get_readiness();
                    Json(probe)
                }
            }
        }))
}
```

### Metrics Endpoint

Add to gateway/src/routes.rs:

```rust
pub fn create_metrics_route() -> Router {
    Router::new()
        .route("/metrics", get(|| async {
            let metrics = METRICS.gather();
            (
                [("Content-Type", "text/plain; version=0.0.4")],
                metrics,
            )
        }))
}
```

### Middleware for Request Tracing

Add to gateway/src/middleware.rs:

```rust
use axum::middleware::Next;
use axum::response::Response;
use std::time::Instant;

pub async fn trace_requests(
    req: axum::extract::Request,
    next: Next,
) -> Response {
    let method = req.method().clone();
    let path = req.uri().path().to_string();
    let start = Instant::now();

    let span = tracing::info_span!(
        "request",
        method = %method,
        path = %path,
    );

    let response = span.in_scope(|| {
        let fut = next.run(req);
        std::pin::pin!(fut)
    }).await;

    let latency_ms = start.elapsed().as_millis() as f64;
    let status = response.status().as_u16();

    tracing::info!(
        status = status,
        latency_ms = latency_ms,
        "Request completed"
    );

    response
}
```

## Health Probes

### Implementing Custom Health Checks

```rust
use observability::HealthManager;

pub struct ApplicationHealth {
    health: Arc<HealthManager>,
    backends: Arc<Vec<Arc<dyn InferenceBackend>>>,
}

impl ApplicationHealth {
    pub async fn check_backends(&self) {
        let mut all_ready = true;

        for backend in self.backends.iter() {
            match backend.health_check().await {
                Ok(healthy) if !healthy => {
                    all_ready = false;
                    self.health.mark_backends_not_ready();
                    break;
                }
                Err(_) => {
                    all_ready = false;
                    self.health.mark_backends_not_ready();
                    break;
                }
                _ => {}
            }
        }

        if all_ready {
            self.health.mark_backends_ready();
        }
    }

    pub async fn check_inference_engine(&self) {
        // Custom check for inference engine
        if self.engine_is_initialized().await {
            self.health.mark_inference_ready();
        }
    }
}
```

## Complete Example

Full integration in main.rs:

```rust
use axum::{Router, routing::post};
use observability::{init_tracing, HealthManager, TracingConfig, METRICS};
use std::sync::Arc;
use inference_backends::{BackendRouter, VLLMBackend, HuggingFaceBackend};
use resilience::{CircuitBreaker, RetryHandler, TimeoutHandler};

#[tokio::main]
async fn main() -> Result<()> {
    // Initialize observability
    let tracing_config = TracingConfig {
        log_level: "info".to_string(),
        json_format: true,
        jaeger_endpoint: Some("http://localhost:6831".to_string()),
    };
    init_tracing(&tracing_config);

    let health = Arc::new(HealthManager::new());

    // Initialize backends with resilience
    let vllm = Arc::new(VLLMBackend::new(...));
    let hf = Arc::new(HuggingFaceBackend::new(...));

    let circuit_breaker = Arc::new(CircuitBreaker::new("vllm", 5, Duration::from_secs(30)));
    let retry_handler = Arc::new(RetryHandler::new("vllm", 3, Duration::from_millis(100)));
    let timeout_handler = Arc::new(TimeoutHandler::new(Duration::from_secs(30)));

    // Backend router with fallback
    let router = Arc::new(BackendRouter::new(vec![vllm.clone(), hf.clone()]));

    // Mark as ready
    health.mark_backends_ready();
    health.mark_inference_ready();

    // Create API routes
    let app = Router::new()
        .route("/infer", post(infer_handler))
        .route("/health/live", get(liveness_handler))
        .route("/health/ready", get(readiness_handler))
        .route("/metrics", get(metrics_handler))
        .with_state((router, health));

    // Start server
    let listener = tokio::net::TcpListener::bind("0.0.0.0:8000").await?;
    axum::serve(listener, app).await?;

    Ok(())
}

async fn infer_handler(
    axum::extract::State((router, health)): axum::extract::State<(Arc<BackendRouter>, Arc<HealthManager>)>,
    Json(request): Json<InferenceRequest>,
) -> Result<Json<InferenceResponse>> {
    let start = Instant::now();

    let response = router.route(&request).await?;
    
    let latency_ms = start.elapsed().as_millis() as f64;
    METRICS.record_inference_request("router", latency_ms);

    Ok(Json(response))
}
```

## Verification Checklist

- [ ] Observability dependency added to Cargo.toml files
- [ ] init_tracing() called at application startup
- [ ] HealthManager created and exposed via endpoints
- [ ] METRICS recorded in all backend implementations
- [ ] Circuit breaker metrics recorded on state changes
- [ ] Retry metrics recorded on attempts and successes
- [ ] Timeout metrics recorded on timeouts
- [ ] Degradation metrics recorded on level changes
- [ ] Health endpoints returning correct JSON
- [ ] Metrics endpoint returning Prometheus format
- [ ] Tracing logs appearing in stdout/Jaeger
- [ ] Request latencies measured and recorded
- [ ] Error cases triggering appropriate metrics
- [ ] Health checks running periodically
- [ ] Prometheus scraping metrics successfully

## Testing

```bash
# Check metrics endpoint
curl http://localhost:8000/metrics | grep aegis_inference

# Check health endpoints
curl http://localhost:8000/health/live
curl http://localhost:8000/health/ready

# Watch logs
tail -f /var/log/aegis/observability.log | jq .

# Query Prometheus
curl 'http://localhost:9090/api/v1/query?query=aegis_inference_requests_total'
```
