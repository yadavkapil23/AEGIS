// Integration tests for Consensus Allocator
// Tests multi-node allocation scenarios

#[cfg(test)]
mod tests {
    use aegis_scheduler::{
        KVCacheAllocator, ConsensusAllocator, AllocationCommand, CommandOutput,
    };
    use aegis_scheduler::consensus::{QuorumConsensus, QuorumConfig, ConsensusState};
    use std::sync::Arc;
    use anyhow::Result;

    /// Helper: Create a 3-node cluster setup
    struct TestCluster {
        node1: Arc<ConsensusAllocator>,
        node2: Arc<ConsensusAllocator>,
        node3: Arc<ConsensusAllocator>,
        consensus1: Arc<QuorumConsensus>,
        consensus2: Arc<QuorumConsensus>,
        consensus3: Arc<QuorumConsensus>,
    }

    impl TestCluster {
        async fn new() -> Result<Self> {
            // Create allocators
            let allocator1 = Arc::new(KVCacheAllocator::new(100 * 1024 * 1024, 16 * 1024)?);
            let allocator2 = Arc::new(KVCacheAllocator::new(100 * 1024 * 1024, 16 * 1024)?);
            let allocator3 = Arc::new(KVCacheAllocator::new(100 * 1024 * 1024, 16 * 1024)?);

            // Create consensus engines
            let config = QuorumConfig::new(
                "node-1",
                vec!["node-1".to_string(), "node-2".to_string(), "node-3".to_string()],
            );

            let consensus1 = Arc::new(QuorumConsensus::new(config.clone()));
            let consensus2 = Arc::new(QuorumConsensus::new(QuorumConfig::new(
                "node-2",
                vec!["node-1".to_string(), "node-2".to_string(), "node-3".to_string()],
            )));
            let consensus3 = Arc::new(QuorumConsensus::new(QuorumConfig::new(
                "node-3",
                vec!["node-1".to_string(), "node-2".to_string(), "node-3".to_string()],
            )));

            // Create consensus allocators
            let node1 = Arc::new(ConsensusAllocator::new(
                allocator1,
                consensus1.clone(),
                "node-1".to_string(),
            ));

            let node2 = Arc::new(ConsensusAllocator::new(
                allocator2,
                consensus2.clone(),
                "node-2".to_string(),
            ));

            let node3 = Arc::new(ConsensusAllocator::new(
                allocator3,
                consensus3.clone(),
                "node-3".to_string(),
            ));

            Ok(Self {
                node1,
                node2,
                node3,
                consensus1,
                consensus2,
                consensus3,
            })
        }

        fn elect_leader(&self) {
            *self.consensus1.state.lock() = ConsensusState::Leader;
            *self.consensus2.state.lock() = ConsensusState::Follower;
            *self.consensus3.state.lock() = ConsensusState::Follower;
        }

        fn make_all_followers(&self) {
            *self.consensus1.state.lock() = ConsensusState::Follower;
            *self.consensus2.state.lock() = ConsensusState::Follower;
            *self.consensus3.state.lock() = ConsensusState::Follower;
        }
    }

    #[tokio::test]
    async fn test_single_node_allocation() -> Result<()> {
        let allocator = Arc::new(KVCacheAllocator::new(10 * 1024 * 1024, 16 * 1024)?);
        let config = QuorumConfig::new("node-1", vec!["node-1".to_string()]);
        let consensus = Arc::new(QuorumConsensus::new(config));

        *consensus.state.lock() = ConsensusState::Leader;

        let consensus_allocator = ConsensusAllocator::new(allocator, consensus, "node-1".to_string());

        // Allocate 5 blocks
        let blocks = consensus_allocator.allocate_blocks(5).await?;
        assert_eq!(blocks.len(), 5);
        assert!(blocks.iter().all(|b| *b >= 0));

        // Verify metrics
        assert_eq!(consensus_allocator.metrics.total_allocations.load(std::sync::atomic::Ordering::SeqCst), 1);

        println!("✅ Single node allocation: {} blocks allocated", blocks.len());
        Ok(())
    }

