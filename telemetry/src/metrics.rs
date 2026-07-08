use prometheus::{
    Registry, Encoder, TextEncoder,
    IntCounter, IntGauge, Histogram, HistogramOpts, Opts,
    opts,
};
use anyhow::Result;
use std::sync::Arc;
use parking_lot::Mutex;
use tracing::info;

lazy_static::lazy_static! {
    static ref REGISTRY: Registry = Registry::new();

    pub static ref INFERENCE_REQUESTS_TOTAL: IntCounter =
        IntCounter::with_opts(opts!("aegis_inference_requests_total", "Total inference requests"))
            .unwrap();

    pub static ref INFERENCE_ERRORS_TOTAL: IntCounter =
        IntCounter::with_opts(opts!("aegis_inference_errors_total", "Total inference errors"))
            .unwrap();

    pub static ref TOKENS_GENERATED_TOTAL: IntCounter =
        IntCounter::with_opts(opts!("aegis_tokens_generated_total", "Total tokens generated"))
            .unwrap();

    pub static ref RATE_LIMITED_TOTAL: IntCounter =
        IntCounter::with_opts(opts!("aegis_rate_limited_total", "Total rate-limited requests"))
            .unwrap();

    pub static ref BACKEND_HEALTH: IntGauge =
        IntGauge::with_opts(opts!("aegis_backend_healthy", "Backend health (1=up, 0=down)"))
            .unwrap();

    pub static ref KV_CACHE_HITS: IntCounter =
        IntCounter::with_opts(opts!("aegis_kv_cache_hits_total", "KV-cache hits"))
            .unwrap();

    pub static ref KV_CACHE_MISSES: IntCounter =
        IntCounter::with_opts(opts!("aegis_kv_cache_misses_total", "KV-cache misses"))
            .unwrap();

    pub static ref INFERENCE_LATENCY: Histogram =
        Histogram::with_opts(HistogramOpts::new("aegis_inference_latency_seconds", "Inference latency in seconds")
            .buckets(vec![0.01, 0.025, 0.05, 0.1, 0.25, 0.5, 1.0, 2.5, 5.0, 10.0]))
            .unwrap();

    pub static ref ACTIVE_REQUESTS: IntGauge =
        IntGauge::with_opts(opts!("aegis_active_requests", "Currently active inference requests"))
            .unwrap();
}

/// Initialize and register all Prometheus metrics.
pub fn init_metrics() -> Result<()> {
    let r = &*REGISTRY;
    r.register(Box::new(INFERENCE_REQUESTS_TOTAL.clone()))?;
    r.register(Box::new(INFERENCE_ERRORS_TOTAL.clone()))?;
    r.register(Box::new(TOKENS_GENERATED_TOTAL.clone()))?;
    r.register(Box::new(RATE_LIMITED_TOTAL.clone()))?;
    r.register(Box::new(BACKEND_HEALTH.clone()))?;
    r.register(Box::new(KV_CACHE_HITS.clone()))?;
    r.register(Box::new(KV_CACHE_MISSES.clone()))?;
    r.register(Box::new(INFERENCE_LATENCY.clone()))?;
    r.register(Box::new(ACTIVE_REQUESTS.clone()))?;

    info!("Prometheus metrics registered (9 metrics)");
    Ok(())
}

/// Get the global Prometheus registry.
pub fn registry() -> &'static Registry {
    &REGISTRY
}

/// Scrape all metrics in Prometheus text format.
pub fn scrape() -> String {
    let encoder = TextEncoder::new();
    let metric_families = REGISTRY.gather();
    let mut buffer = Vec::new();
    encoder.encode(&metric_families, &mut buffer).unwrap();
    String::from_utf8(buffer).unwrap()
}

/// Simple helper to record an inference request.
pub fn record_inference(model: &str, latency_secs: f64, tokens: u32) {
    INFERENCE_REQUESTS_TOTAL.inc();
    TOKENS_GENERATED_TOTAL.inc_by(tokens as u64);
    INFERENCE_LATENCY.observe(latency_secs);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_scrape_returns_prometheus_format() {
        let _ = init_metrics();
        let output = scrape();
        assert!(output.contains("aegis_inference_requests_total"));
    }
}
