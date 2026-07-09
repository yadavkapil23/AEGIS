use std::sync::atomic::{AtomicU32, AtomicU64, AtomicUsize, Ordering};
use std::time::Instant;
use parking_lot::Mutex;
use tracing::info;

/// Circuit breaker states mirrored from llm_backend for independence.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CircuitState {
    Closed,
    Open,
    HalfOpen,
}

impl std::fmt::Display for CircuitState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Closed => write!(f, "closed"),
            Self::Open => write!(f, "open"),
            Self::HalfOpen => write!(f, "half-open"),
        }
    }
}

/// Metrics tracked by the backend manager.
#[derive(Debug, Clone)]
pub struct BackendMetrics {
    pub circuit_breaker_state: String,
    pub consecutive_failures: u32,
    pub bulkhead_available: usize,
    pub total_requests: u64,
    pub successful_requests: u64,
    pub failed_requests: u64,
    pub rate_limited_requests: u64,
    pub avg_latency_ms: f64,
}

/// Backend Manager for handling LLM inference with real circuit breaker,
/// bulkhead concurrency control, and latency tracking.
pub struct BackendManager {
    // ── Circuit breaker ─────────────────────────────────────
    circuit_state: Mutex<CircuitState>,
    failure_count: AtomicU32,
    success_count: AtomicU32,
    failure_threshold: u32,
    success_threshold: u32,
    opened_at: Mutex<Option<Instant>>,

    // ── Bulkhead ────────────────────────────────────────────
    bulkhead_max: usize,
    bulkhead_active: AtomicUsize,

    // ── Counters ────────────────────────────────────────────
    total_requests: AtomicU64,
    successful_requests: AtomicU64,
    failed_requests: AtomicU64,
    rate_limited_requests: AtomicU64,

    // ── Latency tracking ────────────────────────────────────
    latency_sum_ms: AtomicU64,
    latency_count: AtomicU64,
}

impl BackendManager {
    pub fn new() -> Result<Self, Box<dyn std::error::Error>> {
        info!("Initializing backend manager with circuit breaker and bulkhead");
        Ok(Self {
            circuit_state: Mutex::new(CircuitState::Closed),
            failure_count: AtomicU32::new(0),
            success_count: AtomicU32::new(0),
            failure_threshold: 5,
            success_threshold: 3,
            opened_at: Mutex::new(None),

            bulkhead_max: 100,
            bulkhead_active: AtomicUsize::new(0),

            total_requests: AtomicU64::new(0),
            successful_requests: AtomicU64::new(0),
            failed_requests: AtomicU64::new(0),
            rate_limited_requests: AtomicU64::new(0),

            latency_sum_ms: AtomicU64::new(0),
            latency_count: AtomicU64::new(0),
        })
    }

    // ── Circuit breaker ─────────────────────────────────────

    /// Check if the circuit breaker allows requests. Returns Err if open.
    pub fn check_circuit_breaker(&self) -> Result<(), String> {
        if self.is_available() {
            Ok(())
        } else {
            Err("Circuit breaker is open".to_string())
        }
    }

    /// Whether the backend is available for requests.
    pub fn is_available(&self) -> bool {
        let state = *self.circuit_state.lock();
        match state {
            CircuitState::Closed | CircuitState::HalfOpen => true,
            CircuitState::Open => {
                // Check if timeout has elapsed → transition to half-open.
                if let Some(opened) = *self.opened_at.lock() {
                    if opened.elapsed().as_secs() >= 30 {
                        *self.circuit_state.lock() = CircuitState::HalfOpen;
                        self.success_count.store(0, Ordering::Relaxed);
                        info!("Circuit breaker entering half-open state (recovery attempt)");
                        return true;
                    }
                }
                false
            }
        }
    }

    pub fn record_success(&self) {
        self.total_requests.fetch_add(1, Ordering::Relaxed);
        self.successful_requests.fetch_add(1, Ordering::Relaxed);

        let state = *self.circuit_state.lock();
        match state {
            CircuitState::HalfOpen => {
                let succ = self.success_count.fetch_add(1, Ordering::Relaxed) + 1;
                if succ >= self.success_threshold {
                    *self.circuit_state.lock() = CircuitState::Closed;
                    self.failure_count.store(0, Ordering::Relaxed);
                    self.success_count.store(0, Ordering::Relaxed);
                    info!("Circuit breaker closed (backend recovered)");
                }
            }
            CircuitState::Closed => {
                self.failure_count.store(0, Ordering::Relaxed);
            }
            _ => {}
        }
    }

