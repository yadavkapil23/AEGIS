//! Resilience Module
//!
//! Provides patterns and utilities for building resilient distributed systems:
//! - Circuit Breaker: Prevent cascading failures
//! - Retry Logic: Handle transient failures with exponential backoff
//! - Timeout Enforcement: Prevent resource exhaustion
//! - Graceful Degradation: Maintain service under adverse conditions

pub mod circuit_breaker;
pub mod error;
pub mod graceful_degradation;
pub mod retry;
pub mod timeout;

pub use circuit_breaker::{CircuitBreaker, CircuitBreakerConfig, CircuitState};
pub use error::{ResilienceError, Result};
pub use graceful_degradation::{DegradationLevel, GracefulDegradation};
pub use retry::{RetryConfig, RetryHandler};
pub use timeout::TimeoutHandler;

/// Prelude - common imports
pub mod prelude {
    pub use crate::{
        CircuitBreaker, CircuitBreakerConfig, CircuitState, DegradationLevel,
        GracefulDegradation, ResilienceError, Result, RetryConfig, RetryHandler,
        TimeoutHandler,
    };
}