    #[tokio::test]
    async fn test_single_node_deallocation() -> Result<()> {
        let allocator = Arc::new(KVCacheAllocator::new(10 * 1024 * 1024, 16 * 1024)?);
        let config = QuorumConfig::new("node-1", vec!["node-1".to_string()]);
        let consensus = Arc::new(QuorumConsensus::new(config));

        *consensus.state.lock() = ConsensusState::Leader;

        let consensus_allocator = ConsensusAllocator::new(allocator, consensus, "node-1".to_string());

        // Allocate
        let blocks = consensus_allocator.allocate_blocks(10).await?;
        assert_eq!(blocks.len(), 10);

        // Deallocate
        consensus_allocator.deallocate_blocks(&blocks).await?;

        // Verify metrics
        assert_eq!(consensus_allocator.metrics.total_deallocations.load(std::sync::atomic::Ordering::SeqCst), 1);

        println!("✅ Single node deallocation: {} blocks deallocated", blocks.len());
        Ok(())
    }

    #[tokio::test]
    async fn test_multiple_allocations() -> Result<()> {
        let allocator = Arc::new(KVCacheAllocator::new(50 * 1024 * 1024, 16 * 1024)?);
        let config = QuorumConfig::new("node-1", vec!["node-1".to_string()]);
        let consensus = Arc::new(QuorumConsensus::new(config));

        *consensus.state.lock() = ConsensusState::Leader;

        let consensus_allocator = ConsensusAllocator::new(allocator, consensus, "node-1".to_string());

        // Multiple allocations
        let blocks1 = consensus_allocator.allocate_blocks(5).await?;
        let blocks2 = consensus_allocator.allocate_blocks(10).await?;
        let blocks3 = consensus_allocator.allocate_blocks(3).await?;

        assert_eq!(blocks1.len(), 5);
        assert_eq!(blocks2.len(), 10);
        assert_eq!(blocks3.len(), 3);

        // Verify metrics
        assert_eq!(consensus_allocator.metrics.total_allocations.load(std::sync::atomic::Ordering::SeqCst), 3);

        println!("✅ Multiple allocations: {} + {} + {} = {} total blocks", blocks1.len(), blocks2.len(), blocks3.len(), blocks1.len() + blocks2.len() + blocks3.len());
        Ok(())
    }

    #[tokio::test]
    async fn test_not_leader_error() -> Result<()> {
        let allocator = Arc::new(KVCacheAllocator::new(10 * 1024 * 1024, 16 * 1024)?);
        let config = QuorumConfig::new("node-1", vec!["node-1".to_string()]);
        let consensus = Arc::new(QuorumConsensus::new(config));

        // Node is FOLLOWER (not leader)
        *consensus.state.lock() = ConsensusState::Follower;

        let consensus_allocator = ConsensusAllocator::new(allocator, consensus, "node-1".to_string());

        // Allocation should fail
        let result = consensus_allocator.allocate_blocks(5).await;
        assert!(result.is_err());

        let err_msg = result.unwrap_err().to_string();
        assert!(err_msg.contains("Not a leader"));

        println!("✅ Not leader error properly returned");
        Ok(())
    }

    #[tokio::test]
    async fn test_cluster_allocation() -> Result<()> {
        let cluster = TestCluster::new().await?;
        cluster.elect_leader();

        // Allocate through leader
        let blocks = cluster.node1.allocate_blocks(5).await?;
        assert_eq!(blocks.len(), 5);

        // Verify leader recorded it
        assert_eq!(
            cluster.node1.metrics.total_allocations.load(std::sync::atomic::Ordering::SeqCst),
            1
        );

        println!("✅ Cluster allocation: {} blocks allocated via leader", blocks.len());
        Ok(())
    }

    #[tokio::test]
    async fn test_insufficient_blocks() -> Result<()> {
        let allocator = Arc::new(KVCacheAllocator::new(100 * 1024, 16 * 1024)?); // Small cache
        let config = QuorumConfig::new("node-1", vec!["node-1".to_string()]);
        let consensus = Arc::new(QuorumConsensus::new(config));

        *consensus.state.lock() = ConsensusState::Leader;

        let consensus_allocator = ConsensusAllocator::new(allocator, consensus, "node-1".to_string());

        // Try to allocate more than available
        let result = consensus_allocator.allocate_blocks(100).await;
        assert!(result.is_err());

        println!("✅ Insufficient blocks error properly returned");
        Ok(())
    }

