use crate::config::OllamaConfig;
use crate::error::{BackendError, Result};
use crate::models::{HealthStatus, InferenceRequest, InferenceResponse};
use crate::traits::InferenceBackend;
use async_trait::async_trait;
use chrono::Utc;
use dashmap::DashMap;
use serde::{Deserialize, Serialize};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Instant;
use tokio::sync::RwLock;
use tracing::{debug, error, info, warn};

/// Ollama Backend
pub struct OllamaBackend {
    config: OllamaConfig,
    client: reqwest::Client,
    stats: Arc<BackendStats>,
    endpoint_index: Arc<AtomicUsize>,
    endpoint_loads: Arc<DashMap<String, EndpointLoad>>,
}

#[derive(Clone)]
struct BackendStats {
    request_count: Arc<RwLock<u64>>,
    error_count: Arc<RwLock<u64>>,
    total_latency: Arc<RwLock<u64>>,
}

#[derive(Clone, Debug)]
struct EndpointLoad {
    active_requests: Arc<AtomicUsize>,
    total_requests: Arc<RwLock<u64>>,
    total_errors: Arc<RwLock<u64>>,
    avg_latency_ms: Arc<RwLock<f32>>,
}

impl EndpointLoad {
    fn new() -> Self {
        Self {
            active_requests: Arc::new(AtomicUsize::new(0)),
            total_requests: Arc::new(RwLock::new(0)),
            total_errors: Arc::new(RwLock::new(0)),
            avg_latency_ms: Arc::new(RwLock::new(0.0)),
        }
    }

    fn get_active_requests(&self) -> usize {
        self.active_requests.load(Ordering::Relaxed)
    }

    fn increment_active(&self) {
        self.active_requests.fetch_add(1, Ordering::Relaxed);
    }

    fn decrement_active(&self) {
        self.active_requests.fetch_sub(1, Ordering::Relaxed);
    }
}

/// Ollama API request
#[derive(Serialize)]
struct OllamaRequest {
    model: String,
    prompt: String,
    stream: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    options: Option<OllamaOptions>,
}

#[derive(Serialize)]
struct OllamaOptions {
    #[serde(skip_serializing_if = "Option::is_none")]
    num_predict: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    temperature: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    top_p: Option<f32>,
}

/// Ollama API response
#[derive(Deserialize, Debug, Clone)]
struct OllamaResponse {
    response: String,
    done: bool,
}

impl OllamaBackend {
    pub fn new(config: OllamaConfig) -> Result<Self> {
        if !config.enabled {
            return Err(BackendError::BackendNotConfigured("Ollama".to_string()));
        }

        if config.endpoints.is_empty() {
            return Err(BackendError::ConfigError(
                "No Ollama endpoints configured".to_string(),
            ));
        }

        let client = reqwest::Client::builder()
            .timeout(std::time::Duration::from_millis(config.timeout_ms))
            .build()
            .map_err(|e| BackendError::HttpError(e.to_string()))?;

        let endpoint_loads = Arc::new(DashMap::new());
        for endpoint in &config.endpoints {
            endpoint_loads.insert(endpoint.clone(), EndpointLoad::new());
        }

        Ok(Self {
            config,
            client,
            stats: Arc::new(BackendStats {
                request_count: Arc::new(RwLock::new(0)),
                error_count: Arc::new(RwLock::new(0)),
                total_latency: Arc::new(RwLock::new(0)),
            }),
            endpoint_index: Arc::new(AtomicUsize::new(0)),
            endpoint_loads,
        })
    }

    fn select_endpoint(&self) -> String {
        match self.config.load_balancing.as_str() {
            "round_robin" => {
                let idx = self
                    .endpoint_index
                    .fetch_add(1, Ordering::Relaxed)
                    % self.config.endpoints.len();
                self.config.endpoints[idx].clone()
            }
            "least_loaded" => {
                let mut best_endpoint = self.config.endpoints[0].clone();
                let mut min_load = usize::MAX;

                for endpoint in &self.config.endpoints {
                    if let Some(load) = self.endpoint_loads.get(endpoint) {
                        let active = load.get_active_requests();
                        if active < min_load {
                            min_load = active;
                            best_endpoint = endpoint.clone();
                        }
                    }
                }

                best_endpoint
            }
            "random" => {
                use rand::Rng;
                let mut rng = rand::thread_rng();
                let idx = rng.gen_range(0..self.config.endpoints.len());
                self.config.endpoints[idx].clone()
            }
            _ => self.config.endpoints[0].clone(),
        }
    }

