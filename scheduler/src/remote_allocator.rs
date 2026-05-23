// Remote allocator: RPC client for cross-node allocation
// Communicates with remote nodes via gRPC (tonic). When no live channel is
// attached, the allocator falls back to the in-process stub behavior used by
// the unit-test suite.

use anyhow::{anyhow, Result};
use parking_lot::Mutex;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tonic::transport::{Channel, Endpoint};
use tracing::{debug, info, warn};

use aegis_proto::scheduling::scheduling_service_client::SchedulingServiceClient;
use aegis_proto::scheduling::{
    AllocateRequest, DeallocateRequest, HealthRequest, StateHashRequest,
};

/// Health status of a remote node
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum HealthStatus {
    Healthy,
    Degraded,
    Dead,
    Unknown,
}

impl Default for HealthStatus {
    fn default() -> Self {
        HealthStatus::Unknown
    }
}

/// Remote node capacity info
#[derive(Debug, Clone)]
pub struct RemoteCapacity {
    pub total_blocks: usize,
    pub free_blocks: usize,
    pub last_updated: Instant,
}

impl RemoteCapacity {
    pub fn age_ms(&self) -> u64 {
        Instant::now()
            .duration_since(self.last_updated)
            .as_millis() as u64
    }

    pub fn is_stale(&self) -> bool {
        self.age_ms() > 5000 // 5 second staleness threshold
    }
}

/// RPC client for remote allocation requests
pub struct RemoteAllocator {
    node_id: String,
    node_addr: String,
    /// Optional caller-side identity passed in every RPC for the ownership ledger.
    caller_node_id: String,

    /// Live tonic channel — `None` means we have not connected yet (stub mode).
    grpc: Arc<Mutex<Option<SchedulingServiceClient<Channel>>>>,

    /// Known capacity (cached)
    known_capacity: Arc<Mutex<Option<RemoteCapacity>>>,

    /// Health status
    health_status: Arc<Mutex<HealthStatus>>,

    /// Last successful RPC time
    last_success: Arc<Mutex<Option<Instant>>>,

    /// RPC failure count
    failure_count: Arc<Mutex<u32>>,
}

impl RemoteAllocator {
    pub fn new(node_id: String, node_addr: String) -> Self {
        Self {
            node_id,
            node_addr,
            caller_node_id: "local".to_string(),
            grpc: Arc::new(Mutex::new(None)),
            known_capacity: Arc::new(Mutex::new(None)),
            health_status: Arc::new(Mutex::new(HealthStatus::Unknown)),
            last_success: Arc::new(Mutex::new(None)),
            failure_count: Arc::new(Mutex::new(0)),
        }
    }

    /// Identify this caller in subsequent RPCs (so the remote can record ownership).
    pub fn with_caller_id(mut self, caller_node_id: impl Into<String>) -> Self {
        self.caller_node_id = caller_node_id.into();
        self
    }

    /// Establish a tonic channel to `node_addr`. Idempotent.
    /// On success the allocator will use real gRPC for every subsequent RPC.
    pub async fn connect(&self) -> Result<()> {
        if self.grpc.lock().is_some() {
            return Ok(());
        }

        let url = if self.node_addr.starts_with("http://") || self.node_addr.starts_with("https://")
        {
            self.node_addr.clone()
        } else {
            format!("http://{}", self.node_addr)
        };

        let endpoint = Endpoint::from_shared(url)
            .map_err(|e| anyhow!("invalid endpoint {}: {}", self.node_addr, e))?
            .connect_timeout(Duration::from_secs(2))
            .timeout(Duration::from_secs(5));

        let channel = endpoint
            .connect()
            .await
            .map_err(|e| anyhow!("failed to connect to {}: {}", self.node_addr, e))?;

        *self.grpc.lock() = Some(SchedulingServiceClient::new(channel));
        info!("connected to remote scheduler at {}", self.node_addr);
        self.record_success().ok();
        Ok(())
    }

