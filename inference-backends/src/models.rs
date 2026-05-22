use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

/// Backend preference for routing
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum BackendPreference {
    /// Always use Hugging Face API
    HuggingFace,
    /// Always use vLLM cluster
    VLLm,
    /// Let router decide based on heuristics
    Auto,
}

impl Default for BackendPreference {
    fn default() -> Self {
        Self::Auto
    }
}

/// Inference request
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InferenceRequest {
    /// Unique request ID
    pub request_id: String,

    /// Model name/identifier
    pub model: String,

    /// Input prompt/text
    pub prompt: String,

    /// Maximum tokens to generate
    pub max_tokens: Option<u32>,

    /// Temperature for sampling (0.0-2.0)
    pub temperature: Option<f32>,

    /// Top-p sampling parameter
    pub top_p: Option<f32>,

    /// Preferred backend
    pub backend_preference: BackendPreference,

    /// Request timeout in milliseconds
    pub timeout_ms: Option<u64>,

    /// Whether to return token probabilities
    pub include_probabilities: Option<bool>,

    /// Metadata for tracking
    pub metadata: Option<std::collections::HashMap<String, String>>,

    /// Timestamp when request was created
    pub created_at: DateTime<Utc>,
}

impl InferenceRequest {
    pub fn new(model: impl Into<String>, prompt: impl Into<String>) -> Self {
        Self {
            request_id: uuid::Uuid::new_v4().to_string(),
            model: model.into(),
            prompt: prompt.into(),
            max_tokens: None,
            temperature: None,
            top_p: None,
            backend_preference: BackendPreference::Auto,
            timeout_ms: None,
            include_probabilities: None,
            metadata: None,
            created_at: Utc::now(),
        }
    }

    pub fn with_backend(mut self, preference: BackendPreference) -> Self {
        self.backend_preference = preference;
        self
    }

    pub fn with_max_tokens(mut self, max_tokens: u32) -> Self {
        self.max_tokens = Some(max_tokens);
        self
    }

    pub fn with_temperature(mut self, temperature: f32) -> Self {
        self.temperature = Some(temperature);
        self
    }

    pub fn with_timeout_ms(mut self, timeout_ms: u64) -> Self {
        self.timeout_ms = Some(timeout_ms);
        self
    }
}

/// Inference response
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InferenceResponse {
    /// Request ID this response corresponds to
    pub request_id: String,

    /// Generated text
    pub text: String,

    /// Number of tokens generated
    pub tokens_generated: u32,

    /// Which backend was used
    pub backend_used: String,

    /// Processing time in milliseconds
    pub processing_time_ms: u64,

    /// Token probabilities if requested
    pub token_probabilities: Option<Vec<TokenProbability>>,

    /// Finish reason (stop, length, etc.)
    pub finish_reason: String,

    /// Timestamp when response was created
    pub created_at: DateTime<Utc>,
}

/// Token probability information
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TokenProbability {
    pub token: String,
    pub probability: f32,
}

/// Backend health status
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HealthStatus {
    /// Overall status
    pub healthy: bool,

    /// Which backend
    pub backend: String,

    /// Status message
    pub status: String,

    /// Average latency in ms
    pub latency_ms: f32,

    /// Request count
    pub request_count: u64,

    /// Error count
    pub error_count: u64,

    /// Last check time
    pub last_check: DateTime<Utc>,
}

/// Inference statistics
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InferenceStats {
    pub total_requests: u64,
    pub total_errors: u64,
    pub total_tokens_generated: u64,
    pub avg_latency_ms: f32,
    pub p99_latency_ms: f32,
    pub hf_requests: u64,
    pub vllm_requests: u64,
}
