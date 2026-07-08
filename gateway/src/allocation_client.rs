use std::sync::Arc;
use tonic::transport::Channel;
use anyhow::Result;
use tracing::{debug, info, warn, error};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

// Import generated proto code
pub mod proto {
    tonic::include_proto!("scheduler.allocation");
}

use proto::{
    allocation_service_client::AllocationServiceClient,
    AllocateRequest, DeallocateRequest, StatsRequest, HealthRequest, MigrateRequest,
};

/// Per-node circuit breaker state.
struct NodeHealth {
    healthy: bool,
    consecutive_failures: u32,
}

/// Allocation client for communicating with the scheduler cluster via gRPC.
pub struct AllocationClient {
    nodes: Vec<String>,
    current_node: Arc<AtomicUsize>,
    clients: Vec<AllocationServiceClient<Channel>>,
    health: Vec<parking_lot::Mutex<NodeHealth>>,
}

impl AllocationClient {
    pub async fn new(nodes: Vec<String>) -> Result<Self> {
        if nodes.is_empty() {
            return Err(anyhow::anyhow!("No scheduler nodes configured"));
        }

        let mut clients = Vec::new();
        let mut health = Vec::new();

        for node in &nodes {
            match Channel::from_shared(node.clone())
                .map(|ch| ch.connect_timeout(Duration::from_secs(5)))
                .and_then(|ch| Ok(ch.connect_lazy()))
            {
                Ok(channel) => {
                    let client = AllocationServiceClient::new(channel);
                    clients.push(client);
                    health.push(parking_lot::Mutex::new(NodeHealth {
                        healthy: true,
                        consecutive_failures: 0,
                    }));
                    info!("Prepared gRPC channel to scheduler node: {}", node);
                }
                Err(e) => {
                    warn!("Failed to create channel for {}: {}", node, e);
                }
            }
        }

        if clients.is_empty() {
            return Err(anyhow::anyhow!("Failed to connect to any scheduler node"));
        }

        Ok(Self {
            nodes,
            current_node: Arc::new(AtomicUsize::new(0)),
            clients,
            health,
        })
    }

    fn next_node(&self) -> usize {
        let cur = self.current_node.load(Ordering::SeqCst);
        let next = (cur + 1) % self.clients.len();
        self.current_node.store(next, Ordering::SeqCst);
        cur
    }

    fn find_healthy_node(&self) -> Option<usize> {
        let start = self.current_node.load(Ordering::SeqCst);
        for offset in 0..self.clients.len() {
            let idx = (start + offset) % self.clients.len();
            if self.health[idx].lock().healthy {
                return Some(idx);
            }
        }
        None
    }

    fn mark_failure(&self, idx: usize) {
        let mut h = self.health[idx].lock();
        h.consecutive_failures += 1;
        if h.consecutive_failures >= 3 {
            h.healthy = false;
            warn!(node = %self.nodes[idx], "Scheduler node marked unhealthy");
        }
    }

    fn mark_success(&self, idx: usize) {
        let mut h = self.health[idx].lock();
        h.consecutive_failures = 0;
        h.healthy = true;
    }

    // ── Public API ──────────────────────────────────────────

    pub async fn allocate_blocks(
        &self,
        request_id: String,
        num_blocks: u32,
        owner: Option<String>,
    ) -> Result<(Vec<u64>, u32, String)> {
        let idx = self.find_healthy_node()
            .ok_or_else(|| anyhow::anyhow!("No healthy scheduler nodes"))?;

        let req = AllocateRequest {
            request_id,
            num_blocks,
            owner: owner.unwrap_or_default(),
            priority: 5,
        };

        let start = std::time::Instant::now();
        match self.clients[idx].clone().allocate_blocks(req).await {
            Ok(resp) => {
                let r = resp.into_inner();
                self.mark_success(idx);
                if r.success {
                    let latency_ms = start.elapsed().as_millis() as u32;
                    debug!(
                        node = %self.nodes[idx],
                        blocks = r.block_ids.len(),
                        latency_ms = latency_ms,
                        "Blocks allocated"
                    );
                    Ok((r.block_ids, latency_ms, self.nodes[idx].clone()))
                } else {
                    Err(anyhow::anyhow!("Allocation rejected: {}", r.error))
                }
            }
            Err(e) => {
                self.mark_failure(idx);
                error!(node = %self.nodes[idx], error = %e, "gRPC allocate failed");
                Err(anyhow::anyhow!("gRPC error: {}", e))
            }
        }
    }

