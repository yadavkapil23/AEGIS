//! Observability module error types

use std::fmt;

/// Error type for observability operations
#[derive(Debug)]
pub enum ObservabilityError {
    /// Metrics registry initialization failed
    MetricsInitializationFailed(String),

    /// Tracing initialization failed
    TracingInitializationFailed(String),

    /// Health check failed
    HealthCheckFailed(String),

    /// Invalid configuration
    InvalidConfiguration(String),

    /// Jaeger export failed
    JaegerExportFailed(String),

    /// Metrics scrape failed
    MetricsScrape(String),

    /// Serialization error
    SerializationError(String),
}

impl fmt::Display for ObservabilityError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ObservabilityError::MetricsInitializationFailed(msg) => {
                write!(f, "Metrics initialization failed: {}", msg)
            }
            ObservabilityError::TracingInitializationFailed(msg) => {
                write!(f, "Tracing initialization failed: {}", msg)
            }
            ObservabilityError::HealthCheckFailed(msg) => {
                write!(f, "Health check failed: {}", msg)
            }
            ObservabilityError::InvalidConfiguration(msg) => {
                write!(f, "Invalid configuration: {}", msg)
            }
            ObservabilityError::JaegerExportFailed(msg) => {
                write!(f, "Jaeger export failed: {}", msg)
            }
            ObservabilityError::MetricsScrape(msg) => {
                write!(f, "Metrics scrape failed: {}", msg)
            }
            ObservabilityError::SerializationError(msg) => {
                write!(f, "Serialization error: {}", msg)
            }
        }
    }
}

impl std::error::Error for ObservabilityError {}

/// Result type for observability operations
pub type Result<T> = std::result::Result<T, ObservabilityError>;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_error_display() {
        let err = ObservabilityError::MetricsInitializationFailed("failed".to_string());
        assert_eq!(err.to_string(), "Metrics initialization failed: failed");
    }

    #[test]
    fn test_error_debug() {
        let err = ObservabilityError::InvalidConfiguration("bad config".to_string());
        let debug_str = format!("{:?}", err);
        assert!(debug_str.contains("InvalidConfiguration"));
    }
}
