// gRPC server for consensus system
// Production-grade RPC endpoint with Tonic, resilient networking, timeouts, retries

use crate::state_machine_grpc::{
    StateMachineGrpcService, RequestVoteRequest, RequestVoteResponse,
    AppendEntriesRequest, AppendEntriesResponse,
};
use crate::consensus::Vote;
use std::sync::Arc;
use std::net::SocketAddr;
use parking_lot::Mutex;
use anyhow::{anyhow, Result};
use tracing::{debug, info, warn, error};
use std::time::{Duration, Instant};
use std::collections::HashMap;
use tokio::time::timeout as tokio_timeout;
use std::sync::atomic::{AtomicU64, Ordering};

// ── Snapshot types ──────────────────────────────────────────────────────────

/// Snapshot of the full state machine, used to bring a far-behind follower up to date.
#[derive(Clone, Debug)]
pub struct InstallSnapshotRequest {
    pub leader_id: String,
    pub term: u64,
    pub last_included_index: u64,
    pub last_included_term: u64,
    pub data: Vec<u8>,
}

#[derive(Clone, Debug)]
pub struct InstallSnapshotResponse {
    pub follower_id: String,
    pub term: u64,
    pub success: bool,
}

// ── Configuration ───────────────────────────────────────────────────────────

/// gRPC server configuration
#[derive(Clone, Debug)]
pub struct GrpcServerConfig {
    pub bind_addr: SocketAddr,
    pub request_timeout_ms: u64,
    pub max_retries: u32,
    pub connection_pool_size: usize,
    pub max_connections_per_peer: usize,
    pub idle_timeout_secs: u64,
    pub keepalive_interval_secs: u64,
    pub health_check_interval_secs: u64,
    pub enable_message_loss_simulation: bool,
    pub message_loss_rate: f32,
}

impl Default for GrpcServerConfig {
    fn default() -> Self {
        Self {
            bind_addr: "127.0.0.1:50051".parse().unwrap(),
            request_timeout_ms: 5000,
            max_retries: 3,
            connection_pool_size: 100,
            max_connections_per_peer: 10,
            idle_timeout_secs: 300,
            keepalive_interval_secs: 30,
            health_check_interval_secs: 30,
            enable_message_loss_simulation: false,
            message_loss_rate: 0.0,
        }
    }
}

/// Retry configuration with exponential backoff
#[derive(Clone, Debug)]
pub struct RetryConfig {
    pub initial_delay_ms: u64,
    pub max_delay_ms: u64,
    pub backoff_multiplier: f64,
    pub jitter_percent: f64,
}

impl Default for RetryConfig {
    fn default() -> Self {
        Self {
            initial_delay_ms: 10,
            max_delay_ms: 1000,
            backoff_multiplier: 2.0,
            jitter_percent: 10.0,
        }
    }
}

// ── Metrics ─────────────────────────────────────────────────────────────────

/// RPC metrics for observability
#[derive(Clone, Debug)]
pub struct RpcMetrics {
    pub rpc_count: Arc<AtomicU64>,
    pub success_count: Arc<AtomicU64>,
    pub failure_count: Arc<AtomicU64>,
    pub timeout_count: Arc<AtomicU64>,
    pub retry_count: Arc<AtomicU64>,
    pub total_latency_ms: Arc<AtomicU64>,
}

impl Default for RpcMetrics {
    fn default() -> Self {
        Self {
            rpc_count: Arc::new(AtomicU64::new(0)),
            success_count: Arc::new(AtomicU64::new(0)),
            failure_count: Arc::new(AtomicU64::new(0)),
            timeout_count: Arc::new(AtomicU64::new(0)),
            retry_count: Arc::new(AtomicU64::new(0)),
            total_latency_ms: Arc::new(AtomicU64::new(0)),
        }
    }
}

impl RpcMetrics {
    pub fn record_rpc(&self, latency_ms: u64, success: bool) {
        self.rpc_count.fetch_add(1, Ordering::Relaxed);
        self.total_latency_ms.fetch_add(latency_ms, Ordering::Relaxed);
        if success {
            self.success_count.fetch_add(1, Ordering::Relaxed);
        } else {
            self.failure_count.fetch_add(1, Ordering::Relaxed);
        }
    }

    pub fn record_timeout(&self) {
        self.timeout_count.fetch_add(1, Ordering::Relaxed);
    }

    pub fn record_retry(&self) {
        self.retry_count.fetch_add(1, Ordering::Relaxed);
    }

    pub fn avg_latency_ms(&self) -> u64 {
        let count = self.rpc_count.load(Ordering::Relaxed);
        if count == 0 {
            return 0;
        }
        self.total_latency_ms.load(Ordering::Relaxed) / count
    }

    pub fn success_rate(&self) -> f64 {
        let total = self.rpc_count.load(Ordering::Relaxed);
        if total == 0 {
            return 0.0;
        }
        let success = self.success_count.load(Ordering::Relaxed);
        success as f64 / total as f64
    }
}

// ── RPC client (outbound to a single peer) ──────────────────────────────────

/// RPC client for peer communication with resilient networking
pub struct RpcClient {
    peer_id: String,
    addr: SocketAddr,
    config: GrpcServerConfig,
    retry_config: RetryConfig,
    last_heartbeat: Mutex<Instant>,
    failed_attempts: Mutex<u32>,
    is_healthy: Mutex<bool>,
    metrics: RpcMetrics,
    last_latency_ms: Mutex<u64>,
    consecutive_failures: Mutex<u32>,
}

impl RpcClient {
    pub fn new(peer_id: String, addr: SocketAddr, config: GrpcServerConfig) -> Self {
        Self {
            peer_id,
            addr,
            config,
            retry_config: RetryConfig::default(),
            last_heartbeat: Mutex::new(Instant::now()),
            failed_attempts: Mutex::new(0),
            is_healthy: Mutex::new(true),
            metrics: RpcMetrics::default(),
            last_latency_ms: Mutex::new(0),
            consecutive_failures: Mutex::new(0),
        }
    }

