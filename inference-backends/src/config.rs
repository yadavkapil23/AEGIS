use serde::{Deserialize, Serialize};

/// Backend configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BackendConfig {
    /// Hugging Face API configuration
    pub huggingface: Option<HuggingFaceConfig>,

    /// vLLM configuration
    pub vllm: Option<VLLMConfig>,

    /// llama.cpp configuration
    pub llamacpp: Option<LlamaCppConfig>,

    /// Default backend preference
    pub default_preference: String, // "auto", "huggingface", "vllm", "llamacpp"

    /// Fallback order when backends fail
    pub fallback_order: Vec<String>, // ["vllm", "llamacpp", "huggingface"]

    /// Global timeout in milliseconds
    pub default_timeout_ms: u64,

    /// Enable health checks
    pub enable_health_checks: bool,

    /// Health check interval in seconds
    pub health_check_interval_secs: u64,
}

/// Hugging Face API configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HuggingFaceConfig {
    /// Enable this backend
    pub enabled: bool,

    /// API key (from HF_API_KEY env var if not set)
    pub api_key: Option<String>,

    /// API endpoint
    pub endpoint: String,

    /// List of supported models
    pub models: Vec<String>,

    /// Request timeout
    pub timeout_ms: u64,

    /// Max concurrent requests
    pub max_concurrent_requests: usize,

    /// Cache responses
    pub enable_cache: bool,

    /// Cache TTL in seconds
    pub cache_ttl_secs: u64,
}

/// vLLM configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VLLMConfig {
    /// Enable this backend
    pub enabled: bool,

    /// vLLM endpoints (node URLs)
    pub endpoints: Vec<String>,

    /// List of supported models
    pub models: Vec<String>,

    /// Request timeout
    pub timeout_ms: u64,

    /// Max concurrent requests per endpoint
    pub max_concurrent_requests: usize,

    /// Load balancing strategy: "round_robin", "least_loaded", "random"
    pub load_balancing: String,

    /// Enable endpoint caching/reuse
    pub enable_connection_pooling: bool,

    /// Pool size per endpoint
    pub pool_size: usize,
}

/// llama.cpp configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LlamaCppConfig {
    /// Enable this backend
    pub enabled: bool,

    /// llama.cpp server endpoint
    pub endpoint: String,

    /// List of supported models
    pub models: Vec<String>,

    /// Request timeout
    pub timeout_ms: u64,

    /// Max concurrent requests
    pub max_concurrent_requests: usize,

    /// Enable GPU acceleration
    pub gpu_enabled: bool,

    /// Number of GPU layers
    pub gpu_layers: u32,

    /// CPU thread count
    pub threads: u32,

    /// Context size
    pub context_size: u32,

    /// Batch size
    pub batch_size: u32,
}

impl Default for BackendConfig {
    fn default() -> Self {
        Self {
            huggingface: None,
            vllm: None,
            llamacpp: None,
            default_preference: "auto".to_string(),
            fallback_order: vec![
                "vllm".to_string(),
                "llamacpp".to_string(),
                "huggingface".to_string(),
            ],
            default_timeout_ms: 30000,
            enable_health_checks: true,
            health_check_interval_secs: 60,
        }
    }
}

impl Default for HuggingFaceConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            api_key: std::env::var("HF_API_KEY").ok(),
            endpoint: "https://api-inference.huggingface.co".to_string(),
            models: vec![
                "mistralai/Mistral-7B-Instruct-v0.2".to_string(),
                "meta-llama/Llama-2-7b-hf".to_string(),
                "NousResearch/Nous-Hermes-2-Mixtral-8x7B".to_string(),
            ],
            timeout_ms: 30000,
            max_concurrent_requests: 100,
            enable_cache: true,
            cache_ttl_secs: 3600,
        }
    }
}

impl Default for VLLMConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            endpoints: vec!["http://localhost:8000".to_string()],
            models: vec![
                "mistralai/Mistral-7B-Instruct-v0.2".to_string(),
                "meta-llama/Llama-2-7b-hf".to_string(),
            ],
            timeout_ms: 30000,
            max_concurrent_requests: 1000,
            load_balancing: "round_robin".to_string(),
            enable_connection_pooling: true,
            pool_size: 10,
        }
    }
}

impl Default for LlamaCppConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            endpoint: "http://localhost:8080".to_string(),
            models: vec![
                "mistral-7b".to_string(),
                "llama2-7b".to_string(),
            ],
            timeout_ms: 30000,
            max_concurrent_requests: 50,
            gpu_enabled: true,
            gpu_layers: 33,
            threads: 4,
            context_size: 4096,
            batch_size: 512,
        }
    }
}

/// Load configuration from YAML file or environment
pub fn load_config(path: Option<&str>) -> crate::Result<BackendConfig> {
    if let Some(path) = path {
        let content = std::fs::read_to_string(path)
            .map_err(|e| crate::BackendError::ConfigError(format!("Failed to read config: {}", e)))?;
        let config: BackendConfig = serde_yaml::from_str(&content)
            .map_err(|e| crate::BackendError::ConfigError(format!("Failed to parse config: {}", e)))?;
        Ok(config)
    } else {
        Ok(BackendConfig::default())
    }
}
