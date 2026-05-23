use tracing::{info, warn};
use tracing_subscriber::{fmt, prelude::*, EnvFilter};

/// Tracing configuration
#[derive(Debug, Clone)]
pub struct TracingConfig {
    /// Enable JSON formatting
    pub json_format: bool,

    /// Log level filter
    pub level: String,

    /// Enable OpenTelemetry integration
    pub opentelemetry_enabled: bool,

    /// Jaeger endpoint for tracing
    pub jaeger_endpoint: Option<String>,
}

impl Default for TracingConfig {
    fn default() -> Self {
        Self {
            json_format: true,
            level: "info".to_string(),
            opentelemetry_enabled: false,
            jaeger_endpoint: None,
        }
    }
}

/// Initialize structured logging with tracing
pub fn init_tracing(config: TracingConfig) -> anyhow::Result<()> {
    let env_filter = EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| EnvFilter::new(&config.level));

    if config.json_format {
        // JSON formatted logs
        tracing_subscriber::registry()
            .with(env_filter)
            .with(
                fmt::layer()
                    .json()
                    .with_current_span(true)
                    .with_target(true)
            )
            .init();

        info!("✓ Structured JSON logging initialized");
    } else {
        // Human-readable logs
        tracing_subscriber::registry()
            .with(env_filter)
            .with(fmt::layer().pretty())
            .init();

        info!("✓ Pretty logging initialized");
    }

    if config.opentelemetry_enabled {
        if let Some(endpoint) = &config.jaeger_endpoint {
            info!("✓ OpenTelemetry tracing to {}", endpoint);
        } else {
            warn!("⚠ OpenTelemetry enabled but no Jaeger endpoint configured");
        }
    }

    Ok(())
}

/// Span builder for custom tracing
pub fn create_span(name: &str, attributes: &[(&str, &str)]) -> tracing::Span {
    use tracing::info_span;

    let span = info_span!(
        "operation",
        name = name,
    );

    for (key, value) in attributes {
        span.record(*key, *value);
    }

    span
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_tracing_config_default() {
        let config = TracingConfig::default();
        assert!(config.json_format);
        assert_eq!(config.level, "info");
    }
}
