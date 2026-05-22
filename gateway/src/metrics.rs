/// Gateway metrics with Prometheus integration

use parking_lot::Mutex;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use prometheus::{
    Counter, CounterVec, HistogramVec, IntGauge,
    Registry, TextEncoder, Encoder,
};

/// Prometheus-based metrics collector for production observability
#[derive(Clone)]
pub struct PrometheusMetrics {
    // Counters
    pub inference_requests_total: CounterVec,
    pub inference_errors_total: CounterVec,
    pub rate_limited_requests_total: Counter,
    pub circuit_breaker_trips_total: Counter,

    // Histograms (latency distribution)
    pub inference_latency_ms: HistogramVec,
    pub inference_tokens_generated: HistogramVec,

    // Gauges (point-in-time values)
    pub circuit_breaker_state: IntGauge,
    pub bulkhead_active_requests: IntGauge,
    pub bulkhead_max_slots: IntGauge,
    pub cache_hit_ratio_percent: IntGauge,

    registry: Arc<Registry>,
}

impl PrometheusMetrics {
    /// Create new Prometheus metrics collector
    pub fn new() -> Result<Self, Box<dyn std::error::Error>> {
        let registry = Arc::new(Registry::new());

        // Counter: total inference requests by model and status
        let inference_requests_total = CounterVec::new(
            prometheus::Opts::new(
                "inference_requests_total",
                "Total number of inference requests by model and status",
            ),
            &["model", "status"],
        )?;
        registry.register(Box::new(inference_requests_total.clone()))?;

        // Counter: inference errors by type
        let inference_errors_total = CounterVec::new(
            prometheus::Opts::new(
                "inference_errors_total",
                "Total inference errors by error type (timeout, invalid_input, backend_error)",
            ),
            &["error_type"],
        )?;
        registry.register(Box::new(inference_errors_total.clone()))?;

        // Counter: rate limited requests
        let rate_limited_requests_total = Counter::with_opts(
            prometheus::Opts::new(
                "rate_limited_requests_total",
                "Total requests rejected due to rate limiting",
            )
        )?;
        registry.register(Box::new(rate_limited_requests_total.clone()))?;

        // Counter: circuit breaker trips
        let circuit_breaker_trips_total = Counter::with_opts(
            prometheus::Opts::new(
                "circuit_breaker_trips_total",
                "Total times circuit breaker transitioned to Open state",
            )
        )?;
        registry.register(Box::new(circuit_breaker_trips_total.clone()))?;

        // Histogram: inference latency by model
        let inference_latency_ms = HistogramVec::new(
            prometheus::HistogramOpts::new(
                "inference_latency_ms",
                "Inference latency in milliseconds",
            )
            .buckets(vec![10.0, 50.0, 100.0, 250.0, 500.0, 1000.0, 2000.0, 5000.0]),
            &["model"],
        )?;
        registry.register(Box::new(inference_latency_ms.clone()))?;

        // Histogram: tokens generated per inference
        let inference_tokens_generated = HistogramVec::new(
            prometheus::HistogramOpts::new(
                "inference_tokens_generated",
                "Number of tokens generated per inference request",
            )
            .buckets(vec![1.0, 10.0, 50.0, 100.0, 256.0, 512.0, 1024.0, 2048.0, 4096.0]),
            &["model"],
        )?;
        registry.register(Box::new(inference_tokens_generated.clone()))?;

        // Gauge: circuit breaker state
        let circuit_breaker_state = IntGauge::with_opts(
            prometheus::Opts::new(
                "circuit_breaker_state",
                "Circuit breaker state: 0=Closed (healthy), 1=Open (failing), 2=HalfOpen (recovering)",
            )
        )?;
        registry.register(Box::new(circuit_breaker_state.clone()))?;

        // Gauge: active bulkhead requests
        let bulkhead_active_requests = IntGauge::with_opts(
            prometheus::Opts::new(
                "bulkhead_active_requests",
                "Current number of active requests in bulkhead",
            )
        )?;
        registry.register(Box::new(bulkhead_active_requests.clone()))?;

        // Gauge: bulkhead max slots
        let bulkhead_max_slots = IntGauge::with_opts(
            prometheus::Opts::new(
                "bulkhead_max_slots",
                "Maximum concurrent request slots in bulkhead",
            )
        )?;
        registry.register(Box::new(bulkhead_max_slots.clone()))?;

        // Gauge: cache hit ratio
        let cache_hit_ratio_percent = IntGauge::with_opts(
            prometheus::Opts::new(
                "cache_hit_ratio_percent",
                "Request cache hit ratio as percentage (0-100)",
            )
        )?;
        registry.register(Box::new(cache_hit_ratio_percent.clone()))?;

        Ok(PrometheusMetrics {
            inference_requests_total,
            inference_errors_total,
            rate_limited_requests_total,
            circuit_breaker_trips_total,
            inference_latency_ms,
            inference_tokens_generated,
            circuit_breaker_state,
            bulkhead_active_requests,
            bulkhead_max_slots,
            cache_hit_ratio_percent,
            registry,
        })
    }

