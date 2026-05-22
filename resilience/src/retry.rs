use crate::error::{ResilienceError, Result};
use serde::{Deserialize, Serialize};
use std::time::Duration;
use tracing::{debug, warn};

/// Retry configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RetryConfig {
    /// Maximum number of retry attempts
    pub max_attempts: u32,

    /// Initial backoff duration in milliseconds
    pub initial_backoff_ms: u64,

    /// Maximum backoff duration in milliseconds
    pub max_backoff_ms: u64,

    /// Backoff multiplier (exponential factor)
    pub backoff_multiplier: f32,

    /// Add jitter to backoff to prevent thundering herd
    pub enable_jitter: bool,

    /// Name for logging
    pub name: String,
}

impl Default for RetryConfig {
    fn default() -> Self {
        Self {
            max_attempts: 3,
            initial_backoff_ms: 100,
            max_backoff_ms: 10000,
            backoff_multiplier: 2.0,
            enable_jitter: true,
            name: "retry".to_string(),
        }
    }
}

/// Retry handler with exponential backoff
pub struct RetryHandler {
    config: RetryConfig,
}

impl RetryHandler {
    /// Create a new retry handler
    pub fn new(config: RetryConfig) -> Self {
        Self { config }
    }

    /// Calculate backoff duration for attempt
    fn calculate_backoff(&self, attempt: u32) -> Duration {
        let backoff_ms = (self.config.initial_backoff_ms as f32
            * self.config.backoff_multiplier.powi(attempt as i32))
            .min(self.config.max_backoff_ms as f32) as u64;

        let final_ms = if self.config.enable_jitter {
            use rand::Rng;
            let mut rng = rand::thread_rng();
            let jitter = rng.gen_range(0..=backoff_ms / 2);
            backoff_ms + jitter
        } else {
            backoff_ms
        };

        Duration::from_millis(final_ms)
    }

    /// Execute operation with retries
    pub async fn execute<F, Fut, T>(&self, mut operation: F) -> Result<T>
    where
        F: FnMut() -> Fut,
        Fut: std::future::Future<Output = Result<T>>,
    {
        let mut last_error = None;

        for attempt in 0..self.config.max_attempts {
            match operation().await {
                Ok(result) => {
                    if attempt > 0 {
                        debug!(
                            "{}: Succeeded on attempt {} after retries",
                            self.config.name, attempt + 1
                        );
                    }
                    return Ok(result);
                }
                Err(e) => {
                    last_error = Some(e.clone());

                    if attempt < self.config.max_attempts - 1 {
                        let backoff = self.calculate_backoff(attempt);
                        warn!(
                            "{}: Attempt {} failed: {}, retrying in {:?}",
                            self.config.name,
                            attempt + 1,
                            e,
                            backoff
                        );

                        tokio::time::sleep(backoff).await;
                    } else {
                        warn!(
                            "{}: All {} attempts failed: {}",
                            self.config.name,
                            self.config.max_attempts,
                            e
                        );
                    }
                }
            }
        }

        Err(ResilienceError::MaxRetriesExceeded {
            reason: format!("{}: {}", self.config.name, last_error.unwrap()),
        })
    }

    /// Get config
    pub fn config(&self) -> &RetryConfig {
        &self.config
    }
}

/// Retry statistics
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct RetryStats {
    pub total_attempts: u64,
    pub successful_retries: u64,
    pub failed_retries: u64,
    pub average_backoff_ms: f32,
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU32, Ordering};
    use std::sync::Arc;

    #[tokio::test]
    async fn test_retry_succeeds_eventually() {
        let config = RetryConfig {
            max_attempts: 3,
            initial_backoff_ms: 10,
            ..Default::default()
        };

        let handler = RetryHandler::new(config);
        let attempt_count = Arc::new(AtomicU32::new(0));
        let attempt_count_clone = attempt_count.clone();

        let result = handler
            .execute(|| {
                let attempt_count = attempt_count_clone.clone();
                async move {
                    let count = attempt_count.fetch_add(1, Ordering::Relaxed);
                    if count < 2 {
                        Err(ResilienceError::Unknown("test error".to_string()))
                    } else {
                        Ok("success")
                    }
                }
            })
            .await;

        assert!(result.is_ok());
        assert_eq!(attempt_count.load(Ordering::Relaxed), 3);
    }

    #[tokio::test]
    async fn test_retry_fails_after_max_attempts() {
        let config = RetryConfig {
            max_attempts: 2,
            initial_backoff_ms: 10,
            ..Default::default()
        };

        let handler = RetryHandler::new(config);

        let result = handler
            .execute(|| async {
                Err::<&str, _>(ResilienceError::Unknown("persistent error".to_string()))
            })
            .await;

        assert!(result.is_err());
    }
}
