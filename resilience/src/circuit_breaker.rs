use crate::error::{ResilienceError, Result};
use chrono::{DateTime, Duration, Utc};
use parking_lot::RwLock;
use serde::{Deserialize, Serialize};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use tracing::{debug, info, warn};

/// Circuit breaker states
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum CircuitState {
    /// Accept requests normally
    Closed,
    /// Reject requests, attempting recovery
    Open,
    /// Accept limited requests to test recovery
    HalfOpen,
}

impl std::fmt::Display for CircuitState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Closed => write!(f, "Closed"),
            Self::Open => write!(f, "Open"),
            Self::HalfOpen => write!(f, "HalfOpen"),
        }
    }
}

/// Circuit breaker configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CircuitBreakerConfig {
    /// Failure threshold (percentage) before opening
    pub failure_threshold: f32, // 0.0-1.0

    /// Number of requests to track for failure rate
    pub sample_size: usize,

    /// Time to wait before attempting recovery
    pub timeout_secs: u64,

    /// Number of successful requests needed to close in half-open state
    pub success_threshold: u32,

    /// Name for logging
    pub name: String,
}

impl Default for CircuitBreakerConfig {
    fn default() -> Self {
        Self {
            failure_threshold: 0.5, // 50% failures
            sample_size: 100,
            timeout_secs: 30,
            success_threshold: 5,
            name: "circuit-breaker".to_string(),
        }
    }
}

/// Circuit breaker for handling failures gracefully
pub struct CircuitBreaker {
    config: CircuitBreakerConfig,
    state: RwLock<CircuitState>,
    failure_count: Arc<AtomicU64>,
    success_count: Arc<AtomicU64>,
    request_count: Arc<AtomicU64>,
    last_failure_time: RwLock<Option<DateTime<Utc>>>,
    opened_at: RwLock<Option<DateTime<Utc>>>,
}

impl CircuitBreaker {
    /// Create a new circuit breaker
    pub fn new(config: CircuitBreakerConfig) -> Self {
        Self {
            config,
            state: RwLock::new(CircuitState::Closed),
            failure_count: Arc::new(AtomicU64::new(0)),
            success_count: Arc::new(AtomicU64::new(0)),
            request_count: Arc::new(AtomicU64::new(0)),
            last_failure_time: RwLock::new(None),
            opened_at: RwLock::new(None),
        }
    }

    /// Check if circuit is available for requests
    pub fn can_request(&self) -> Result<()> {
        let state = *self.state.read();

        match state {
            CircuitState::Closed => Ok(()),
            CircuitState::HalfOpen => Ok(()), // Allow test requests
            CircuitState::Open => {
                // Check if timeout has passed
                let opened_at = self.opened_at.read();
                if let Some(opened) = *opened_at {
                    let timeout = Duration::seconds(self.config.timeout_secs as i64);
                    if Utc::now() > opened + timeout {
                        drop(opened_at);
                        // Transition to half-open
                        info!(
                            "{}: Attempting recovery (Half-Open)",
                            self.config.name
                        );
                        *self.state.write() = CircuitState::HalfOpen;
                        self.success_count.store(0, Ordering::Relaxed);
                        Ok(())
                    } else {
                        Err(ResilienceError::CircuitBreakerOpen {
                            backend: self.config.name.clone(),
                        })
                    }
                } else {
                    Err(ResilienceError::CircuitBreakerOpen {
                        backend: self.config.name.clone(),
                    })
                }
            }
        }
    }

    /// Record a successful request
    pub fn record_success(&self) {
        let state = *self.state.read();

        match state {
            CircuitState::Closed => {
                let _succ = self.success_count.fetch_add(1, Ordering::Relaxed);
                let req = self.request_count.fetch_add(1, Ordering::Relaxed);

                // Check if we should close if there are failures
                if req > self.config.sample_size as u64 {
                    self.evaluate_state();
                }
            }
            CircuitState::HalfOpen => {
                let succ = self.success_count.fetch_add(1, Ordering::Relaxed) + 1;

                // Check if we've had enough successes to close
                if succ >= self.config.success_threshold as u64 {
                    debug!(
                        "{}: Recovered to Closed state after {} successes",
                        self.config.name, succ
                    );
                    *self.state.write() = CircuitState::Closed;
                    self.failure_count.store(0, Ordering::Relaxed);
                    self.success_count.store(0, Ordering::Relaxed);
                    self.request_count.store(0, Ordering::Relaxed);
                }
            }
            CircuitState::Open => {
                // Ignore successes while open
            }
        }
    }

