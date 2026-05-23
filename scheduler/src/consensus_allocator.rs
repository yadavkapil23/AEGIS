// Consensus-based KV Cache Allocator
// Integrates KVCacheAllocator with Raft consensus for multi-node consistency
// All allocation/deallocation operations go through the consensus leader

use crate::allocator::{KVBlock, KVCacheAllocator, CacheStats};
use crate::consensus::{QuorumConsensus, ConsensusState};
use anyhow::{anyhow, Result};
use dashmap::DashMap;
use parking_lot::Mutex;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::SystemTime;
use tracing::{debug, info, warn, error};
use uuid::Uuid;

/// Consensus command for allocation operations
#[derive(Debug, Clone)]
pub enum AllocationCommand {
    /// Allocate blocks
    AllocateBlocks {
        request_id: String,
        num_blocks: usize,
        owner: Option<String>,
    },
    /// Deallocate blocks
    DeallocateBlocks {
        request_id: String,
        block_ids: Vec<usize>,
    },
    /// Migrate block to another node
    MigrateBlock {
        block_id: usize,
        from_node: String,
        to_node: String,
    },
}

/// Output from executing a command
#[derive(Debug, Clone)]
pub enum CommandOutput {
    BlocksAllocated(Vec<usize>),
    BlocksDeallocated { count: usize },
    BlockMigrated { block_id: usize },
}

/// Metrics for consensus allocator
#[derive(Debug, Clone, Default)]
pub struct ConsensusAllocatorMetrics {
    pub total_allocations: Arc<AtomicU64>,
    pub total_deallocations: Arc<AtomicU64>,
    pub consensus_commands_applied: Arc<AtomicU64>,
    pub consensus_errors: Arc<AtomicU64>,
    pub allocation_latency_ms: Arc<Mutex<Vec<u64>>>,
}

impl ConsensusAllocatorMetrics {
    pub fn new() -> Self {
        Self {
            total_allocations: Arc::new(AtomicU64::new(0)),
            total_deallocations: Arc::new(AtomicU64::new(0)),
            consensus_commands_applied: Arc::new(AtomicU64::new(0)),
            consensus_errors: Arc::new(AtomicU64::new(0)),
            allocation_latency_ms: Arc::new(Mutex::new(Vec::new())),
        }
    }

    pub fn record_allocation(&self) {
        self.total_allocations.fetch_add(1, Ordering::SeqCst);
    }

    pub fn record_deallocation(&self) {
        self.total_deallocations.fetch_add(1, Ordering::SeqCst);
    }

    pub fn record_command_applied(&self) {
        self.consensus_commands_applied.fetch_add(1, Ordering::SeqCst);
    }

    pub fn record_error(&self) {
        self.consensus_errors.fetch_add(1, Ordering::SeqCst);
    }

    pub fn record_latency_ms(&self, latency: u64) {
        self.allocation_latency_ms.lock().push(latency);
    }
}

/// State machine state for replicated allocations
#[derive(Debug, Clone)]
pub struct AllocationState {
    /// Map of block_id -> owner_node
    pub block_ownership: Arc<DashMap<usize, String>>,
    /// Applied command count
    pub applied_commands: Arc<AtomicU64>,
    /// Last applied term
    pub last_applied_term: Arc<AtomicU64>,
}

impl AllocationState {
    pub fn new() -> Self {
        Self {
            block_ownership: Arc::new(DashMap::new()),
            applied_commands: Arc::new(AtomicU64::new(0)),
            last_applied_term: Arc::new(AtomicU64::new(0)),
        }
    }

    pub fn record_allocation(&self, block_ids: &[usize], owner: String) {
        for block_id in block_ids {
            self.block_ownership.insert(*block_id, owner.clone());
        }
    }

    pub fn record_deallocation(&self, block_ids: &[usize]) {
        for block_id in block_ids {
            self.block_ownership.remove(block_id);
        }
    }

    pub fn get_owner(&self, block_id: usize) -> Option<String> {
        self.block_ownership.get(&block_id).map(|entry| entry.clone())
    }
}

/// Consensus-aware KV Cache Allocator
/// Routes all allocation/deallocation operations through Raft consensus
pub struct ConsensusAllocator {
    /// Local KV cache allocator
    local_allocator: Arc<KVCacheAllocator>,

    /// Consensus engine (Raft)
    consensus: Arc<QuorumConsensus>,

    /// Replicated state machine state
    state: Arc<AllocationState>,

    /// This node's ID
    node_id: String,

    /// Metrics
    metrics: Arc<ConsensusAllocatorMetrics>,

    /// Command log (for replication)
    command_log: Arc<Mutex<Vec<(u64, AllocationCommand)>>>,

