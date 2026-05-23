use crate::config::HuggingFaceConfig;
use crate::error::{BackendError, Result};
use crate::models::{HealthStatus, InferenceRequest, InferenceResponse};
use crate::traits::InferenceBackend;
use async_trait::async_trait;
use chrono::Utc;
use dashmap::DashMap;
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::RwLock;
use tracing::{debug, error, info, warn};

/// Hugging Face Inference API Backend
pub struct HuggingFaceBackend {
    config: HuggingFaceConfig,
    client: reqwest::Client,
    stats: Arc<BackendStats>,
    cache: Arc<DashMap<String, CachedResponse>>,
}

#[derive(Clone)]
struct BackendStats {
    request_count: Arc<RwLock<u64>>,
    error_count: Arc<RwLock<u64>>,
    total_latency: Arc<RwLock<u64>>,
}

#[derive(Clone)]
struct CachedResponse {
    response: InferenceResponse,
    cached_at: Instant,
}

/// HF API request payload
#[derive(Serialize)]
struct HFRequest {
    inputs: String,
    parameters: HFParameters,
}

#[derive(Serialize)]
struct HFParameters {
    #[serde(skip_serializing_if = "Option::is_none")]
    max_new_tokens: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    temperature: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    top_p: Option<f32>,
    return_full_text: bool,
}

/// HF API response
#[derive(Deserialize, Debug)]
struct HFResponse {
    #[serde(default)]
    generated_text: Option<String>,
    #[serde(default)]
    score: Option<f32>,
    #[serde(default)]
    token: Option<HFToken>,
}

#[derive(Deserialize, Debug)]
struct HFToken {
    id: u32,
    text: String,
    logprob: f32,
    special: bool,
}

impl HuggingFaceBackend {
    pub fn new(config: HuggingFaceConfig) -> Result<Self> {
        if !config.enabled {
            return Err(BackendError::BackendNotConfigured(
                "HuggingFace".to_string(),
            ));
        }

        if config.api_key.is_none() {
            warn!("HuggingFace API key not configured");
        }

        let client = reqwest::Client::builder()
            .timeout(Duration::from_millis(config.timeout_ms))
            .build()
            .map_err(|e| BackendError::HttpError(e.to_string()))?;

        Ok(Self {
            config,
            client,
            stats: Arc::new(BackendStats {
                request_count: Arc::new(RwLock::new(0)),
                error_count: Arc::new(RwLock::new(0)),
                total_latency: Arc::new(RwLock::new(0)),
            }),
            cache: Arc::new(DashMap::new()),
        })
    }

    fn build_cache_key(request: &InferenceRequest) -> String {
        format!(
            "{}:{}:{}:{}",
            request.model,
            request.prompt,
            request.max_tokens.unwrap_or(0),
            request.temperature.map(|t| (t * 100.0) as u32).unwrap_or(0)
        )
    }

    fn is_cache_valid(cached: &CachedResponse, ttl_secs: u64) -> bool {
        cached.cached_at.elapsed().as_secs() < ttl_secs
    }

    async fn call_hf_api(&self, request: &InferenceRequest) -> Result<String> {
        let api_key = self.config.api_key.as_ref().ok_or_else(|| {
            BackendError::HuggingFaceError("API key not configured".to_string())
        })?;

        let url = format!(
            "{}/models/{}",
            self.config.endpoint, request.model
        );

        let hf_request = HFRequest {
            inputs: request.prompt.clone(),
            parameters: HFParameters {
                max_new_tokens: request.max_tokens,
                temperature: request.temperature,
                top_p: request.top_p,
                return_full_text: false,
            },
        };

        debug!("Calling HF API: {}", url);

        let response = self
            .client
            .post(&url)
            .header("Authorization", format!("Bearer {}", api_key))
            .json(&hf_request)
            .send()
            .await
            .map_err(|e| BackendError::HuggingFaceError(e.to_string()))?;

        if !response.status().is_success() {
            let status = response.status();
            let text = response.text().await.unwrap_or_default();
            error!("HF API error: {} - {}", status, text);
            return Err(BackendError::HuggingFaceError(format!(
                "API returned: {} - {}",
                status, text
            )));
        }

        let hf_response: Vec<HFResponse> = response
            .json()
            .await
            .map_err(|e| BackendError::HuggingFaceError(e.to_string()))?;

        let generated_text = hf_response
            .first()
            .and_then(|r| r.generated_text.clone())
            .ok_or_else(|| {
                BackendError::HuggingFaceError("No generated text in response".to_string())
            })?;

        Ok(generated_text)
    }
}

#[async_trait]
impl InferenceBackend for HuggingFaceBackend {
    async fn infer(&self, request: InferenceRequest) -> Result<InferenceResponse> {
        // Check cache
        if self.config.enable_cache {
            let cache_key = Self::build_cache_key(&request);
            if let Some(cached) = self.cache.get(&cache_key) {
                if Self::is_cache_valid(&cached, self.config.cache_ttl_secs) {
                    debug!("Cache hit for request {}", request.request_id);
                    return Ok(cached.response.clone());
                }
            }
        }

        // Check model support
        if !self.config.models.contains(&request.model) {
            return Err(BackendError::ModelNotFound(request.model.clone()));
        }

        let start = Instant::now();

        // Call API
        let text = self.call_hf_api(&request).await?;

        let latency = start.elapsed();
        let latency_ms = latency.as_millis() as u64;

        // Update stats
        let mut count = self.stats.request_count.write().await;
        *count += 1;
        drop(count);

        let mut total_latency = self.stats.total_latency.write().await;
        *total_latency += latency_ms;
        drop(total_latency);

        // Extract token count (rough estimation)
        let tokens_generated = text.split_whitespace().count() as u32;

        let response = InferenceResponse {
            request_id: request.request_id.clone(),
            text: text.clone(),
            tokens_generated,
            backend_used: "huggingface".to_string(),
            processing_time_ms: latency_ms,
            token_probabilities: None,
            finish_reason: "length".to_string(),
            created_at: Utc::now(),
        };

        // Cache response
        if self.config.enable_cache {
            let cache_key = Self::build_cache_key(&request);
            self.cache.insert(
                cache_key,
                CachedResponse {
                    response: response.clone(),
                    cached_at: Instant::now(),
                },
            );
        }

        info!(
            "HF inference complete: {} tokens in {}ms",
            tokens_generated, latency_ms
        );

        Ok(response)
    }

    async fn health_check(&self) -> Result<HealthStatus> {
        let start = Instant::now();

        // Try to call a simple model
        let test_request = InferenceRequest::new(
            self.config.models.first().cloned().unwrap_or_default(),
            "Hello",
        );

        match self.call_hf_api(&test_request).await {
            Ok(_) => {
                let latency_ms = start.elapsed().as_millis() as f32;
                let request_count = *self.stats.request_count.read().await;
                let error_count = *self.stats.error_count.read().await;

                Ok(HealthStatus {
                    healthy: true,
                    backend: "huggingface".to_string(),
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

                Ok(HealthStatus {
                    healthy: false,
                    backend: "huggingface".to_string(),
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
        "huggingface"
    }

    async fn supports_model(&self, model: &str) -> Result<bool> {
        Ok(self.config.models.iter().any(|m| m.contains(model)))
    }

    async fn get_models(&self) -> Result<Vec<String>> {
        Ok(self.config.models.clone())
    }
}
