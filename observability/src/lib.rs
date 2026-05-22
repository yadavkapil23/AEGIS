//! AEGIS Observability Module
//!
//! Comprehensive observability framework providing metrics collection, distributed tracing,
//! and health probing for production inference systems.
//!
//! # Features
//!
//! - **Metrics**: Prometheus-compatible metrics for inference, backend health, resilience patterns
//! - **Tracing**: Structured JSON logging with OpenTelemetry integration for distributed tracing
//! - **Health Probes**: Liveness and readiness probes for Kubernetes and load balancers
//!
//! # Quick Start
//!
//! ```ignore
//! use observability::{
//!     MetricsRegistry, TracingConfig, HealthManager,
//!     init_tracing, create_span,
//! };
//!
//! // Initialize tracing
//! let config = TracingConfig::default();
//! init_tracing(&config);
//!
//! // Get metrics instance
//! let metrics = &observability::METRICS;
//! metrics.record_inference_request("hf-api", 150.0);
//! metrics.record_backend_health("vllm-1", 1.0);
//!
//! // Create health manager
//! let health = HealthManager::new();
//! health.mark_inference_ready();
//! let readiness = health.get_readiness();
//! ```

pub mod error;
pub mod health;
pub mod metrics;
pub mod tracing;

pub use error::{ObservabilityError, Result};
pub use health::{HealthManager, HealthState, LivenessProbe, ReadinessProbe, ReadyCheck};
pub use metrics::{MetricsRegistry, METRICS};
pub use tracing::{create_span, init_tracing, TracingConfig};

/// Observability prelude - commonly used items
pub mod prelude {
    pub use crate::health::{HealthManager, HealthState};
    pub use crate::metrics::METRICS;
    pub use crate::tracing::{create_span, init_tracing, TracingConfig};
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_metrics_exports() {
        // Verify metrics can be accessed globally
        let _metrics = &METRICS;
        // Should not panic
    }

    #[test]
    fn test_health_manager_creation() {
        let health = HealthManager::new();
        assert_eq!(health.get_state(), HealthState::NotReady);
    }

    #[test]
    fn test_tracing_config_defaults() {
        let config = TracingConfig::default();
        assert_eq!(config.log_level, "info");
        assert!(!config.json_format);
    }
}
