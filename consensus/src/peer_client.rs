// Peer client: gRPC client for consensus RPCs to peer nodes

use anyhow::Result;
use std::collections::HashMap;
use parking_lot::RwLock;
use tonic::transport::Channel;
use tracing::{info, warn, debug, error};

pub mod proto {
    tonic::include_proto!("aegis.consensus");
}

use proto::{
    consensus_service_client::ConsensusServiceClient,
    RequestVoteRequest, AppendEntriesRequest, LogEntry as ProtoLogEntry,
    InstallSnapshotRequest,
};

/// Connection to a single peer node.
struct PeerConnection {
    client: ConsensusServiceClient<Channel>,
    healthy: bool,
}

/// Manages gRPC connections to all peer nodes and sends consensus RPCs.
pub struct PeerClient {
    node_id: String,
    peers: RwLock<HashMap<String, PeerConnection>>,
}

impl PeerClient {
    pub fn new(node_id: String) -> Self {
        Self {
            node_id,
            peers: RwLock::new(HashMap::new()),
        }
    }

    /// Connect to a peer node (lazy connection).
    pub fn connect_peer(&self, peer_id: &str, addr: &str) {
        match Channel::from_shared(addr.to_string())
            .map(|ch| ch.connect_timeout(std::time::Duration::from_secs(3))).map(|ch| ch.connect_lazy())
        {
            Ok(channel) => {
                let client = ConsensusServiceClient::new(channel);
                self.peers.write().insert(
                    peer_id.to_string(),
                    PeerConnection {
                        client,
                        healthy: true,
                    },
                );
                info!(peer = peer_id, addr = addr, "Connected to peer");
            }
            Err(e) => {
                warn!(peer = peer_id, addr = addr, error = %e, "Failed to connect to peer");
            }
        }
    }

    /// Send a RequestVote RPC to a specific peer.
    pub async fn request_vote(
        &self,
        peer_id: &str,
        candidate_id: &str,
        term: u64,
        last_log_index: u64,
        last_log_term: u64,
    ) -> Result<(u64, bool)> {
        let mut peers = self.peers.write();
        let peer = peers.get_mut(peer_id)
            .ok_or_else(|| anyhow::anyhow!("Unknown peer: {}", peer_id))?;

        if !peer.healthy {
            return Err(anyhow::anyhow!("Peer {} is marked unhealthy", peer_id));
        }

        let req = RequestVoteRequest {
            candidate_id: candidate_id.to_string(),
            term,
            last_log_index,
            last_log_term,
        };

        match peer.client.request_vote(req).await {
            Ok(resp) => {
                let r = resp.into_inner();
                debug!(
                    peer = peer_id,
                    term = r.term,
                    granted = r.vote_granted,
                    "RequestVote response"
                );
                Ok((r.term, r.vote_granted))
            }
            Err(e) => {
                peer.healthy = false;
                error!(peer = peer_id, error = %e, "RequestVote RPC failed");
                Err(anyhow::anyhow!("RPC error: {}", e))
            }
        }
    }

    /// Send AppendEntries (heartbeat or log replication) to a specific peer.
    pub async fn append_entries(
        &self,
        peer_id: &str,
        leader_id: &str,
        term: u64,
        prev_log_index: u64,
        prev_log_term: u64,
        entries: Vec<(u64, u64, String)>,  // (index, term, data)
        leader_commit: u64,
    ) -> Result<(u64, bool, u64)> {
        let mut peers = self.peers.write();
        let peer = peers.get_mut(peer_id)
            .ok_or_else(|| anyhow::anyhow!("Unknown peer: {}", peer_id))?;

        if !peer.healthy {
            return Err(anyhow::anyhow!("Peer {} is marked unhealthy", peer_id));
        }

        let proto_entries: Vec<ProtoLogEntry> = entries
            .into_iter()
            .map(|(index, term, data)| ProtoLogEntry { index, term, data })
            .collect();

        let req = AppendEntriesRequest {
            leader_id: leader_id.to_string(),
            term,
            prev_log_index,
            prev_log_term,
            entries: proto_entries,
            leader_commit,
        };

        match peer.client.append_entries(req).await {
            Ok(resp) => {
                let r = resp.into_inner();
                debug!(
                    peer = peer_id,
                    term = r.term,
                    success = r.success,
                    match_index = r.match_index,
                    "AppendEntries response"
                );
                Ok((r.term, r.success, r.match_index))
            }
            Err(e) => {
                peer.healthy = false;
                error!(peer = peer_id, error = %e, "AppendEntries RPC failed");
                Err(anyhow::anyhow!("RPC error: {}", e))
            }
        }
    }