    pub fn record_failure(&self) {
        self.total_requests.fetch_add(1, Ordering::Relaxed);
        self.failed_requests.fetch_add(1, Ordering::Relaxed);

        let state = *self.circuit_state.lock();
        match state {
            CircuitState::Closed => {
                let failures = self.failure_count.fetch_add(1, Ordering::Relaxed) + 1;
                if failures >= self.failure_threshold {
                    *self.circuit_state.lock() = CircuitState::Open;
                    *self.opened_at.lock() = Some(Instant::now());
                    self.success_count.store(0, Ordering::Relaxed);
                    tracing::warn!(
                        "Circuit breaker opened ({} consecutive failures)",
                        failures
                    );
                }
            }
            CircuitState::HalfOpen => {
                *self.circuit_state.lock() = CircuitState::Open;
                *self.opened_at.lock() = Some(Instant::now());
                self.failure_count.store(1, Ordering::Relaxed);
                self.success_count.store(0, Ordering::Relaxed);
                tracing::warn!("Circuit breaker reopened (recovery failed)");
            }
            CircuitState::Open => {}
        }
    }

    pub fn reset_circuit_breaker(&self) {
        *self.circuit_state.lock() = CircuitState::Closed;
        self.failure_count.store(0, Ordering::Relaxed);
        self.success_count.store(0, Ordering::Relaxed);
        *self.opened_at.lock() = None;
        info!("Circuit breaker manually reset");
    }

    // ── Bulkhead ────────────────────────────────────────────

    /// Try to acquire a concurrency slot.  Returns `false` if at capacity.
    pub fn try_acquire_bulkhead(&self) -> bool {
        let prev = self.bulkhead_active.fetch_add(1, Ordering::SeqCst);
        if prev < self.bulkhead_max {
            true
        } else {
            self.bulkhead_active.fetch_sub(1, Ordering::SeqCst);
            false
        }
    }

    pub fn release_bulkhead(&self) {
        self.bulkhead_active.fetch_sub(1, Ordering::SeqCst);
    }

    // ── Rate limiting counter ───────────────────────────────

    pub fn record_rate_limited(&self) {
        self.rate_limited_requests.fetch_add(1, Ordering::Relaxed);
    }

    // ── Latency tracking ────────────────────────────────────

    pub fn record_latency(&self, latency_ms: u64) {
        self.latency_sum_ms.fetch_add(latency_ms, Ordering::Relaxed);
        self.latency_count.fetch_add(1, Ordering::Relaxed);
    }

    // ── Metrics snapshot ────────────────────────────────────

    pub fn metrics(&self) -> BackendMetrics {
        let count = self.latency_count.load(Ordering::Relaxed);
        let sum = self.latency_sum_ms.load(Ordering::Relaxed);
        let avg = if count > 0 { sum as f64 / count as f64 } else { 0.0 };

        BackendMetrics {
            circuit_breaker_state: self.circuit_state.lock().to_string(),
            consecutive_failures: self.failure_count.load(Ordering::Relaxed),
            bulkhead_available: self.bulkhead_max.saturating_sub(
                self.bulkhead_active.load(Ordering::Relaxed),
            ),
            total_requests: self.total_requests.load(Ordering::Relaxed),
            successful_requests: self.successful_requests.load(Ordering::Relaxed),
            failed_requests: self.failed_requests.load(Ordering::Relaxed),
            rate_limited_requests: self.rate_limited_requests.load(Ordering::Relaxed),
            avg_latency_ms: avg,
        }
    }
}

impl Default for BackendManager {
    fn default() -> Self {
        Self::new().unwrap_or_else(|_| {
            // Fallback — should never happen since new() only fails on Box<dyn Error>
            panic!("Failed to create default BackendManager")
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn starts_closed_and_available() {
        let bm = BackendManager::new().unwrap();
        assert!(bm.is_available());
        assert_eq!(bm.metrics().circuit_breaker_state, "closed");
    }

    #[test]
    fn opens_after_threshold_failures() {
        let bm = BackendManager::new().unwrap();
        for _ in 0..5 {
            bm.record_failure();
        }
        assert!(!bm.is_available());
        assert_eq!(bm.metrics().circuit_breaker_state, "open");
    }

    #[test]
    fn success_resets_failure_count() {
        let bm = BackendManager::new().unwrap();
        for _ in 0..3 {
            bm.record_failure();
        }
        bm.record_success();
        // Should still be closed because success reset the count.
        assert!(bm.is_available());
    }

    #[test]
    fn bulkhead_rejects_when_full() {
        let bm = BackendManager::new().unwrap();
        // Fill all 100 slots.
        for _ in 0..100 {
            assert!(bm.try_acquire_bulkhead());
        }
        assert!(!bm.try_acquire_bulkhead());
        // Release one.
        bm.release_bulkhead();
        assert!(bm.try_acquire_bulkhead());
    }

    #[test]
    fn latency_tracking() {
        let bm = BackendManager::new().unwrap();
        bm.record_latency(100);
        bm.record_latency(200);
        let m = bm.metrics();
        assert!((m.avg_latency_ms - 150.0).abs() < f64::EPSILON);
    }

    #[test]
    fn manual_reset() {
        let bm = BackendManager::new().unwrap();
        for _ in 0..5 {
            bm.record_failure();
        }
        assert!(!bm.is_available());
        bm.reset_circuit_breaker();
        assert!(bm.is_available());
    }
}
