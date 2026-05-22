use thiserror::Error;

/// Resilience layer error types
#[derive(Error, Debug, Clone)]
pub enum ResilienceError {
    #[error("Circuit breaker is open for {backend}")]
    CircuitBreakerOpen { backend: String },

    #[error("Max retries exceeded: {reason}")]
    MaxRetriesExceeded { reason: String },

    #[error("Request timeout after {timeout_ms}ms")]
    Timeout { timeout_ms: u64 },

    #[error("Degraded service: {reason}")]
    DegradedService { reason: String },

    #[error("All backend attempts failed: {reason}")]
    AllAttemptsFailed { reason: String },

    #[error("Invalid configuration: {0}")]
    InvalidConfig(String),

    #[error("Health check failed: {0}")]
    HealthCheckFailed(String),

    #[error("Rate limited: {reason}")]
    RateLimited { reason: String },

    #[error("Backend unavailable: {reason}")]
    BackendUnavailable { reason: String },

    #[error("Unknown error: {0}")]
    Unknown(String),
}

pub type Result<T> = std::result::Result<T, ResilienceError>;
