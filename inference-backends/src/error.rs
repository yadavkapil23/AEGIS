use thiserror::Error;

/// Backend error types
#[derive(Error, Debug)]
pub enum BackendError {
    #[error("HuggingFace API error: {0}")]
    HuggingFaceError(String),

    #[error("vLLM backend error: {0}")]
    VLLMError(String),

    #[error("All backends unavailable")]
    AllBackendsUnavailable,

    #[error("Backend not configured: {0}")]
    BackendNotConfigured(String),

    #[error("Request timeout")]
    Timeout,

    #[error("Invalid request: {0}")]
    InvalidRequest(String),

    #[error("Model not found: {0}")]
    ModelNotFound(String),

    #[error("HTTP error: {0}")]
    HttpError(String),

    #[error("Serialization error: {0}")]
    SerializationError(#[from] serde_json::Error),

    #[error("Configuration error: {0}")]
    ConfigError(String),

    #[error("Health check failed: {0}")]
    HealthCheckFailed(String),

    #[error("Unknown error: {0}")]
    Unknown(String),

    #[error("Inference error: {0}")]
    InferenceError(String),

    #[error("Rate limited")]
    RateLimited,

    #[error("Circuit breaker open")]
    CircuitBreakerOpen,
}

pub type Result<T> = std::result::Result<T, BackendError>;
