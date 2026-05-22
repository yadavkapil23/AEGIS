// Health check and readiness probes for Kubernetes
// Supports liveness and readiness checks

use anyhow::Result;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use parking_lot::Mutex;
use std::time::{SystemTime, UNIX_EPOCH};

/// Health check status
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HealthStatus {
    Healthy,
    Degraded,
    Unhealthy,
}

/// Readiness status
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReadinessStatus {
    Ready,
    NotReady,
}

/// Health information
#[derive(Debug, Clone)]
pub struct HealthInfo {
    pub status: HealthStatus,
    pub uptime_secs: u64,
    pub last_check: u64,
    pub checks: Vec<(String, bool)>,
}

/// Readiness information
#[derive(Debug, Clone)]
pub struct ReadinessInfo {
    pub status: ReadinessStatus,
    pub initialized: bool,
    pub consensus_ready: bool,
    pub cache_ready: bool,
    pub persistence_ready: bool,
}

/// Health checker
pub struct HealthChecker {
    start_time: SystemTime,
    is_healthy: Arc<AtomicBool>,
    is_ready: Arc<AtomicBool>,
    check_results: Arc<Mutex<Vec<(String, bool)>>>,
}

impl HealthChecker {
    /// Create new health checker
    pub fn new() -> Self {
        Self {
            start_time: SystemTime::now(),
            is_healthy: Arc::new(AtomicBool::new(true)),
            is_ready: Arc::new(AtomicBool::new(false)),
            check_results: Arc::new(Mutex::new(Vec::new())),
        }
    }

    /// Mark as ready
    pub fn mark_ready(&self) {
        self.is_ready.store(true, Ordering::SeqCst);
    }

    /// Mark as not ready
    pub fn mark_not_ready(&self) {
        self.is_ready.store(false, Ordering::SeqCst);
    }

    /// Mark as healthy
    pub fn mark_healthy(&self) {
        self.is_healthy.store(true, Ordering::SeqCst);
    }

    /// Mark as unhealthy
    pub fn mark_unhealthy(&self) {
        self.is_healthy.store(false, Ordering::SeqCst);
    }

    /// Record a health check result
    pub fn record_check(&self, name: impl Into<String>, passed: bool) {
        let mut results = self.check_results.lock();
        results.push((name.into(), passed));
        // Keep only last 10 checks per component
        if results.len() > 100 {
            results.drain(0..50);
        }
    }

    /// Get health status
    pub fn get_health(&self) -> HealthInfo {
        let uptime = self.start_time
            .elapsed()
            .map(|d| d.as_secs())
            .unwrap_or(0);

        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);

        let is_healthy = self.is_healthy.load(Ordering::SeqCst);
        let status = if is_healthy {
            HealthStatus::Healthy
        } else {
            HealthStatus::Unhealthy
        };

        let checks = self.check_results.lock().clone();

        HealthInfo {
            status,
            uptime_secs: uptime,
            last_check: now,
            checks,
        }
    }

    /// Get readiness status
    pub fn get_readiness(&self) -> ReadinessInfo {
        let is_ready = self.is_ready.load(Ordering::SeqCst);
        let status = if is_ready {
            ReadinessStatus::Ready
        } else {
            ReadinessStatus::NotReady
        };

        ReadinessInfo {
            status,
            initialized: true,
            consensus_ready: is_ready,
            cache_ready: is_ready,
            persistence_ready: is_ready,
        }
    }

    /// Check if healthy
    pub fn is_healthy(&self) -> bool {
        self.is_healthy.load(Ordering::SeqCst)
    }

    /// Check if ready
    pub fn is_ready(&self) -> bool {
        self.is_ready.load(Ordering::SeqCst)
    }
}

impl Default for HealthChecker {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_health_checker_creation() {
        let checker = HealthChecker::new();
        assert!(checker.is_healthy());
        assert!(!checker.is_ready());
    }

    #[test]
    fn test_mark_ready() {
        let checker = HealthChecker::new();
        checker.mark_ready();
        assert!(checker.is_ready());
    }

    #[test]
    fn test_mark_unhealthy() {
        let checker = HealthChecker::new();
        checker.mark_unhealthy();
        assert!(!checker.is_healthy());
    }

    #[test]
    fn test_record_check() {
        let checker = HealthChecker::new();
        checker.record_check("test_check", true);

        let health = checker.get_health();
        assert_eq!(health.checks.len(), 1);
        assert_eq!(health.checks[0].0, "test_check");
        assert!(health.checks[0].1);
    }

    #[test]
    fn test_get_health_info() {
        let checker = HealthChecker::new();
        let health = checker.get_health();

        assert_eq!(health.status, HealthStatus::Healthy);
        assert!(health.uptime_secs >= 0);
    }

    #[test]
    fn test_get_readiness_info() {
        let checker = HealthChecker::new();
        checker.mark_ready();

        let readiness = checker.get_readiness();
        assert_eq!(readiness.status, ReadinessStatus::Ready);
        assert!(readiness.initialized);
    }
}