    async fn call_ollama_api(
        &self,
        endpoint: &str,
        request: &InferenceRequest,
    ) -> Result<String> {
        let url = format!("{}/api/generate", endpoint);

        let options = if request.max_tokens.is_some() || request.temperature.is_some() || request.top_p.is_some() {
            Some(OllamaOptions {
                num_predict: request.max_tokens,
                temperature: request.temperature,
                top_p: request.top_p,
            })
        } else {
            None
        };

        let ollama_request = OllamaRequest {
            model: request.model.clone(),
            prompt: request.prompt.clone(),
            stream: false,
            options,
        };

        debug!("Calling Ollama API: {}", url);

        let response = self
            .client
            .post(&url)
            .json(&ollama_request)
            .send()
            .await
            .map_err(|e| BackendError::HttpError(e.to_string()))?;

        if !response.status().is_success() {
            let status = response.status();
            let text = response.text().await.unwrap_or_default();
            error!("Ollama API error: {} - {}", status, text);
            return Err(BackendError::HttpError(format!(
                "API returned: {} - {}",
                status, text
            )));
        }

        let ollama_response: OllamaResponse = response
            .json()
            .await
            .map_err(|e| BackendError::HttpError(e.to_string()))?;

        Ok(ollama_response.response)
    }
}

#[async_trait]
impl InferenceBackend for OllamaBackend {
    async fn infer(&self, request: InferenceRequest) -> Result<InferenceResponse> {
        // Check model support
        if !self.config.models.contains(&request.model) {
            return Err(BackendError::ModelNotFound(request.model.clone()));
        }

        let endpoint = self.select_endpoint();
        let load = self
            .endpoint_loads
            .get(&endpoint)
            .ok_or_else(|| {
                BackendError::HttpError("Endpoint not found".to_string())
            })?;

        load.increment_active();

        let start = Instant::now();

        let result = self.call_ollama_api(&endpoint, &request).await;

        load.decrement_active();

        let text = result?;
        let latency = start.elapsed();
        let latency_ms = latency.as_millis() as u64;

        // Update endpoint stats
        {
            let mut total_reqs = load.total_requests.write().await;
            *total_reqs += 1;
        }
        {
            let mut avg_lat = load.avg_latency_ms.write().await;
            *avg_lat = (*avg_lat + latency_ms as f32) / 2.0;
        }

        // Update global stats
        let mut count = self.stats.request_count.write().await;
        *count += 1;
        drop(count);

        let mut total_latency = self.stats.total_latency.write().await;
        *total_latency += latency_ms;
        drop(total_latency);

        let tokens_generated = text.split_whitespace().count() as u32;

        let response = InferenceResponse {
            request_id: request.request_id.clone(),
            text,
            tokens_generated,
            backend_used: format!("ollama:{}", endpoint),
            processing_time_ms: latency_ms,
            token_probabilities: None,
            finish_reason: "stop".to_string(),
            created_at: Utc::now(),
        };

        info!(
            "Ollama inference complete: {} tokens in {}ms from {}",
            tokens_generated, latency_ms, endpoint
        );

        Ok(response)
    }

    async fn health_check(&self) -> Result<HealthStatus> {
        let mut all_healthy = true;
        let mut total_latency = 0.0;
        let mut checked_count = 0;

        for endpoint in &self.config.endpoints {
            let start = Instant::now();
            let url = format!("{}/api/tags", endpoint);

            match self
                .client
                .get(&url)
                .send()
                .await
            {
                Ok(response) if response.status().is_success() => {
                    total_latency += start.elapsed().as_millis() as f32;
                    checked_count += 1;
                }
                _ => {
                    all_healthy = false;
                    warn!("Health check failed for endpoint: {}", endpoint);
                }
            }
        }

        let request_count = *self.stats.request_count.read().await;
        let error_count = *self.stats.error_count.read().await;

        Ok(HealthStatus {
            healthy: all_healthy,
            backend: "ollama".to_string(),
            status: if all_healthy {
                "healthy".to_string()
            } else {
                "degraded".to_string()
            },
            latency_ms: if checked_count > 0 {
                total_latency / checked_count as f32
            } else {
                0.0
            },
            request_count,
            error_count,
            last_check: Utc::now(),
        })
    }

    fn name(&self) -> &str {
        "ollama"
    }

    async fn supports_model(&self, model: &str) -> Result<bool> {
        Ok(self.config.models.iter().any(|m| m.contains(model)))
    }

    async fn get_models(&self) -> Result<Vec<String>> {
        Ok(self.config.models.clone())
    }

    async fn warmup(&self) -> Result<()> {
        info!("Warming up Ollama endpoints");

        for endpoint in &self.config.endpoints {
            let url = format!("{}/api/tags", endpoint);
            match self.client.get(&url).send().await {
                Ok(_) => info!("Ollama endpoint {} is ready", endpoint),
                Err(e) => warn!("Failed to warmup {}: {}", endpoint, e),
            }
        }

        Ok(())
    }
}
