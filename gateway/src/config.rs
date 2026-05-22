/// Gateway Configuration
/// Loads from environment variables with sensible defaults
/// ✅ REAL BACKENDS (vLLM, llama.cpp) enabled by default
/// ❌ MOCK BACKEND disabled by default (for testing only)

use std::env;
use tracing::info;

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
    /// Rate limit (RPS)
    pub rate_limit_rps: u32,
    /// Circuit breaker threshold
    pub circuit_breaker_threshold: u32,
}

impl GatewayConfig {
    /// Load configuration from environment
    /// ✅ Real backends (vLLM, llama.cpp) enabled by default
    /// ❌ Mock backend disabled by default
    pub fn from_env() -> Self {
        // Check if mock backend is enabled (testing only)
        let mock_enabled = env::var("MOCK_BACKEND_ENABLED")
            .unwrap_or_else(|_| "false".to_string())
            .parse::<bool>()
            .unwrap_or(false);

        if mock_enabled {
            info!("⚠️  WARNING: MOCK BACKEND ENABLED - Generates FAKE tokens!");
            info!("For production, use real backends:");
            info!("  - vLLM (high-performance): VLLM_ENDPOINTS=http://localhost:8000");
            info!("  - llama.cpp (lightweight): LLAMACPP_ENDPOINT=http://localhost:8001");
        } else {
            info!("✅ REAL BACKENDS enabled (vLLM + llama.cpp)");
        }

        // vLLM configuration (PRIMARY backend)
        let vllm_endpoint = env::var("VLLM_ENDPOINTS")
            .unwrap_or_else(|_| "http://localhost:8000".to_string());
        info!("vLLM Endpoint: {}", vllm_endpoint);

        // llama.cpp configuration (FALLBACK backend)
        let llamacpp_endpoint = env::var("LLAMACPP_ENDPOINT")
            .unwrap_or_else(|_| "http://localhost:8001".to_string());
        info!("llama.cpp Endpoint: {}", llamacpp_endpoint);

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
            rate_limit_rps: env::var("RATE_LIMIT_RPS")
                .ok()
                .and_then(|r| r.parse().ok())
                .unwrap_or(100),
            circuit_breaker_threshold: env::var("CIRCUIT_BREAKER_THRESHOLD")
                .ok()
                .and_then(|t| t.parse().ok())
                .unwrap_or(5),
        }
    }

    /// Default configuration (uses real backends)
    pub fn default() -> Self {
        Self {
            host: "0.0.0.0".to_string(),
            port: 8080,
            scheduler_nodes: vec!["http://localhost:50052".to_string()],
            cache_size: 1000,
            request_timeout_secs: 30,
            log_level: "info".to_string(),
            rate_limit_rps: 100,
            circuit_breaker_threshold: 5,
        }
    }
}