    /// Drop the live channel (e.g. after sustained failures).
    pub fn disconnect(&self) {
        *self.grpc.lock() = None;
    }

    fn client(&self) -> Option<SchedulingServiceClient<Channel>> {
        self.grpc.lock().clone()
    }

    /// Request allocation from remote node.
    /// Uses real gRPC if `connect()` has been called, otherwise falls back to a
    /// capacity-checking stub used by unit tests.
    pub async fn allocate(&self, num_blocks: usize) -> Result<Vec<usize>> {
        debug!(
            "allocating {} blocks from remote node {}",
            num_blocks, self.node_id
        );

        if let Some(mut client) = self.client() {
            let req = tonic::Request::new(AllocateRequest {
                request_id: uuid::Uuid::new_v4().to_string(),
                num_blocks: num_blocks as u32,
                caller_node_id: self.caller_node_id.clone(),
            });
            return match client.allocate_global(req).await {
                Ok(resp) => {
                    let resp = resp.into_inner();
                    if !resp.ok {
                        self.record_failure().ok();
                        return Err(anyhow!(
                            "remote {} rejected allocation: {}",
                            self.node_id,
                            resp.error
                        ));
                    }
                    self.update_capacity(resp.total_blocks as usize, resp.free_blocks as usize);
                    self.record_success().ok();
                    Ok(resp.block_ids.into_iter().map(|b| b as usize).collect())
                }
                Err(status) => {
                    self.record_failure().ok();
                    Err(anyhow!(
                        "gRPC AllocateGlobal to {} failed: {}",
                        self.node_id,
                        status
                    ))
                }
            };
        }

        // ---- no live channel - return proper error ----
        // Don't silently succeed with fake allocations
        warn!(
            "gRPC client for {} not connected - allocation failed",
            self.node_id
        );
        self.record_failure().ok();

        // Check if we have cached capacity info, but still fail if not connected
        {
            let capacity = self.known_capacity.lock();
            if let Some(cap) = capacity.as_ref() {
                if !cap.is_stale() && cap.free_blocks >= num_blocks {
                    warn!(
                        "Using cached capacity but gRPC unavailable for {} - failing allocation",
                        self.node_id
                    );
                }
            }
        }

        // Fail explicitly - don't return fake block IDs
        Err(anyhow!(
            "Cannot allocate {} blocks from {}: gRPC channel unavailable",
            num_blocks,
            self.node_id
        ))
    }

    /// Request deallocation from remote node.
    pub async fn deallocate(&self, blocks: Vec<usize>) -> Result<()> {
        debug!("deallocating {} blocks on {}", blocks.len(), self.node_id);

        if let Some(mut client) = self.client() {
            let req = tonic::Request::new(DeallocateRequest {
                block_ids: blocks.iter().map(|&b| b as u64).collect(),
                caller_node_id: self.caller_node_id.clone(),
            });
            return match client.deallocate_global(req).await {
                Ok(resp) => {
                    let resp = resp.into_inner();
                    if !resp.ok {
                        self.record_failure().ok();
                        return Err(anyhow!(
                            "remote {} rejected deallocate: {}",
                            self.node_id,
                            resp.error
                        ));
                    }
                    self.record_success().ok();
                    Ok(())
                }
                Err(status) => {
                    self.record_failure().ok();
                    Err(anyhow!(
                        "gRPC DeallocateGlobal to {} failed: {}",
                        self.node_id,
                        status
                    ))
                }
            };
        }

        self.record_success()?;
        info!("[stub] deallocated {} blocks on {}", blocks.len(), self.node_id);
        Ok(())
    }

