// Consensus-driven KV cache: Integrates DistributedKVCache with consensus layer
// All allocation requests go through the consensus system for consistency

use crate::distributed::DistributedKVCache;
use crate::state_machine_coordinator::StateMachineCoordinator;
use crate::state_machine_grpc::StateMachineGrpcService;
use std::sync::Arc;
use parking_lot::Mutex;
use anyhow::{anyhow, Result};
use tracing::{debug, info, warn};

/// Consensus-driven allocation request
#[derive(Clone, Debug)]
pub struct AllocationRequest {
    pub request_id: String,
    pub num_blocks: usize,
    pub requester_id: Option<String>,
}

/// Allocation result
#[derive(Clone, Debug)]
pub struct AllocationResult {
    pub request_id: String,
    pub block_ids: Vec<usize>,
    pub lsn: u64,
    pub applied: bool,
}

/// Consensus-backed KV Cache
pub struct ConsensusKVCache {
    kv_cache: Arc<DistributedKVCache>,
    coordinator: Arc<StateMachineCoordinator>,
    grpc_service: Arc<StateMachineGrpcService>,
    pending_requests: Arc<Mutex<Vec<AllocationRequest>>>,
}

impl ConsensusKVCache {
    /// Create new consensus-backed KV cache
    pub fn new(
        kv_cache: Arc<DistributedKVCache>,
        coordinator: Arc<StateMachineCoordinator>,
        grpc_service: Arc<StateMachineGrpcService>,
    ) -> Self {
        Self {
            kv_cache,
            coordinator,
            grpc_service,
            pending_requests: Arc::new(Mutex::new(Vec::new())),
        }
    }

    /// Allocate KV cache blocks (goes through consensus)
    pub fn allocate(&self, request_id: &str, num_blocks: usize) -> Result<AllocationResult> {
        // Check if we're the leader
        if !self.coordinator.is_leader() {
            return Err(anyhow!(
                "Not leader (current term: {})",
                self.coordinator.current_term()
            ));
        }

        debug!(
            "Leader allocating: request_id={}, blocks={}",
            request_id, num_blocks
        );

        // Append to log via coordinator
        let lsn = self.coordinator.allocate(request_id, num_blocks)?;

        // Add to pending requests
        self.pending_requests.lock().push(AllocationRequest {
            request_id: request_id.to_string(),
            num_blocks,
            requester_id: None,
        });

        Ok(AllocationResult {
            request_id: request_id.to_string(),
            block_ids: vec![],
            lsn,
            applied: false,
        })
    }

    /// Deallocate KV cache blocks
    pub fn deallocate(&self, request_id: &str, block_ids: Vec<usize>) -> Result<u64> {
        if !self.coordinator.is_leader() {
            return Err(anyhow!("Not leader"));
        }

        debug!(
            "Leader deallocating: request_id={}, blocks={:?}",
            request_id, block_ids
        );

        self.coordinator.deallocate(request_id, block_ids)
    }

    /// Commit allocated blocks (after replication)
    pub fn commit_allocation(&self, lsn: u64) -> Result<()> {
        if !self.coordinator.is_leader() {
            return Err(anyhow!("Not leader"));
        }

        debug!("Committing allocation at LSN {}", lsn);
        self.coordinator.commit_to_lsn(lsn)?;
        Ok(())
    }

    /// Apply pending allocations to state machine and KV cache
    pub async fn apply_pending_allocations(&self) -> Result<usize> {
        let applied_count = self.coordinator.apply_pending()?;

        debug!("Applied {} pending allocations", applied_count);

        // Process applied allocations in KV cache
        let pending_reqs: Vec<_> = {
            let pending = self.pending_requests.lock();
            pending.iter().cloned().collect()
        };
        for req in pending_reqs {
            // Perform actual allocation in KV cache
            match self
                .kv_cache
                .allocate_global(&req.request_id, req.num_blocks)
                .await
            {
                Ok(block_ids) => {
                    info!(
                        "Allocated blocks for {}: {:?}",
                        req.request_id, block_ids
                    );
                }
                Err(e) => {
                    warn!("Failed to allocate blocks for {}: {}", req.request_id, e);
                }
            }
        }

        Ok(applied_count)
    }

    /// Get allocation status from state machine
    pub fn get_allocation_status(&self, request_id: &str) -> Option<AllocationStatus> {
        self.coordinator.get_allocation(request_id).map(|alloc| {
            AllocationStatus {
                request_id: alloc.request_id.clone(),
                num_blocks: alloc.num_blocks,
                applied_at_ms: alloc.applied_at,
                is_applied: true,
            }
        })
    }

    /// Check if node is leader
    pub fn is_leader(&self) -> bool {
        self.coordinator.is_leader()
    }

    /// Get current term
    pub fn current_term(&self) -> u64 {
        self.coordinator.current_term()
    }

    /// Get log length
    pub fn log_length(&self) -> usize {
        self.coordinator.log_len()
    }

    /// Get commit index
    pub fn commit_index(&self) -> u64 {
        self.coordinator.commit_index()
    }

    /// Get state hash for consistency verification
    pub fn state_hash(&self) -> String {
        self.coordinator.state_hash().to_hex().to_string()
    }

    /// Get KV cache statistics
    pub fn cache_stats(&self) -> Result<crate::allocator::CacheStats> {
        Ok(self.kv_cache.local_stats())
    }

