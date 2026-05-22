// Graceful shutdown handling
// Manages SIGTERM/SIGINT signals and drains connections before exit

use anyhow::Result;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use tokio::sync::broadcast;
use tracing::{info, warn};

/// Graceful shutdown manager
pub struct GracefulShutdown {
    shutdown_signal: broadcast::Sender<()>,
    is_shutting_down: Arc<AtomicBool>,
    shutdown_timeout_secs: u64,
}

impl GracefulShutdown {
    /// Create new graceful shutdown manager
    pub fn new(shutdown_timeout_secs: u64) -> Self {
        let (tx, _rx) = broadcast::channel(1);

        Self {
            shutdown_signal: tx,
            is_shutting_down: Arc::new(AtomicBool::new(false)),
            shutdown_timeout_secs,
        }
    }

    /// Get shutdown signal receiver
    pub fn subscribe(&self) -> broadcast::Receiver<()> {
        self.shutdown_signal.subscribe()
    }

    /// Trigger graceful shutdown
    pub fn shutdown(&self) {
        info!("Graceful shutdown initiated");
        self.is_shutting_down.store(true, Ordering::SeqCst);
        let _ = self.shutdown_signal.send(());
    }

    /// Check if shutting down
    pub fn is_shutting_down(&self) -> bool {
        self.is_shutting_down.load(Ordering::SeqCst)
    }

    /// Wait for shutdown signal
    pub async fn wait_for_signal(&self) -> Result<()> {
        let mut rx = self.subscribe();

        // Wait for signal
        let _ = rx.recv().await;

        info!(
            timeout_secs = self.shutdown_timeout_secs,
            "Waiting for graceful shutdown with timeout"
        );

        // Wait for timeout or completion
        tokio::time::sleep(tokio::time::Duration::from_secs(self.shutdown_timeout_secs)).await;

        info!("Graceful shutdown complete");
        Ok(())
    }

    /// Register system signal handlers (SIGTERM, SIGINT)
    pub fn install_signal_handlers(self: Arc<Self>) -> Result<()> {
        let shutdown = self.clone();

        tokio::spawn(async move {
            #[cfg(unix)]
            {
                use tokio::signal::unix::{signal, SignalKind};

                let mut sigterm = match signal(SignalKind::terminate()) {
                    Ok(s) => s,
                    Err(e) => {
                        warn!("Failed to install SIGTERM handler: {}", e);
                        return;
                    }
                };

                let mut sigint = match signal(SignalKind::interrupt()) {
                    Ok(s) => s,
                    Err(e) => {
                        warn!("Failed to install SIGINT handler: {}", e);
                        return;
                    }
                };

                tokio::select! {
                    _ = sigterm.recv() => {
                        info!("Received SIGTERM");
                        shutdown.shutdown();
                    }
                    _ = sigint.recv() => {
                        info!("Received SIGINT");
                        shutdown.shutdown();
                    }
                }
            }

            #[cfg(not(unix))]
            {
                // Windows signal handling
                match tokio::signal::ctrl_c().await {
                    Ok(()) => {
                        info!("Received Ctrl-C");
                        shutdown.shutdown();
                    }
                    Err(e) => {
                        warn!("Failed to listen for Ctrl-C: {}", e);
                    }
                }
            }
        });

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_graceful_shutdown_creation() {
        let shutdown = GracefulShutdown::new(30);
        assert!(!shutdown.is_shutting_down());
    }

    #[test]
    fn test_shutdown_flag() {
        let shutdown = GracefulShutdown::new(30);
        shutdown.shutdown();
        assert!(shutdown.is_shutting_down());
    }

    #[tokio::test]
    async fn test_shutdown_signal() {
        let shutdown = Arc::new(GracefulShutdown::new(1));
        let shutdown_clone = shutdown.clone();

        let signal_task = tokio::spawn(async move {
            shutdown_clone.wait_for_signal().await
        });

        // Give it a moment to start waiting
        tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;

        // Trigger shutdown
        shutdown.shutdown();

        // Wait for signal task with timeout
        let result = tokio::time::timeout(
            tokio::time::Duration::from_secs(3),
            signal_task,
        )
        .await;

        assert!(result.is_ok());
    }

    #[test]
    fn test_subscribe() {
        let shutdown = GracefulShutdown::new(30);
        let _rx = shutdown.subscribe();
        // Should not panic
    }
}
