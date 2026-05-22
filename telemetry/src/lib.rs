// Telemetry module: OpenTelemetry + Prometheus integration

pub mod metrics;
pub mod distributed_tracing;
pub mod otlp_export;

pub use metrics::*;
pub use distributed_tracing::*;
pub use otlp_export::*;

use anyhow::Result;
use tracing::info;

/// Initialize distributed tracing
fn init_tracing(service_name: &str) -> Result<()> {
    info!("Initializing tracing for service: {}", service_name);
    // Tracing initialization - uses existing tracing-subscriber infrastructure
    Ok(())
}

/// Initialize telemetry for AEGIS
pub async fn init_telemetry(service_name: &str) -> Result<()> {
    // Initialize tracing
    init_tracing(service_name)?;

    // Initialize metrics
    metrics::init_metrics()?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_telemetry_init() {
        let result = init_telemetry("aegis-test").await;
        assert!(result.is_ok());
    }
}
