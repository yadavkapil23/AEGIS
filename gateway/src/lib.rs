// Gateway module: HTTP/gRPC entry point for AEGIS inference

pub mod service;
pub mod auth;
pub mod rate_limiter;
pub mod metrics;
pub mod request_queue;

pub use service::InferenceService;
pub use auth::AuthMiddleware;
pub use rate_limiter::RateLimiter;
pub use metrics::GatewayMetrics;
pub use request_queue::RequestQueue;

use anyhow::Result;
use std::sync::Arc;
use tonic::transport::Server;
use tracing::info;

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
        let addr = format!("{}:{}", self.config.listen_addr, self.config.listen_port)
            .parse()
            .expect("Invalid listen address");

        info!(addr = %addr, "Starting AEGIS Gateway");

        // Create gRPC service
        let service = self.service.clone();
        let inference_svc = aegis_proto::inference_service_server::InferenceServiceServer::new(service);

        // Create graceful shutdown channel
        let (tx, rx) = tokio::sync::oneshot::channel();

        // Spawn server task
        let server_task = tokio::spawn(async move {
            Server::builder()
                .add_service(inference_svc)
                .serve_with_shutdown(addr, async {
                    rx.await.ok();
                })
                .await
        });

        info!("AEGIS Gateway listening on {}", addr);

        // Handle graceful shutdown
        tokio::select! {
            res = server_task => {
                match res {
                    Ok(Ok(())) => {
                        info!("Gateway server shut down gracefully");
                        Ok(())
                    }
                    Ok(Err(e)) => {
                        tracing::error!("Gateway server error: {}", e);
                        Err(e.into())
                    }
                    Err(e) => {
                        tracing::error!("Gateway task error: {}", e);
                        Err(e.into())
                    }
                }
            }
            _ = tokio::signal::ctrl_c() => {
                info!("Received shutdown signal, gracefully shutting down");
                let _ = tx.send(());
                Ok(())
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
