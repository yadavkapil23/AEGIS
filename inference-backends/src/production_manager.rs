/// Production-Grade Backend Manager
/// Provides resilience patterns, circuit breaking, retries, and fallback handling

use crate::models::InferenceRequest;
use crate::models::InferenceResponse;
use crate::traits::InferenceBackend;
use crate::error::Result;
use std::sync::Arc;
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::{Instant, Duration};
use tokio::sync::RwLock;
use tracing::{warn, error, info, debug};

/// Circuit breaker state
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CircuitState {
    Closed,      // Normal operation
    Open,        // Failing, reject requests
    HalfOpen,    // Testing recovery
}

/// Circuit breaker configuration
#[derive(Clone, Debug)]
pub struct CircuitBreakerConfig {
    pub failure_threshold: u32,      // Failures before opening
    pub reset_timeout_secs: u32,     // Time before trying to recover
    pub half_open_requests: u32,     // Requests to try when half-open
}

impl Default for CircuitBreakerConfig {
    fn default() -> Self {
        Self {
            failure_threshold: 5,
            reset_timeout_secs: 30,
            half_open_requests: 3,
        }
    }
}

/// Circuit breaker for a single backend
pub struct CircuitBreaker {
    state: Arc<RwLock<CircuitState>>,
    failure_count: Arc<AtomicU32>,
    success_count: Arc<AtomicU32>,
    last_failure_time: Arc<RwLock<Option<Instant>>>,
    config: CircuitBreakerConfig,
}

impl CircuitBreaker {
    pub fn new(config: CircuitBreakerConfig) -> Self {
        Self {
            state: Arc::new(RwLock::new(CircuitState::Closed)),
            failure_count: Arc::new(AtomicU32::new(0)),
            success_count: Arc::new(AtomicU32::new(0)),
            last_failure_time: Arc::new(RwLock::new(None)),
            config,
        }
    }

    /// Check if request should be allowed
    pub async fn allow_request(&self) -> bool {
        let state = *self.state.read().await;

        match state {
            CircuitState::Closed => true,
            CircuitState::Open => {
                // Check if we should try recovery
                if let Some(last_failure) = *self.last_failure_time.read().await {
                    if last_failure.elapsed() >= Duration::from_secs(self.config.reset_timeout_secs as u64) {
                        // Time to try recovery
                        *self.state.write().await = CircuitState::HalfOpen;
                        true
                    } else {
                        false
                    }
                } else {
                    false
                }
            }
            CircuitState::HalfOpen => true,
        }
    }

    /// Record a successful request
    pub async fn record_success(&self) {
        let state = *self.state.read().await;

        if state == CircuitState::HalfOpen {
            // Increment recovery counter
            let successes = self.success_count.fetch_add(1, Ordering::SeqCst);

            if successes + 1 >= self.config.half_open_requests {
                // Recovered! Close the circuit
                *self.state.write().await = CircuitState::Closed;
                self.failure_count.store(0, Ordering::SeqCst);
                self.success_count.store(0, Ordering::SeqCst);
                info!("Circuit breaker closed (recovered)");
            }
        }
    }

    /// Record a failed request
    pub async fn record_failure(&self) {
        let failures = self.failure_count.fetch_add(1, Ordering::SeqCst);
        *self.last_failure_time.write().await = Some(Instant::now());

        let state = *self.state.read().await;

        if failures + 1 >= self.config.failure_threshold && state == CircuitState::Closed {
            // Open the circuit
            *self.state.write().await = CircuitState::Open;
            warn!("Circuit breaker OPEN after {} failures", failures + 1);
        } else if state == CircuitState::HalfOpen {
            // Failed recovery attempt, back to open
            *self.state.write().await = CircuitState::Open;
            self.success_count.store(0, Ordering::SeqCst);
            warn!("Circuit breaker reopened (recovery failed)");
        }
    }

    /// Get current state
    pub async fn state(&self) -> CircuitState {
        *self.state.read().await
    }
}

/// Retry configuration
#[derive(Clone, Debug)]
pub struct RetryConfig {
    pub max_retries: u32,
    pub initial_backoff_ms: u32,
    pub max_backoff_ms: u32,
    pub backoff_multiplier: f32,
}

