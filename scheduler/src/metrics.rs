// Scheduler metrics: KV cache hit rate, fragmentation, allocation latency

use parking_lot::Mutex;
use std::sync::atomic::{AtomicU64, Ordering};

/// SchedulerMetrics: tracks KV cache performance
pub struct SchedulerMetrics {
    // Counters
    total_allocations: AtomicU64,
    total_deallocations: AtomicU64,
    total_evictions: AtomicU64,

    // Histograms
    allocation_latencies_us: Mutex<Vec<u64>>,
    fragmentation_percentages: Mutex<Vec<f64>>,

    // Hit tracking
    cache_hits: AtomicU64,
    cache_misses: AtomicU64,
}

impl SchedulerMetrics {
    pub fn new() -> Self {
        Self {
            total_allocations: AtomicU64::new(0),
            total_deallocations: AtomicU64::new(0),
            total_evictions: AtomicU64::new(0),

            allocation_latencies_us: Mutex::new(Vec::new()),
            fragmentation_percentages: Mutex::new(Vec::new()),

            cache_hits: AtomicU64::new(0),
            cache_misses: AtomicU64::new(0),
        }
    }

    pub fn record_allocation(&self, num_blocks: usize) {
        self.total_allocations.fetch_add(num_blocks as u64, Ordering::SeqCst);
    }

    pub fn record_deallocation(&self, num_blocks: usize) {
        self.total_deallocations.fetch_add(num_blocks as u64, Ordering::SeqCst);
    }

    pub fn record_eviction(&self, num_blocks: usize) {
        self.total_evictions.fetch_add(num_blocks as u64, Ordering::SeqCst);
    }

    pub fn record_allocation_latency_us(&self, latency_us: u64) {
        let mut latencies = self.allocation_latencies_us.lock();
        latencies.push(latency_us);
    }

    pub fn record_fragmentation(&self, fragmentation: f64) {
        let mut frags = self.fragmentation_percentages.lock();
        frags.push(fragmentation);
    }

    pub fn record_cache_hit(&self) {
        self.cache_hits.fetch_add(1, Ordering::SeqCst);
    }

    pub fn record_cache_miss(&self) {
        self.cache_misses.fetch_add(1, Ordering::SeqCst);
    }

    // Accessors
    pub fn get_total_allocations(&self) -> u64 {
        self.total_allocations.load(Ordering::SeqCst)
    }

    pub fn get_total_deallocations(&self) -> u64 {
        self.total_deallocations.load(Ordering::SeqCst)
    }

    pub fn get_total_evictions(&self) -> u64 {
        self.total_evictions.load(Ordering::SeqCst)
    }

    pub fn get_cache_hit_rate(&self) -> f64 {
        let hits = self.cache_hits.load(Ordering::SeqCst);
        let misses = self.cache_misses.load(Ordering::SeqCst);
        let total = hits + misses;

        if total == 0 {
            return 0.0;
        }

        (hits as f64) / (total as f64)
    }

    pub fn get_avg_allocation_latency_us(&self) -> f64 {
        let latencies = self.allocation_latencies_us.lock();
        if latencies.is_empty() {
            return 0.0;
        }
        let sum: u64 = latencies.iter().sum();
        (sum as f64) / (latencies.len() as f64)
    }

    pub fn get_avg_fragmentation(&self) -> f64 {
        let frags = self.fragmentation_percentages.lock();
        if frags.is_empty() {
            return 0.0;
        }
        frags.iter().sum::<f64>() / frags.len() as f64
    }

    pub fn summary(&self) -> SchedulerMetricsSummary {
        SchedulerMetricsSummary {
            total_allocations: self.get_total_allocations(),
            total_deallocations: self.get_total_deallocations(),
            total_evictions: self.get_total_evictions(),
            cache_hit_rate: self.get_cache_hit_rate(),
            avg_allocation_latency_us: self.get_avg_allocation_latency_us(),
            avg_fragmentation: self.get_avg_fragmentation(),
        }
    }
}

impl Default for SchedulerMetrics {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Debug, Clone)]
pub struct SchedulerMetricsSummary {
    pub total_allocations: u64,
    pub total_deallocations: u64,
    pub total_evictions: u64,
    pub cache_hit_rate: f64,
    pub avg_allocation_latency_us: f64,
    pub avg_fragmentation: f64,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_metrics_tracking() {
        let metrics = SchedulerMetrics::new();

        metrics.record_allocation(10);
        metrics.record_cache_hit();
        metrics.record_cache_hit();
        metrics.record_cache_miss();

        assert_eq!(metrics.get_total_allocations(), 10);
        assert_eq!(metrics.get_cache_hit_rate(), 2.0 / 3.0);
    }
}