    /// Verify consistency: state machine hash should match across nodes
    pub fn verify_consistency(&self, expected_hash: &str) -> bool {
        self.state_hash() == expected_hash
    }

    // Internal accessors
    pub fn coordinator(&self) -> Arc<StateMachineCoordinator> {
        self.coordinator.clone()
    }

    pub fn kv_cache(&self) -> Arc<DistributedKVCache> {
        self.kv_cache.clone()
    }

    pub fn grpc_service(&self) -> Arc<StateMachineGrpcService> {
        self.grpc_service.clone()
    }
}

/// Allocation status
#[derive(Clone, Debug)]
pub struct AllocationStatus {
    pub request_id: String,
    pub num_blocks: usize,
    pub applied_at_ms: u64,
    pub is_applied: bool,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::consensus::QuorumConfig;
    use crate::block_ownership::BlockOwnership;

    fn create_consensus_kv_cache() -> ConsensusKVCache {
        let config = QuorumConfig::new("node-1", vec![
            "node-1".to_string(),
            "node-2".to_string(),
            "node-3".to_string(),
        ]);

        let coordinator = Arc::new(StateMachineCoordinator::new(config, 100));
        let replication = Arc::new(crate::state_machine_replication::StateMachineReplication::new(
            coordinator.clone(),
        ));
        let grpc_service = Arc::new(StateMachineGrpcService::new(coordinator.clone(), replication));

        let kv_cache = Arc::new(DistributedKVCache::new(
            8 * 1024 * 1024,
            16 * 1024,
            Arc::new(BlockOwnership::new()),
        ));

        ConsensusKVCache::new(kv_cache, coordinator, grpc_service)
    }

    #[test]
    fn test_consensus_kv_cache_creation() {
        let cache = create_consensus_kv_cache();
        assert!(!cache.is_leader());
    }

    #[test]
    fn test_allocate_requires_leadership() {
        let cache = create_consensus_kv_cache();

        let result = cache.allocate("req-1", 100);
        assert!(result.is_err());
    }

    #[test]
    fn test_leader_allocate() {
        let cache = create_consensus_kv_cache();

        // Become leader
        cache.coordinator().consensus().request_votes().ok();
        cache
            .coordinator()
            .consensus()
            .receive_vote("node-2", crate::consensus::Vote::Yes)
            .ok();
        cache.coordinator().consensus().check_election_won();

        // Allocate
        let result = cache.allocate("req-1", 100);
        assert!(result.is_ok());
        let alloc = result.unwrap();
        assert_eq!(alloc.request_id, "req-1");
        assert_eq!(alloc.lsn, 1);
    }

    #[tokio::test]
    async fn test_commit_and_apply() {
        let cache = create_consensus_kv_cache();

        // Become leader
        cache.coordinator().consensus().request_votes().ok();
        cache
            .coordinator()
            .consensus()
            .receive_vote("node-2", crate::consensus::Vote::Yes)
            .ok();
        cache.coordinator().consensus().check_election_won();

        // Allocate
        let alloc = cache.allocate("req-1", 100).unwrap();

        // Commit
        cache.commit_allocation(alloc.lsn).ok();

        // Apply
        let applied = cache.apply_pending_allocations().await.unwrap();
        assert_eq!(applied, 1);

        // Verify in state
        let status = cache.get_allocation_status("req-1");
        assert!(status.is_some());
    }

    #[tokio::test]
    async fn test_state_hash_verification() {
        let cache = create_consensus_kv_cache();

        // Become leader
        cache.coordinator().consensus().request_votes().ok();
        cache
            .coordinator()
            .consensus()
            .receive_vote("node-2", crate::consensus::Vote::Yes)
            .ok();
        cache.coordinator().consensus().check_election_won();

        let hash1 = cache.state_hash();

        // Allocate and apply
        let alloc = cache.allocate("req-1", 100).unwrap();
        cache.commit_allocation(alloc.lsn).ok();
        cache.apply_pending_allocations().await.ok();

        let hash2 = cache.state_hash();
        assert_ne!(hash1, hash2); // Should change after allocation
    }

    #[test]
    fn test_deallocate() {
        let cache = create_consensus_kv_cache();

        // Become leader
        cache.coordinator().consensus().request_votes().ok();
        cache
            .coordinator()
            .consensus()
            .receive_vote("node-2", crate::consensus::Vote::Yes)
            .ok();
        cache.coordinator().consensus().check_election_won();

        // Allocate
        let alloc = cache.allocate("req-1", 100).unwrap();
        cache.commit_allocation(alloc.lsn).ok();

        // Deallocate
        let result = cache.deallocate("req-1", vec![0, 1, 2]);
        assert!(result.is_ok());
    }

    #[test]
    fn test_log_length_tracking() {
        let cache = create_consensus_kv_cache();

        // Become leader
        cache.coordinator().consensus().request_votes().ok();
        cache
            .coordinator()
            .consensus()
            .receive_vote("node-2", crate::consensus::Vote::Yes)
            .ok();
        cache.coordinator().consensus().check_election_won();

        assert_eq!(cache.log_length(), 0);

        // Allocate multiple
        for i in 1..=5 {
            cache.allocate(&format!("req-{}", i), 10).ok();
        }

        assert_eq!(cache.log_length(), 5);
    }
}
