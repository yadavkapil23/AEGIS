use prometheus::{
    Registry, Encoder, TextEncoder,
    IntCounter, IntGauge, Histogram, HistogramOpts,
    opts,
};
use anyhow::Result;
use std::sync::OnceLock;
use tracing::info;

fn registry() -> &'static Registry {
    static REG: OnceLock<Registry> = OnceLock::new();
    REG.get_or_init(Registry::new)
}

fn inference_requests() -> &'static IntCounter {
    static M: OnceLock<IntCounter> = OnceLock::new();
    M.get_or_init(|| IntCounter::with_opts(opts!("aegis_inference_requests_total", "Total inference requests")).unwrap())
}

fn inference_errors() -> &'static IntCounter {
    static M: OnceLock<IntCounter> = OnceLock::new();
    M.get_or_init(|| IntCounter::with_opts(opts!("aegis_inference_errors_total", "Total inference errors")).unwrap())
}

fn tokens_generated() -> &'static IntCounter {
    static M: OnceLock<IntCounter> = OnceLock::new();
    M.get_or_init(|| IntCounter::with_opts(opts!("aegis_tokens_generated_total", "Total tokens generated")).unwrap())
}

fn rate_limited() -> &'static IntCounter {
    static M: OnceLock<IntCounter> = OnceLock::new();
    M.get_or_init(|| IntCounter::with_opts(opts!("aegis_rate_limited_total", "Total rate-limited requests")).unwrap())
}

fn backend_health() -> &'static IntGauge {
    static M: OnceLock<IntGauge> = OnceLock::new();
    M.get_or_init(|| IntGauge::with_opts(opts!("aegis_backend_healthy", "Backend health (1=up, 0=down)")).unwrap())
}

fn kv_cache_hits() -> &'static IntCounter {
    static M: OnceLock<IntCounter> = OnceLock::new();
    M.get_or_init(|| IntCounter::with_opts(opts!("aegis_kv_cache_hits_total", "KV-cache hits")).unwrap())
}

fn kv_cache_misses() -> &'static IntCounter {
    static M: OnceLock<IntCounter> = OnceLock::new();
    M.get_or_init(|| IntCounter::with_opts(opts!("aegis_kv_cache_misses_total", "KV-cache misses")).unwrap())
}

fn inference_latency() -> &'static Histogram {
    static M: OnceLock<Histogram> = OnceLock::new();
    M.get_or_init(|| Histogram::with_opts(
        HistogramOpts::new("aegis_inference_latency_seconds", "Inference latency in seconds")
            .buckets(vec![0.01, 0.025, 0.05, 0.1, 0.25, 0.5, 1.0, 2.5, 5.0, 10.0])
    ).unwrap())
}

fn active_requests() -> &'static IntGauge {
    static M: OnceLock<IntGauge> = OnceLock::new();
    M.get_or_init(|| IntGauge::with_opts(opts!("aegis_active_requests", "Currently active inference requests")).unwrap())
}

/// Initialize and register all Prometheus metrics.
pub fn init_metrics() -> Result<()> {
    let r = registry();
    r.register(Box::new(inference_requests().clone()))?;
    r.register(Box::new(inference_errors().clone()))?;
    r.register(Box::new(tokens_generated().clone()))?;
    r.register(Box::new(rate_limited().clone()))?;
    r.register(Box::new(backend_health().clone()))?;
    r.register(Box::new(kv_cache_hits().clone()))?;
    r.register(Box::new(kv_cache_misses().clone()))?;
    r.register(Box::new(inference_latency().clone()))?;
    r.register(Box::new(active_requests().clone()))?;
    info!("Prometheus metrics registered (9 metrics)");
    Ok(())
}

/// Get the global Prometheus registry.
pub fn get_registry() -> &'static Registry {
    registry()
}

/// Scrape all metrics in Prometheus text format.
pub fn scrape() -> String {
    let encoder = TextEncoder::new();
    let metric_families = registry().gather();
    let mut buffer = Vec::new();
    encoder.encode(&metric_families, &mut buffer).unwrap();
    String::from_utf8(buffer).unwrap()
}

/// Record an inference request.
pub fn record_inference(_model: &str, latency_secs: f64, tokens: u32) {
    inference_requests().inc();
    tokens_generated().inc_by(tokens as u64);
    inference_latency().observe(latency_secs);
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
