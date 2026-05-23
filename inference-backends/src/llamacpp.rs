use crate::error::{BackendError, Result};
use crate::models::{HealthStatus, InferenceRequest, InferenceResponse};
use crate::traits::InferenceBackend;
use async_trait::async_trait;
use chrono::Utc;
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use std::time::Instant;
use tokio::sync::RwLock;
use tracing::{debug, error, info, warn};

/// llama.cpp Backend Configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LlamaCppConfig {
    /// Enable this backend
    pub enabled: bool,

    /// llama.cpp server endpoint (usually http://localhost:8080)
    pub endpoint: String,

    /// List of supported models
    pub models: Vec<String>,

    /// Request timeout in milliseconds
    pub timeout_ms: u64,

    /// Max concurrent requests
    pub max_concurrent_requests: usize,

    /// Enable GPU acceleration (if available)
    pub gpu_enabled: bool,

    /// Number of GPU layers (0 = CPU only)
    pub gpu_layers: u32,

    /// Thread count for CPU inference
    pub threads: u32,

    /// Context size
    pub context_size: u32,

    /// Batch size
    pub batch_size: u32,
}

impl Default for LlamaCppConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            endpoint: "http://localhost:8080".to_string(),
            models: vec![
                "mistral-7b".to_string(),
                "llama2-7b".to_string(),
            ],
            timeout_ms: 30000,
            max_concurrent_requests: 50,
            gpu_enabled: true,
            gpu_layers: 33,
            threads: 4,
            context_size: 4096,
            batch_size: 512,
        }
    }
}

/// llama.cpp API request
#[derive(Serialize)]
struct LlamaCppRequest {
    prompt: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    n_predict: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    temperature: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    top_p: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    repeat_penalty: Option<f32>,
}

/// llama.cpp API response
#[derive(Deserialize, Debug, Clone)]
struct LlamaCppResponse {
    content: String,
    #[serde(default)]
    stop: bool,
    #[serde(default)]
    tokens_predicted: u32,
}

/// Backend statistics
#[derive(Clone)]
struct BackendStats {
    request_count: Arc<RwLock<u64>>,
    error_count: Arc<RwLock<u64>>,
    total_latency: Arc<RwLock<u64>>,
}

/// llama.cpp Local Inference Backend
pub struct LlamaCppBackend {
    config: LlamaCppConfig,
    client: reqwest::Client,
    stats: Arc<BackendStats>,
}

impl LlamaCppBackend {
    /// Create a new llama.cpp backend
    pub fn new(config: LlamaCppConfig) -> Result<Self> {
        if !config.enabled {
            return Err(BackendError::BackendNotConfigured(
                "llama.cpp".to_string(),
            ));
        }

        let client = reqwest::Client::builder()
            .timeout(std::time::Duration::from_millis(config.timeout_ms))
            .build()
            .map_err(|e| BackendError::HttpError(e.to_string()))?;

        info!(
            "llama.cpp backend initialized at {} (GPU: {}, threads: {})",
            config.endpoint, config.gpu_enabled, config.threads
        );

        Ok(Self {
            config,
            client,
            stats: Arc::new(BackendStats {
                request_count: Arc::new(RwLock::new(0)),
                error_count: Arc::new(RwLock::new(0)),
                total_latency: Arc::new(RwLock::new(0)),
            }),
        })
    }

    /// Call llama.cpp completion API
    async fn call_completion(&self, request: &InferenceRequest) -> Result<String> {
        let url = format!("{}/completion", self.config.endpoint);

        let llama_request = LlamaCppRequest {
            prompt: request.prompt.clone(),
            n_predict: request.max_tokens,
            temperature: request.temperature,
            top_p: request.top_p,
            repeat_penalty: Some(1.1),
        };

        debug!("Calling llama.cpp: {}", url);

        let response = self
            .client
            .post(&url)
            .json(&llama_request)
            .send()
            .await
            .map_err(|e| BackendError::HttpError(format!("llama.cpp request failed: {}", e)))?;

        if !response.status().is_success() {
            let status = response.status();
            let text = response.text().await.unwrap_or_default();
            error!("llama.cpp error: {} - {}", status, text);
            return Err(BackendError::Unknown(format!(
                "llama.cpp returned: {} - {}",
                status, text
            )));
        }

        let llama_response: LlamaCppResponse = response
            .json()
            .await
            .map_err(|e| BackendError::Unknown(format!("Failed to parse response: {}", e)))?;

        Ok(llama_response.content)
    }