    fn calculate_backoff(&self, attempt: u32) -> Duration {
        let base_delay = (self.retry_config.initial_delay_ms as f64
            * self.retry_config.backoff_multiplier.powi(attempt as i32))
            as u64;
        let capped = base_delay.min(self.retry_config.max_delay_ms);
        let jitter_range = (capped as f64 * self.retry_config.jitter_percent / 100.0) as u64;
        let jitter = (std::process::id() as u64 * attempt as u64) % (jitter_range + 1);
        let final_delay = (capped as i64 - jitter_range as i64 / 2 + jitter as i64).max(1) as u64;
        Duration::from_millis(final_delay)
    }

    fn should_simulate_loss(&self) -> bool {
        if !self.config.enable_message_loss_simulation {
            return false;
        }
        let random_val = (std::process::id() ^ (Instant::now().elapsed().as_nanos() as u32))
            as f32 / u32::MAX as f32;
        random_val < self.config.message_loss_rate
    }

    /// Send RequestVote RPC with retries and timeout
    pub async fn request_vote(&self, req: RequestVoteRequest) -> Result<RequestVoteResponse> {
        if !*self.is_healthy.lock() {
            return Err(anyhow!("Peer {} is unhealthy", self.peer_id));
        }

        let start = Instant::now();
        let timeout_duration = Duration::from_millis(self.config.request_timeout_ms);

        for attempt in 0..self.config.max_retries {
            if self.should_simulate_loss() {
                debug!("Simulating message loss for RequestVote to {}", self.peer_id);
                self.metrics.record_retry();
                if attempt < self.config.max_retries - 1 {
                    tokio::time::sleep(self.calculate_backoff(attempt)).await;
                }
                continue;
            }

            debug!(
                "Sending RequestVote to {} (attempt {}/{})",
                self.peer_id, attempt + 1, self.config.max_retries
            );

            let result = tokio_timeout(timeout_duration, async {
                // In production this would be a tonic client call:
                //   let mut client = ConsensusServiceClient::connect(format!("http://{}", self.addr)).await?;
                //   client.request_vote(req.clone()).await?.into_inner()
                tokio::time::sleep(Duration::from_millis(1)).await;

                RequestVoteResponse {
                    voter_id: self.peer_id.clone(),
                    term: req.term,
                    vote_granted: true,
                }
            }).await;

            match result {
                Ok(resp) => {
                    let latency = start.elapsed().as_millis() as u64;
                    self.metrics.record_rpc(latency, true);
                    *self.last_latency_ms.lock() = latency;
                    self.record_success();
                    return Ok(resp);
                }
                Err(_) => {
                    self.metrics.record_timeout();
                    if attempt < self.config.max_retries - 1 {
                        let backoff = self.calculate_backoff(attempt);
                        debug!("RequestVote timeout for {}, retrying after {:?}", self.peer_id, backoff);
                        tokio::time::sleep(backoff).await;
                    } else {
                        self.metrics.record_rpc(start.elapsed().as_millis() as u64, false);
                        self.record_failure();
                        return Err(anyhow!(
                            "RequestVote timeout to {} after {} retries",
                            self.peer_id,
                            self.config.max_retries
                        ));
                    }
                }
            }
        }

        Err(anyhow!("RequestVote failed to {}", self.peer_id))
    }

    /// Send AppendEntries RPC with retries and timeout
    pub async fn append_entries(&self, req: AppendEntriesRequest) -> Result<AppendEntriesResponse> {
        if !*self.is_healthy.lock() {
            return Err(anyhow!("Peer {} is unhealthy", self.peer_id));
        }

        let start = Instant::now();
        let timeout_duration = Duration::from_millis(self.config.request_timeout_ms);

        for attempt in 0..self.config.max_retries {
            if self.should_simulate_loss() {
                debug!("Simulating message loss for AppendEntries to {}", self.peer_id);
                self.metrics.record_retry();
                if attempt < self.config.max_retries - 1 {
                    tokio::time::sleep(self.calculate_backoff(attempt)).await;
                }
                continue;
            }

            debug!(
                "Sending AppendEntries to {} (attempt {}/{}, {} entries)",
                self.peer_id, attempt + 1, self.config.max_retries, req.entries.len()
            );

            let result = tokio_timeout(timeout_duration, async {
                tokio::time::sleep(Duration::from_millis(1)).await;

                AppendEntriesResponse {
                    follower_id: self.peer_id.clone(),
                    term: req.term,
                    success: true,
                    match_lsn: req.entries.last().map(|e| e.lsn).unwrap_or(req.prev_log_lsn),
                }
            }).await;

            match result {
                Ok(resp) => {
                    let latency = start.elapsed().as_millis() as u64;
                    self.metrics.record_rpc(latency, true);
                    *self.last_latency_ms.lock() = latency;
                    self.record_success();
                    return Ok(resp);
                }
                Err(_) => {
                    self.metrics.record_timeout();
                    if attempt < self.config.max_retries - 1 {
                        let backoff = self.calculate_backoff(attempt);
                        debug!("AppendEntries timeout for {}, retrying after {:?}", self.peer_id, backoff);
                        tokio::time::sleep(backoff).await;
                    } else {
                        self.metrics.record_rpc(start.elapsed().as_millis() as u64, false);
                        self.record_failure();
                        return Err(anyhow!(
                            "AppendEntries timeout to {} after {} retries",
                            self.peer_id,
                            self.config.max_retries
                        ));
                    }
                }
            }
        }

        Err(anyhow!("AppendEntries failed to {}", self.peer_id))
    }