impl Default for RetryConfig {
    fn default() -> Self {
        Self {
            max_retries: 3,
            initial_backoff_ms: 100,
            max_backoff_ms: 5000,
            backoff_multiplier: 2.0,
        }
    }
}

/// Rate limiter using token bucket algorithm
pub struct RateLimiter {
    tokens: Arc<AtomicU32>,
    capacity: u32,
    refill_rate: u32,  // tokens per second
    last_refill: Arc<RwLock<Instant>>,
}

impl RateLimiter {
    pub fn new(capacity: u32, refill_rate: u32) -> Self {
        Self {
            tokens: Arc::new(AtomicU32::new(capacity)),
            capacity,
            refill_rate,
            last_refill: Arc::new(RwLock::new(Instant::now())),
        }
    }

    /// Try to acquire a token
    pub async fn acquire(&self) -> bool {
        // Refill based on elapsed time
        let now = Instant::now();
        let mut last = self.last_refill.write().await;
        let elapsed = now.duration_since(*last).as_millis() as u32;

        if elapsed > 0 {
            let tokens_to_add = (elapsed as u64 * self.refill_rate as u64 / 1000) as u32;
            let current = self.tokens.load(Ordering::SeqCst);
            let new_tokens = std::cmp::min(current + tokens_to_add, self.capacity);

            self.tokens.store(new_tokens, Ordering::SeqCst);
            *last = now;
        }

        // Try to consume a token
        let mut current = self.tokens.load(Ordering::SeqCst);
        loop {
            if current == 0 {
                return false;
            }

            match self.tokens.compare_exchange(
                current,
                current - 1,
                Ordering::SeqCst,
                Ordering::SeqCst,
            ) {
                Ok(_) => return true,
                Err(actual) => current = actual,
            }
        }
    }
}

/// Bulkhead pattern - isolation between workloads
#[derive(Clone)]
pub struct Bulkhead {
    semaphore: Arc<tokio::sync::Semaphore>,
    limit: usize,
}

impl Bulkhead {
    pub fn new(limit: usize) -> Self {
        Self {
            semaphore: Arc::new(tokio::sync::Semaphore::new(limit)),
            limit,
        }
    }

    pub async fn acquire(&self) -> Result<tokio::sync::SemaphorePermit<'_>> {
        self.semaphore.acquire().await.map_err(|e| {
            crate::error::BackendError::InferenceError(format!("Bulkhead limit exceeded: {}", e))
        })
    }

    pub fn available(&self) -> usize {
        self.semaphore.available_permits()
    }
}

/// Health check configuration
#[derive(Clone, Debug)]
pub struct HealthCheckConfig {
    pub interval_secs: u32,
    pub timeout_secs: u32,
    pub unhealthy_threshold: u32,
}

impl Default for HealthCheckConfig {
    fn default() -> Self {
        Self {
            interval_secs: 30,
            timeout_secs: 5,
            unhealthy_threshold: 3,
        }
    }
}

/// Production backend wrapper with all resilience patterns
pub struct ProductionBackendManager {
    backend: Arc<dyn InferenceBackend>,
    circuit_breaker: CircuitBreaker,
    retry_config: RetryConfig,
    rate_limiter: RateLimiter,
    bulkhead: Bulkhead,
    health_check_config: HealthCheckConfig,
    consecutive_failures: Arc<AtomicU32>,
}

impl ProductionBackendManager {
    pub fn new(
        backend: Arc<dyn InferenceBackend>,
        cb_config: CircuitBreakerConfig,
        retry_config: RetryConfig,
        rate_limit_rps: u32,
    ) -> Self {
        Self {
            backend,
            circuit_breaker: CircuitBreaker::new(cb_config),
            retry_config,
            rate_limiter: RateLimiter::new(rate_limit_rps, rate_limit_rps),
            bulkhead: Bulkhead::new(100),
            health_check_config: HealthCheckConfig::default(),
            consecutive_failures: Arc::new(AtomicU32::new(0)),
        }
    }

