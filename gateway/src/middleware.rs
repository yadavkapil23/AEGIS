use std::sync::Arc;
use serde_json::{json, Value};

use crate::allocation_client::AllocationClient;
use crate::cache::RequestCache;
use crate::config::GatewayConfig;
use crate::database::DbPool;
use crate::llm_backend::LLMBackend;
use crate::backend_manager::BackendManager;
use crate::metrics::PrometheusMetrics;
use crate::request_queue::RequestQueue;

/// Central application state shared across all handlers.
///
/// Every handler extracts this via `web::Data<GatewayState>` to access
/// subsystems without coupling to specific implementations.
#[derive(Clone)]
pub struct GatewayState {
    pub allocation_client: Arc<AllocationClient>,
    pub cache: Arc<RequestCache>,
    pub config: Arc<GatewayConfig>,
    pub db_pool: DbPool,
    pub llm_backend: Arc<LLMBackend>,
    pub backend_manager: Arc<BackendManager>,
    pub metrics: Arc<PrometheusMetrics>,
    pub request_queue: Arc<RequestQueue>,
}

impl GatewayState {
    pub fn new(
        allocation_client: Arc<AllocationClient>,
        cache: Arc<RequestCache>,
        config: Arc<GatewayConfig>,
        db_pool: DbPool,
        llm_backend: Arc<LLMBackend>,
        backend_manager: Arc<BackendManager>,
        metrics: Arc<PrometheusMetrics>,
        request_queue: Arc<RequestQueue>,
    ) -> Self {
        Self {
            allocation_client,
            cache,
            config,
            db_pool,
            llm_backend,
            backend_manager,
            metrics,
            request_queue,
        }
    }

    /// Aggregate health status of all subsystems.
    pub async fn health_status(&self) -> Value {
        let vllm_ok = self.llm_backend.check_vllm_health().await;
        let llamacpp_ok = self.llm_backend.check_llamacpp_health().await;
        let ollama_ok = self.llm_backend.check_ollama_health().await;
        let hf_ok = self.llm_backend.check_hf_health().await;
        let backend_available = self.backend_manager.is_available();
        let db_ok = crate::database::health_check(&self.db_pool).await;

        let backends_up = [vllm_ok, llamacpp_ok, ollama_ok, hf_ok]
            .iter()
            .filter(|&&b| b)
            .count();

        let overall = if backends_up > 0 && db_ok { "healthy" } else { "degraded" };

        json!({
            "status": overall,
            "backends": {
                "vllm": vllm_ok,
                "llamacpp": llamacpp_ok,
                "ollama": ollama_ok,
                "huggingface": hf_ok,
                "available": backend_available,
            },
            "database": db_ok,
            "backends_healthy": backends_up,
        })
    }

    /// Aggregate operational stats.
    pub fn stats(&self) -> Value {
        let metrics = self.backend_manager.metrics();
        json!({
            "circuit_breaker": metrics.circuit_breaker_state,
            "total_requests": metrics.total_requests,
            "successful_requests": metrics.successful_requests,
            "failed_requests": metrics.failed_requests,
            "rate_limited_requests": metrics.rate_limited_requests,
            "avg_latency_ms": metrics.avg_latency_ms,
            "cache_size": self.cache.len(),
            "queue_depth": self.request_queue.depth(),
            "queue_active": self.request_queue.active(),
        })
    }
}