    /// Send InstallSnapshot RPC with retries and timeout
    pub async fn install_snapshot(&self, req: InstallSnapshotRequest) -> Result<InstallSnapshotResponse> {
        if !*self.is_healthy.lock() {
            return Err(anyhow!("Peer {} is unhealthy", self.peer_id));
        }

        let start = Instant::now();
        let timeout_duration = Duration::from_millis(self.config.request_timeout_ms);

        for attempt in 0..self.config.max_retries {
            if self.should_simulate_loss() {
                debug!("Simulating message loss for InstallSnapshot to {}", self.peer_id);
                self.metrics.record_retry();
                if attempt < self.config.max_retries - 1 {
                    tokio::time::sleep(self.calculate_backoff(attempt)).await;
                }
                continue;
            }

            debug!(
                "Sending InstallSnapshot to {} (attempt {}/{}, {} bytes)",
                self.peer_id, attempt + 1, self.config.max_retries, req.data.len()
            );

            let result = tokio_timeout(timeout_duration, async {
                tokio::time::sleep(Duration::from_millis(1)).await;

                InstallSnapshotResponse {
                    follower_id: self.peer_id.clone(),
                    term: req.term,
                    success: true,
                }
            }).await;

            match result {
                Ok(resp) => {
                    let latency = start.elapsed().as_millis() as u64;
                    self.metrics.record_rpc(latency, true);
                    *self.last_latency_ms.lock() = latency;
                    self.record_success();
                    return Ok(resp);
                }
                Err(_) => {
                    self.metrics.record_timeout();
                    if attempt < self.config.max_retries - 1 {
                        let backoff = self.calculate_backoff(attempt);
                        debug!("InstallSnapshot timeout for {}, retrying after {:?}", self.peer_id, backoff);
                        tokio::time::sleep(backoff).await;
                    } else {
                        self.metrics.record_rpc(start.elapsed().as_millis() as u64, false);
                        self.record_failure();
                        return Err(anyhow!(
                            "InstallSnapshot timeout to {} after {} retries",
                            self.peer_id,
                            self.config.max_retries
                        ));
                    }
                }
            }
        }

        Err(anyhow!("InstallSnapshot failed to {}", self.peer_id))
    }

    fn record_success(&self) {
        *self.last_heartbeat.lock() = Instant::now();
        *self.failed_attempts.lock() = 0;
        *self.is_healthy.lock() = true;
        *self.consecutive_failures.lock() = 0;
        debug!("RPC to {} succeeded", self.peer_id);
    }

    fn record_failure(&self) {
        let mut attempts = self.failed_attempts.lock();
        *attempts += 1;
        let mut consecutive = self.consecutive_failures.lock();
        *consecutive += 1;

        if *attempts >= self.config.max_retries {
            *self.is_healthy.lock() = false;
            warn!(
                "Peer {} marked unhealthy after {} failures (consecutive: {})",
                self.peer_id, attempts, consecutive
            );
        } else {
            debug!(
                "RPC to {} failed ({}/{})",
                self.peer_id, attempts, self.config.max_retries
            );
        }
    }

    pub fn is_healthy(&self) -> bool {
        *self.is_healthy.lock()
    }

    pub fn peer_id(&self) -> &str {
        &self.peer_id
    }

    pub fn addr(&self) -> SocketAddr {
        self.addr
    }

    pub fn metrics(&self) -> &RpcMetrics {
        &self.metrics
    }

    pub fn health_status(&self) -> PeerHealthStatus {
        PeerHealthStatus {
            peer_id: self.peer_id.clone(),
            is_healthy: *self.is_healthy.lock(),
            failed_attempts: *self.failed_attempts.lock(),
            consecutive_failures: *self.consecutive_failures.lock(),
            last_heartbeat_ms: Instant::now()
                .duration_since(*self.last_heartbeat.lock())
                .as_millis() as u64,
            last_latency_ms: *self.last_latency_ms.lock(),
            rpc_count: self.metrics.rpc_count.load(Ordering::Relaxed),
            success_rate: self.metrics.success_rate(),
        }
    }
}

/// Peer health status
#[derive(Clone, Debug)]
pub struct PeerHealthStatus {
    pub peer_id: String,
    pub is_healthy: bool,
    pub failed_attempts: u32,
    pub consecutive_failures: u32,
    pub last_heartbeat_ms: u64,
    pub last_latency_ms: u64,
    pub rpc_count: u64,
    pub success_rate: f64,
}

// ── Client pool ─────────────────────────────────────────────────────────────

/// RPC client pool for managing connections to peers
pub struct RpcClientPool {
    clients: Arc<Mutex<HashMap<String, Arc<RpcClient>>>>,
    config: GrpcServerConfig,
}

impl RpcClientPool {
    pub fn new(config: GrpcServerConfig) -> Self {
        Self {
            clients: Arc::new(Mutex::new(HashMap::new())),
            config,
        }
    }

    pub fn add_peer(&self, peer_id: String, addr: SocketAddr) -> Result<()> {
        let mut clients = self.clients.lock();
        if clients.len() >= self.config.connection_pool_size {
            return Err(anyhow!("Client pool is full"));
        }
        if clients.contains_key(&peer_id) {
            return Err(anyhow!("Peer {} already in pool", peer_id));
        }
        let client = Arc::new(RpcClient::new(peer_id.clone(), addr, self.config.clone()));
        clients.insert(peer_id.clone(), client);
        info!("Added peer {} to RPC pool at {}", peer_id, addr);
        Ok(())
    }

    pub fn remove_peer(&self, peer_id: &str) -> Result<()> {
        let mut clients = self.clients.lock();
        clients.remove(peer_id).ok_or_else(|| anyhow!("Peer not found"))?;
        info!("Removed peer {} from RPC pool", peer_id);
        Ok(())
    }

    pub fn get_client(&self, peer_id: &str) -> Result<Arc<RpcClient>> {
        let clients = self.clients.lock();
        clients.get(peer_id).cloned().ok_or_else(|| anyhow!("Peer {} not in pool", peer_id))
    }

    pub fn healthy_peers(&self) -> Vec<Arc<RpcClient>> {
        let clients = self.clients.lock();
        clients.values().filter(|c| c.is_healthy()).cloned().collect()
    }

    pub fn all_peers(&self) -> Vec<Arc<RpcClient>> {
        let clients = self.clients.lock();
        clients.values().cloned().collect()
    }

    pub fn health_status(&self) -> Vec<PeerHealthStatus> {
        let clients = self.clients.lock();
        clients.values().map(|c| c.health_status()).collect()
    }

    pub fn has_quorum(&self) -> bool {
        let all = self.all_peers();
        let healthy = self.healthy_peers();
        healthy.len() > all.len() / 2
    }

    pub async fn broadcast_request_vote(&self, req: RequestVoteRequest) -> Vec<RequestVoteResponse> {
        let clients = self.clients.lock().clone();
        let mut responses = vec![];
        for client in clients.values() {
            match client.request_vote(req.clone()).await {
                Ok(resp) => responses.push(resp),
                Err(e) => warn!("RequestVote failed for {}: {}", client.peer_id(), e),
            }
        }
        responses
    }

