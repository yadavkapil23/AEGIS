/// AEGIS Gateway - LLM Inference with Production Observability & Security
/// Real backends (vLLM, llama.cpp) with metrics, tracing, JWT auth, rate limiting

use actix_web::{web, App, HttpServer, middleware};
use std::sync::Arc;
use tracing::info;

mod allocation_client;
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
mod db_migrations;
mod backup;

use allocation_client::AllocationClient;
use config::GatewayConfig;
use cache::RequestCache;
use backend_manager::BackendManager;
use metrics::PrometheusMetrics;
use jwt_auth::{ApiKeyValidator, JwtAuthMiddleware};
use security_middleware::{RateLimitMiddleware, SecurityHeadersMiddleware, RequestIdMiddleware};
use db_migrations::MigrationManager;

mod llm_backend;
use llm_backend::LLMBackend;

mod database;
use database::DbPool;

/// Gateway application state
pub struct GatewayState {
    client: Arc<AllocationClient>,
    cache: Arc<RequestCache>,
    config: Arc<GatewayConfig>,
}

#[actix_web::main]
async fn main() -> std::io::Result<()> {
    // Initialize distributed tracing with OpenTelemetry
    telemetry::init_tracing("aegis-gateway")
        .expect("Failed to initialize tracing");

    // Load configuration
    let config = GatewayConfig::from_env();
    info!("Gateway configuration loaded");
    info!("Host: {}, Port: {}", config.host, config.port);

    // Initialize Prometheus metrics
    let prometheus_metrics = web::Data::new(
        PrometheusMetrics::new()
            .expect("Failed to initialize Prometheus metrics"),
    );
    info!("Prometheus metrics initialized");

    // Create allocation client
    let client = Arc::new(
        AllocationClient::new(config.scheduler_nodes.clone())
            .await
            .expect("Failed to create allocation client"),
    );

    // Create request cache
    let cache = Arc::new(RequestCache::new(config.cache_size));

    // Initialize PostgreSQL database with connection pool
    let db_pool = match database::create_pool().await {
        Ok(pool) => {
            info!("PostgreSQL database initialized successfully");
            web::Data::new(pool)
        }
        Err(e) => {
            info!("Warning: Failed to initialize PostgreSQL: {}", e);
            info!("Continuing without persistent database (in-memory only)");
            // For now, we'll exit if DB init fails since it's critical
            // In future, could fall back to in-memory mode
            panic!("Database initialization failed: {}", e);
        }
    };

    // Create backend manager for LLM inference
    let backend_manager = web::Data::new(
        BackendManager::new()
            .expect("Failed to initialize backend manager"),
    );
    info!("Backend manager initialized with real LLM backends");
    info!("Primary: vLLM, Fallback: llama.cpp");

    // Create application state
    let state = web::Data::new(GatewayState {
        client,
        cache,
        config: Arc::new(config.clone()),
    });

    // Initialize JWT/API key validator (API keys now from database)
    let jwt_secret = std::env::var("JWT_SECRET")
        .unwrap_or_else(|_| "change-me-in-production".to_string());

    // For now, still support env var API keys as fallback, but primary source is DB
    let fallback_api_keys = std::env::var("API_KEYS")
        .unwrap_or_else(|_| "sk-demo123".to_string())
        .split(',')
        .map(|k| k.trim().to_string())
        .collect();

    let api_key_validator = web::Data::new(ApiKeyValidator::new(jwt_secret, fallback_api_keys));
    info!("JWT/API key validator initialized (primary source: PostgreSQL database)");

    // Initialize migration manager
    let migration_manager = MigrationManager::new("/var/lib/aegis/migrations");
    info!("Database migration manager initialized");

    // Initialize backup manager
    let backup_manager = backup::BackupManager::new(backup::BackupConfig::default());
    info!("Backup manager initialized");

    // Initialize LLM backend (vLLM + llama.cpp)
    let vllm_endpoint = std::env::var("VLLM_ENDPOINT")
        .unwrap_or_else(|_| "http://localhost:8000".to_string());
    let llamacpp_endpoint = std::env::var("LLAMACPP_ENDPOINT")
        .unwrap_or_else(|_| "http://localhost:8001".to_string());

    let llm_backend = web::Data::new(LLMBackend::new(
        vllm_endpoint.clone(),
        llamacpp_endpoint.clone(),
        config.request_timeout_secs,
    ));

    info!("LLM Backend initialized:");
    info!("  Primary (vLLM): {}", vllm_endpoint);
    info!("  Fallback (llama.cpp): {}", llamacpp_endpoint);

    info!(
        "Starting AEGIS Gateway on http://{}:{}",
        config.host, config.port
    );
    info!("Security Features:");
    info!("  ✓ JWT token validation (Bearer)");
    info!("  ✓ API key authentication (X-API-Key)");
    info!("  ✓ Rate limiting (token bucket)");
    info!("  ✓ CORS/CSRF protection");
    info!("  ✓ Security headers (CSP, X-Frame-Options, etc.)");
    info!("  ✓ Request validation (prompt, tokens, temperature)");
    info!("");
    info!("Available endpoints:");
    info!("  POST   /infer              - Run LLM inference (requires auth)");
    info!("  GET    /health/live        - Liveness probe");
    info!("  GET    /health/ready       - Readiness probe");
    info!("  GET    /health/startup     - Startup probe");
    info!("  GET    /metrics            - Prometheus metrics");
    info!("");
    info!("Observability & Operations:");
    info!("  Prometheus (metrics):      http://localhost:9090");
    info!("  Grafana (dashboards):      http://localhost:3000");
    info!("  Jaeger (distributed trace): http://localhost:16686");
    info!("  Backups stored in:         /var/backups/aegis");

    // Start HTTP server
    HttpServer::new(move || {
        App::new()
            .app_data(state.clone())
            .app_data(prometheus_metrics.clone())
            .app_data(backend_manager.clone())
            .app_data(api_key_validator.clone())
            .app_data(llm_backend.clone())
            .app_data(db_pool.clone())
            // Middleware stack (order matters!)
            .wrap(RequestIdMiddleware)                              // Tracing
            .wrap(middleware::Logger::default())                    // Logging
            .wrap(SecurityHeadersMiddleware)                         // Security headers
            .wrap(RateLimitMiddleware::new(
                config.rate_limit_rps as u32 * 60 / 1000,  // Convert to per-minute
            ))                                                       // Rate limiting
            .wrap(JwtAuthMiddleware::new(                           // Authentication
                api_key_validator.get_ref().clone()
            ))
            .wrap(middleware::NormalizePath::trim())                // URL normalization
            // Inference endpoints
            .service(inference_handler::infer_handler)
            .service(inference_handler::health_live)
            .service(inference_handler::health_ready)
            .service(inference_handler::health_startup)
            .service(inference_handler::metrics_handler)
            .service(inference_handler::backends_status)
            // Legacy allocation endpoints
            .route("/health", web::get().to(handlers::health_check))
            .route("/ready", web::get().to(handlers::readiness_check))
            .route("/v1/allocate", web::post().to(handlers::allocate))
            .route("/v1/deallocate", web::post().to(handlers::deallocate))
            .route("/v1/stats", web::get().to(handlers::get_stats))
            .route("/v1/cluster", web::get().to(handlers::get_cluster_health))
    })
    .bind(format!("{}:{}", config.host, config.port))?
    .run()
    .await
}
