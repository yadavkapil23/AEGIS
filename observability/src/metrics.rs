use lazy_static::lazy_static;
use prometheus::{Counter, CounterVec, Gauge, GaugeVec, HistogramVec, Registry, Opts, HistogramOpts, Encoder, TextEncoder};
use std::sync::Arc;

/// Prometheus metrics registry
pub struct MetricsRegistry {
    pub registry: Registry,

    // Inference metrics
    pub inference_requests_total: CounterVec,
    pub inference_errors_total: CounterVec,
    pub inference_latency_ms: HistogramVec,
    pub inference_tokens_generated: Counter,

    // Backend metrics
    pub backend_health: GaugeVec,
    pub backend_active_requests: GaugeVec,
    pub backend_latency_ms: HistogramVec,

    // Circuit breaker metrics
    pub circuit_breaker_state: GaugeVec,
    pub circuit_breaker_failures_total: CounterVec,
    pub circuit_breaker_opens_total: CounterVec,

    // Retry metrics
    pub retry_attempts_total: CounterVec,
    pub retry_successes_total: CounterVec,

    // Timeout metrics
    pub timeout_errors_total: Counter,

    // Degradation metrics
    pub degradation_level: Gauge,
    pub fallback_uses_total: Counter,
}

impl MetricsRegistry {
    /// Create a new metrics registry
    pub fn new() -> Result<Self, Box<dyn std::error::Error>> {
        let registry = Registry::new();

        // Inference metrics
        let inference_requests_total = CounterVec::new(
            Opts::new("aegis_inference_requests_total", "Total inference requests"),
            &["model", "status"],
        )?;
        registry.register(Box::new(inference_requests_total.clone()))?;

        let inference_errors_total = CounterVec::new(
            Opts::new("aegis_inference_errors_total", "Total inference errors"),
            &["error_type"],
        )?;
        registry.register(Box::new(inference_errors_total.clone()))?;

        let inference_latency_ms = HistogramVec::new(
            HistogramOpts::new("aegis_inference_latency_ms", "Inference latency in milliseconds"),
            &["model"],
        )?;
        registry.register(Box::new(inference_latency_ms.clone()))?;

        let inference_tokens_generated = Counter::new(
            "aegis_inference_tokens_generated_total",
            "Total tokens generated",
        )?;
        registry.register(Box::new(inference_tokens_generated.clone()))?;

        // Backend metrics
        let backend_health = GaugeVec::new(
            Opts::new("aegis_backend_healthy", "Backend health status"),
            &["backend"],
        )?;
        registry.register(Box::new(backend_health.clone()))?;

        let backend_active_requests = GaugeVec::new(
            Opts::new("aegis_backend_active_requests", "Active requests per backend"),
            &["backend"],
        )?;
        registry.register(Box::new(backend_active_requests.clone()))?;

        let backend_latency_ms = HistogramVec::new(
            HistogramOpts::new("aegis_backend_latency_ms", "Backend latency in milliseconds"),
            &["backend"],
        )?;
        registry.register(Box::new(backend_latency_ms.clone()))?;

        // Circuit breaker metrics
        let circuit_breaker_state = GaugeVec::new(
            Opts::new("aegis_circuit_breaker_state", "Circuit breaker state"),
            &["backend"],
        )?;
        registry.register(Box::new(circuit_breaker_state.clone()))?;

        let circuit_breaker_failures_total = CounterVec::new(
            Opts::new("aegis_circuit_breaker_failures_total", "Circuit breaker failures"),
            &["backend"],
        )?;
        registry.register(Box::new(circuit_breaker_failures_total.clone()))?;

        let circuit_breaker_opens_total = CounterVec::new(
            Opts::new("aegis_circuit_breaker_opens_total", "Circuit breaker opens"),
            &["backend"],
        )?;
        registry.register(Box::new(circuit_breaker_opens_total.clone()))?;

        // Retry metrics
        let retry_attempts_total = CounterVec::new(
            Opts::new("aegis_retry_attempts_total", "Total retry attempts"),
            &["operation"],
        )?;
        registry.register(Box::new(retry_attempts_total.clone()))?;

        let retry_successes_total = CounterVec::new(
            Opts::new("aegis_retry_successes_total", "Successful retries"),
            &["operation"],
        )?;
        registry.register(Box::new(retry_successes_total.clone()))?;

        // Timeout metrics
        let timeout_errors_total = Counter::new(
            "aegis_timeout_errors_total",
            "Total timeout errors",
        )?;
        registry.register(Box::new(timeout_errors_total.clone()))?;

        // Degradation metrics
        let degradation_level = Gauge::new(
            "aegis_degradation_level",
            "Service degradation level",
        )?;
        registry.register(Box::new(degradation_level.clone()))?;

        let fallback_uses_total = Counter::new(
            "aegis_fallback_uses_total",
            "Total fallback uses",
        )?;
        registry.register(Box::new(fallback_uses_total.clone()))?;

        Ok(Self {
            registry,
            inference_requests_total,
            inference_errors_total,
            inference_latency_ms,
            inference_tokens_generated,
            backend_health,
            backend_active_requests,
            backend_latency_ms,
            circuit_breaker_state,
            circuit_breaker_failures_total,
            circuit_breaker_opens_total,
            retry_attempts_total,
            retry_successes_total,
            timeout_errors_total,
            degradation_level,
            fallback_uses_total,
        })
    }

    /// Get metrics in Prometheus format
    pub fn gather(&self) -> Result<String, Box<dyn std::error::Error>> {
        let encoder = TextEncoder::new();
        let metric_families = self.registry.gather();
        let mut buf = Vec::new();
        encoder.encode(&metric_families, &mut buf)?;
        Ok(String::from_utf8(buf)?)
    }
}

impl Default for MetricsRegistry {
    fn default() -> Self {
        Self::new().expect("Failed to create metrics registry")
    }
}

lazy_static! {
    /// Global metrics registry
    pub static ref METRICS: Arc<MetricsRegistry> = Arc::new(MetricsRegistry::default());
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_metrics_registry_creation() {
        let registry = MetricsRegistry::new();
        assert!(registry.is_ok());
    }

    #[test]
    fn test_metrics_gather() {
        let registry = MetricsRegistry::new().unwrap();
        let output = registry.gather();
        assert!(output.is_ok());
    }
}