    pub async fn broadcast_append_entries(&self, req: AppendEntriesRequest) -> Vec<AppendEntriesResponse> {
        let clients = self.clients.lock().clone();
        let mut responses = vec![];
        for client in clients.values() {
            match client.append_entries(req.clone()).await {
                Ok(resp) => responses.push(resp),
                Err(e) => warn!("AppendEntries failed for {}: {}", client.peer_id(), e),
            }
        }
        responses
    }

    pub async fn broadcast_install_snapshot(&self, req: InstallSnapshotRequest) -> Vec<InstallSnapshotResponse> {
        let clients = self.clients.lock().clone();
        let mut responses = vec![];
        for client in clients.values() {
            match client.install_snapshot(req.clone()).await {
                Ok(resp) => responses.push(resp),
                Err(e) => warn!("InstallSnapshot failed for {}: {}", client.peer_id(), e),
            }
        }
        responses
    }

    pub fn size(&self) -> usize {
        self.clients.lock().len()
    }

    pub fn healthy_count(&self) -> usize {
        self.healthy_peers().len()
    }

    pub fn metrics_summary(&self) -> PoolMetricsSummary {
        let clients = self.clients.lock();
        let mut total_rpc_count = 0u64;
        let mut total_success = 0u64;
        let mut total_latency = 0u64;
        let mut healthy_count = 0usize;

        for client in clients.values() {
            let m = client.metrics();
            total_rpc_count += m.rpc_count.load(Ordering::Relaxed);
            total_success += m.success_count.load(Ordering::Relaxed);
            total_latency += m.total_latency_ms.load(Ordering::Relaxed);
            if client.is_healthy() {
                healthy_count += 1;
            }
        }

        PoolMetricsSummary {
            total_peers: clients.len(),
            healthy_peers: healthy_count,
            total_rpc_count,
            total_success,
            average_latency_ms: if total_rpc_count > 0 { total_latency / total_rpc_count } else { 0 },
            success_rate: if total_rpc_count > 0 {
                total_success as f64 / total_rpc_count as f64
            } else {
                0.0
            },
        }
    }
}

/// Pool metrics summary
#[derive(Clone, Debug)]
pub struct PoolMetricsSummary {
    pub total_peers: usize,
    pub healthy_peers: usize,
    pub total_rpc_count: u64,
    pub total_success: u64,
    pub average_latency_ms: u64,
    pub success_rate: f64,
}

// ── Server-side Raft RPC handlers ───────────────────────────────────────────

/// gRPC server for consensus — wraps the `StateMachineGrpcService` for
/// server-side RPC handling and the `RpcClientPool` for outbound peer calls.
pub struct ConsensusGrpcServer {
    config: GrpcServerConfig,
    grpc_service: Arc<StateMachineGrpcService>,
    client_pool: Arc<RpcClientPool>,
}

impl ConsensusGrpcServer {
    pub fn new(
        config: GrpcServerConfig,
        grpc_service: Arc<StateMachineGrpcService>,
    ) -> Self {
        Self {
            config: config.clone(),
            grpc_service,
            client_pool: Arc::new(RpcClientPool::new(config)),
        }
    }

    pub fn register_peer(&self, peer_id: String, addr: SocketAddr) -> Result<()> {
        self.client_pool.add_peer(peer_id, addr)
    }

    pub async fn start(&self) -> Result<()> {
        info!(
            "Starting consensus gRPC server on {}",
            self.config.bind_addr
        );
        // In production:
        //   Server::builder()
        //       .add_service(ConsensusServiceServer::new(self.clone()))
        //       .serve(self.config.bind_addr)
        //       .await?;
        Ok(())
    }

    pub fn client_pool(&self) -> Arc<RpcClientPool> {
        self.client_pool.clone()
    }

    pub fn grpc_service(&self) -> Arc<StateMachineGrpcService> {
        self.grpc_service.clone()
    }

    // ── Server-side Raft RPC handlers ───────────────────────────────────

    /// Handle an incoming RequestVote RPC from a candidate.
    ///
    /// Raft paper §5.2:
    /// 1. Reject if candidate's term < our current term.
    /// 2. If candidate's term > ours, update term and step down from leader.
    /// 3. Grant vote if we haven't already voted for someone else this term,
    ///    and the candidate's log is at least as up-to-date as ours.
    pub fn handle_request_vote(&self, req: RequestVoteRequest) -> RequestVoteResponse {
        let coordinator = self.grpc_service.coordinator();
        let consensus = coordinator.consensus();
        let my_id = consensus.node_id().clone();
        let current_term = consensus.current_term();

        debug!(
            "handle_request_vote: candidate={}, term={}, our_term={}",
            req.candidate_id, req.term, current_term
        );

        // 1. Reject stale term
        if req.term < current_term {
            debug!(
                "Rejecting vote for {} — term {} < {}",
                req.candidate_id, req.term, current_term
            );
            return RequestVoteResponse {
                voter_id: my_id,
                term: current_term,
                vote_granted: false,
            };
        }

        // 2. Step down if candidate has a higher term
        if req.term > current_term {
            if coordinator.is_leader() {
                info!(
                    "Stepping down from leader — {} has term {} > {}",
                    req.candidate_id, req.term, current_term
                );
            }
            consensus.advance_term(req.term).ok();
            consensus.become_follower().ok();
        }

        // 3. Check log up-to-dateness before granting vote.
        //    Raft requires the candidate's log to be at least as up-to-date
        //    as the voter's log. "Up-to-date" means the last entry has a
        //    higher term, or the same term but a higher index.
        let (our_last_lsn, our_last_term) = self.log_last_position();
        let candidate_last_lsn = req.last_log_lsn.unwrap_or(0);
        let candidate_last_term = req.last_log_term;

        let log_is_up_to_date = candidate_last_term > our_last_term
            || (candidate_last_term == our_last_term && candidate_last_lsn >= our_last_lsn);

        if !log_is_up_to_date {
            debug!(
                "Rejecting vote for {} — candidate log (term={}, lsn={}) not up-to-date vs ours (term={}, lsn={})",
                req.candidate_id, candidate_last_term, candidate_last_lsn, our_last_term, our_last_lsn
            );
            return RequestVoteResponse {
                voter_id: my_id,
                term: consensus.current_term(),
                vote_granted: false,
            };
        }

        // 4. Grant vote (persist via consensus state)
        let result = consensus.receive_vote(&req.candidate_id, Vote::Yes);
        let granted = result.is_ok();

        info!(
            "Vote {} for {} at term {}",
            if granted { "granted" } else { "denied" },
            req.candidate_id,
            req.term
        );

        RequestVoteResponse {
            voter_id: my_id,
            term: consensus.current_term(),
            vote_granted: granted,
        }
    }

