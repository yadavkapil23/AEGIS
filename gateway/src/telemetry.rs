/// Observability & Telemetry Module
/// Combines OpenTelemetry tracing, structured logging, and Prometheus metrics

use opentelemetry::{
    global, trace::TracerProvider, KeyValue,
    sdk::trace as sdk_trace,
};
use opentelemetry_otlp::new_pipeline;
use tracing::{info, warn, error, instrument};
use tracing_subscriber::{fmt, prelude::*, EnvFilter};
use std::time::Instant;

/// Initialize distributed tracing with OpenTelemetry
pub fn init_tracing(service_name: &str) -> Result<(), Box<dyn std::error::Error>> {
    // Create OTLP exporter
    let tracer = new_pipeline()
        .tracing()
        .with_exporter(
            opentelemetry_otlp::new_exporter()
                .tonic()
                .with_endpoint("http://localhost:4317"),  // Jaeger collector
        )
        .with_trace_config(
            sdk_trace::Config::default()
                .with_sampler(sdk_trace::Sampler::TraceIdRatioBased(0.1))
                .with_resource(opentelemetry::sdk::Resource::new(vec![
                    KeyValue::new("service.name", service_name.to_string()),
                ]))
        )
        .install_batch(opentelemetry::runtime::Tokio)?;

    let telemetry = tracing_opentelemetry::layer().with_tracer(tracer);

    // Initialize structured JSON logging
    let fmt_layer = fmt::layer()
        .json()
        .with_target(true)
        .with_thread_ids(true)
        .with_span_list(true);

    let env_filter = EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| EnvFilter::new("info"));

    // Compose layers
    tracing_subscriber::registry()
        .with(env_filter)
        .with(fmt_layer)
        .with(telemetry)
        .init();

    info!("🔍 Tracing initialized with OpenTelemetry");
    info!("📊 Structured logging enabled (JSON format)");

    Ok(())
}

/// Trace key operations
#[instrument(skip_all, fields(request_id = %request_id))]
pub async fn trace_inference_request(
    request_id: &str,
    model: &str,
    prompt_len: usize,
) -> Result<std::time::Duration, Box<dyn std::error::Error>> {
    let start = Instant::now();

    info!(
        model = model,
        prompt_length = prompt_len,
        "Starting inference request"
    );

    // Simulate work
    tokio::time::sleep(std::time::Duration::from_millis(100)).await;

    let duration = start.elapsed();
    info!(
        latency_ms = duration.as_millis(),
        "Inference request completed"
    );

    Ok(duration)
}

/// Trace allocation operations
#[instrument(skip_all)]
pub fn trace_allocation(request_id: &str, num_blocks: u32) {
    info!(
        request_id = request_id,
        blocks_requested = num_blocks,
        "KV cache allocation started"
    );
}

/// Trace error conditions
#[instrument(skip_all)]
pub fn trace_error(error_type: &str, message: &str) {
    error!(
        error_type = error_type,
        message = message,
        "Error occurred"
    );
}

/// Trace circuit breaker events
#[instrument(skip_all)]
pub fn trace_circuit_breaker(event: &str, state: &str) {
    warn!(
        event = event,
        state = state,
        "Circuit breaker state changed"
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_tracing_disabled_in_tests() {
        // Tracing should be initialized by test harness
        // This test just verifies the module compiles
    }
}