    /// Execute inference with all resilience patterns
    pub async fn infer(&self, request: InferenceRequest) -> Result<InferenceResponse> {
        // 1. Check rate limiter
        if !self.rate_limiter.acquire().await {
            debug!("Rate limit exceeded");
            return Err(crate::error::BackendError::RateLimited);
        }

        // 2. Check circuit breaker
        if !self.circuit_breaker.allow_request().await {
            error!("Circuit breaker is open");
            return Err(crate::error::BackendError::CircuitBreakerOpen);
        }

        // 3. Acquire bulkhead permit (limits concurrent requests)
        let _permit = self.bulkhead.acquire().await?;

        // 4. Execute with retries
        let mut last_error = None;
        let mut backoff_ms = self.retry_config.initial_backoff_ms;

        for attempt in 0..=self.retry_config.max_retries {
            debug!("Inference attempt {}/{}", attempt + 1, self.retry_config.max_retries + 1);

            match self.backend.infer(request.clone()).await {
                Ok(response) => {
                    // Success! Reset failure counter
                    self.consecutive_failures.store(0, Ordering::SeqCst);
                    self.circuit_breaker.record_success().await;
                    return Ok(response);
                }
                Err(e) => {
                    last_error = Some(e);

                    // Record failure
                    self.consecutive_failures.fetch_add(1, Ordering::SeqCst);
                    self.circuit_breaker.record_failure().await;

                    // Don't retry on last attempt
                    if attempt < self.retry_config.max_retries {
                        // Exponential backoff with jitter
                        let jitter = (rand::random::<u32>() % (backoff_ms / 2)) as u64;
                        let sleep_ms = backoff_ms as u64 + jitter;

                        debug!(
                            "Retry in {}ms (backoff: {}, attempt: {})",
                            sleep_ms, backoff_ms, attempt + 1
                        );

                        tokio::time::sleep(Duration::from_millis(sleep_ms)).await;

                        // Increase backoff
                        backoff_ms = std::cmp::min(
                            (backoff_ms as f32 * self.retry_config.backoff_multiplier) as u32,
                            self.retry_config.max_backoff_ms,
                        );
                    }
                }
            }
        }

        // All retries exhausted
        error!("All retries exhausted");
        Err(last_error.unwrap_or_else(|| {
            crate::error::BackendError::InferenceError("Unknown error".to_string())
        }))
    }

    /// Get circuit breaker state
    pub async fn circuit_breaker_state(&self) -> CircuitState {
        self.circuit_breaker.state().await
    }

    /// Get health status
    pub async fn health_check(&self) -> Result<()> {
        self.backend.health_check().await?;
        Ok(())
    }

    /// Get consecutive failures count
    pub fn consecutive_failures(&self) -> u32 {
        self.consecutive_failures.load(Ordering::SeqCst)
    }

    /// Get bulkhead availability
    pub fn bulkhead_available(&self) -> usize {
        self.bulkhead.available()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_circuit_breaker_opens_after_threshold() {
        let cb = CircuitBreaker::new(CircuitBreakerConfig {
            failure_threshold: 3,
            ..Default::default()
        });

        // Allow requests initially
        assert!(cb.allow_request().await);
        assert_eq!(cb.state().await, CircuitState::Closed);

        // Simulate failures
        cb.record_failure().await;
        cb.record_failure().await;
        cb.record_failure().await;

        // Circuit should open
        assert_eq!(cb.state().await, CircuitState::Open);
        assert!(!cb.allow_request().await);
    }

    #[tokio::test]
    async fn test_rate_limiter() {
        let limiter = RateLimiter::new(5, 10); // 5 tokens, 10 per second

        // Should allow up to capacity
        for _ in 0..5 {
            assert!(limiter.acquire().await);
        }

        // Should deny when exhausted
        assert!(!limiter.acquire().await);

        // Wait for refill
        tokio::time::sleep(Duration::from_millis(500)).await;
        assert!(limiter.acquire().await);
    }

    #[tokio::test]
    async fn test_bulkhead() {
        let bulkhead = Bulkhead::new(2);

        // Acquire all permits
        let _p1 = bulkhead.acquire().await.unwrap();
        let _p2 = bulkhead.acquire().await.unwrap();

        // Should be exhausted
        assert_eq!(bulkhead.available(), 0);

        // Drop a permit
        drop(_p1);

        // Should have capacity now
        assert!(bulkhead.available() > 0);
    }
}
