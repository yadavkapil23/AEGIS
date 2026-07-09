/// AEGIS Gateway - LLM Inference with Production Observability & Security
/// Real backends (vLLM, llama.cpp) with metrics, tracing, JWT auth, rate limiting

use actix_web::{web, App, HttpServer, middleware as actix_mw};
use actix_cors::Cors;
use std::sync::Arc;
use tracing::info;

mod allocation_client;
mod api_key_handlers;
mod handlers;
mod config;
mod cache;
mod backend_manager;
mod inference_handler;
mod telemetry;
mod metrics;
mod jwt_auth;
mod security_middleware;
mod request_validator;
mod request_queue;
mod middleware;
mod database;
mod llm_backend;

use allocation_client::AllocationClient;
use config::GatewayConfig;
use cache::RequestCache;
use backend_manager::BackendManager;
use metrics::PrometheusMetrics;
use jwt_auth::{ApiKeyValidator, JwtAuthMiddleware};
use security_middleware::{RateLimitMiddleware, SecurityHeadersMiddleware, RequestIdMiddleware};
use middleware::GatewayState;
use request_queue::RequestQueue;
use llm_backend::LLMBackend;

#[actix_web::main]
async fn main() -> std::io::Result<()> {
    // ── Initialize tracing ─────────────────────────────────
    telemetry::init_tracing("aegis-gateway").expect("Failed to initialize tracing");

    // ── Load config ────────────────────────────────────────
    let config = GatewayConfig::from_env();
    let config_arc = Arc::new(config.clone());
    info!(host = %config.host, port = config.port, "Configuration loaded");

    // ── Prometheus metrics ─────────────────────────────────
    let prometheus_metrics = Arc::new(
        PrometheusMetrics::new().expect("Failed to initialize Prometheus metrics"),
    );

    // ── Allocation client (gRPC to scheduler) ──────────────
    let allocation_client = Arc::new(
        AllocationClient::new(config.scheduler_nodes.clone())
            .await
            .expect("Failed to create allocation client"),
    );

    // ── Request cache ──────────────────────────────────────
    let request_cache = Arc::new(RequestCache::new(config.cache_size));

    // ── Request queue ──────────────────────────────────────
    let request_queue = Arc::new(RequestQueue::new(
        100,                                    // max concurrent
        config.request_timeout_secs * 1000,     // timeout in ms
    ));

    // ── PostgreSQL ─────────────────────────────────────────
    let db_pool = database::create_pool().await
        .expect("Database initialization failed");

    // ── Backend manager (circuit breaker + bulkhead) ───────
    let backend_manager = Arc::new(
        BackendManager::new().expect("Failed to initialize backend manager"),
    );

    // ── LLM backend (vLLM + llama.cpp + Ollama + HuggingFace)
    let llm_backend = Arc::new(LLMBackend::new(
        config.vllm_endpoint.clone(),
        config.llamacpp_endpoint.clone(),
        config.request_timeout_secs,
    ));

    // ── JWT / API key validator ────────────────────────────
    let jwt_secret = std::env::var("JWT_SECRET")
        .unwrap_or_else(|_| "change-me-in-production".into());
    let fallback_keys: Vec<String> = std::env::var("API_KEYS")
        .unwrap_or_else(|_| "sk-demo123".into())
        .split(',')
        .map(|s| s.trim().to_string())
        .collect();
    let api_key_validator = web::Data::new(ApiKeyValidator::new(
        jwt_secret,
        fallback_keys,
        Some(db_pool.clone()),
    ));

    // ── Build shared GatewayState (single source of truth) ─
    let gw_state = web::Data::new(GatewayState::new(
        allocation_client,
        request_cache,
        config_arc.clone(),
        db_pool.clone(),
        llm_backend.clone(),
        backend_manager.clone(),
        prometheus_metrics.clone(),
        request_queue,
    ));

    info!(
        "Starting AEGIS Gateway on http://{}:{}",
        config.host, config.port
    );
    info!("Endpoints:");
    info!("  POST   /infer              - LLM inference");
    info!("  POST   /infer/stream       - LLM inference (streaming SSE)");
    info!("  POST   /v1/chat/completions - OpenAI-compatible chat completions");
    info!("  POST   /v1/allocate        - KV-cache allocation");
    info!("  POST   /v1/deallocate      - KV-cache deallocation");
    info!("  GET    /v1/stats            - Cache statistics");
    info!("  GET    /v1/cluster          - Cluster health");
    info!("  GET    /health              - Deep health check");
    info!("  GET    /ready               - Readiness probe");
    info!("  GET    /metrics             - Prometheus metrics");
    info!("  POST   /api/keys            - Create API key");
    info!("  GET    /api/keys            - List API keys");
    info!("  DELETE /api/keys/{{key}}      - Revoke API key");

    // ── Start HTTP server ──────────────────────────────────
    let rate_limit = config.rate_limit_rps;
    HttpServer::new(move || {
        let cors = Cors::default()
            .allow_any_origin()
            .allow_any_method()
            .allow_any_header()
            .max_age(3600);

        App::new()
            .app_data(gw_state.clone())
            .app_data(api_key_validator.clone())
            // CORS (outermost)
            .wrap(cors)
            // Middleware (order matters - outermost first)
            .wrap(RequestIdMiddleware)
            .wrap(actix_mw::Logger::default())
            .wrap(SecurityHeadersMiddleware)
            .wrap(RateLimitMiddleware::new(rate_limit))
            .wrap(JwtAuthMiddleware::new(api_key_validator.get_ref().clone()))
            .wrap(actix_mw::NormalizePath::trim())
            // Inference endpoints
            .service(inference_handler::infer_handler)
            .service(inference_handler::infer_stream_handler)
            .service(inference_handler::chat_completions_handler)
            .service(inference_handler::health_live)
            .service(inference_handler::health_ready)
            .service(inference_handler::health_startup)
            .service(inference_handler::metrics_handler)
            .service(inference_handler::backends_status)
            // Allocation endpoints
            .service(handlers::health_check)
            .service(handlers::readiness_check)
            .service(handlers::allocate)
            .service(handlers::deallocate)
            .service(handlers::get_stats)
            .service(handlers::get_cluster_health)
            // API key CRUD endpoints
            .service(api_key_handlers::create_api_key)
            .service(api_key_handlers::get_api_keys)
            .service(api_key_handlers::get_api_key)
            .service(api_key_handlers::revoke_api_key)
    })
    .bind(format!("{}:{}", config.host, config.port))?
    .run()
    .await
}
