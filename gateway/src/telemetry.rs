/// Telemetry Module
/// Simplified tracing and logging for the gateway

use tracing_subscriber::{fmt, prelude::*, EnvFilter};

/// Initialize tracing
pub fn init_tracing(service_name: &str) -> Result<(), Box<dyn std::error::Error>> {
    // Initialize structured JSON logging
    let fmt_layer = fmt::layer()
        .json()
        .with_target(true)
        .with_thread_ids(true);

    let env_filter = EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| EnvFilter::new("info"));

    // Compose layers
    tracing_subscriber::registry()
        .with(env_filter)
        .with(fmt_layer)
        .init();

    eprintln!("Tracing initialized for service: {}", service_name);
    Ok(())
}
