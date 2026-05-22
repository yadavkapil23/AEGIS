use crate::error::{ResilienceError, Result};
use serde::{Deserialize, Serialize};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use tracing::{info, warn};

/// Degradation level
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum DegradationLevel {
    /// All systems normal
    Healthy,
    /// Some systems degraded, service continues
    Degraded,
    /// Service severely impaired
    Critical,
}

impl std::fmt::Display for DegradationLevel {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Healthy => write!(f, "Healthy"),
            Self::Degraded => write!(f, "Degraded"),
            Self::Critical => write!(f, "Critical"),
        }
    }
}

/// Graceful degradation handler
pub struct GracefulDegradation {
    current_level: Arc<parking_lot::RwLock<DegradationLevel>>,
    is_degraded: Arc<AtomicBool>,
    reason: Arc<parking_lot::RwLock<String>>,
    fallback_enabled: Arc<AtomicBool>,
}

impl GracefulDegradation {
    /// Create new degradation handler
    pub fn new() -> Self {
        Self {
            current_level: Arc::new(parking_lot::RwLock::new(DegradationLevel::Healthy)),
            is_degraded: Arc::new(AtomicBool::new(false)),
            reason: Arc::new(parking_lot::RwLock::new(String::new())),
            fallback_enabled: Arc::new(AtomicBool::new(true)),
        }
    }

    /// Set degradation level
    pub fn set_degradation(&self, level: DegradationLevel, reason: impl Into<String>) {
        let reason_str = reason.into();
        *self.current_level.write() = level;
        *self.reason.write() = reason_str.clone();

        match level {
            DegradationLevel::Healthy => {
                self.is_degraded.store(false, Ordering::Relaxed);
                info!("System returned to healthy state");
            }
            DegradationLevel::Degraded => {
                self.is_degraded.store(true, Ordering::Relaxed);
                warn!("System degraded: {}", reason_str);
            }
            DegradationLevel::Critical => {
                self.is_degraded.store(true, Ordering::Relaxed);
                warn!("System critical: {}", reason_str);
            }
        }
    }

    /// Get current degradation level
    pub fn level(&self) -> DegradationLevel {
        *self.current_level.read()
    }

    /// Check if system is degraded
    pub fn is_degraded(&self) -> bool {
        self.is_degraded.load(Ordering::Relaxed)
    }

    /// Get degradation reason
    pub fn reason(&self) -> String {
        self.reason.read().clone()
    }

    /// Enable fallback mode
    pub fn enable_fallback(&self) {
        self.fallback_enabled.store(true, Ordering::Relaxed);
        info!("Fallback mode enabled");
    }

    /// Disable fallback mode
    pub fn disable_fallback(&self) {
        self.fallback_enabled.store(false, Ordering::Relaxed);
        warn!("Fallback mode disabled");
    }

    /// Check if fallback is enabled
    pub fn is_fallback_enabled(&self) -> bool {
        self.fallback_enabled.load(Ordering::Relaxed)
    }

    /// Execute with graceful degradation
    pub async fn execute_with_fallback<F, Fut, T, B, BFut>(
        &self,
        primary: F,
        fallback: B,
    ) -> Result<T>
    where
        F: std::future::Future<Output = Result<T>>,
        B: std::future::Future<Output = Result<T>>,
    {
        // Try primary
        match primary.await {
            Ok(result) => {
                if self.is_degraded() {
                    self.set_degradation(DegradationLevel::Healthy, "Primary service recovered");
                }
                Ok(result)
            }
            Err(e) => {
                if self.is_fallback_enabled() {
                    warn!("Primary failed, using fallback: {}", e);
                    self.set_degradation(
                        DegradationLevel::Degraded,
                        format!("Using fallback: {}", e),
                    );

                    match fallback.await {
                        Ok(result) => Ok(result),
                        Err(fallback_err) => {
                            self.set_degradation(
                                DegradationLevel::Critical,
                                format!("Both primary and fallback failed: {}", fallback_err),
                            );
                            Err(ResilienceError::DegradedService {
                                reason: fallback_err.to_string(),
                            })
                        }
                    }
                } else {
                    Err(ResilienceError::BackendUnavailable {
                        reason: e.to_string(),
                    })
                }
            }
        }
    }

    /// Get metrics
    pub fn metrics(&self) -> DegradationMetrics {
        DegradationMetrics {
            level: self.level(),
            is_degraded: self.is_degraded(),
            fallback_enabled: self.is_fallback_enabled(),
            reason: self.reason(),
        }
    }
}

impl Default for GracefulDegradation {
    fn default() -> Self {
        Self::new()
    }
}

/// Degradation metrics
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DegradationMetrics {
    pub level: DegradationLevel,
    pub is_degraded: bool,
    pub fallback_enabled: bool,
    pub reason: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_degradation_level_changes() {
        let degradation = GracefulDegradation::new();

        assert_eq!(degradation.level(), DegradationLevel::Healthy);
        assert!(!degradation.is_degraded());

        degradation.set_degradation(DegradationLevel::Degraded, "test degradation");
        assert_eq!(degradation.level(), DegradationLevel::Degraded);
        assert!(degradation.is_degraded());
    }

    #[tokio::test]
    async fn test_fallback_works() {
        let degradation = GracefulDegradation::new();

        let result = degradation
            .execute_with_fallback(
                async {
                    Err::<&str, _>(ResilienceError::Unknown(
                        "primary failed".to_string(),
                    ))
                },
                async { Ok::<&str, _>("fallback success") },
            )
            .await;

        assert!(result.is_ok());
        assert!(degradation.is_degraded());
    }
}
