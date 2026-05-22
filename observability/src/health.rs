use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

/// Health check status
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum HealthState {
    /// Service is healthy and ready
    Healthy,
    /// Service is degraded but operational
    Degraded,
    /// Service is not ready
    NotReady,
    /// Service is unhealthy
    Unhealthy,
}

impl std::fmt::Display for HealthState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Healthy => write!(f, "healthy"),
            Self::Degraded => write!(f, "degraded"),
            Self::NotReady => write!(f, "not_ready"),
            Self::Unhealthy => write!(f, "unhealthy"),
        }
    }
}

/// Liveness probe response
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LivenessProbe {
    /// Is the service alive?
    pub alive: bool,

    /// Current state
    pub state: String,

    /// Timestamp
    pub timestamp: DateTime<Utc>,
}

/// Readiness probe response
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReadinessProbe {
    /// Is the service ready?
    pub ready: bool,

    /// Details
    pub ready_checks: Vec<ReadyCheck>,

    /// Timestamp
    pub timestamp: DateTime<Utc>,
}

/// Individual readiness check
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReadyCheck {
    /// Component name
    pub component: String,

    /// Is it ready?
    pub ready: bool,

    /// Status message
    pub status: String,
}

/// Health check manager
pub struct HealthManager {
    state: Arc<parking_lot::RwLock<HealthState>>,
    backends_ready: Arc<AtomicBool>,
    inference_ready: Arc<AtomicBool>,
    started_at: DateTime<Utc>,
}

impl HealthManager {
    /// Create a new health manager
    pub fn new() -> Self {
        Self {
            state: Arc::new(parking_lot::RwLock::new(HealthState::NotReady)),
            backends_ready: Arc::new(AtomicBool::new(false)),
            inference_ready: Arc::new(AtomicBool::new(false)),
            started_at: Utc::now(),
        }
    }

    /// Set health state
    pub fn set_state(&self, state: HealthState) {
        *self.state.write() = state;
    }

    /// Get current health state
    pub fn get_state(&self) -> HealthState {
        *self.state.read()
    }

    /// Mark backends as ready
    pub fn mark_backends_ready(&self) {
        self.backends_ready.store(true, Ordering::Relaxed);
        self.update_state();
    }

    /// Mark backends as not ready
    pub fn mark_backends_not_ready(&self) {
        self.backends_ready.store(false, Ordering::Relaxed);
        self.update_state();
    }

    /// Mark inference as ready
    pub fn mark_inference_ready(&self) {
        self.inference_ready.store(true, Ordering::Relaxed);
        self.update_state();
    }

    /// Update overall state based on components
    fn update_state(&self) {
        let backends_ok = self.backends_ready.load(Ordering::Relaxed);
        let inference_ok = self.inference_ready.load(Ordering::Relaxed);

        let new_state = if backends_ok && inference_ok {
            HealthState::Healthy
        } else if backends_ok || inference_ok {
            HealthState::Degraded
        } else {
            HealthState::NotReady
        };

        *self.state.write() = new_state;
    }

    /// Get liveness probe response
    pub fn get_liveness(&self) -> LivenessProbe {
        let state = self.get_state();
        LivenessProbe {
            alive: state != HealthState::Unhealthy,
            state: state.to_string(),
            timestamp: Utc::now(),
        }
    }

    /// Get readiness probe response
    pub fn get_readiness(&self) -> ReadinessProbe {
        let backends_ready = self.backends_ready.load(Ordering::Relaxed);
        let inference_ready = self.inference_ready.load(Ordering::Relaxed);

        ReadinessProbe {
            ready: backends_ready && inference_ready,
            ready_checks: vec![
                ReadyCheck {
                    component: "backends".to_string(),
                    ready: backends_ready,
                    status: if backends_ready {
                        "ready".to_string()
                    } else {
                        "initializing".to_string()
                    },
                },
                ReadyCheck {
                    component: "inference".to_string(),
                    ready: inference_ready,
                    status: if inference_ready {
                        "ready".to_string()
                    } else {
                        "initializing".to_string()
                    },
                },
            ],
            timestamp: Utc::now(),
        }
    }

    /// Get startup time
    pub fn uptime_secs(&self) -> i64 {
        (Utc::now() - self.started_at).num_seconds()
    }
}

impl Default for HealthManager {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_health_state_display() {
        assert_eq!(HealthState::Healthy.to_string(), "healthy");
        assert_eq!(HealthState::Unhealthy.to_string(), "unhealthy");
    }

    #[test]
    fn test_health_manager() {
        let manager = HealthManager::new();

        let liveness = manager.get_liveness();
        assert!(liveness.alive);

        let readiness = manager.get_readiness();
        assert!(!readiness.ready);

        manager.mark_backends_ready();
        manager.mark_inference_ready();

        let readiness = manager.get_readiness();
        assert!(readiness.ready);
    }
}