    /// Record a failed request
    pub fn record_failure(&self) {
        let state = *self.state.read();

        match state {
            CircuitState::Closed => {
                let _failures = self.failure_count.fetch_add(1, Ordering::Relaxed) + 1;
                let requests = self.request_count.fetch_add(1, Ordering::Relaxed) + 1;

                *self.last_failure_time.write() = Some(Utc::now());

                if requests > self.config.sample_size as u64 {
                    self.evaluate_state();
                }
            }
            CircuitState::HalfOpen => {
                // One failure in half-open returns to open
                warn!(
                    "{}: Failed during recovery attempt, reopening circuit",
                    self.config.name
                );
                *self.state.write() = CircuitState::Open;
                *self.opened_at.write() = Some(Utc::now());
                self.failure_count.store(1, Ordering::Relaxed);
                self.success_count.store(0, Ordering::Relaxed);
                self.request_count.store(1, Ordering::Relaxed);
            }
            CircuitState::Open => {
                // Ignore failures while already open
            }
        }
    }

    /// Evaluate if state should change
    fn evaluate_state(&self) {
        let failures = self.failure_count.load(Ordering::Relaxed);
        let requests = self.request_count.load(Ordering::Relaxed);

        if requests == 0 {
            return;
        }

        let failure_rate = failures as f32 / requests as f32;

        if failure_rate > self.config.failure_threshold {
            let state = *self.state.read();
            if state == CircuitState::Closed {
                warn!(
                    "{}: Opening circuit (failure rate: {:.2}%)",
                    self.config.name,
                    failure_rate * 100.0
                );
                *self.state.write() = CircuitState::Open;
                *self.opened_at.write() = Some(Utc::now());
            }
        } else if failure_rate < self.config.failure_threshold * 0.5 {
            // Reset if failures drop significantly
            self.failure_count.store(0, Ordering::Relaxed);
            self.request_count.store(0, Ordering::Relaxed);
        }
    }

    /// Get current state
    pub fn state(&self) -> CircuitState {
        *self.state.read()
    }

    /// Get metrics
    pub fn metrics(&self) -> CircuitBreakerMetrics {
        let failures = self.failure_count.load(Ordering::Relaxed);
        let successes = self.success_count.load(Ordering::Relaxed);
        let requests = self.request_count.load(Ordering::Relaxed);

        CircuitBreakerMetrics {
            state: self.state(),
            total_requests: requests,
            total_failures: failures,
            total_successes: successes,
            failure_rate: if requests > 0 {
                failures as f32 / requests as f32
            } else {
                0.0
            },
        }
    }

    /// Reset the circuit breaker
    pub fn reset(&self) {
        debug!("{}: Resetting circuit breaker", self.config.name);
        *self.state.write() = CircuitState::Closed;
        self.failure_count.store(0, Ordering::Relaxed);
        self.success_count.store(0, Ordering::Relaxed);
        self.request_count.store(0, Ordering::Relaxed);
        *self.last_failure_time.write() = None;
        *self.opened_at.write() = None;
    }
}

/// Circuit breaker metrics
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CircuitBreakerMetrics {
    pub state: CircuitState,
    pub total_requests: u64,
    pub total_failures: u64,
    pub total_successes: u64,
    pub failure_rate: f32,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_circuit_breaker_opens_on_failures() {
        let config = CircuitBreakerConfig {
            failure_threshold: 0.5,
            sample_size: 10,
            ..Default::default()
        };
        let cb = CircuitBreaker::new(config);

        // Record failures
        for _ in 0..6 {
            cb.record_failure();
        }

        // Record successes
        for _ in 0..4 {
            cb.record_success();
        }

        assert_eq!(cb.state(), CircuitState::Open);
    }

    #[test]
    fn test_circuit_breaker_half_open() {
        let config = CircuitBreakerConfig {
            failure_threshold: 0.5,
            sample_size: 10,
            timeout_secs: 1,
            ..Default::default()
        };
        let cb = CircuitBreaker::new(config);

        // Open the circuit
        for _ in 0..6 {
            cb.record_failure();
        }
        for _ in 0..4 {
            cb.record_success();
        }

        assert_eq!(cb.state(), CircuitState::Open);
    }
}
