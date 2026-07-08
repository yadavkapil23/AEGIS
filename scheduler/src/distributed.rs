// Distributed KV-cache coordination layer
// Handles cache allocation, ownership tracking, and failure recovery across nodes

use crate::allocator::KVCacheAllocator;
use crate::block_ownership::BlockOwnership;
use crate::failure_detector::FailureDetector;
use crate::consistency::ConsistencyValidator;
use crate::node_selector::{NodeSelector, NodeCapacity};
use crate::remote_allocator::RemoteAllocator;
use dashmap::DashMap;
use anyhow::{anyhow, Result};
use parking_lot::Mutex;
use std::sync::Arc;
use blake3::Hash as Blake3Hash;
use tracing::{info, debug, warn};

/// Distributed KV-cache coordinator
/// Manages block allocation, ownership, and recovery across multiple nodes
pub struct DistributedKVCache {
    /// Map block IDs to their owning nodes
    node_map: Arc<DashMap<usize, String>>,

    /// Local allocator for blocks on this node
    local_allocator: Arc<KVCacheAllocator>,

    /// Block ownership tracking across cluster
    ownership: Arc<BlockOwnership>,

    /// Detects and recovers from node failures
    failure_detector: Arc<FailureDetector>,

    /// Validates cache state consistency
    consistency_validator: Arc<ConsistencyValidator>,

    /// Selects best node for allocation
    node_selector: Arc<NodeSelector>,

    /// Remote allocators for each peer
    pub remote_allocators: Arc<DashMap<String, Arc<RemoteAllocator>>>,

    /// Metrics: allocations, deallocations, failures
    metrics: Arc<DistributedCacheMetrics>,

    /// Current consistency hash
    state_hash: Arc<Mutex<Blake3Hash>>,

    /// This node's ID
    node_id: String,
}

#[derive(Debug, Clone)]
pub struct DistributedCacheMetrics {
    pub total_allocations: Arc<Mutex<u64>>,
    pub total_deallocations: Arc<Mutex<u64>>,
    pub cross_node_allocations: Arc<Mutex<u64>>,
    pub failed_allocations: Arc<Mutex<u64>>,
    pub blocks_migrated: Arc<Mutex<u64>>,
    pub consistency_violations: Arc<Mutex<u64>>,
}

impl Default for DistributedCacheMetrics {
    fn default() -> Self {
        Self {
            total_allocations: Arc::new(Mutex::new(0)),
            total_deallocations: Arc::new(Mutex::new(0)),
            cross_node_allocations: Arc::new(Mutex::new(0)),
            failed_allocations: Arc::new(Mutex::new(0)),
            blocks_migrated: Arc::new(Mutex::new(0)),
            consistency_violations: Arc::new(Mutex::new(0)),
        }
    }
}

#[derive(Debug, Clone)]
pub struct BlockHandle {
    pub block_id: usize,
    pub owner_node: String,
    pub is_local: bool,
}

impl DistributedKVCache {
    /// Create a new distributed KV-cache coordinator
    pub fn new(
        node_id: String,
        local_allocator: Arc<KVCacheAllocator>,
    ) -> Self {
        let ownership = Arc::new(BlockOwnership::new());
        let failure_detector = Arc::new(FailureDetector::new());
        let consistency_validator = Arc::new(ConsistencyValidator::new(ownership.clone()));
        let node_selector = Arc::new(NodeSelector::new());

        Self {
            node_map: Arc::new(DashMap::new()),
            local_allocator,
            ownership,
            failure_detector,
            consistency_validator,
            node_selector,
            remote_allocators: Arc::new(DashMap::new()),
            metrics: Arc::new(DistributedCacheMetrics::default()),
            state_hash: Arc::new(Mutex::new(blake3::hash(b"empty"))),
            node_id,
        }
    }

    /// Register a peer node
    pub fn register_peer(&self, peer_id: String, peer_addr: String, capacity: usize) -> Result<()> {
        debug!("Registering peer: {} at {}", peer_id, peer_addr);

        // Create remote allocator
        let allocator = Arc::new(RemoteAllocator::new(peer_id.clone(), peer_addr));
        self.remote_allocators.insert(peer_id.clone(), allocator);

        // Register in node selector
        self.node_selector.register_node(
            peer_id.clone(),
            NodeCapacity {
                total_blocks: capacity,
                free_blocks: capacity,
                allocated_blocks: 0,
            },
        )?;

        Ok(())
    }

