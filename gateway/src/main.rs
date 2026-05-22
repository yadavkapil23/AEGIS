/// AEGIS Gateway - LLM Inference with Production Observability
/// Real backends (vLLM, llama.cpp) with Prometheus metrics, OpenTelemetry tracing

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

use allocation_client::AllocationClient;
use config::GatewayConfig;
use cache::RequestCache;
use backend_manager::BackendManager;
use metrics::PrometheusMetrics;

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

    info!(
        "Starting AEGIS Gateway on http://{}:{}",
        config.host, config.port
    );
    info!("Available endpoints:");
    info!("  POST   /infer              - Run LLM inference");
    info!("  GET    /health/live        - Liveness probe");
    info!("  GET    /health/ready       - Readiness probe");
    info!("  GET    /health/startup     - Startup probe");
    info!("  GET    /metrics            - Prometheus metrics");
    info!("");
    info!("Observability:");
    info!("  Prometheus (metrics):    http://localhost:9090");
    info!("  Grafana (dashboards):    http://localhost:3000");
    info!("  Jaeger (distributed trace): http://localhost:16686");

    // Start HTTP server
    HttpServer::new(move || {
        App::new()
            .app_data(state.clone())
            .app_data(prometheus_metrics.clone())
            .app_data(backend_manager.clone())
            .wrap(middleware::Logger::default())
            .wrap(middleware::NormalizePath::trim())
            // Inference endpoints
            .service(inference_handler::infer_handler)
            .service(inference_handler::health_live)
            .service(inference_handler::health_ready)
            .service(inference_handler::health_startup)
            .service(inference_handler::metrics_handler)
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
