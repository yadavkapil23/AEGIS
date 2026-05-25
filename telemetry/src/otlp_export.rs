// OpenTelemetry OTLP Exporter
// Exports traces and metrics to OTLP-compatible collectors

use anyhow::Result;
use opentelemetry::global;
use opentelemetry_otlp::WithExportConfig;
use opentelemetry_sdk::trace as sdktrace;
use opentelemetry_sdk::Resource;
use opentelemetry::KeyValue;

use tracing::info;

/// OTLP exporter configuration
#[derive(Clone, Debug)]
pub struct OtlpExporterConfig {
    pub endpoint: String,
    pub service_name: String,
    pub enabled: bool,
}

impl Default for OtlpExporterConfig {
    fn default() -> Self {
        Self {
            endpoint: "http://localhost:4317".to_string(),
            service_name: "aegis-scheduler".to_string(),
            enabled: false,
        }
    }
}

/// Initialize OTLP exporter
pub fn init_otlp_exporter(config: OtlpExporterConfig) -> Result<()> {
    if !config.enabled {
        info!("OTLP exporter is disabled");
        return Ok(());
    }

    info!("Initializing OTLP exporter to {}", config.endpoint);

    let tracer = opentelemetry_otlp::new_pipeline()
        .tracing()
        .with_exporter(
            opentelemetry_otlp::new_exporter()
                .tonic()
                .with_endpoint(&config.endpoint),
        )
        .with_trace_config(
            sdktrace::config().with_resource(Resource::new(vec![KeyValue::new(
                "service.name",
                config.service_name,
            )])),
        )
        .install_batch(opentelemetry_sdk::runtime::Tokio)?;

    global::set_tracer_provider(tracer.provider().unwrap());

    // NOTE: Application must also set up tracing_subscriber globally using this tracer.
    // That is usually done in the main entry point.

    info!("OTLP exporter initialized successfully");
    Ok(())
}

