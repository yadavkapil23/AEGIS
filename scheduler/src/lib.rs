// Scheduler module: KV cache allocation and management

pub mod allocator;
pub mod policy;
pub mod metrics;
pub mod distributed;
pub mod block_ownership;
pub mod failure_detector;
pub mod consistency;
pub mod node_selector;
pub mod remote_allocator;
pub mod grpc_server;
pub mod tracing_integration;
pub mod consensus;
pub mod replicated_log;
pub mod state_machine;
pub mod state_machine_coordinator;
pub mod state_machine_replication;
pub mod state_machine_grpc;
pub mod consensus_kv_cache;
pub mod persistence;
pub mod consensus_allocator;
pub mod config;
pub mod health;
pub mod graceful_shutdown;

pub use allocator::KVCacheAllocator;
pub use config::SchedulerConfig as ConfigModule;
pub use health::{HealthChecker, HealthStatus, HealthInfo, ReadinessStatus, ReadinessInfo};
pub use graceful_shutdown::GracefulShutdown;
pub use consensus_allocator::{ConsensusAllocator, AllocationCommand, CommandOutput, ConsensusAllocatorMetrics};
pub use policy::EvictionPolicy;
pub use metrics::SchedulerMetrics;
pub use distributed::DistributedKVCache;
pub use block_ownership::BlockOwnership;
pub use failure_detector::FailureDetector;
pub use consistency::ConsistencyValidator;
pub use node_selector::NodeSelector;
pub use remote_allocator::RemoteAllocator;
pub use grpc_server::{
    serve as serve_scheduling, serve_with_shutdown as serve_scheduling_with_shutdown,
    SchedulingServiceImpl,
};
pub use tracing_integration::{SchedulerTracing, AllocationSpan, DeallocationSpan, GrpcCallSpan, RemoteAllocationSpan};

use anyhow::Result;
use std::sync::Arc;

/// SchedulerConfig: configuration for KV cache scheduler
#[derive(Debug, Clone)]
pub struct SchedulerConfig {
    pub total_cache_bytes: usize,    // total KV cache size
    pub block_size_bytes: usize,     // minimum allocation unit
    pub eviction_policy: String,     // "lru" or "lfu"
    pub enable_predictive: bool,     // enable predictive allocation
}

impl Default for SchedulerConfig {
    fn default() -> Self {
        Self {
            total_cache_bytes: 8 * 1024 * 1024 * 1024, // 8GB
            block_size_bytes: 16 * 1024,               // 16KB blocks
            eviction_policy: "lru".to_string(),
            enable_predictive: true,
        }
    }
}

/// KVScheduler: orchestrates KV cache allocation across requests
pub struct KVScheduler {
    allocator: Arc<KVCacheAllocator>,
    metrics: Arc<SchedulerMetrics>,
}

impl KVScheduler {
    pub fn new(config: SchedulerConfig) -> Result<Self> {
        let allocator = Arc::new(KVCacheAllocator::new(
            config.total_cache_bytes,
            config.block_size_bytes,
        )?);

        let metrics = Arc::new(SchedulerMetrics::new());

        Ok(Self {
            allocator,
            metrics,
        })
    }

    /// Allocate KV cache blocks for a request
    pub fn allocate(&self, request_id: &str, num_blocks: usize) -> Result<Vec<usize>> {
        let block_ids = self.allocator.allocate(num_blocks)?;
        self.metrics.record_allocation(num_blocks);
        Ok(block_ids)
    }

    /// Deallocate KV cache blocks
    pub fn deallocate(&self, block_ids: &[usize]) -> Result<()> {
        self.allocator.deallocate(block_ids)?;
        self.metrics.record_deallocation(block_ids.len());
        Ok(())
    }

    /// Get cache statistics
    pub fn stats(&self) -> allocator::CacheStats {
        self.allocator.stats()
    }

    /// Get metrics
    pub fn metrics(&self) -> Arc<SchedulerMetrics> {
        self.metrics.clone()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_scheduler_creation() {
        let config = SchedulerConfig::default();
        let scheduler = KVScheduler::new(config);
        assert!(scheduler.is_ok());
    }

    #[test]
    fn test_allocate() {
        let config = SchedulerConfig::default();
        let scheduler = KVScheduler::new(config).unwrap();

        let blocks = scheduler.allocate("req-1", 10);
        assert!(blocks.is_ok());
        assert_eq!(blocks.unwrap().len(), 10);
    }

    #[test]
    fn test_deallocate() {
        let config = SchedulerConfig::default();
        let scheduler = KVScheduler::new(config).unwrap();

        let blocks = scheduler.allocate("req-1", 10).unwrap();
        let result = scheduler.deallocate(&blocks);
        assert!(result.is_ok());
    }
}
pub mod consensus_grpc_server;

pub use consensus_grpc_server::{
    GrpcServerConfig, ConsensusGrpcServer, RpcClientPool, RpcClient, RpcMetrics,
    PeerHealthStatus, ServerHealthStatus, PoolMetricsSummary,
};
