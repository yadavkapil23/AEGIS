// Inference Gateway with ConsensusAllocator Integration
// Provides REST API for KV cache allocation to inference engines

use actix_web::{web, App, HttpServer, HttpResponse, middleware};
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use tokio::sync::RwLock;
use tonic::transport::Channel;
use tracing::{info, debug, error};

mod allocation_client;
mod handlers;
mod config;
mod cache;

use allocation_client::AllocationClient;
use config::GatewayConfig;
use cache::RequestCache;

/// Allocation request from inference engine
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AllocationRequest {
    /// Unique request ID
    pub request_id: String,
    /// Number of KV cache blocks needed
    pub num_blocks: u32,
    /// Model name (optional)
    pub model: Option<String>,
    /// Owner/application name (optional)
    pub owner: Option<String>,
    /// Priority level 0-10 (optional)
    pub priority: Option<u32>,
}

/// Allocation response
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AllocationResponse {
    /// Request ID
    pub request_id: String,
    /// Success status
    pub success: bool,
    /// Allocated block IDs
    pub block_ids: Vec<u64>,
    /// Error message if failed
    pub error: Option<String>,
    /// Operation latency in ms
    pub latency_ms: u32,
    /// Node that performed allocation
    pub node_id: String,
}

/// Deallocation request
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DeallocationRequest {
    /// Unique request ID
    pub request_id: String,
    /// Block IDs to deallocate
    pub block_ids: Vec<u64>,
}

/// Deallocation response
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DeallocationResponse {
    /// Request ID
    pub request_id: String,
    /// Success status
    pub success: bool,
    /// Number of blocks deallocated
    pub count: u32,
    /// Error message if failed
    pub error: Option<String>,
    /// Operation latency in ms
    pub latency_ms: u32,
}

/// Cache statistics
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CacheStats {
    /// Total blocks available
    pub total_blocks: u64,
    /// Allocated blocks
    pub allocated_blocks: u64,
    /// Free blocks
    pub free_blocks: u64,
    /// Utilization percentage
    pub utilization_percent: u32,
    /// Total allocations
    pub total_allocations: u64,
    /// Total deallocations
    pub total_deallocations: u64,
    /// Average latency
    pub avg_latency_ms: u32,
    /// Node ID
    pub node_id: String,
}

/// Cluster health status
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ClusterHealth {
    /// Cluster is healthy
    pub healthy: bool,
    /// Total nodes
    pub total_nodes: u32,
    /// Healthy nodes
    pub healthy_nodes: u32,
    /// Leader node ID
    pub leader_id: String,
    /// Quorum status
    pub quorum_status: String,
}

/// Gateway application state
pub struct GatewayState {
    client: Arc<AllocationClient>,
    cache: Arc<RequestCache>,
    config: Arc<GatewayConfig>,
}

#[actix_web::main]
async fn main() -> std::io::Result<()> {
    // Initialize tracing
    tracing_subscriber::fmt()
        .with_max_level(tracing::Level::INFO)
        .init();

    // Load configuration
    let config = GatewayConfig::from_env();
    info!("Gateway config: {:?}", config);

    // Create allocation client
    let client = Arc::new(
        AllocationClient::new(config.scheduler_nodes.clone())
            .await
            .expect("Failed to create allocation client"),
    );

    // Create request cache
    let cache = Arc::new(RequestCache::new(1000));

    // Create application state
    let state = web::Data::new(GatewayState {
        client,
        cache,
        config: Arc::new(config),
    });

    info!(
        "Starting gateway on http://{}:{}",
        config.host, config.port
    );

    // Start HTTP server
    HttpServer::new(move || {
        App::new()
            .app_data(state.clone())
            .wrap(middleware::Logger::default())
            .wrap(middleware::NormalizePath::trim())
            // Health endpoints
            .route("/health", web::get().to(handlers::health_check))
            .route("/ready", web::get().to(handlers::readiness_check))
            // Allocation endpoints
            .route("/v1/allocate", web::post().to(handlers::allocate))
            .route("/v1/deallocate", web::post().to(handlers::deallocate))
            // Statistics endpoints
            .route("/v1/stats", web::get().to(handlers::get_stats))
            .route("/v1/cluster", web::get().to(handlers::get_cluster_health))
            // Metrics endpoint
            .route("/metrics", web::get().to(handlers::metrics))
    })
    .bind(format!("{}:{}", config.host, config.port))?
    .run()
    .await
}
