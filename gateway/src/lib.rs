// Gateway module: HTTP/gRPC entry point for AEGIS inference

// Legacy modules
pub mod service;
pub mod auth;
pub mod rate_limiter;
pub mod metrics;
pub mod request_queue;

// Security modules (Axum-based)
pub mod credentials;
pub mod middleware;
pub mod api_key_handlers;

// NEW: Actix-web security & operations modules
pub mod jwt_auth;
pub mod security_middleware;
pub mod request_validator;
pub mod db_migrations;
pub mod backup;
pub mod telemetry;
pub mod backend_manager;
pub mod inference_handler;
pub mod llm_backend;
pub mod database;

// Re-exports
pub use service::InferenceService;
pub use auth::AuthMiddleware;
pub use rate_limiter::RateLimiter;
pub use metrics::GatewayMetrics;
pub use request_queue::RequestQueue;

// Security re-exports
pub use credentials::{extract_client_ip, extract_request_id, Credential};
pub use middleware::GatewayState;

use anyhow::Result;
use std::sync::Arc;
use tracing::info;
use tokio::net::TcpListener;

/// GatewayConfig: configuration for the gateway
#[derive(Debug, Clone)]
pub struct GatewayConfig {
    pub listen_addr: String,
    pub listen_port: u16,
    pub max_concurrent_requests: usize,
    pub request_timeout_ms: u64,
    pub rate_limit_rps: u32,
}

impl Default for GatewayConfig {
    fn default() -> Self {
        Self {
            listen_addr: "127.0.0.1".to_string(),
            listen_port: 50051,
            max_concurrent_requests: 1000,
            request_timeout_ms: 60000,
            rate_limit_rps: 1000,
        }
    }
}

/// GatewayServer: main entry point
pub struct GatewayServer {
    config: GatewayConfig,
    service: Arc<InferenceService>,
    metrics: Arc<GatewayMetrics>,
}

impl GatewayServer {
    pub fn new(config: GatewayConfig) -> Self {
        let metrics = Arc::new(GatewayMetrics::new());
        let service = Arc::new(InferenceService::new(
            config.max_concurrent_requests,
            config.request_timeout_ms,
            metrics.clone(),
        ));

        Self {
            config,
            service,
            metrics,
        }
    }

    /// Run the gateway server
    pub async fn run(&self) -> Result<()> {
        let addr: std::net::SocketAddr = format!("{}:{}", self.config.listen_addr, self.config.listen_port)
            .parse()
            .expect("Invalid listen address");

        info!(addr = %addr, "Starting AEGIS Gateway");

        // Bind to the configured address
        let listener = TcpListener::bind(addr).await?;
        info!("AEGIS Gateway listening on {}", addr);

        // Accept connections until shutdown signal
        tokio::select! {
            result = self.accept_connections(&listener) => {
                result?;
            }
            _ = tokio::signal::ctrl_c() => {
                info!("Received shutdown signal, gracefully shutting down");
            }
        }

        Ok(())
    }

    /// Accept and handle incoming connections
    async fn accept_connections(&self, listener: &TcpListener) -> Result<()> {
        loop {
            match listener.accept().await {
                Ok((_socket, peer_addr)) => {
                    info!("Accepted connection from {}", peer_addr);
                    // For now, just accept and close
                    // Real gRPC handling will be implemented next
                }
                Err(e) => {
                    tracing::error!("Accept error: {}", e);
                }
            }
        }
    }

    pub fn metrics(&self) -> Arc<GatewayMetrics> {
        self.metrics.clone()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_config() {
        let cfg = GatewayConfig::default();
        assert_eq!(cfg.listen_port, 50051);
        assert_eq!(cfg.max_concurrent_requests, 1000);
    }
}