    #[tokio::test]
    async fn test_stats_reporting() -> Result<()> {
        let allocator = Arc::new(KVCacheAllocator::new(10 * 1024 * 1024, 16 * 1024)?);
        let config = QuorumConfig::new("node-1", vec!["node-1".to_string()]);
        let consensus = Arc::new(QuorumConsensus::new(config));

        *consensus.state.lock() = ConsensusState::Leader;

        let consensus_allocator = ConsensusAllocator::new(allocator, consensus, "node-1".to_string());

        // Allocate some blocks
        let _blocks = consensus_allocator.allocate_blocks(10).await?;

        // Get stats
        let stats = consensus_allocator.stats()?;
        assert!(stats.allocated_blocks > 0);
        assert!(stats.total_blocks > 0);

        println!("✅ Cache stats: {} total blocks, {} allocated", stats.total_blocks, stats.allocated_blocks);
        Ok(())
    }

    #[tokio::test]
    async fn test_command_log() -> Result<()> {
        let allocator = Arc::new(KVCacheAllocator::new(10 * 1024 * 1024, 16 * 1024)?);
        let config = QuorumConfig::new("node-1", vec!["node-1".to_string()]);
        let consensus = Arc::new(QuorumConsensus::new(config));

        *consensus.state.lock() = ConsensusState::Leader;

        let consensus_allocator = ConsensusAllocator::new(allocator, consensus, "node-1".to_string());

        // Issue commands
        let _blocks1 = consensus_allocator.allocate_blocks(5).await?;
        let _blocks2 = consensus_allocator.allocate_blocks(3).await?;

        // Check command log
        let log = consensus_allocator.command_log();
        assert_eq!(log.len(), 2);

        println!("✅ Command log: {} commands recorded", log.len());
        Ok(())
    }

    #[tokio::test]
    async fn test_allocate_deallocate_cycle() -> Result<()> {
        let allocator = Arc::new(KVCacheAllocator::new(10 * 1024 * 1024, 16 * 1024)?);
        let config = QuorumConfig::new("node-1", vec!["node-1".to_string()]);
        let consensus = Arc::new(QuorumConsensus::new(config));

        *consensus.state.lock() = ConsensusState::Leader;

        let consensus_allocator = ConsensusAllocator::new(allocator, consensus, "node-1".to_string());

        // Allocate
        let blocks1 = consensus_allocator.allocate_blocks(5).await?;
        let blocks2 = consensus_allocator.allocate_blocks(3).await?;

        // Deallocate first batch
        consensus_allocator.deallocate_blocks(&blocks1).await?;

        // Allocate again (reusing freed blocks)
        let blocks3 = consensus_allocator.allocate_blocks(5).await?;
        assert_eq!(blocks3.len(), 5);

        // Deallocate remaining
        consensus_allocator.deallocate_blocks(&blocks2).await?;
        consensus_allocator.deallocate_blocks(&blocks3).await?;

        // Verify metrics
        assert_eq!(consensus_allocator.metrics.total_allocations.load(std::sync::atomic::Ordering::SeqCst), 3);
        assert_eq!(consensus_allocator.metrics.total_deallocations.load(std::sync::atomic::Ordering::SeqCst), 3);

        println!("✅ Allocate/deallocate cycle: 3 allocations, 3 deallocations");
        Ok(())
    }

    #[tokio::test]
    async fn test_leader_state_query() -> Result<()> {
        let allocator = Arc::new(KVCacheAllocator::new(10 * 1024 * 1024, 16 * 1024)?);
        let config = QuorumConfig::new("node-1", vec!["node-1".to_string()]);
        let consensus = Arc::new(QuorumConsensus::new(config));

        *consensus.state.lock() = ConsensusState::Leader;

        let consensus_allocator = ConsensusAllocator::new(allocator, consensus, "node-1".to_string());

        // Check leader status
        assert!(consensus_allocator.is_leader());
        assert_eq!(consensus_allocator.node_id(), "node-1");

        println!("✅ Leader state: is_leader={}, node_id={}", consensus_allocator.is_leader(), consensus_allocator.node_id());
        Ok(())
    }
}