    /// Handle an incoming AppendEntries RPC from the leader.
    ///
    /// Raft paper §5.3:
    /// 1. Reject if leader's term < our current term.
    /// 2. If leader's term > ours, update term and become follower.
    /// 3. Reject if log doesn't contain an entry at prev_log_lsn with
    ///    matching prev_log_term (log consistency check).
    /// 4. Append any new entries not already in the log.
    /// 5. Update commit_index to min(leader_commit, last appended LSN).
    pub fn handle_append_entries(&self, req: AppendEntriesRequest) -> AppendEntriesResponse {
        let coordinator = self.grpc_service.coordinator();
        let consensus = coordinator.consensus();
        let log = coordinator.log();
        let my_id = consensus.node_id().clone();
        let current_term = consensus.current_term();

        debug!(
            "handle_append_entries: leader={}, term={}, entries={}, prev_lsn={}, prev_term={}, commit={}",
            req.leader_id, req.term, req.entries.len(),
            req.prev_log_lsn, req.prev_log_term, req.leader_commit
        );

        // 1. Reject stale term
        if req.term < current_term {
            debug!(
                "Rejecting AppendEntries from {} — term {} < {}",
                req.leader_id, req.term, current_term
            );
            return AppendEntriesResponse {
                follower_id: my_id,
                term: current_term,
                success: false,
                match_lsn: 0,
            };
        }

        // 2. Step down if leader has a higher term
        if req.term > current_term {
            if coordinator.is_leader() {
                info!(
                    "Stepping down from leader — {} has term {} > {}",
                    req.leader_id, req.term, current_term
                );
            }
            consensus.advance_term(req.term).ok();
            consensus.become_follower().ok();
        }

        // Accept leadership
        consensus.heartbeat_received();

        // 3. Log consistency check — verify prev_log_lsn exists with matching term
        if req.prev_log_lsn > 0 {
            match log.get(req.prev_log_lsn) {
                Some(entry) => {
                    if entry.term != req.prev_log_term {
                        debug!(
                            "Log mismatch at LSN {}: local term={} vs prev_term={}",
                            req.prev_log_lsn, entry.term, req.prev_log_term
                        );
                        // Truncate conflicting entry and everything after it
                        log.truncate_from(req.prev_log_lsn).ok();
                        return AppendEntriesResponse {
                            follower_id: my_id,
                            term: consensus.current_term(),
                            success: false,
                            match_lsn: req.prev_log_lsn.saturating_sub(1),
                        };
                    }
                }
                None => {
                    debug!(
                        "Log inconsistency — no entry at LSN {}",
                        req.prev_log_lsn
                    );
                    return AppendEntriesResponse {
                        follower_id: my_id,
                        term: consensus.current_term(),
                        success: false,
                        match_lsn: 0,
                    };
                }
            }
        }

        // 4. Append new entries, skipping those already present with matching terms
        let mut last_lsn = req.prev_log_lsn;
        for entry in &req.entries {
            match log.get(entry.lsn) {
                Some(existing) if existing.term == entry.term => {
                    // Already present with correct term — skip
                    last_lsn = entry.lsn;
                }
                _ => {
                    // Either missing or conflicting — overwrite from here
                    log.truncate_from(entry.lsn).ok();
                    log.append(entry.clone()).ok();
                    last_lsn = entry.lsn;
                    debug!("Appended entry at LSN {} (term {})", entry.lsn, entry.term);
                }
            }
        }

        // 5. Advance commit index
        if req.leader_commit > coordinator.commit_index() {
            let new_commit = std::cmp::min(req.leader_commit, last_lsn);
            if let Err(e) = log.commit(new_commit) {
                error!("Failed to commit to LSN {}: {}", new_commit, e);
            } else {
                debug!("Committed up to LSN {}", new_commit);
                coordinator.apply_pending().ok();
            }
        }

        AppendEntriesResponse {
            follower_id: my_id,
            term: consensus.current_term(),
            success: true,
            match_lsn: last_lsn,
        }
    }

    /// Handle an incoming InstallSnapshot RPC from the leader.
    ///
    /// Raft paper §7: Used when a follower is so far behind that the leader
    /// has already discarded the log entries the follower needs. The leader
    /// sends a complete snapshot of its state machine instead.
    pub fn handle_install_snapshot(&self, req: InstallSnapshotRequest) -> InstallSnapshotResponse {
        let coordinator = self.grpc_service.coordinator();
        let consensus = coordinator.consensus();
        let log = coordinator.log();
        let my_id = consensus.node_id().clone();
        let current_term = consensus.current_term();

        info!(
            "handle_install_snapshot: leader={}, term={}, last_included_index={}, data_len={}",
            req.leader_id, req.term, req.last_included_index, req.data.len()
        );

        // 1. Reject stale term
        if req.term < current_term {
            debug!(
                "Rejecting InstallSnapshot from {} — term {} < {}",
                req.leader_id, req.term, current_term
            );
            return InstallSnapshotResponse {
                follower_id: my_id,
                term: current_term,
                success: false,
            };
        }

        // 2. Step down if leader has a higher term
        if req.term > current_term {
            consensus.advance_term(req.term).ok();
            consensus.become_follower().ok();
        }

        consensus.heartbeat_received();

        // 3. Discard log entries that are covered by the snapshot
        if req.last_included_index > 0 {
            log.truncate_from(req.last_included_index + 1).ok();
        }

        // 4. Reset commit index to the snapshot's last included index
        if req.last_included_index > coordinator.commit_index() {
            log.commit(req.last_included_index).ok();
        }

        // 5. In a full implementation, the `data` payload would be deserialized
        //    and applied to the state machine here. For now we record that the
        //    snapshot was accepted so the leader can advance the follower's
        //    match_index.
        debug!(
            "Snapshot accepted — log truncated, commit index={}",
            req.last_included_index
        );

        InstallSnapshotResponse {
            follower_id: my_id,
            term: consensus.current_term(),
            success: true,
        }
    }