    pub async fn deallocate_blocks(
        &self,
        request_id: String,
        block_ids: Vec<u64>,
    ) -> Result<(u32, u32, String)> {
        let idx = self.find_healthy_node()
            .ok_or_else(|| anyhow::anyhow!("No healthy scheduler nodes"))?;

        let req = DeallocateRequest { request_id, block_ids };

        let start = std::time::Instant::now();
        match self.clients[idx].clone().deallocate_blocks(req).await {
            Ok(resp) => {
                let r = resp.into_inner();
                self.mark_success(idx);
                let latency_ms = start.elapsed().as_millis() as u32;
                if r.success {
                    Ok((r.count, latency_ms, self.nodes[idx].clone()))
                } else {
                    Err(anyhow::anyhow!("Deallocation rejected: {}", r.error))
                }
            }
            Err(e) => {
                self.mark_failure(idx);
                error!(node = %self.nodes[idx], error = %e, "gRPC deallocate failed");
                Err(anyhow::anyhow!("gRPC error: {}", e))
            }
        }
    }

    pub async fn get_stats(&self) -> Result<(u64, u64, u32)> {
        let idx = self.find_healthy_node()
            .ok_or_else(|| anyhow::anyhow!("No healthy scheduler nodes"))?;

        match self.clients[idx].clone().get_stats(StatsRequest {}).await {
            Ok(resp) => {
                let r = resp.into_inner();
                self.mark_success(idx);
                Ok((r.total_blocks, r.allocated_blocks, r.free_blocks as u32))
            }
            Err(e) => {
                self.mark_failure(idx);
                Err(anyhow::anyhow!("gRPC error: {}", e))
            }
        }
    }

    pub async fn get_cluster_health(&self) -> Result<(bool, u32, u32, String)> {
        let idx = self.find_healthy_node()
            .ok_or_else(|| anyhow::anyhow!("No healthy scheduler nodes"))?;

        match self.clients[idx].clone().get_cluster_health(HealthRequest {}).await {
            Ok(resp) => {
                let r = resp.into_inner();
                self.mark_success(idx);
                Ok((r.healthy, r.total_nodes, r.healthy_nodes, r.leader_id))
            }
            Err(e) => {
                self.mark_failure(idx);
                Err(anyhow::anyhow!("gRPC error: {}", e))
            }
        }
    }

    pub async fn migrate_block(
        &self,
        block_id: u64,
        from_node: &str,
        to_node: &str,
    ) -> Result<()> {
        let idx = self.find_healthy_node()
            .ok_or_else(|| anyhow::anyhow!("No healthy scheduler nodes"))?;

        let req = MigrateRequest {
            block_id,
            from_node: from_node.to_string(),
            to_node: to_node.to_string(),
        };

        match self.clients[idx].clone().migrate_block(req).await {
            Ok(resp) => {
                let r = resp.into_inner();
                self.mark_success(idx);
                if r.success {
                    Ok(())
                } else {
                    Err(anyhow::anyhow!("Migration rejected: {}", r.error))
                }
            }
            Err(e) => {
                self.mark_failure(idx);
                Err(anyhow::anyhow!("gRPC error: {}", e))
            }
        }
    }

    pub fn node_count(&self) -> usize {
        self.nodes.len()
    }

    pub fn get_nodes(&self) -> &[String] {
        &self.nodes
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_creation() {
        let result = AllocationClient::new(vec!["http://localhost:50052".into()]).await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn test_empty_nodes_fails() {
        let result = AllocationClient::new(vec![]).await;
        assert!(result.is_err());
    }
}
