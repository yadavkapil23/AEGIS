use crate::models::{HealthStatus, InferenceRequest, InferenceResponse};
use crate::traits::InferenceBackend;
use async_trait::async_trait;
use chrono::Utc;
use serde::{Deserialize, Serialize};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Instant;
use tokio::sync::RwLock;
use tracing::info;

/// Mock backend configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MockConfig {
    /// Enable this backend
    pub enabled: bool,

    /// List of "supported" models
    pub models: Vec<String>,

    /// Simulated latency in milliseconds
    pub simulated_latency_ms: u64,

    /// Whether to simulate occasional failures
    pub simulate_failures: bool,

    /// Failure rate (0.0-1.0)
    pub failure_rate: f32,

    /// Name for logging
    pub name: String,
}

impl Default for MockConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            models: vec![
                "mock-model-7b".to_string(),
                "mock-model-13b".to_string(),
            ],
            simulated_latency_ms: 100,
            simulate_failures: false,
            failure_rate: 0.0,
            name: "mock".to_string(),
        }
    }
}

/// Mock backend for testing (generates synthetic responses)
///
/// ⚠️ **WARNING**: This backend generates FAKE tokens and is for testing only.
/// DO NOT use in production. Use only for:
/// - Unit testing
/// - Integration testing
/// - Development and debugging
/// - CI/CD pipelines
/// - Load testing without infrastructure
pub struct MockBackend {
    config: MockConfig,
    call_count: Arc<AtomicU64>,
    stats: Arc<MockStats>,
}

#[derive(Clone)]
struct MockStats {
    request_count: Arc<RwLock<u64>>,
    error_count: Arc<RwLock<u64>>,
    total_latency: Arc<RwLock<u64>>,
}

impl MockBackend {
    /// Create a new mock backend
    pub fn new(config: MockConfig) -> Self {
        if !config.enabled {
            panic!("Mock backend not enabled in config");
        }

        info!(
            "⚠️ WARNING: MockBackend initialized - generates FAKE tokens for testing only"
        );

        Self {
            config,
            call_count: Arc::new(AtomicU64::new(0)),
            stats: Arc::new(MockStats {
                request_count: Arc::new(RwLock::new(0)),
                error_count: Arc::new(RwLock::new(0)),
                total_latency: Arc::new(RwLock::new(0)),
            }),
        }
    }

    /// Generate a realistic-looking but fake response
    fn generate_fake_response(&self, prompt: &str, max_tokens: Option<u32>) -> String {
        let token_words = vec![
            "The", "quick", "brown", "fox", "jumps", "over", "the", "lazy", "dog",
            "Rust", "is", "a", "systems", "programming", "language", "that", "runs",
            "blazingly", "fast", "and", "prevents", "segfaults", "Machine", "learning",
            "inference", "is", "the", "process", "of", "using", "a", "trained", "model",
            "to", "make", "predictions", "Distributed", "systems", "are", "hard", "to",
            "debug", "but", "essential", "for", "scalability",
        ];

        let num_tokens = max_tokens.unwrap_or(50).min(500) as usize;
        let call_num = self.call_count.load(Ordering::Relaxed);

        // Generate a somewhat coherent-looking response
        let mut response = format!("Mock response to: '{}'\n\n", prompt);
        for i in 0..num_tokens {
            let word_idx = ((call_num * 7 + i as u64) % token_words.len() as u64) as usize;
            response.push_str(token_words[word_idx]);
            if (i + 1) % 10 == 0 {
                response.push('\n');
            } else {
                response.push(' ');
            }
        }

        response
    }

    /// Check if should simulate failure
    fn should_fail(&self) -> bool {
        if !self.config.simulate_failures {
            return false;
        }

        use rand::Rng;
        let mut rng = rand::thread_rng();
        rng.gen::<f32>() < self.config.failure_rate
    }
}

#[async_trait]
impl InferenceBackend for MockBackend {
    async fn infer(&self, request: InferenceRequest) -> crate::error::Result<InferenceResponse> {
        let start = Instant::now();

        // Simulate latency
        if self.config.simulated_latency_ms > 0 {
            tokio::time::sleep(std::time::Duration::from_millis(
                self.config.simulated_latency_ms,
            ))
            .await;
        }

        // Check if should simulate failure
        if self.should_fail() {
            let mut error_count = self.stats.error_count.write().await;
            *error_count += 1;

            return Err(crate::error::BackendError::Unknown(
                "Mock backend simulated failure".to_string(),
            ));
        }

        // Update stats
        self.call_count.fetch_add(1, Ordering::Relaxed);
        let mut count = self.stats.request_count.write().await;
        *count += 1;
        drop(count);

        let latency_ms = start.elapsed().as_millis() as u64;
        let mut total_latency = self.stats.total_latency.write().await;
        *total_latency += latency_ms;
        drop(total_latency);

        // Generate fake response
        let text = self.generate_fake_response(&request.prompt, request.max_tokens);
        let tokens_generated = text.split_whitespace().count() as u32;

        let response = InferenceResponse {
            request_id: request.request_id.clone(),
            text,
            tokens_generated,
            backend_used: format!("mock (test only)"),
            processing_time_ms: latency_ms,
            token_probabilities: None,
            finish_reason: "length".to_string(),
            created_at: Utc::now(),
        };

        info!(
            "⚠️ Mock inference (test): {} tokens in {}ms",
            tokens_generated, latency_ms
        );

        Ok(response)
    }

    async fn health_check(&self) -> crate::error::Result<HealthStatus> {
        let request_count = *self.stats.request_count.read().await;
        let error_count = *self.stats.error_count.read().await;

        Ok(HealthStatus {
            healthy: true,
            backend: "mock (TEST ONLY)".to_string(),
            status: "healthy (fake responses)".to_string(),
            latency_ms: self.config.simulated_latency_ms as f32,
            request_count,
            error_count,
            last_check: Utc::now(),
        })
    }

    fn name(&self) -> &str {
        "mock"
    }

    async fn supports_model(&self, model: &str) -> crate::error::Result<bool> {
        Ok(self.config.models.iter().any(|m| m.contains(model)))
    }

    async fn get_models(&self) -> crate::error::Result<Vec<String>> {
        Ok(self.config.models.clone())
    }

    async fn warmup(&self) -> crate::error::Result<()> {
        info!("⚠️ Mock backend warmed up (no-op)");
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_mock_generates_response() {
        let config = MockConfig {
            enabled: true,
            ..Default::default()
        };

        let backend = MockBackend::new(config);
        let request = InferenceRequest::new("mock-model", "Hello");

        let response = backend.infer(request).await;
        assert!(response.is_ok());

        let resp = response.unwrap();
        assert!(!resp.text.is_empty());
        assert!(resp.backend_used.contains("mock"));
    }

    #[tokio::test]
    async fn test_mock_simulates_failures() {
        let config = MockConfig {
            enabled: true,
            simulate_failures: true,
            failure_rate: 1.0, // Always fail
            ..Default::default()
        };

        let backend = MockBackend::new(config);
        let request = InferenceRequest::new("mock-model", "Hello");

        let response = backend.infer(request).await;
        assert!(response.is_err());
    }

    #[tokio::test]
    async fn test_mock_health_check() {
        let config = MockConfig {
            enabled: true,
            ..Default::default()
        };

        let backend = MockBackend::new(config);
        let health = backend.health_check().await;

        assert!(health.is_ok());
        let status = health.unwrap();
        assert!(status.healthy);
    }
}