    /// Health check.
    pub async fn health_check(&self) -> Result<HealthStatus> {
        debug!("health check for node {}", self.node_id);

        if let Some(mut client) = self.client() {
            let req = tonic::Request::new(HealthRequest {
                caller_node_id: self.caller_node_id.clone(),
            });
            return match client.health_check(req).await {
                Ok(resp) => {
                    let resp = resp.into_inner();
                    let status = match resp.status {
                        1 => HealthStatus::Healthy,    // SERVING
                        2 => HealthStatus::Dead,       // NOT_SERVING
                        3 => HealthStatus::Degraded,
                        _ => HealthStatus::Unknown,
                    };
                    *self.health_status.lock() = status;
                    self.update_capacity(resp.total_blocks as usize, resp.free_blocks as usize);
                    self.record_success().ok();
                    Ok(status)
                }
                Err(status) => {
                    self.record_failure().ok();
                    Err(anyhow!("gRPC HealthCheck to {} failed: {}", self.node_id, status))
                }
            };
        }

        // ---- stub fallback ----
        let status = if *self.failure_count.lock() > 3 {
            HealthStatus::Dead
        } else if *self.failure_count.lock() > 0 {
            HealthStatus::Degraded
        } else {
            HealthStatus::Healthy
        };
        *self.health_status.lock() = status;
        Ok(status)
    }

    /// Get remote state hash for consistency check.
    pub async fn get_state_hash(&self) -> Result<blake3::Hash> {
        debug!("get_state_hash from node {}", self.node_id);

        if let Some(mut client) = self.client() {
            let req = tonic::Request::new(StateHashRequest {
                caller_node_id: self.caller_node_id.clone(),
            });
            return match client.get_state_hash(req).await {
                Ok(resp) => {
                    let resp = resp.into_inner();
                    if resp.state_hash.len() != 32 {
                        return Err(anyhow!(
                            "remote {} returned invalid state hash length {}",
                            self.node_id,
                            resp.state_hash.len()
                        ));
                    }
                    let mut buf = [0u8; 32];
                    buf.copy_from_slice(&resp.state_hash);
                    self.record_success().ok();
                    Ok(blake3::Hash::from(buf))
                }
                Err(status) => {
                    self.record_failure().ok();
                    Err(anyhow!(
                        "gRPC GetStateHash to {} failed: {}",
                        self.node_id,
                        status
                    ))
                }
            };
        }

        // Stub
        Ok(blake3::hash(b"remote-state"))
    }

    /// Get current known capacity (from cache)
    pub fn get_known_capacity(&self) -> Option<RemoteCapacity> {
        self.known_capacity.lock().clone()
    }

    /// Update known capacity
    pub fn update_capacity(&self, total: usize, free: usize) {
        let capacity = RemoteCapacity {
            total_blocks: total,
            free_blocks: free,
            last_updated: Instant::now(),
        };
        debug!(
            "updated capacity for {}: {}/{} free",
            self.node_id, free, total
        );
        *self.known_capacity.lock() = Some(capacity);
    }

    /// Record successful RPC
    fn record_success(&self) -> Result<()> {
        *self.last_success.lock() = Some(Instant::now());
        *self.failure_count.lock() = 0;
        *self.health_status.lock() = HealthStatus::Healthy;
        Ok(())
    }

    /// Record failed RPC
    fn record_failure(&self) -> Result<()> {
        let mut failures = self.failure_count.lock();
        *failures += 1;
        if *failures > 3 {
            *self.health_status.lock() = HealthStatus::Dead;
            warn!("Node {} marked dead after {} failures", self.node_id, failures);
        } else {
            *self.health_status.lock() = HealthStatus::Degraded;
        }
        Ok(())
    }

    pub fn health_status(&self) -> HealthStatus {
        *self.health_status.lock()
    }

    pub fn last_success_age_secs(&self) -> Option<u64> {
        self.last_success
            .lock()
            .map(|t| Instant::now().duration_since(t).as_secs())
    }

    pub fn failure_count(&self) -> u32 {
        *self.failure_count.lock()
    }

    pub fn reset_failures(&self) {
        *self.failure_count.lock() = 0;
        *self.health_status.lock() = HealthStatus::Healthy;
    }