    /// Export all metrics in Prometheus text format
    pub fn export(&self) -> Result<String, Box<dyn std::error::Error>> {
        let encoder = TextEncoder::new();
        let metric_families = self.registry.gather();
        let mut buffer = vec![];
        encoder.encode(&metric_families, &mut buffer)?;
        Ok(String::from_utf8(buffer)?)
    }

    /// Record successful inference
    pub fn record_inference_success(
        &self,
        model: &str,
        latency_ms: u32,
        tokens_generated: u32,
    ) {
        self.inference_requests_total
            .with_label_values(&[model, "success"])
            .inc();

        self.inference_latency_ms
            .with_label_values(&[model])
            .observe(latency_ms as f64);

        self.inference_tokens_generated
            .with_label_values(&[model])
            .observe(tokens_generated as f64);
    }

    /// Record inference error
    pub fn record_inference_error(&self, error_type: &str) {
        self.inference_errors_total
            .with_label_values(&[error_type])
            .inc();
    }

    /// Record rate limited request
    pub fn record_rate_limited(&self) {
        self.rate_limited_requests_total.inc();
    }

    /// Record circuit breaker trip
    pub fn record_circuit_breaker_trip(&self) {
        self.circuit_breaker_trips_total.inc();
    }

    /// Update circuit breaker state (0=Closed, 1=Open, 2=HalfOpen)
    pub fn set_circuit_breaker_state(&self, state: i64) {
        self.circuit_breaker_state.set(state);
    }

    /// Update active bulkhead request count
    pub fn set_bulkhead_active_requests(&self, count: i64) {
        self.bulkhead_active_requests.set(count);
    }

    /// Set bulkhead maximum concurrent slots
    pub fn set_bulkhead_max_slots(&self, max_slots: i64) {
        self.bulkhead_max_slots.set(max_slots);
    }

    /// Update cache hit ratio (0-100)
    pub fn set_cache_hit_ratio(&self, ratio_percent: i64) {
        self.cache_hit_ratio_percent.set(ratio_percent);
    }
}

impl Default for PrometheusMetrics {
    fn default() -> Self {
        Self::new().expect("Failed to initialize Prometheus metrics")
    }
}

/// GatewayMetrics for backward compatibility
pub struct GatewayMetrics {
    pub prometheus: PrometheusMetrics,
    total_requests: AtomicU64,
}

impl GatewayMetrics {
    pub fn new() -> Self {
        Self {
            prometheus: PrometheusMetrics::default(),
            total_requests: AtomicU64::new(0),
        }
    }

    pub fn record_request(&self) {
        self.total_requests.fetch_add(1, Ordering::SeqCst);
    }

    pub fn record_completed(&self) {}
    pub fn record_failed(&self) {}
    pub fn record_rate_limited(&self) {
        self.prometheus.record_rate_limited();
    }
    pub fn record_latency(&self, _latency_ms: f64) {}
    pub fn record_queue_depth(&self, _depth: usize) {}
    pub fn record_queued(&self) {}
    pub fn record_dequeued(&self) {}

    pub fn get_total_requests(&self) -> u64 {
        self.total_requests.load(Ordering::SeqCst)
    }

    pub fn get_total_completed(&self) -> u64 { 0 }
    pub fn get_total_failed(&self) -> u64 { 0 }
    pub fn get_total_rate_limited(&self) -> u64 { 0 }
    pub fn get_active_streams(&self) -> u64 { 0 }
    pub fn get_avg_latency_ms(&self) -> f64 { 0.0 }
    pub fn get_p99_latency_ms(&self) -> f64 { 0.0 }

    pub fn summary(&self) -> GatewayMetricsSummary {
        GatewayMetricsSummary {
            total_requests: self.get_total_requests(),
            total_completed: 0,
            total_failed: 0,
            total_rate_limited: 0,
            active_streams: 0,
            avg_latency_ms: 0.0,
            p99_latency_ms: 0.0,
        }
    }
}

impl Default for GatewayMetrics {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Debug, Clone)]
pub struct GatewayMetricsSummary {
    pub total_requests: u64,
    pub total_completed: u64,
    pub total_failed: u64,
    pub total_rate_limited: u64,
    pub active_streams: u64,
    pub avg_latency_ms: f64,
    pub p99_latency_ms: f64,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_prometheus_metrics_creation() {
        let metrics = PrometheusMetrics::new();
        assert!(metrics.is_ok());
    }

    #[test]
    fn test_record_inference_success() {
        let metrics = PrometheusMetrics::new().unwrap();
        metrics.record_inference_success("llama-7b", 250, 50);

        let export = metrics.export().unwrap();
        assert!(export.contains("inference_requests_total"));
        assert!(export.contains("inference_latency_ms"));
    }

    #[test]
    fn test_legacy_metrics_tracking() {
        let metrics = GatewayMetrics::new();
        metrics.record_request();
        metrics.record_latency(100.0);
        metrics.record_completed();

        assert_eq!(metrics.get_total_requests(), 1);
        assert_eq!(metrics.get_total_completed(), 1);
        assert_eq!(metrics.get_avg_latency_ms(), 100.0);
    }
}
