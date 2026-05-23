use crate::error::{ResilienceError, Result};
use std::time::Duration;
use tokio::time::timeout;
use tracing::warn;

/// Timeout handler
pub struct TimeoutHandler {
    default_timeout: Duration,
}

impl TimeoutHandler {
    /// Create a new timeout handler
    pub fn new(timeout_ms: u64) -> Self {
        Self {
            default_timeout: Duration::from_millis(timeout_ms),
        }
    }

    /// Execute operation with timeout
    pub async fn execute<Fut, T>(&self, future: Fut) -> Result<T>
    where
        Fut: std::future::Future<Output = Result<T>>,
    {
        self.execute_with_timeout(future, self.default_timeout)
            .await
    }

    /// Execute operation with custom timeout
    pub async fn execute_with_timeout<Fut, T>(
        &self,
        future: Fut,
        timeout_duration: Duration,
    ) -> Result<T>
    where
        Fut: std::future::Future<Output = Result<T>>,
    {
        match timeout(timeout_duration, future).await {
            Ok(result) => result,
            Err(_) => {
                warn!(
                    "Operation timed out after {:?}",
                    timeout_duration
                );
                Err(ResilienceError::Timeout {
                    timeout_ms: timeout_duration.as_millis() as u64,
                })
            }
        }
    }

    /// Get default timeout
    pub fn default_timeout(&self) -> Duration {
        self.default_timeout
    }

    /// Set new default timeout
    pub fn set_timeout(&mut self, timeout_ms: u64) {
        self.default_timeout = Duration::from_millis(timeout_ms);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_timeout_succeeds() {
        let handler = TimeoutHandler::new(1000);

        let result = handler
            .execute(async { Ok::<&str, ResilienceError>("success") })
            .await;

        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn test_timeout_fails() {
        let handler = TimeoutHandler::new(100);

        let result = handler
            .execute(async {
                tokio::time::sleep(Duration::from_millis(500)).await;
                Ok::<&str, ResilienceError>("never reached")
            })
            .await;

        assert!(matches!(result, Err(ResilienceError::Timeout { .. })));
    }
}