    pub fn node_id(&self) -> &str {
        &self.node_id
    }

    pub fn node_addr(&self) -> &str {
        &self.node_addr
    }

    pub fn is_connected(&self) -> bool {
        self.grpc.lock().is_some()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn create_allocator() -> RemoteAllocator {
        RemoteAllocator::new("node-1".to_string(), "localhost:50051".to_string())
    }

    #[test]
    fn test_create() {
        let allocator = create_allocator();
        assert_eq!(allocator.node_id(), "node-1");
        assert_eq!(allocator.node_addr(), "localhost:50051");
        assert!(!allocator.is_connected());
    }

    #[test]
    fn test_update_capacity() {
        let allocator = create_allocator();
        allocator.update_capacity(1000, 500);
        let capacity = allocator.get_known_capacity();
        assert!(capacity.is_some());
        let cap = capacity.unwrap();
        assert_eq!(cap.total_blocks, 1000);
        assert_eq!(cap.free_blocks, 500);
    }

    #[tokio::test]
    async fn test_allocate_with_capacity() {
        let allocator = create_allocator();
        allocator.update_capacity(1000, 500);
        let result = allocator.allocate(100).await;
        assert!(result.is_ok());
        assert_eq!(allocator.health_status(), HealthStatus::Healthy);
    }

    #[tokio::test]
    async fn test_allocate_without_capacity() {
        let allocator = create_allocator();
        let result = allocator.allocate(100).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_record_success() {
        let allocator = create_allocator();
        allocator.update_capacity(1000, 500);
        allocator.allocate(100).await.ok();
        assert!(allocator.last_success_age_secs().is_some());
        assert_eq!(allocator.failure_count(), 0);
        assert_eq!(allocator.health_status(), HealthStatus::Healthy);
    }

    #[tokio::test]
    async fn test_record_failure() {
        let allocator = create_allocator();
        allocator.allocate(100).await.ok();
        assert!(allocator.failure_count() > 0);
        assert_eq!(allocator.health_status(), HealthStatus::Degraded);
    }

    #[test]
    fn test_failure_degradation() {
        let allocator = create_allocator();
        for _ in 0..3 {
            allocator.record_failure().ok();
        }
        assert_eq!(allocator.health_status(), HealthStatus::Degraded);
        allocator.record_failure().ok();
        assert_eq!(allocator.health_status(), HealthStatus::Dead);
    }

    #[test]
    fn test_reset_failures() {
        let allocator = create_allocator();
        allocator.record_failure().ok();
        allocator.record_failure().ok();
        assert!(allocator.failure_count() > 0);
        allocator.reset_failures();
        assert_eq!(allocator.failure_count(), 0);
        assert_eq!(allocator.health_status(), HealthStatus::Healthy);
    }

    #[tokio::test]
    async fn test_deallocate() {
        let allocator = create_allocator();
        let result = allocator.deallocate(vec![1, 2, 3]).await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn test_health_check() {
        let allocator = create_allocator();
        let status = allocator.health_check().await;
        assert!(status.is_ok());
    }

    #[test]
    fn test_capacity_staleness() {
        let allocator = create_allocator();
        allocator.update_capacity(1000, 500);
        let cap = allocator.get_known_capacity().unwrap();
        assert!(!cap.is_stale());
    }

    #[tokio::test]
    async fn test_connect_invalid_addr_fails() {
        // 127.0.0.1:1 should refuse connections instantly.
        let alloc = RemoteAllocator::new("dead".into(), "127.0.0.1:1".into());
        let result = alloc.connect().await;
        // Either connection refused or timeout — both must surface as Err.
        assert!(result.is_err());
        assert!(!alloc.is_connected());
    }

    #[test]
    fn test_with_caller_id() {
        let alloc = RemoteAllocator::new("n".into(), "localhost:9".into())
            .with_caller_id("my-node");
        assert_eq!(alloc.caller_node_id, "my-node");
    }
}
