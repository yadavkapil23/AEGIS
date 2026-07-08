use anyhow::Result;
use tracing_subscriber::fmt;
use tracing_subscriber::EnvFilter;

/// Initialize structured tracing with env filter and optional JSON output.
pub fn init_tracing(service_name: &str) -> Result<()> {
    let env_filter = EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| EnvFilter::new("info"));

    fmt::Subscriber::builder()
        .with_env_filter(env_filter)
        .with_target(true)
        .with_thread_ids(true)
        .json()
        .init();

    tracing::info!(
        service = service_name,
        pid = std::process::id(),
        "Tracing initialized"
    );

    Ok(())
}

/// Initialize full telemetry stack: tracing + metrics.
pub async fn init_telemetry(service_name: &str) -> Result<()> {
    init_tracing(service_name)?;
    metrics::init_metrics()?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_init_tracing() {
        // Just ensure it doesn't panic (subscriber may already be set)
        let _ = init_tracing("test");
    }
}
