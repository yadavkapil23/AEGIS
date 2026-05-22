// Backend metrics tracking

use parking_lot::Mutex;
use std::sync::atomic::{AtomicU64, Ordering};

/// Metrics for inference backend performance
pub struct BackendMetrics {
    total_generations: AtomicU64,
    total_tokens_generated: AtomicU64,
    total_prompt_tokens: AtomicU64,

    latencies_ms: Mutex<Vec<f64>>,
    tokens_per_batch: Mutex<Vec<usize>>,
}

impl BackendMetrics {
    pub fn new() -> Self {
        Self {
            total_generations: AtomicU64::new(0),
            total_tokens_generated: AtomicU64::new(0),
            total_prompt_tokens: AtomicU64::new(0),

            latencies_ms: Mutex::new(Vec::new()),
            tokens_per_batch: Mutex::new(Vec::new()),
        }
    }

    pub fn record_generation(&self, latency_ms: f64, prompt_tokens: usize, completion_tokens: usize) {
        self.total_generations.fetch_add(1, Ordering::SeqCst);
        self.total_tokens_generated.fetch_add(completion_tokens as u64, Ordering::SeqCst);
        self.total_prompt_tokens.fetch_add(prompt_tokens as u64, Ordering::SeqCst);

        let mut latencies = self.latencies_ms.lock();
        latencies.push(latency_ms);

        let mut batches = self.tokens_per_batch.lock();
        batches.push(completion_tokens);
    }

    // Accessors
    pub fn get_total_generations(&self) -> u64 {
        self.total_generations.load(Ordering::SeqCst)
    }

    pub fn get_total_tokens(&self) -> u64 {
        self.total_tokens_generated.load(Ordering::SeqCst)
    }

    pub fn get_avg_latency_ms(&self) -> f64 {
        let latencies = self.latencies_ms.lock();
        if latencies.is_empty() {
            return 0.0;
        }
        latencies.iter().sum::<f64>() / latencies.len() as f64
    }

    pub fn get_p99_latency_ms(&self) -> f64 {
        let mut latencies = self.latencies_ms.lock().clone();
        if latencies.is_empty() {
            return 0.0;
        }
        latencies.sort_by(|a, b| a.partial_cmp(b).unwrap());
        let idx = (latencies.len() as f64 * 0.99) as usize;
        latencies[idx.min(latencies.len() - 1)]
    }

    pub fn get_throughput_tokens_per_sec(&self) -> f64 {
        let total_tokens = self.total_tokens_generated.load(Ordering::SeqCst);
        let avg_latency_ms = self.get_avg_latency_ms();

        if avg_latency_ms == 0.0 {
            return 0.0;
        }

        (total_tokens as f64 * 1000.0) / (avg_latency_ms * self.get_total_generations() as f64)
    }

    pub fn summary(&self) -> BackendMetricsSummary {
        BackendMetricsSummary {
            total_generations: self.get_total_generations(),
            total_tokens: self.get_total_tokens(),
            avg_latency_ms: self.get_avg_latency_ms(),
            p99_latency_ms: self.get_p99_latency_ms(),
            throughput_tokens_per_sec: self.get_throughput_tokens_per_sec(),
        }
    }
}

impl Default for BackendMetrics {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Debug, Clone)]
pub struct BackendMetricsSummary {
    pub total_generations: u64,
    pub total_tokens: u64,
    pub avg_latency_ms: f64,
    pub p99_latency_ms: f64,
    pub throughput_tokens_per_sec: f64,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_backend_metrics() {
        let metrics = BackendMetrics::new();

        metrics.record_generation(10.0, 5, 10);
        metrics.record_generation(15.0, 5, 10);
        metrics.record_generation(12.0, 5, 10);

        assert_eq!(metrics.get_total_generations(), 3);
        assert_eq!(metrics.get_total_tokens(), 30);

        let avg = metrics.get_avg_latency_ms();
        assert!((avg - 12.33).abs() < 0.1);
    }
}