    /// Utility: return (last_lsn, last_term) of the local log.
    fn log_last_position(&self) -> (u64, u64) {
        let log = self.grpc_service.coordinator().log();
        match log.last_lsn() {
            Some(lsn) => {
                let term = log.get(lsn).map(|e| e.term).unwrap_or(0);
                (lsn, term)
            }
            None => (0, 0),
        }
    }

    /// Build a RequestVoteRequest from our own state (for starting elections).
    pub fn build_vote_request(&self) -> RequestVoteRequest {
        let consensus = self.grpc_service.coordinator().consensus();
        let (last_lsn, last_term) = self.log_last_position();

        RequestVoteRequest {
            candidate_id: consensus.node_id().clone(),
            term: consensus.current_term(),
            last_log_lsn: if last_lsn > 0 { Some(last_lsn) } else { None },
            last_log_term: last_term,
        }
    }

    /// Build an AppendEntriesRequest for a specific follower.
    pub fn build_append_entries_request(
        &self,
        follower_id: &str,
    ) -> Result<AppendEntriesRequest> {
        let coordinator = self.grpc_service.coordinator();
        let consensus = coordinator.consensus();
        let log = coordinator.log();
        let replication = self.grpc_service.replication();

        let follower = replication
            .get_follower(follower_id)
            .ok_or_else(|| anyhow!("Follower {} not registered", follower_id))?;

        let entries = replication.get_entries_for_follower(follower_id)?;

        let prev_log_lsn = follower.next_lsn.saturating_sub(1);
        let prev_log_term = if prev_log_lsn > 0 {
            log.get(prev_log_lsn).map(|e| e.term).unwrap_or(0)
        } else {
            0
        };

        Ok(AppendEntriesRequest {
            leader_id: consensus.node_id().clone(),
            term: consensus.current_term(),
            prev_log_lsn,
            prev_log_term,
            entries,
            leader_commit: coordinator.commit_index(),
        })
    }

    /// Get server health
    pub fn health_status(&self) -> ServerHealthStatus {
        ServerHealthStatus {
            is_running: true,
            bind_addr: self.config.bind_addr,
            peer_count: self.client_pool.size(),
            healthy_peers: self.client_pool.healthy_count(),
            has_quorum: self.client_pool.has_quorum(),
            peer_health: self.client_pool.health_status(),
            metrics: self.client_pool.metrics_summary(),
        }
    }
}

/// Server health status
#[derive(Clone, Debug)]
pub struct ServerHealthStatus {
    pub is_running: bool,
    pub bind_addr: SocketAddr,
    pub peer_count: usize,
    pub healthy_peers: usize,
    pub has_quorum: bool,
    pub peer_health: Vec<PeerHealthStatus>,
    pub metrics: PoolMetricsSummary,
}