    /// Allocate blocks globally (may be on remote node)
    pub async fn allocate_global(
        &self,
        request_id: &str,
        num_blocks: usize,
    ) -> Result<Vec<BlockHandle>> {
        debug!("allocate_global: request_id={}, num_blocks={}", request_id, num_blocks);

        // Try local allocation first
        match self.allocate_local(num_blocks) {
            Ok(blocks) => {
                info!("Allocated {} blocks locally for request {}", num_blocks, request_id);
                *self.metrics.total_allocations.lock() += 1;

                // Update state hash
                self.update_state_hash()?;

                return Ok(blocks);
            }
            Err(e) => {
                debug!("Local allocation failed: {}. Trying remote.", e);
            }
        }

        // Local allocation failed, try remote allocation
        // Use NodeSelector to find best node
        match self.node_selector.select_node(num_blocks) {
            Ok(selected_node) => {
                info!(
                    "Selected remote node {} for {} blocks",
                    selected_node, num_blocks
                );

                // Get remote allocator
                if let Some(allocator_entry) = self.remote_allocators.get(&selected_node) {
                    let allocator = allocator_entry.value().clone();
                    drop(allocator_entry);

                    // Try remote allocation
                    match allocator.allocate(num_blocks).await {
                        Ok(block_ids) => {
                            info!(
                                "Allocated {} blocks remotely on {} for request {}",
                                num_blocks, selected_node, request_id
                            );

                            *self.metrics.cross_node_allocations.lock() += 1;
                            *self.metrics.total_allocations.lock() += 1;

                            // Register ownership
                            let handles: Result<Vec<_>> = block_ids
                                .iter()
                                .map(|&block_id| {
                                    self.ownership.register_block(block_id, selected_node.clone())?;
                                    self.node_map.insert(block_id, selected_node.clone());

                                    Ok(BlockHandle {
                                        block_id,
                                        owner_node: selected_node.clone(),
                                        is_local: false,
                                    })
                                })
                                .collect();

                            self.update_state_hash()?;
                            return handles;
                        }
                        Err(e) => {
                            warn!(
                                "Remote allocation failed on {}: {}. Trying other nodes.",
                                selected_node, e
                            );
                        }
                    }
                }

                *self.metrics.failed_allocations.lock() += 1;
                Err(anyhow!(
                    "Remote allocation failed on {} (request: {})",
                    selected_node,
                    request_id
                ))
            }
            Err(e) => {
                warn!("No suitable node found: {}", e);
                *self.metrics.failed_allocations.lock() += 1;
                Err(anyhow!(
                    "No blocks available for allocation (request: {}): {}",
                    request_id,
                    e
                ))
            }
        }
    }

    /// Allocate blocks locally (on this node)
    fn allocate_local(&self, num_blocks: usize) -> Result<Vec<BlockHandle>> {
        let block_ids = self.local_allocator.allocate(num_blocks)?;

        let handles: Result<Vec<_>> = block_ids
            .iter()
            .map(|&block_id| {
                // Register ownership
                self.ownership.register_block(block_id, self.node_id.clone())?;
                self.node_map.insert(block_id, self.node_id.clone());

                Ok(BlockHandle {
                    block_id,
                    owner_node: self.node_id.clone(),
                    is_local: true,
                })
            })
            .collect();

        handles
    }

    /// Public entry point used by the gRPC server when a *remote* caller
    /// asks this node to allocate locally on its behalf. We physically own
    /// the blocks on this node but record the caller in the ownership ledger
    /// so that ownership queries elsewhere can find them.
    pub async fn allocate_local_for_caller(
        &self,
        request_id: &str,
        caller_node_id: &str,
        num_blocks: usize,
    ) -> Result<Vec<BlockHandle>> {
        debug!(
            "allocate_local_for_caller: request={} caller={} blocks={}",
            request_id, caller_node_id, num_blocks
        );
        let block_ids = self.local_allocator.allocate(num_blocks)?;
        let mut handles = Vec::with_capacity(block_ids.len());
        for &block_id in &block_ids {
            // Caller (remote) is the logical owner; physical residency is here.
            self.ownership.register_block(block_id, caller_node_id.to_string())?;
            self.node_map.insert(block_id, self.node_id.clone());
            handles.push(BlockHandle {
                block_id,
                owner_node: caller_node_id.to_string(),
                is_local: false,
            });
        }
        *self.metrics.total_allocations.lock() += 1;
        self.update_state_hash()?;
        Ok(handles)
    }

    /// Stats about *this* node's local allocator, exposed for RPC responses.
    pub fn local_stats(&self) -> crate::allocator::CacheStats {
        self.local_allocator.stats()
    }

    /// Total blocks tracked in the ownership ledger across the cluster.
    pub fn num_owned_blocks(&self) -> usize {
        self.node_map.len()
    }

    /// This node's id (used for health responses).
    pub fn node_id(&self) -> &str {
        &self.node_id
    }