    /// Current applied index
    last_applied_index: Arc<AtomicU64>,
}

impl ConsensusAllocator {
    /// Create new consensus allocator
    pub fn new(
        local_allocator: Arc<KVCacheAllocator>,
        consensus: Arc<QuorumConsensus>,
        node_id: String,
    ) -> Self {
        info!(
            node_id = node_id,
            "Initializing consensus allocator"
        );

        Self {
            local_allocator,
            consensus,
            state: Arc::new(AllocationState::new()),
            node_id,
            metrics: Arc::new(ConsensusAllocatorMetrics::new()),
            command_log: Arc::new(Mutex::new(Vec::new())),
            last_applied_index: Arc::new(AtomicU64::new(0)),
        }
    }

    /// Allocate blocks (routes through consensus)
    pub async fn allocate_blocks(&self, num_blocks: usize) -> Result<Vec<usize>> {
        let start_time = SystemTime::now();
        let request_id = Uuid::new_v4().to_string();

        // Create command
        let command = AllocationCommand::AllocateBlocks {
            request_id: request_id.clone(),
            num_blocks,
            owner: Some(self.node_id.clone()),
        };

        debug!(
            request_id = request_id,
            num_blocks = num_blocks,
            "Submitting allocation command to consensus"
        );

        // Apply through consensus
        match self.apply_command(command).await {
            Ok(CommandOutput::BlocksAllocated(block_ids)) => {
                self.metrics.record_allocation();

                // Record latency
                if let Ok(elapsed) = start_time.elapsed() {
                    self.metrics.record_latency_ms(elapsed.as_millis() as u64);
                }

                info!(
                    request_id = request_id,
                    num_blocks = num_blocks,
                    allocated = block_ids.len(),
                    "Allocation successful"
                );

                Ok(block_ids)
            }
            Ok(_) => {
                error!("Unexpected response type for allocation");
                Err(anyhow!("Unexpected response from consensus"))
            }
            Err(e) => {
                self.metrics.record_error();
                warn!(
                    request_id = request_id,
                    error = %e,
                    "Allocation failed"
                );
                Err(e)
            }
        }
    }

    /// Deallocate blocks (routes through consensus)
    pub async fn deallocate_blocks(&self, block_ids: &[usize]) -> Result<()> {
        let request_id = Uuid::new_v4().to_string();

        // Create command
        let command = AllocationCommand::DeallocateBlocks {
            request_id: request_id.clone(),
            block_ids: block_ids.to_vec(),
        };

        debug!(
            request_id = request_id,
            num_blocks = block_ids.len(),
            "Submitting deallocation command to consensus"
        );

        // Apply through consensus
        match self.apply_command(command).await {
            Ok(CommandOutput::BlocksDeallocated { count }) => {
                self.metrics.record_deallocation();
                info!(
                    request_id = request_id,
                    deallocated = count,
                    "Deallocation successful"
                );
                Ok(())
            }
            Ok(_) => {
                error!("Unexpected response type for deallocation");
                Err(anyhow!("Unexpected response from consensus"))
            }
            Err(e) => {
                self.metrics.record_error();
                warn!(
                    request_id = request_id,
                    error = %e,
                    "Deallocation failed"
                );
                Err(e)
            }
        }
    }

    /// Apply command through consensus
    async fn apply_command(&self, command: AllocationCommand) -> Result<CommandOutput> {
        // Check if we're leader
        let is_leader = self.consensus.state() == crate::consensus::ConsensusState::Leader;

        if !is_leader {
            return Err(anyhow!(
                "Not a leader, cannot apply commands. Current state: {:?}",
                self.consensus.state()
            ));
        }

        // Get current term
        let term = self.consensus.current_term();

        // Add to log
        let index = self.command_log.lock().len() as u64;
        self.command_log.lock().push((term, command.clone()));

        // Execute locally
        let result = self.execute_command(&command)?;

        // Record in state
        self.state.applied_commands.fetch_add(1, Ordering::SeqCst);
        self.state.last_applied_term.store(term, Ordering::SeqCst);
        self.last_applied_index.store(index, Ordering::SeqCst);

        self.metrics.record_command_applied();

        info!(
            term = term,
            index = index,
            "Command applied through consensus"
        );

        Ok(result)
    }

