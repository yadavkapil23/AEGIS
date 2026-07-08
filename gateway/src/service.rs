use std::sync::Arc;
use std::time::Instant;
use tracing::{info, warn, error};

use crate::backend_manager::BackendManager;
use crate::cache::RequestCache;
use crate::database::{self, DbPool};
use crate::llm_backend::{LLMBackend, InferenceResult};
use crate::metrics::PrometheusMetrics;
use crate::request_queue::{RequestQueue, QueuePermit};
use crate::request_validator::{validate_request, InferenceRequest as ValidationInput, ValidatedRequest};

// ── Public request / response types ───────────────────────────

#[derive(Debug, Clone)]
pub struct InferenceResponse {
    pub success: bool,
    pub output: Option<String>,
    pub tokens_generated: u32,
    pub latency_ms: u32,
    pub backend: String,
    pub error: Option<String>,
}

#[derive(Debug, Clone)]
pub struct ServiceStats {
    pub queue_depth: usize,
    pub active_requests: usize,
    pub cache_size: usize,
    pub total_requests: u64,
}

// ── Errors ────────────────────────────────────────────────────

#[derive(Debug)]
pub enum ServiceError {
    Validation(String),
    QueueFull,
    QueueTimeout,
    BackendUnavailable(String),
    InferenceFailed(String),
}

impl std::fmt::Display for ServiceError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Validation(e) => write!(f, "validation error: {}", e),
            Self::QueueFull => write!(f, "request queue is full"),
            Self::QueueTimeout => write!(f, "timed out waiting for queue slot"),
            Self::BackendUnavailable(e) => write!(f, "backend unavailable: {}", e),
            Self::InferenceFailed(e) => write!(f, "inference failed: {}", e),
        }
    }
}

// ── Service ───────────────────────────────────────────────────

/// Orchestrates the full inference pipeline:
/// validate → queue → circuit breaker check → infer → metrics → DB log → respond.
pub struct InferenceService {
    llm_backend: Arc<LLMBackend>,
    backend_manager: Arc<BackendManager>,
    request_queue: Arc<RequestQueue>,
    db_pool: DbPool,
    cache: Arc<RequestCache>,
    metrics: Arc<PrometheusMetrics>,
}

impl InferenceService {
    pub fn new(
        llm_backend: Arc<LLMBackend>,
        backend_manager: Arc<BackendManager>,
        request_queue: Arc<RequestQueue>,
        db_pool: DbPool,
        cache: Arc<RequestCache>,
        metrics: Arc<PrometheusMetrics>,
    ) -> Self {
        info!("Initializing InferenceService (real pipeline)");
        Self {
            llm_backend,
            backend_manager,
            request_queue,
            db_pool,
            cache,
            metrics,
        }
    }

    /// Run the full inference pipeline.
    pub async fn infer(&self, req: ValidationInput) -> Result<InferenceResponse, ServiceError> {
        let start = Instant::now();

        // 1. Validate
        let validated = validate_request(&req)
            .map_err(|e| ServiceError::Validation(e.error))?;

        // 2. Check circuit breaker
        if !self.backend_manager.is_available() {
            return Err(ServiceError::BackendUnavailable(
                "circuit breaker open".into(),
            ));
        }

        // 3. Acquire queue slot (with timeout)
        let _permit: QueuePermit = self.request_queue.acquire_slot().await
            .map_err(|_| ServiceError::QueueTimeout)?;

        // 4. Acquire bulkhead
        if !self.backend_manager.try_acquire_bulkhead() {
            return Err(ServiceError::BackendUnavailable(
                "bulkhead full".into(),
            ));
        }

        // 5. Infer
        let result = self.llm_backend.infer(
            &validated.model,
            &validated.prompt,
            validated.max_tokens,
            validated.temperature,
            validated.top_p,
        ).await;

        // Release bulkhead
        self.backend_manager.release_bulkhead();

        match result {
            Ok(inference) => {
                self.backend_manager.record_success();
                self.backend_manager.record_latency(inference.latency_ms);

                let latency_ms = start.elapsed().as_millis() as u32;

                // Record metrics
                self.metrics.record_inference_success(
                    &validated.model,
                    latency_ms,
                    inference.tokens_generated,
                );

                // Log to DB async (non-blocking)
                let db = self.db_pool.clone();
                let model = validated.model.clone();
                let backend = inference.backend.clone();
                let tokens = inference.tokens_generated;
                tokio::spawn(async move {
                    if let Err(e) = database::log_inference(
                        &db, &model, "success", latency_ms as i32,
                        Some(tokens as i32), Some(&backend), None,
                    ).await {
                        error!("Failed to log inference to database: {}", e);
                    }
                });

                info!(
                    model = %validated.model,
                    backend = %inference.backend,
                    tokens = inference.tokens_generated,
                    latency_ms = latency_ms,
                    "Inference completed"
                );

                Ok(InferenceResponse {
                    success: true,
                    output: Some(inference.output),
                    tokens_generated: inference.tokens_generated,
                    latency_ms,
                    backend: inference.backend,
                    error: None,
                })
            }
            Err(e) => {
                self.backend_manager.record_failure();

                let latency_ms = start.elapsed().as_millis() as u32;
                self.metrics.record_inference_error("inference_failed");

                // Log failure to DB async
                let db = self.db_pool.clone();
                let model = validated.model.clone();
                let err_msg = e.clone();
                tokio::spawn(async move {
                    if let Err(db_err) = database::log_inference(
                        &db, &model, "failure", latency_ms as i32,
                        None, None, Some(&err_msg),
                    ).await {
                        error!("Failed to log inference failure: {}", db_err);
                    }
                });

                error!(model = %validated.model, error = %e, "Inference failed");

                Err(ServiceError::InferenceFailed(e))
            }
        }
    }

    /// Health check across all subsystems.
    pub async fn health_check(&self) -> serde_json::Value {
        let vllm_ok = self.llm_backend.check_vllm_health().await;
        let llamacpp_ok = self.llm_backend.check_llamacpp_health().await;
        let db_ok = database::health_check(&self.db_pool).await;
        let backend_ok = self.backend_manager.is_available();

        let healthy = (vllm_ok || llamacpp_ok) && db_ok && backend_ok;

        serde_json::json!({
            "healthy": healthy,
            "backends": { "vllm": vllm_ok, "llamacpp": llamacpp_ok },
            "database": db_ok,
            "circuit_breaker": self.backend_manager.metrics().circuit_breaker_state,
        })
    }

    /// Whether the service can accept new requests.
    pub fn ready(&self) -> bool {
        self.backend_manager.is_available() && !self.request_queue.is_full()
    }

    /// Operational stats.
    pub fn stats(&self) -> ServiceStats {
        ServiceStats {
            queue_depth: self.request_queue.depth(),
            active_requests: self.request_queue.active(),
            cache_size: self.cache.len(),
            total_requests: self.backend_manager.metrics().total_requests,
        }
    }
}