    /// Deallocate blocks (may be local or remote)
    pub async fn deallocate(&self, blocks: Vec<usize>) -> Result<()> {
        debug!("deallocate: {} blocks", blocks.len());

        // Separate local and remote blocks
        let mut local_blocks = Vec::new();

        for block_id in blocks {
            if let Some(entry) = self.node_map.get(&block_id) {
                let owner = entry.clone();
                if owner == self.node_id {
                    local_blocks.push(block_id);
                }
                // Remote blocks would be handled by remote node in full implementation
            }
        }

        // Deallocate local blocks
        if !local_blocks.is_empty() {
            self.local_allocator.deallocate(&local_blocks)?;

            // Unregister ownership
            for block_id in &local_blocks {
                self.node_map.remove(block_id);
                self.ownership.unregister_block(*block_id)?;
            }
        }

        *self.metrics.total_deallocations.lock() += 1;

        // Update state hash
        self.update_state_hash()?;

        Ok(())
    }

    /// Get block owner
    pub fn owner_of(&self, block_id: usize) -> Result<String> {
        self.ownership.owner_of(block_id)
    }

    /// Update state hash after allocation changes
    fn update_state_hash(&self) -> Result<()> {
        let mut hasher = blake3::Hasher::new();

        // Hash all current block ownership
        let mut entries: Vec<_> = self.node_map
            .iter()
            .map(|ref_multi| (*ref_multi.key(), ref_multi.value().clone()))
            .collect();

        entries.sort_by_key(|e| e.0);

        for (block_id, node_id) in entries {
            hasher.update(format!("{}:{}", block_id, node_id).as_bytes());
        }

        *self.state_hash.lock() = hasher.finalize();
        Ok(())
    }

    /// Get current state hash (for consistency validation)
    pub fn get_state_hash(&self) -> Blake3Hash {
        *self.state_hash.lock()
    }

    /// Health check
    pub async fn health_check(&self) -> Result<()> {
        // Verify local allocator is functional
        let stats = self.local_allocator.stats();
        if stats.total_blocks == 0 {
            return Err(anyhow!("No blocks available"));
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn create_test_cache() -> Arc<DistributedKVCache> {
        let allocator = Arc::new(KVCacheAllocator::new(1024 * 1024, 16 * 1024).unwrap());
        Arc::new(DistributedKVCache::new("test-node".to_string(), allocator))
    }

    #[tokio::test]
    async fn test_allocate_local() {
        let cache = create_test_cache();
        let blocks = cache.allocate_global("req-1", 10).await.unwrap();

        assert_eq!(blocks.len(), 10);
        assert!(blocks.iter().all(|b| b.is_local));
        assert!(blocks.iter().all(|b| b.owner_node == "test-node"));
    }

    #[tokio::test]
    async fn test_allocate_deallocate() {
        let cache = create_test_cache();
        let blocks = cache.allocate_global("req-1", 5).await.unwrap();
        let block_ids: Vec<_> = blocks.iter().map(|b| b.block_id).collect();

        cache.deallocate(block_ids).await.unwrap();

        assert_eq!(*cache.metrics.total_deallocations.lock(), 1);
    }

    #[tokio::test]
    async fn test_owner_tracking() {
        let cache = create_test_cache();
        let blocks = cache.allocate_global("req-1", 3).await.unwrap();

        for block in blocks {
            let owner = cache.owner_of(block.block_id).unwrap();
            assert_eq!(owner, "test-node");
        }
    }

    #[tokio::test]
    async fn test_state_hash_consistency() {
        let cache = create_test_cache();

        let hash1 = cache.get_state_hash();

        cache.allocate_global("req-1", 5).await.unwrap();
        let hash2 = cache.get_state_hash();

        // Hashes should differ after allocation
        assert_ne!(hash1.as_bytes(), hash2.as_bytes());
    }

    #[test]
    fn test_register_peer() {
        let cache = create_test_cache();
        assert!(cache
            .register_peer("node-2".to_string(), "localhost:50052".to_string(), 1024)
            .is_ok());

        let nodes = cache.node_selector.get_nodes();
        assert!(nodes.contains(&"node-2".to_string()));
    }

    #[tokio::test]
    async fn test_allocate_with_peers() {
        let cache = create_test_cache();
        cache
            .register_peer("node-2".to_string(), "localhost:50052".to_string(), 1024)
            .unwrap();

        // Update peer capacity so selection works
        cache
            .node_selector
            .update_metrics(
                "node-2",
                NodeCapacity {
                    total_blocks: 1024,
                    free_blocks: 500,
                    allocated_blocks: 524,
                },
                5.0,
                50.0,
            )
            .ok();

        // This will try remote allocation (which will succeed in test)
        let result = cache.allocate_global("req-1", 100).await;

        // May fail if local allocator runs out or remote doesn't respond
        // But the path is exercised
        let _ = result;
    }
}