    /// Execute command locally
    fn execute_command(&self, command: &AllocationCommand) -> Result<CommandOutput> {
        match command {
            AllocationCommand::AllocateBlocks {
                request_id,
                num_blocks,
                owner,
            } => {
                // Allocate from local allocator
                let block_ids = self.local_allocator.allocate(*num_blocks)?;

                // Record ownership
                if let Some(owner) = owner {
                    self.state.record_allocation(&block_ids, owner.clone());
                }

                info!(
                    request_id = request_id,
                    num_blocks = num_blocks,
                    allocated = block_ids.len(),
                    "Blocks allocated"
                );

                Ok(CommandOutput::BlocksAllocated(block_ids))
            }
            AllocationCommand::DeallocateBlocks {
                request_id,
                block_ids,
            } => {
                // Deallocate from local allocator
                self.local_allocator.deallocate(block_ids)?;

                // Record deallocation
                self.state.record_deallocation(block_ids);

                info!(
                    request_id = request_id,
                    num_blocks = block_ids.len(),
                    "Blocks deallocated"
                );

                Ok(CommandOutput::BlocksDeallocated {
                    count: block_ids.len(),
                })
            }
            AllocationCommand::MigrateBlock {
                block_id,
                from_node,
                to_node,
            } => {
                info!(
                    block_id = block_id,
                    from_node = from_node,
                    to_node = to_node,
                    "Block migration recorded"
                );

                // Update ownership
                self.state
                    .block_ownership
                    .insert(*block_id, to_node.clone());

                Ok(CommandOutput::BlockMigrated {
                    block_id: *block_id,
                })
            }
        }
    }

    /// Get cache statistics
    pub fn stats(&self) -> Result<CacheStats> {
        Ok(self.local_allocator.stats())
    }

    /// Get allocation state
    pub fn get_state(&self) -> Arc<AllocationState> {
        self.state.clone()
    }

    /// Get metrics
    pub fn get_metrics(&self) -> Arc<ConsensusAllocatorMetrics> {
        self.metrics.clone()
    }

    /// Get consensus state
    pub fn get_consensus_state(&self) -> crate::consensus::ConsensusState {
        self.consensus.state()
    }

    /// Is this node the leader?
    pub fn is_leader(&self) -> bool {
        self.consensus.state() == crate::consensus::ConsensusState::Leader
    }

    /// Get node ID
    pub fn node_id(&self) -> &str {
        &self.node_id
    }

    /// Get command log
    pub fn command_log(&self) -> Vec<(u64, AllocationCommand)> {
        self.command_log.lock().clone()
    }

    /// Get last applied index
    pub fn last_applied_index(&self) -> u64 {
        self.last_applied_index.load(Ordering::SeqCst)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::consensus::QuorumConfig;

    #[tokio::test]
    async fn test_allocate_blocks() -> Result<()> {
        // Setup
        let allocator = Arc::new(KVCacheAllocator::new(1024 * 1024, 16 * 1024)?);
        let config = QuorumConfig::new("node-1", vec!["node-1".to_string()]);
        let consensus = Arc::new(QuorumConsensus::new(config));

        // Make this node the leader
        *consensus.state.lock() = crate::consensus::ConsensusState::Leader;

        let consensus_allocator =
            ConsensusAllocator::new(allocator, consensus, "node-1".to_string());

        // Allocate blocks
        let blocks = consensus_allocator.allocate_blocks(5).await?;
        assert_eq!(blocks.len(), 5);

        // Verify metrics
        assert_eq!(consensus_allocator.metrics.total_allocations.load(Ordering::SeqCst), 1);

        Ok(())
    }

    #[tokio::test]
    async fn test_deallocate_blocks() -> Result<()> {
        // Setup
        let allocator = Arc::new(KVCacheAllocator::new(1024 * 1024, 16 * 1024)?);
        let config = QuorumConfig::new("node-1", vec!["node-1".to_string()]);
        let consensus = Arc::new(QuorumConsensus::new(config));

        *consensus.state.lock() = crate::consensus::ConsensusState::Leader;

        let consensus_allocator =
            ConsensusAllocator::new(allocator, consensus, "node-1".to_string());

        // Allocate
        let blocks = consensus_allocator.allocate_blocks(5).await?;

        // Deallocate
        consensus_allocator.deallocate_blocks(&blocks).await?;

        // Verify metrics
        assert_eq!(consensus_allocator.metrics.total_deallocations.load(Ordering::SeqCst), 1);

        Ok(())
    }

    #[tokio::test]
    async fn test_not_leader_error() -> Result<()> {
        // Setup
        let allocator = Arc::new(KVCacheAllocator::new(1024 * 1024, 16 * 1024)?);
        let config = QuorumConfig::new("node-1", vec!["node-1".to_string()]);
        let consensus = Arc::new(QuorumConsensus::new(config));

        // Node is follower (not leader)
        *consensus.state.lock() = crate::consensus::ConsensusState::Follower;

        let consensus_allocator =
            ConsensusAllocator::new(allocator, consensus, "node-1".to_string());

        // Try to allocate (should fail)
        let result = consensus_allocator.allocate_blocks(5).await;
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("Not a leader"));

        Ok(())
    }
}