    /// Get server info
    async fn get_server_info(&self) -> Result<ServerInfo> {
        let url = format!("{}/info", self.config.endpoint);

        let response = self
            .client
            .get(&url)
            .send()
            .await
            .map_err(|e| BackendError::HttpError(e.to_string()))?;

        if !response.status().is_success() {
            return Err(BackendError::HealthCheckFailed(
                "Failed to get server info".to_string(),
            ));
        }

        response
            .json()
            .await
            .map_err(|e| BackendError::Unknown(e.to_string()))
    }
}

#[derive(Deserialize, Debug)]
struct ServerInfo {
    version: String,
    #[serde(default)]
    total_tokens: u64,
}

#[async_trait]
impl InferenceBackend for LlamaCppBackend {
    async fn infer(&self, request: InferenceRequest) -> Result<InferenceResponse> {
        // Check model support
        if !self.config.models.contains(&request.model) {
            return Err(BackendError::ModelNotFound(request.model.clone()));
        }

        let start = Instant::now();

        // Call llama.cpp
        let text = self.call_completion(&request).await?;

        let latency = start.elapsed();
        let latency_ms = latency.as_millis() as u64;

        // Update stats
        let mut count = self.stats.request_count.write().await;
        *count += 1;
        drop(count);

        let mut total_latency = self.stats.total_latency.write().await;
        *total_latency += latency_ms;
        drop(total_latency);

        // Estimate tokens generated
        let tokens_generated = text.split_whitespace().count() as u32;

        let response = InferenceResponse {
            request_id: request.request_id.clone(),
            text,
            tokens_generated,
            backend_used: format!("llamacpp:{}", self.config.endpoint),
            processing_time_ms: latency_ms,
            token_probabilities: None,
            finish_reason: "stop".to_string(),
            created_at: Utc::now(),
        };

        info!(
            "llama.cpp inference complete: {} tokens in {}ms",
            tokens_generated, latency_ms
        );

        Ok(response)
    }

    async fn health_check(&self) -> Result<HealthStatus> {
        let start = Instant::now();

        match self.get_server_info().await {
            Ok(info) => {
                let latency_ms = start.elapsed().as_millis() as f32;
                let request_count = *self.stats.request_count.read().await;
                let error_count = *self.stats.error_count.read().await;

                Ok(HealthStatus {
                    healthy: true,
                    backend: format!("llamacpp ({})", info.version),
                    status: "healthy".to_string(),
                    latency_ms,
                    request_count,
                    error_count,
                    last_check: Utc::now(),
                })
            }
            Err(e) => {
                let mut count = self.stats.error_count.write().await;
                *count += 1;

                warn!("llama.cpp health check failed: {}", e);

                Ok(HealthStatus {
                    healthy: false,
                    backend: "llamacpp".to_string(),
                    status: format!("unhealthy: {}", e),
                    latency_ms: start.elapsed().as_millis() as f32,
                    request_count: *self.stats.request_count.read().await,
                    error_count: *self.stats.error_count.read().await,
                    last_check: Utc::now(),
                })
            }
        }
    }

    fn name(&self) -> &str {
        "llamacpp"
    }

    async fn supports_model(&self, model: &str) -> Result<bool> {
        Ok(self.config.models.iter().any(|m| m.contains(model)))
    }

    async fn get_models(&self) -> Result<Vec<String>> {
        Ok(self.config.models.clone())
    }

    async fn warmup(&self) -> Result<()> {
        info!("Warming up llama.cpp backend");

        // Test inference
        let test_request = InferenceRequest::new(
            self.config.models.first().cloned().unwrap_or_default(),
            "test",
        )
        .with_max_tokens(5);

        match self.call_completion(&test_request).await {
            Ok(_) => {
                info!("llama.cpp warmup successful");
                Ok(())
            }
            Err(e) => {
                warn!("llama.cpp warmup failed: {}", e);
                Ok(()) // Don't fail on warmup
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_llama_cpp_config_default() {
        let config = LlamaCppConfig::default();
        assert!(!config.enabled);
        assert_eq!(config.endpoint, "http://localhost:8080");
        assert!(config.gpu_enabled);
    }

    #[test]
    fn test_llama_cpp_backend_creation() {
        let config = LlamaCppConfig {
            enabled: false,
            ..Default::default()
        };
        let result = LlamaCppBackend::new(config);
        assert!(result.is_err());
    }
}