    /// Send InstallSnapshot RPC to a specific peer.
    pub async fn install_snapshot(
        &self,
        peer_id: &str,
        leader_id: &str,
        term: u64,
        last_included_index: u64,
        last_included_term: u64,
        data: Vec<u8>,
    ) -> Result<u64> {
        let mut peers = self.peers.write();
        let peer = peers.get_mut(peer_id)
            .ok_or_else(|| anyhow::anyhow!("Unknown peer: {}", peer_id))?;

        if !peer.healthy {
            return Err(anyhow::anyhow!("Peer {} is marked unhealthy", peer_id));
        }

        let req = InstallSnapshotRequest {
            leader_id: leader_id.to_string(),
            term,
            last_included_index,
            last_included_term,
            data,
        };

        match peer.client.install_snapshot(req).await {
            Ok(resp) => {
                let r = resp.into_inner();
                debug!(
                    peer = peer_id,
                    term = r.term,
                    "InstallSnapshot response"
                );
                Ok(r.term)
            }
            Err(e) => {
                peer.healthy = false;
                error!(peer = peer_id, error = %e, "InstallSnapshot RPC failed");
                Err(anyhow::anyhow!("RPC error: {}", e))
            }
        }
    }

    /// Broadcast RequestVote to all peers and collect votes.
    /// Returns (votes_received, total_peers).
    pub async fn broadcast_request_vote(
        &self,
        candidate_id: &str,
        term: u64,
        last_log_index: u64,
        last_log_term: u64,
    ) -> (u64, u64) {
        let peer_ids: Vec<String> = self.peers.read().keys().cloned().collect();
        let total = peer_ids.len() as u64;
        let mut votes = 0u64;

        for peer_id in &peer_ids {
            if let Ok((_term, granted)) = self.request_vote(
                peer_id, candidate_id, term, last_log_index, last_log_term,
            ).await {
                if granted {
                    votes += 1;
                }
            }
        }

        info!(
            candidate = candidate_id,
            votes = votes,
            total_peers = total,
            "Broadcast RequestVote complete"
        );

        (votes, total)
    }

    /// Broadcast AppendEntries (heartbeat) to all peers.
    /// Returns (successes, total_peers).
    pub async fn broadcast_heartbeat(
        &self,
        leader_id: &str,
        term: u64,
        prev_log_index: u64,
        prev_log_term: u64,
        leader_commit: u64,
    ) -> (u64, u64) {
        let peer_ids: Vec<String> = self.peers.read().keys().cloned().collect();
        let total = peer_ids.len() as u64;
        let mut successes = 0u64;

        for peer_id in &peer_ids {
            match self.append_entries(
                peer_id, leader_id, term,
                prev_log_index, prev_log_term,
                vec![],  // heartbeat has no entries
                leader_commit,
            ).await {
                Ok((_term, success, _match)) => {
                    if success {
                        successes += 1;
                    }
                }
                Err(_) => {}
            }
        }

        (successes, total)
    }

    /// Get count of connected healthy peers.
    pub fn peer_count(&self) -> usize {
        self.peers.read().len()
    }

    /// Mark a peer as unhealthy (e.g., after repeated failures).
    pub fn mark_unhealthy(&self, peer_id: &str) {
        if let Some(peer) = self.peers.write().get_mut(peer_id) {
            peer.healthy = false;
        }
    }

    /// Mark a peer as healthy again.
    pub fn mark_healthy(&self, peer_id: &str) {
        if let Some(peer) = self.peers.write().get_mut(peer_id) {
            peer.healthy = true;
        }
    }
}