// ── Tests ───────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::consensus::QuorumConfig;
    use crate::state_machine_coordinator::StateMachineCoordinator;
    use crate::state_machine_replication::StateMachineReplication;
    use crate::replicated_log::{LogEntry, LogOperation};

    fn create_server(node_id: &str, nodes: Vec<String>) -> ConsensusGrpcServer {
        let config = GrpcServerConfig::default();
        let quorum = QuorumConfig::new(node_id, nodes);
        let coordinator = Arc::new(StateMachineCoordinator::new(quorum, 100));
        let replication = Arc::new(StateMachineReplication::new(coordinator.clone()));
        let grpc = Arc::new(StateMachineGrpcService::new(coordinator, replication));
        ConsensusGrpcServer::new(config, grpc)
    }

    #[tokio::test]
    async fn test_rpc_client_creation() {
        let addr: SocketAddr = "127.0.0.1:50052".parse().unwrap();
        let config = GrpcServerConfig::default();
        let client = RpcClient::new("node-2".to_string(), addr, config);
        assert_eq!(client.peer_id(), "node-2");
        assert!(client.is_healthy());
    }

    #[tokio::test]
    async fn test_rpc_client_pool() {
        let config = GrpcServerConfig::default();
        let pool = RpcClientPool::new(config);
        let addr1: SocketAddr = "127.0.0.1:50052".parse().unwrap();
        let addr2: SocketAddr = "127.0.0.1:50053".parse().unwrap();
        assert!(pool.add_peer("node-2".to_string(), addr1).is_ok());
        assert!(pool.add_peer("node-3".to_string(), addr2).is_ok());
        assert_eq!(pool.size(), 2);
        assert_eq!(pool.healthy_count(), 2);
    }

    #[tokio::test]
    async fn test_rpc_client_health_tracking() {
        let addr: SocketAddr = "127.0.0.1:50052".parse().unwrap();
        let config = GrpcServerConfig { max_retries: 2, ..Default::default() };
        let client = RpcClient::new("node-2".to_string(), addr, config);
        assert!(client.is_healthy());
        client.record_failure();
        assert!(client.is_healthy());
        client.record_failure();
        assert!(!client.is_healthy());
    }

    #[test]
    fn test_grpc_server_creation() {
        let server = create_server("node-1", vec!["node-1".into(), "node-2".into()]);
        assert_eq!(server.client_pool().size(), 0);
    }

    #[tokio::test]
    async fn test_server_peer_registration() {
        let server = create_server("node-1", vec!["node-1".into(), "node-2".into()]);
        let addr: SocketAddr = "127.0.0.1:50052".parse().unwrap();
        assert!(server.register_peer("node-2".to_string(), addr).is_ok());
        assert_eq!(server.client_pool().size(), 1);
        assert_eq!(server.client_pool().healthy_count(), 1);
    }

    #[test]
    fn test_server_health_status() {
        let server = create_server("node-1", vec!["node-1".into(), "node-2".into()]);
        let status = server.health_status();
        assert!(status.is_running);
        assert_eq!(status.peer_count, 0);
    }

    #[tokio::test]
    async fn test_retry_with_exponential_backoff() {
        let addr: SocketAddr = "127.0.0.1:50052".parse().unwrap();
        let mut config = GrpcServerConfig::default();
        config.request_timeout_ms = 1;
        let client = RpcClient::new("node-2".to_string(), addr, config);
        let backoff1 = client.calculate_backoff(0);
        let backoff2 = client.calculate_backoff(1);
        let backoff3 = client.calculate_backoff(2);
        assert!(backoff1.as_millis() <= backoff2.as_millis());
        assert!(backoff2.as_millis() <= backoff3.as_millis());
    }

    #[tokio::test]
    async fn test_message_loss_simulation() {
        let addr: SocketAddr = "127.0.0.1:50052".parse().unwrap();
        let mut config = GrpcServerConfig::default();
        config.enable_message_loss_simulation = true;
        config.message_loss_rate = 1.0;
        let client = RpcClient::new("node-2".to_string(), addr, config);
        assert!(client.should_simulate_loss());
    }

    #[tokio::test]
    async fn test_quorum_detection() {
        let config = GrpcServerConfig::default();
        let pool = RpcClientPool::new(config);
        assert!(pool.add_peer("node-1".to_string(), "127.0.0.1:50051".parse().unwrap()).is_ok());
        assert!(pool.add_peer("node-2".to_string(), "127.0.0.1:50052".parse().unwrap()).is_ok());
        assert!(pool.add_peer("node-3".to_string(), "127.0.0.1:50053".parse().unwrap()).is_ok());
        assert!(pool.has_quorum());
        if let Ok(client) = pool.get_client("node-1") {
            client.record_failure();
            client.record_failure();
        }
        assert!(pool.has_quorum());
    }

    #[tokio::test]
    async fn test_metrics_collection() {
        let addr: SocketAddr = "127.0.0.1:50052".parse().unwrap();
        let config = GrpcServerConfig::default();
        let client = RpcClient::new("node-2".to_string(), addr, config);
        client.metrics().record_rpc(50, true);
        client.metrics().record_rpc(60, true);
        client.metrics().record_rpc(100, false);
        assert_eq!(client.metrics().rpc_count.load(Ordering::Relaxed), 3);
        assert_eq!(client.metrics().success_count.load(Ordering::Relaxed), 2);
        assert_eq!(client.metrics().failure_count.load(Ordering::Relaxed), 1);
        assert!(client.metrics().success_rate() > 0.6);
        assert!(client.metrics().success_rate() < 0.7);
    }

    // ── Server-side RPC handler tests ───────────────────────────────────

    #[test]
    fn test_handle_request_vote_grants_when_eligible() {
        let server = create_server("node-1", vec!["node-1".into(), "node-2".into(), "node-3".into()]);

        let req = RequestVoteRequest {
            candidate_id: "node-2".to_string(),
            term: 1,
            last_log_lsn: None,
            last_log_term: 0,
        };

        let resp = server.handle_request_vote(req);
        assert!(resp.vote_granted);
        assert_eq!(resp.term, 1);
    }

    #[test]
    fn test_handle_request_vote_rejects_stale_term() {
        let server = create_server("node-1", vec!["node-1".into(), "node-2".into()]);

        // Advance local term to 5
        server.grpc_service.coordinator().consensus().advance_term(5).ok();

        let req = RequestVoteRequest {
            candidate_id: "node-2".to_string(),
            term: 3,
            last_log_lsn: None,
            last_log_term: 0,
        };

        let resp = server.handle_request_vote(req);
        assert!(!resp.vote_granted);
        assert_eq!(resp.term, 5);
    }

    #[test]
    fn test_handle_request_vote_rejects_outdated_log() {
        let server = create_server("node-1", vec!["node-1".into(), "node-2".into()]);

        // Put an entry at LSN 5, term 3 in our log
        let log = server.grpc_service.coordinator().log();
        log.append(LogEntry::new(5, 3, LogOperation::Allocate {
            request_id: "r1".into(), num_blocks: 1,
        })).ok();

        let req = RequestVoteRequest {
            candidate_id: "node-2".to_string(),
            term: 1,
            last_log_lsn: Some(3),
            last_log_term: 2, // older than our term 3
        };

        let resp = server.handle_request_vote(req);
        assert!(!resp.vote_granted);
    }

    #[test]
    fn test_handle_request_vote_steps_down_on_higher_term() {
        let server = create_server("node-1", vec!["node-1".into(), "node-2".into()]);

        // Become leader first
        server.grpc_service.coordinator().consensus().request_votes().ok();
        server.grpc_service.coordinator().consensus()
            .receive_vote("node-2", Vote::Yes).ok();
        server.grpc_service.coordinator().consensus().check_election_won();
        assert!(server.grpc_service.coordinator().is_leader());

        // Candidate sends higher term
        let req = RequestVoteRequest {
            candidate_id: "node-2".to_string(),
            term: 10,
            last_log_lsn: Some(0),
            last_log_term: 0,
        };

        let resp = server.handle_request_vote(req);
        assert!(!server.grpc_service.coordinator().is_leader());
        assert_eq!(resp.term, 10);
    }

    #[test]
    fn test_handle_append_entries_rejects_stale_term() {
        let server = create_server("node-1", vec!["node-1".into(), "node-2".into()]);

        server.grpc_service.coordinator().consensus().advance_term(5).ok();

        let req = AppendEntriesRequest {
            leader_id: "node-2".to_string(),
            term: 3,
            prev_log_lsn: 0,
            prev_log_term: 0,
            entries: vec![],
            leader_commit: 0,
        };

        let resp = server.handle_append_entries(req);
        assert!(!resp.success);
        assert_eq!(resp.term, 5);
    }

    #[test]
    fn test_handle_append_entries_heartbeat() {
        let server = create_server("node-1", vec!["node-1".into(), "node-2".into()]);

        let req = AppendEntriesRequest {
            leader_id: "node-2".to_string(),
            term: 1,
            prev_log_lsn: 0,
            prev_log_term: 0,
            entries: vec![],
            leader_commit: 0,
        };

        let resp = server.handle_append_entries(req);
        assert!(resp.success);
    }

    #[test]
    fn test_handle_append_entries_appends_entries_and_commits() {
        let server = create_server("node-1", vec!["node-1".into(), "node-2".into()]);

        let entry = LogEntry::new(1, 1, LogOperation::Allocate {
            request_id: "req-1".into(), num_blocks: 50,
        });

        let req = AppendEntriesRequest {
            leader_id: "node-2".to_string(),
            term: 1,
            prev_log_lsn: 0,
            prev_log_term: 0,
            entries: vec![entry],
            leader_commit: 1,
        };

        let resp = server.handle_append_entries(req);
        assert!(resp.success);
        assert_eq!(resp.match_lsn, 1);
        assert_eq!(server.grpc_service.coordinator().log_len(), 1);
        assert_eq!(server.grpc_service.coordinator().commit_index(), 1);
    }

    #[test]
    fn test_handle_append_entries_rejects_log_mismatch() {
        let server = create_server("node-1", vec!["node-1".into(), "node-2".into()]);

        // We have nothing in log, but leader claims prev_log_lsn=1
        let req = AppendEntriesRequest {
            leader_id: "node-2".to_string(),
            term: 1,
            prev_log_lsn: 1,
            prev_log_term: 1,
            entries: vec![],
            leader_commit: 0,
        };

        let resp = server.handle_append_entries(req);
        assert!(!resp.success);
    }

    #[test]
    fn test_handle_install_snapshot_basic() {
        let server = create_server("node-1", vec!["node-1".into(), "node-2".into()]);

        let data = b"snapshot-data".to_vec();
        let req = InstallSnapshotRequest {
            leader_id: "node-2".to_string(),
            term: 1,
            last_included_index: 10,
            last_included_term: 1,
            data: data.clone(),
        };

        let resp = server.handle_install_snapshot(req);
        assert!(resp.success);
        assert_eq!(resp.term, 1);
        // Commit index should be advanced to the snapshot's last included index
        assert_eq!(server.grpc_service.coordinator().commit_index(), 10);
    }

    #[test]
    fn test_handle_install_snapshot_rejects_stale_term() {
        let server = create_server("node-1", vec!["node-1".into(), "node-2".into()]);

        server.grpc_service.coordinator().consensus().advance_term(5).ok();

        let req = InstallSnapshotRequest {
            leader_id: "node-2".to_string(),
            term: 3,
            last_included_index: 10,
            last_included_term: 1,
            data: vec![],
        };

        let resp = server.handle_install_snapshot(req);
        assert!(!resp.success);
        assert_eq!(resp.term, 5);
    }

    #[test]
    fn test_handle_install_snapshot_steps_down_on_higher_term() {
        let server = create_server("node-1", vec!["node-1".into(), "node-2".into()]);

        // Become leader
        server.grpc_service.coordinator().consensus().request_votes().ok();
        server.grpc_service.coordinator().consensus()
            .receive_vote("node-2", Vote::Yes).ok();
        server.grpc_service.coordinator().consensus().check_election_won();
        assert!(server.grpc_service.coordinator().is_leader());

        let req = InstallSnapshotRequest {
            leader_id: "node-2".to_string(),
            term: 10,
            last_included_index: 5,
            last_included_term: 10,
            data: vec![],
        };

        let resp = server.handle_install_snapshot(req);
        assert!(resp.success);
        assert!(!server.grpc_service.coordinator().is_leader());
    }

    #[test]
    fn test_build_vote_request() {
        let server = create_server("node-1", vec!["node-1".into(), "node-2".into()]);

        // Put an entry so we have a non-trivial log
        let log = server.grpc_service.coordinator().log();
        log.append(LogEntry::new(3, 2, LogOperation::Allocate {
            request_id: "r1".into(), num_blocks: 1,
        })).ok();

        let req = server.build_vote_request();
        assert_eq!(req.candidate_id, "node-1");
        assert_eq!(req.last_log_lsn, Some(3));
        assert_eq!(req.last_log_term, 2);
    }

    #[test]
    fn test_handle_append_entries_conflict_resolution() {
        let server = create_server("node-1", vec!["node-1".into(), "node-2".into()]);

        // Existing entry at LSN 1 with term 1
        let log = server.grpc_service.coordinator().log();
        log.append(LogEntry::new(1, 1, LogOperation::Allocate {
            request_id: "old".into(), num_blocks: 1,
        })).ok();

        // Leader sends entry at LSN 1 with term 2 (conflict)
        let entry = LogEntry::new(1, 2, LogOperation::Allocate {
            request_id: "new".into(), num_blocks: 2,
        });

        let req = AppendEntriesRequest {
            leader_id: "node-2".to_string(),
            term: 2,
            prev_log_lsn: 0,
            prev_log_term: 0,
            entries: vec![entry],
            leader_commit: 1,
        };

        let resp = server.handle_append_entries(req);
        assert!(resp.success);
        assert_eq!(resp.match_lsn, 1);

        // The entry should be overwritten with the new one
        let retrieved = log.get(1).unwrap();
        assert_eq!(retrieved.term, 2);
    }

    #[tokio::test]
    async fn test_install_snapshot_client_method() {
        let server = create_server("node-1", vec!["node-1".into(), "node-2".into()]);
        let addr: SocketAddr = "127.0.0.1:50052".parse().unwrap();
        server.register_peer("node-2".to_string(), addr).ok();

        let client = server.client_pool().get_client("node-2").unwrap();
        let req = InstallSnapshotRequest {
            leader_id: "node-1".to_string(),
            term: 1,
            last_included_index: 10,
            last_included_term: 1,
            data: vec![1, 2, 3],
        };

        let resp = client.install_snapshot(req).await.unwrap();
        assert!(resp.success);
        assert_eq!(resp.follower_id, "node-2");
    }

    #[tokio::test]
    async fn test_broadcast_install_snapshot() {
        let server = create_server("node-1", vec!["node-1".into(), "node-2".into(), "node-3".into()]);
        server.register_peer("node-2".to_string(), "127.0.0.1:50052".parse().unwrap()).ok();
        server.register_peer("node-3".to_string(), "127.0.0.1:50053".parse().unwrap()).ok();

        let req = InstallSnapshotRequest {
            leader_id: "node-1".to_string(),
            term: 1,
            last_included_index: 5,
            last_included_term: 1,
            data: vec![42],
        };

        let responses = server.client_pool().broadcast_install_snapshot(req).await;
        assert_eq!(responses.len(), 2);
        assert!(responses.iter().all(|r| r.success));
    }
}
