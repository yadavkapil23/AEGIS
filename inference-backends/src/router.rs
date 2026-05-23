use crate::config::BackendConfig;
use crate::error::{BackendError, Result};
use crate::huggingface::HuggingFaceBackend;
use crate::llamacpp::LlamaCppBackend;
use crate::models::{BackendPreference, HealthStatus, InferenceRequest, InferenceResponse};
use crate::traits::InferenceBackend;
use crate::vllm::VLLMBackend;
use std::sync::Arc;
use tokio::sync::RwLock;
use tracing::{debug, error, info, warn};

/// Intelligent backend router with fallback support
pub struct BackendRouter {
    config: BackendConfig,
    hf_backend: Option<Arc<HuggingFaceBackend>>,
    vllm_backend: Option<Arc<VLLMBackend>>,
    llamacpp_backend: Option<Arc<LlamaCppBackend>>,
    health_status: Arc<RwLock<BackendHealthStatus>>,
}

#[derive(Clone, Debug)]
struct BackendHealthStatus {
    hf_healthy: bool,
    vllm_healthy: bool,
    llamacpp_healthy: bool,
}

impl BackendRouter {
    /// Create a new router with the given config
    pub async fn new(config: BackendConfig) -> Result<Self> {
        let hf_backend = if let Some(hf_config) = &config.huggingface {
            match HuggingFaceBackend::new(hf_config.clone()) {
                Ok(backend) => {
                    info!("HuggingFace backend initialized");
                    Some(Arc::new(backend))
                }
                Err(e) => {
                    warn!("Failed to initialize HuggingFace backend: {}", e);
                    None
                }
            }
        } else {
            None
        };

        let vllm_backend = if let Some(vllm_config) = &config.vllm {
            match VLLMBackend::new(vllm_config.clone()) {
                Ok(backend) => {
                    info!("vLLM backend initialized");
                    Some(Arc::new(backend))
                }
                Err(e) => {
                    warn!("Failed to initialize vLLM backend: {}", e);
                    None
                }
            }
        } else {
            None
        };

        let llamacpp_backend = if let Some(config_llamacpp) = &config.llamacpp {
            // Convert from config::LlamaCppConfig to crate::llamacpp::LlamaCppConfig
            let llamacpp_config = crate::llamacpp::LlamaCppConfig {
                enabled: config_llamacpp.enabled,
                endpoint: config_llamacpp.endpoint.clone(),
                models: config_llamacpp.models.clone(),
                timeout_ms: config_llamacpp.timeout_ms,
                max_concurrent_requests: config_llamacpp.max_concurrent_requests,
                gpu_enabled: config_llamacpp.gpu_enabled,
                gpu_layers: config_llamacpp.gpu_layers,
                threads: config_llamacpp.threads,
                context_size: config_llamacpp.context_size,
                batch_size: config_llamacpp.batch_size,
            };

            match crate::llamacpp::LlamaCppBackend::new(llamacpp_config) {
                Ok(backend) => {
                    info!("llama.cpp backend initialized");
                    Some(Arc::new(backend))
                }
                Err(e) => {
                    warn!("Failed to initialize llama.cpp backend: {}", e);
                    None
                }
            }
        } else {
            None
        };

        if hf_backend.is_none() && vllm_backend.is_none() && llamacpp_backend.is_none() {
            return Err(BackendError::ConfigError(
                "No backends configured".to_string(),
            ));
        }

        Ok(Self {
            config,
            hf_backend,
            vllm_backend,
            llamacpp_backend,
            health_status: Arc::new(RwLock::new(BackendHealthStatus {
                hf_healthy: true,
                vllm_healthy: true,
                llamacpp_healthy: true,
            })),
        })
    }

    /// Route a request to the appropriate backend
    pub async fn infer(&self, request: InferenceRequest) -> Result<InferenceResponse> {
        // Determine which backend(s) to try
        let backends_to_try = self.get_backends_to_try(&request).await;

        if backends_to_try.is_empty() {
            return Err(BackendError::AllBackendsUnavailable);
        }

        // Try each backend in order
        let mut last_error = None;

        for backend_name in backends_to_try {
            debug!("Attempting inference with backend: {}", backend_name);

            let result = match backend_name.as_str() {
                "huggingface" => {
                    if let Some(backend) = &self.hf_backend {
                        backend.infer(request.clone()).await
                    } else {
                        continue;
                    }
                }
                "vllm" => {
                    if let Some(backend) = &self.vllm_backend {
                        backend.infer(request.clone()).await
                    } else {
                        continue;
                    }
                }
                "llamacpp" => {
                    if let Some(backend) = &self.llamacpp_backend {
                        backend.infer(request.clone()).await
                    } else {
                        continue;
                    }
                }
                _ => continue,
            };

            match result {
                Ok(response) => {
                    info!(
                        "Successfully processed request {} with {}",
                        request.request_id, backend_name
                    );
                    return Ok(response);
                }
                Err(e) => {
                    error!(
                        "Backend {} failed for request {}: {}",
                        backend_name, request.request_id, e
                    );
                    last_error = Some(e);
                    continue;
                }
            }
        }

        // All backends failed
        match last_error {
            Some(e) => {
                error!("All backends failed for request {}", request.request_id);
                Err(e)
            }
            None => Err(BackendError::AllBackendsUnavailable),
        }
    }

