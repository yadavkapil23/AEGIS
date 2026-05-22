/// Production Backend Manager Integration with Gateway
/// Provides resilience patterns (circuit breaker, retry, rate limiting, bulkhead)

use aegis_inference_backends::prelude::*;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use tokio::sync::RwLock;
use tracing::{info, warn, error, debug};
use serde::{Deserialize, Serialize};
use std::time::Instant;

/// Metrics tracked by the backend manager
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BackendMetrics {
    pub circuit_breaker_state: String,
    pub consecutive_failures: u32,
    pub bulkhead_available: usize,
    pub total_requests: u64,
    pub successful_requests: u64,
    pub failed_requests: u64,
    pub rate_limited_requests: u64,
    pub avg_latency_ms: f64,
}

/// Backend Manager wrapping production features
pub struct BackendManager {
    manager: Arc<ProductionBackendManager>,
    metrics: Arc<BackendManagerMetrics>,
}

/// Internal metrics structure
struct BackendManagerMetrics {
    total_requests: AtomicU64,
    successful_requests: AtomicU64,
    failed_requests: AtomicU64,
    rate_limited_requests: AtomicU64,
    latencies: RwLock<Vec<u32>>,
}

impl BackendManager {
    /// Create a new backend manager with production features
    pub async fn new(
        backend_config: BackendConfig,
        rate_limit_rps: u32,
    ) -> Result<Self, Box<dyn std::error::Error>> {
        info!("Initializing production backend manager with {} RPS limit", rate_limit_rps);

        // Create backend router
        let router = BackendRouter::new(backend_config)
            .await
            .map_err(|e| format!("Failed to create backend router: {}", e))?;

        let backend = Arc::new(router);

        // Create production manager with resilience patterns
        let manager = Arc::new(ProductionBackendManager::new(
            backend,
            CircuitBreakerConfig {
                failure_threshold: 5,
                reset_timeout_secs: 30,
                half_open_requests: 3,
            },
            RetryConfig {
                max_retries: 3,
                initial_backoff_ms: 100,
                max_backoff_ms: 5000,
                backoff_multiplier: 2.0,
            },
            rate_limit_rps,
        ));

        info!("Backend manager initialized with resilience patterns:");
        info!("  - Circuit breaker: threshold=5, timeout=30s");
        info!("  - Retry: max=3, backoff=100-5000ms");
        info!("  - Rate limiting: {} RPS", rate_limit_rps);
        info!("  - Bulkhead: 100 concurrent slots");

        Ok(Self {
            manager,
            metrics: Arc::new(BackendManagerMetrics {
                total_requests: AtomicU64::new(0),
                successful_requests: AtomicU64::new(0),
                failed_requests: AtomicU64::new(0),
                rate_limited_requests: AtomicU64::new(0),
                latencies: RwLock::new(Vec::new()),
            }),
        })
    }

    /// Execute inference with all production safeguards
    pub async fn infer(
        &self,
        request: InferenceRequest,
    ) -> Result<InferenceResponse, BackendError> {
        let start = Instant::now();
        self.metrics.total_requests.fetch_add(1, Ordering::SeqCst);

        debug!("Inference request: model={}, prompt_len={}",
               request.model, request.prompt.len());

        match self.manager.infer(request).await {
            Ok(response) => {
                self.metrics.successful_requests.fetch_add(1, Ordering::SeqCst);
                let latency = start.elapsed().as_millis() as u32;

                // Record latency
                let mut latencies = self.metrics.latencies.blocking_write();
                latencies.push(latency);
                if latencies.len() > 1000 {
                    latencies.remove(0); // Keep last 1000
                }

                info!("Inference success: {}ms, tokens={}", latency, response.tokens_generated);
                Ok(response)
            }
            Err(BackendError::RateLimited) => {
                self.metrics.rate_limited_requests.fetch_add(1, Ordering::SeqCst);
                warn!("Rate limited");
                Err(BackendError::RateLimited)
            }
            Err(BackendError::CircuitBreakerOpen) => {
                self.metrics.failed_requests.fetch_add(1, Ordering::SeqCst);
                warn!("Circuit breaker is open");
                Err(BackendError::CircuitBreakerOpen)
            }
            Err(e) => {
                self.metrics.failed_requests.fetch_add(1, Ordering::SeqCst);
                error!("Inference error: {}", e);
                Err(e)
            }
        }
    }

    /// Get current metrics
    pub async fn get_metrics(&self) -> BackendMetrics {
        let latencies = self.metrics.latencies.read().await;
        let avg_latency = if latencies.is_empty() {
            0.0
        } else {
            latencies.iter().map(|&l| l as f64).sum::<f64>() / latencies.len() as f64
        };

        let circuit_state = self.manager.circuit_breaker_state().await;
        let state_str = match circuit_state {
            aegis_inference_backends::CircuitState::Closed => "closed".to_string(),
            aegis_inference_backends::CircuitState::Open => "open".to_string(),
            aegis_inference_backends::CircuitState::HalfOpen => "half_open".to_string(),
        };

        BackendMetrics {
            circuit_breaker_state: state_str,
            consecutive_failures: self.manager.consecutive_failures(),
            bulkhead_available: self.manager.bulkhead_available(),
            total_requests: self.metrics.total_requests.load(Ordering::SeqCst),
            successful_requests: self.metrics.successful_requests.load(Ordering::SeqCst),
            failed_requests: self.metrics.failed_requests.load(Ordering::SeqCst),
            rate_limited_requests: self.metrics.rate_limited_requests.load(Ordering::SeqCst),
            avg_latency_ms: avg_latency,
        }
    }

    /// Get circuit breaker state as string
    pub async fn circuit_breaker_state(&self) -> String {
        match self.manager.circuit_breaker_state().await {
            aegis_inference_backends::CircuitState::Closed => "closed".to_string(),
            aegis_inference_backends::CircuitState::Open => "open".to_string(),
            aegis_inference_backends::CircuitState::HalfOpen => "half_open".to_string(),
        }
    }

    /// Check if backend is healthy
    pub async fn health_check(&self) -> bool {
        self.manager.circuit_breaker_state().await != aegis_inference_backends::CircuitState::Open
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_backend_manager_creation() -> Result<(), Box<dyn std::error::Error>> {
        let config = BackendConfig {
            // Mock configuration for testing
            ..Default::default()
        };

        let manager = BackendManager::new(config, 100).await?;
        let metrics = manager.get_metrics().await;

        assert_eq!(metrics.total_requests, 0);
        assert_eq!(metrics.circuit_breaker_state, "closed");

        Ok(())
    }

    #[tokio::test]
    async fn test_metrics_tracking() -> Result<(), Box<dyn std::error::Error>> {
        let config = BackendConfig::default();
        let manager = BackendManager::new(config, 100).await?;

        let metrics1 = manager.get_metrics().await;
        assert_eq!(metrics1.total_requests, 0);

        // Note: Can't test actual inference without mock backend
        // but metrics structure is validated

        Ok(())
    }
}
