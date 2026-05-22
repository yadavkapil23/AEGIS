// Gateway configuration

use std::env;

#[derive(Debug, Clone)]
pub struct GatewayConfig {
    /// Gateway host
    pub host: String,
    /// Gateway port
    pub port: u16,
    /// Scheduler nodes (gRPC endpoints)
    pub scheduler_nodes: Vec<String>,
    /// Request cache size
    pub cache_size: usize,
    /// Request timeout in seconds
    pub request_timeout_secs: u64,
    /// Log level
    pub log_level: String,
}

impl GatewayConfig {
    /// Load configuration from environment
    pub fn from_env() -> Self {
        Self {
            host: env::var("GATEWAY_HOST").unwrap_or_else(|_| "0.0.0.0".to_string()),
            port: env::var("GATEWAY_PORT")
                .ok()
                .and_then(|p| p.parse().ok())
                .unwrap_or(8080),
            scheduler_nodes: env::var("SCHEDULER_NODES")
                .unwrap_or_else(|_| "http://localhost:50052".to_string())
                .split(',')
                .map(|s| s.to_string())
                .collect(),
            cache_size: env::var("GATEWAY_CACHE_SIZE")
                .ok()
                .and_then(|s| s.parse().ok())
                .unwrap_or(1000),
            request_timeout_secs: env::var("GATEWAY_TIMEOUT")
                .ok()
                .and_then(|t| t.parse().ok())
                .unwrap_or(30),
            log_level: env::var("GATEWAY_LOG_LEVEL").unwrap_or_else(|_| "info".to_string()),
        }
    }

    /// Default configuration
    pub fn default() -> Self {
        Self {
            host: "0.0.0.0".to_string(),
            port: 8080,
            scheduler_nodes: vec!["http://localhost:50052".to_string()],
            cache_size: 1000,
            request_timeout_secs: 30,
            log_level: "info".to_string(),
        }
    }
}