    /// Get list of backends to try in order
    async fn get_backends_to_try(&self, request: &InferenceRequest) -> Vec<String> {
        let health = self.health_status.read().await;

        match request.backend_preference {
            BackendPreference::HuggingFace => {
                if health.hf_healthy && self.hf_backend.is_some() {
                    vec!["huggingface".to_string()]
                } else {
                    vec![]
                }
            }
            BackendPreference::VLLm => {
                if health.vllm_healthy && self.vllm_backend.is_some() {
                    vec!["vllm".to_string()]
                } else {
                    vec![]
                }
            }
            BackendPreference::Auto => {
                // Smart routing based on heuristics
                let mut backends = Vec::new();

                // Prefer vLLM for low-latency requirements
                if request.timeout_ms.map_or(false, |t| t < 5000) {
                    if health.vllm_healthy && self.vllm_backend.is_some() {
                        backends.push("vllm".to_string());
                    }
                    // Then try llama.cpp for local low-latency
                    if health.llamacpp_healthy && self.llamacpp_backend.is_some() {
                        backends.push("llamacpp".to_string());
                    }
                }

                // Add fallback order for others
                for backend_name in &self.config.fallback_order {
                    if !backends.contains(backend_name) {
                        match backend_name.as_str() {
                            "vllm" if health.vllm_healthy && self.vllm_backend.is_some() => {
                                backends.push(backend_name.clone());
                            }
                            "llamacpp" if health.llamacpp_healthy && self.llamacpp_backend.is_some() => {
                                backends.push(backend_name.clone());
                            }
                            "huggingface" if health.hf_healthy && self.hf_backend.is_some() => {
                                backends.push(backend_name.clone());
                            }
                            _ => {}
                        }
                    }
                }

                backends
            }
        }
    }

    /// Get health status of all backends
    pub async fn health_check(&self) -> Result<Vec<HealthStatus>> {
        let mut statuses = Vec::new();

        // Check HF backend
        if let Some(backend) = &self.hf_backend {
            match backend.health_check().await {
                Ok(status) => {
                    let mut health_status = self.health_status.write().await;
                    health_status.hf_healthy = status.healthy;
                    drop(health_status);
                    statuses.push(status);
                }
                Err(e) => {
                    error!("HF health check failed: {}", e);
                    let mut health_status = self.health_status.write().await;
                    health_status.hf_healthy = false;
                }
            }
        }

        // Check vLLM backend
        if let Some(backend) = &self.vllm_backend {
            match backend.health_check().await {
                Ok(status) => {
                    let mut health_status = self.health_status.write().await;
                    health_status.vllm_healthy = status.healthy;
                    drop(health_status);
                    statuses.push(status);
                }
                Err(e) => {
                    error!("vLLM health check failed: {}", e);
                    let mut health_status = self.health_status.write().await;
                    health_status.vllm_healthy = false;
                }
            }
        }

        // Check llama.cpp backend
        if let Some(backend) = &self.llamacpp_backend {
            match backend.health_check().await {
                Ok(status) => {
                    let mut health_status = self.health_status.write().await;
                    health_status.llamacpp_healthy = status.healthy;
                    drop(health_status);
                    statuses.push(status);
                }
                Err(e) => {
                    error!("llama.cpp health check failed: {}", e);
                    let mut health_status = self.health_status.write().await;
                    health_status.llamacpp_healthy = false;
                }
            }
        }

        Ok(statuses)
    }

    /// Get available models across all backends
    pub async fn get_available_models(&self) -> Result<Vec<String>> {
        let mut models = Vec::new();

        if let Some(backend) = &self.hf_backend {
            if let Ok(mut backend_models) = backend.get_models().await {
                models.append(&mut backend_models);
            }
        }

        if let Some(backend) = &self.vllm_backend {
            if let Ok(mut backend_models) = backend.get_models().await {
                models.append(&mut backend_models);
            }
        }

        if let Some(backend) = &self.llamacpp_backend {
            if let Ok(mut backend_models) = backend.get_models().await {
                models.append(&mut backend_models);
            }
        }

        models.sort();
        models.dedup();

        Ok(models)
    }

    /// Check if a model is supported
    pub async fn supports_model(&self, model: &str) -> Result<bool> {
        if let Some(backend) = &self.hf_backend {
            if backend.supports_model(model).await? {
                return Ok(true);
            }
        }

        if let Some(backend) = &self.vllm_backend {
            if backend.supports_model(model).await? {
                return Ok(true);
            }
        }

        if let Some(backend) = &self.llamacpp_backend {
            if backend.supports_model(model).await? {
                return Ok(true);
            }
        }

        Ok(false)
    }

    /// Warmup all backends
    pub async fn warmup(&self) -> Result<()> {
        info!("Warming up all backends");

        if let Some(backend) = &self.hf_backend {
            if let Err(e) = backend.warmup().await {
                warn!("HF backend warmup failed: {}", e);
            }
        }

        if let Some(backend) = &self.vllm_backend {
            if let Err(e) = backend.warmup().await {
                warn!("vLLM backend warmup failed: {}", e);
            }
        }

        if let Some(backend) = &self.llamacpp_backend {
            if let Err(e) = backend.warmup().await {
                warn!("llama.cpp backend warmup failed: {}", e);
            }
        }

        Ok(())
    }
}
