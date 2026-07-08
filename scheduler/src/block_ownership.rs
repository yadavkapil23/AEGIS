// Block ownership tracking
// Maintains mapping of which node owns which KV cache blocks

use dashmap::DashMap;
use anyhow::{anyhow, Result};
use std::sync::Arc;
use std::time::Instant;
use tracing::{debug};

/// Tracks which node owns each cache block
pub struct BlockOwnership {
    // Block ID → Node ID mapping
    block_to_node: Arc<DashMap<usize, String>>,

    // Node ID → Vec<Block IDs> mapping
    node_to_blocks: Arc<DashMap<String, Vec<usize>>>,

    // When ownership was assigned (for migration decisions)
    ownership_timestamps: Arc<DashMap<usize, Instant>>,
}

impl BlockOwnership {
    pub fn new() -> Self {
        Self {
            block_to_node: Arc::new(DashMap::new()),
            node_to_blocks: Arc::new(DashMap::new()),
            ownership_timestamps: Arc::new(DashMap::new()),
        }
    }

    /// Register a block as owned by a node
    pub fn register_block(&self, block_id: usize, node_id: String) -> Result<()> {
        debug!("Registering block {} to node {}", block_id, node_id);

        // Add to block→node mapping
        self.block_to_node.insert(block_id, node_id.clone());

        // Add to node→blocks mapping
        self.node_to_blocks
            .entry(node_id.clone())
            .or_default()
            .push(block_id);

        // Record timestamp
        self.ownership_timestamps.insert(block_id, Instant::now());

        Ok(())
    }

    /// Unregister a block (e.g., when deallocated)
    pub fn unregister_block(&self, block_id: usize) -> Result<()> {
        debug!("Unregistering block {}", block_id);

        // Find owner first
        let owner = if let Some(entry) = self.block_to_node.get(&block_id) {
            entry.value().clone()
        } else {
            return Err(anyhow!("Block {} not registered", block_id));
        };

        // Remove from block→node mapping
        self.block_to_node.remove(&block_id);

        // Remove from node→blocks mapping
        if let Some(mut entry) = self.node_to_blocks.get_mut(&owner) {
            entry.retain(|&id| id != block_id);
        }

        // Remove timestamp
        self.ownership_timestamps.remove(&block_id);

        Ok(())
    }

    /// Get owner of a block
    pub fn owner_of(&self, block_id: usize) -> Result<String> {
        self.block_to_node
            .get(&block_id)
            .map(|entry| entry.value().clone())
            .ok_or_else(|| anyhow!("Block {} not owned by any node", block_id))
    }

    /// Get all blocks owned by a node
    pub fn blocks_owned_by(&self, node_id: &str) -> Vec<usize> {
        self.node_to_blocks
            .get(node_id)
            .map(|entry| entry.value().clone())
            .unwrap_or_default()
    }

    /// Migrate blocks from one node to another
    pub fn migrate_blocks(
        &self,
        from_node: String,
        to_node: String,
        blocks: Vec<usize>,
    ) -> Result<()> {
        for block_id in blocks {
            // Update block→node mapping
            self.block_to_node.insert(block_id, to_node.clone());

            // Update node→blocks mappings
            if let Some(mut entry) = self.node_to_blocks.get_mut(&from_node) {
                entry.retain(|&id| id != block_id);
            }

            self.node_to_blocks
                .entry(to_node.clone())
                .or_default()
                .push(block_id);
        }

        Ok(())
    }

    /// Get all nodes that own at least one block
    pub fn all_owner_nodes(&self) -> Vec<String> {
        self.node_to_blocks
            .iter()
            .map(|entry| entry.key().clone())
            .collect()
    }

    /// Get all registered blocks
    pub fn all_blocks(&self) -> Vec<usize> {
        self.block_to_node
            .iter()
            .map(|entry| *entry.key())
            .collect()
    }

    /// Check if block is owned by a specific node
    pub fn is_owned_by(&self, block_id: usize, node_id: &str) -> bool {
        self.block_to_node
            .get(&block_id)
            .map(|entry| entry.value() == node_id)
            .unwrap_or(false)
    }

    /// Total number of registered blocks
    pub fn total_blocks(&self) -> usize {
        self.block_to_node.len()
    }
}

impl Default for BlockOwnership {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_register_block() {
        let ownership = BlockOwnership::new();
        ownership
            .register_block(1, "node-1".to_string())
            .unwrap();

        assert_eq!(ownership.owner_of(1).unwrap(), "node-1");
    }

    #[test]
    fn test_blocks_owned_by() {
        let ownership = BlockOwnership::new();
        ownership
            .register_block(1, "node-1".to_string())
            .unwrap();
        ownership
            .register_block(2, "node-1".to_string())
            .unwrap();
        ownership
            .register_block(3, "node-2".to_string())
            .unwrap();

        let blocks = ownership.blocks_owned_by("node-1");
        assert_eq!(blocks.len(), 2);
    }

    #[test]
    fn test_migrate_blocks() {
        let ownership = BlockOwnership::new();
        ownership
            .register_block(1, "node-1".to_string())
            .unwrap();
        ownership
            .register_block(2, "node-1".to_string())
            .unwrap();

        ownership
            .migrate_blocks("node-1".to_string(), "node-2".to_string(), vec![1, 2])
            .unwrap();

        assert_eq!(ownership.owner_of(1).unwrap(), "node-2");
        assert_eq!(ownership.blocks_owned_by("node-1").len(), 0);
    }
}
