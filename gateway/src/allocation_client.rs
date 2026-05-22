// gRPC client for Allocation Service
// Handles communication with scheduler nodes

use std::sync::Arc;
use tonic::transport::Channel;
use anyhow::Result;
use tracing::{debug, info, warn, error};
use std::sync::atomic::{AtomicUsize, Ordering};

// Proto-generated code (would be imported from tonic)
// For now, we define the interface

/// Allocation client for communicating with scheduler
pub struct AllocationClient {
    nodes: Vec<String>,
    current_node: Arc<AtomicUsize>,
    channels: Arc<Vec<Channel>>,
}

impl AllocationClient {
    /// Create new allocation client
    pub async fn new(nodes: Vec<String>) -> Result<Self> {
        debug!("Initializing allocation client with nodes: {:?}", nodes);

        if nodes.is_empty() {
            return Err(anyhow::anyhow!("No scheduler nodes configured"));
        }

        // Create channels to each node
        let mut channels = Vec::new();
        for node in &nodes {
            match Channel::from_shared(node.clone())
                .and_then(|ch| Ok(ch.connect_lazy()))
            {
                Ok(ch) => {
                    channels.push(ch);
                    info!("Connected to scheduler node: {}", node);
                }
                Err(e) => {
                    warn!("Failed to connect to node {}: {}", node, e);
                }
            }
        }

        if channels.is_empty() {
            return Err(anyhow::anyhow!("Failed to connect to any scheduler node"));
        }

        Ok(Self {
            nodes,
            current_node: Arc::new(AtomicUsize::new(0)),
            channels: Arc::new(channels),
        })
    }

    /// Get next node (round-robin load balancing)
    fn get_next_node(&self) -> usize {
        let current = self.current_node.load(Ordering::SeqCst);
        let next = (current + 1) % self.channels.len();
        self.current_node.store(next, Ordering::SeqCst);
        next
    }

    /// Allocate blocks
    pub async fn allocate_blocks(
        &self,
        request_id: String,
        num_blocks: u32,
        owner: Option<String>,
    ) -> Result<(Vec<u64>, u32, String)> {
        debug!(
            request_id = request_id,
            num_blocks = num_blocks,
            "Allocating blocks via gRPC client"
        );

        let node_idx = self.get_next_node();
        let node_addr = self.nodes[node_idx].clone();

        // In production, this would make actual gRPC calls
        // For now, return a simulated response
        info!(
            request_id = request_id,
            node = node_addr,
            "Allocation request sent"
        );

        // Simulate allocation
        let block_ids: Vec<u64> = (1..=num_blocks as u64).collect();
        Ok((block_ids, 10, node_addr))
    }

    /// Deallocate blocks
    pub async fn deallocate_blocks(
        &self,
        request_id: String,
        block_ids: Vec<u64>,
    ) -> Result<(u32, u32, String)> {
        debug!(
            request_id = request_id,
            num_blocks = block_ids.len(),
            "Deallocating blocks via gRPC client"
        );

        let node_idx = self.get_next_node();
        let node_addr = self.nodes[node_idx].clone();

        info!(
            request_id = request_id,
            node = node_addr,
            "Deallocation request sent"
        );

        // Simulate deallocation
        Ok((block_ids.len() as u32, 10, node_addr))
    }

    /// Get cache statistics
    pub async fn get_stats(&self) -> Result<(u64, u64, u32)> {
        let node_idx = self.current_node.load(Ordering::SeqCst);
        let node_addr = self.nodes.get(node_idx).cloned().unwrap_or_default();

        debug!("Getting stats from node: {}", node_addr);

        // Simulate stats
        Ok((1000, 750, 75))
    }

    /// Get cluster health
    pub async fn get_cluster_health(&self) -> Result<(bool, u32, u32, String)> {
        debug!("Getting cluster health");

        // Simulate health check
        Ok((true, 3, 3, "node-1".to_string()))
    }

    /// Number of configured nodes
    pub fn node_count(&self) -> usize {
        self.nodes.len()
    }

    /// Get connected nodes
    pub fn get_nodes(&self) -> &[String] {
        &self.nodes
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_allocation_client_creation() {
        let nodes = vec!["http://localhost:50052".to_string()];
        let client = AllocationClient::new(nodes).await;
        assert!(client.is_ok());
    }

    #[tokio::test]
    async fn test_round_robin_load_balancing() {
        let nodes = vec![
            "http://node1:50052".to_string(),
            "http://node2:50052".to_string(),
            "http://node3:50052".to_string(),
        ];
        let client = AllocationClient::new(nodes).await.unwrap();

        let node1 = client.get_next_node();
        let node2 = client.get_next_node();
        let node3 = client.get_next_node();
        let node1_again = client.get_next_node();

        assert_eq!(node1, 0);
        assert_eq!(node2, 1);
        assert_eq!(node3, 2);
        assert_eq!(node1_again, 0);
    }
}
