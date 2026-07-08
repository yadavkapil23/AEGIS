// Consensus module: distributed state synchronization with leader election

pub mod log;
pub mod state;
pub mod peer_client;

pub use log::ReplicatedLog;
pub use state::ExecutionState;
pub use peer_client::PeerClient;

use anyhow::Result;
use parking_lot::RwLock;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use tracing::{info, debug};
use std::time::{Duration, Instant};

/// Node role in the cluster.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NodeRole {
    Follower,
    Candidate,
    Leader,
}

impl std::fmt::Display for NodeRole {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Follower => write!(f, "Follower"),
            Self::Candidate => write!(f, "Candidate"),
            Self::Leader => write!(f, "Leader"),
        }
    }
}

/// Configuration for the consensus engine.
#[derive(Debug, Clone)]
pub struct ConsensusConfig {
    pub node_id: String,
    pub peers: Vec<String>,
    pub election_timeout_ms: u64,
    pub heartbeat_interval_ms: u64,
    pub log_persistence: bool,
}

impl Default for ConsensusConfig {
    fn default() -> Self {
        Self {
            node_id: uuid::Uuid::new_v4().to_string(),
            peers: vec![],
            election_timeout_ms: 300,
            heartbeat_interval_ms: 100,
            log_persistence: false,
        }
    }
}

/// Raft-inspired consensus engine with leader election.
pub struct ConsensusEngine {
    config: ConsensusConfig,
    log: Arc<ReplicatedLog>,
    role: RwLock<NodeRole>,
    current_term: AtomicU64,
    voted_for: RwLock<Option<String>>,
    leader_id: RwLock<Option<String>>,
    last_heartbeat: RwLock<Instant>,
    peer_votes_received: AtomicU64,
    alive_peers: AtomicU64,
}

impl ConsensusEngine {
    pub fn new(config: ConsensusConfig) -> Result<Self> {
        let peer_count = config.peers.len() as u64;
        info!(
            node_id = %config.node_id,
            peers = config.peers.len(),
            "Consensus engine created"
        );

        Ok(Self {
            log: Arc::new(ReplicatedLog::new(config.node_id.clone())),
            role: RwLock::new(NodeRole::Follower),
            current_term: AtomicU64::new(0),
            voted_for: RwLock::new(None),
            leader_id: RwLock::new(None),
            last_heartbeat: RwLock::new(Instant::now()),
            peer_votes_received: AtomicU64::new(0),
            alive_peers: AtomicU64::new(peer_count),
            config,
        })
    }

    // ── State accessors ────────────────────────────────────

    pub fn role(&self) -> NodeRole {
        *self.role.read()
    }

    pub fn current_term(&self) -> u64 {
        self.current_term.load(Ordering::SeqCst)
    }

    pub fn is_leader(&self) -> bool {
        self.role() == NodeRole::Leader
    }

    pub fn leader_id(&self) -> Option<String> {
        self.leader_id.read().clone()
    }

    pub fn node_id(&self) -> &str {
        &self.config.node_id
    }

    // ── Leader election ────────────────────────────────────

    /// Check if election timeout has elapsed without a heartbeat.
    pub fn election_timeout_elapsed(&self) -> bool {
        self.last_heartbeat.read().elapsed()
            >= Duration::from_millis(self.config.election_timeout_ms)
    }

    /// Start an election (increment term, vote for self, solicit votes).
    pub fn start_election(&self) {
        let new_term = self.current_term.fetch_add(1, Ordering::SeqCst) + 1;
        *self.role.write() = NodeRole::Candidate;
        *self.voted_for.write() = Some(self.config.node_id.clone());
        self.peer_votes_received.store(1, Ordering::SeqCst); // Vote for self

        info!(
            node_id = %self.config.node_id,
            term = new_term,
            "Starting election"
        );
    }

    /// Handle a vote response from a peer.
    /// Returns `true` if this node has won the election.
    pub fn handle_vote_response(&self, granted: bool) -> bool {
        if granted {
            let votes = self.peer_votes_received.fetch_add(1, Ordering::SeqCst) + 1;
            let total_nodes = self.alive_peers.load(Ordering::SeqCst) + 1; // +1 for self
            let majority = total_nodes / 2 + 1;

            if votes >= majority {
                *self.role.write() = NodeRole::Leader;
                *self.leader_id.write() = Some(self.config.node_id.clone());
                info!(
                    node_id = %self.config.node_id,
                    votes = votes,
                    majority = majority,
                    "Won election, became leader"
                );
                return true;
            }
        }
        false
    }

    /// Handle receiving a RequestVote RPC from a candidate.
    pub fn handle_request_vote(
        &self,
        candidate_id: &str,
        candidate_term: u64,
        candidate_log_index: u64,
    ) -> bool {
        let current_term = self.current_term.load(Ordering::SeqCst);

        // Reject if candidate's term is stale
        if candidate_term < current_term {
            return false;
        }

        // If candidate has a newer term, update and step down
        if candidate_term > current_term {
            self.current_term.store(candidate_term, Ordering::SeqCst);
            *self.role.write() = NodeRole::Follower;
            *self.voted_for.write() = None;
        }

        // Grant vote if we haven't voted for someone else this term
        let can_vote = match &*self.voted_for.read() {
            None => true,
            Some(id) => id == candidate_id,
        };

        let log_ok = candidate_log_index >= self.log.last_index();

        if can_vote && log_ok {
            *self.voted_for.write() = Some(candidate_id.to_string());
            *self.last_heartbeat.write() = Instant::now();
            info!(
                node_id = %self.config.node_id,
                candidate = candidate_id,
                term = candidate_term,
                "Vote granted"
            );
            return true;
        }

        false
    }

    /// Handle receiving an AppendEntries (heartbeat) from the leader.
    pub fn handle_append_entries(
        &self,
        leader_id: &str,
        leader_term: u64,
        entries: Vec<String>,
    ) -> bool {
        let current_term = self.current_term.load(Ordering::SeqCst);

        if leader_term < current_term {
            return false;
        }

        // Accept the leader
        if leader_term > current_term {
            self.current_term.store(leader_term, Ordering::SeqCst);
        }

        *self.role.write() = NodeRole::Follower;
        *self.leader_id.write() = Some(leader_id.to_string());
        *self.voted_for.write() = None;
        *self.last_heartbeat.write() = Instant::now();

        // Append any new entries
        for entry in entries {
            let _ = self.log.append(entry);
        }

        true
    }

    /// Step down: leader → follower.
    pub fn step_down(&self) {
        let old_role = *self.role.read();
        if old_role != NodeRole::Follower {
            *self.role.write() = NodeRole::Follower;
            *self.voted_for.write() = None;
            info!(
                node_id = %self.config.node_id,
                old_role = %old_role,
                "Stepped down to follower"
            );
        }
    }

    // ── Log operations ─────────────────────────────────────

    pub fn append_entry(&self, data: String) -> Result<u64> {
        if !self.is_leader() {
            return Err(anyhow::anyhow!("Only the leader can append entries"));
        }
        self.log.append(data)
    }

    pub fn get_last_index(&self) -> u64 {
        self.log.last_index()
    }

    pub fn replay(&self) -> Result<Vec<String>> {
        self.log.replay()
    }

    pub fn log_ref(&self) -> &ReplicatedLog {
        &self.log
    }

    // ── Peer replication (uses PeerClient for real gRPC) ───

    /// Start an election and actually solicit votes from peers via gRPC.
    pub async fn start_election_with_peers(&self, peer_client: &PeerClient) {
        self.start_election();
        let term = self.current_term();
        let last_idx = self.log.last_index();

        let (votes, total_peers) = peer_client
            .broadcast_request_vote(&self.config.node_id, term, last_idx, 0)
            .await;

        // +1 for self-vote
        let total = total_peers + 1;
        let majority = total / 2 + 1;
        let all_votes = votes + 1; // include self

        if all_votes >= majority {
            *self.role.write() = NodeRole::Leader;
            *self.leader_id.write() = Some(self.config.node_id.clone());
            info!(
                node_id = %self.config.node_id,
                votes = all_votes,
                majority = majority,
                "Won election via peer voting"
            );
        } else {
            info!(
                node_id = %self.config.node_id,
                votes = all_votes,
                majority = majority,
                "Election lost, staying follower"
            );
            self.step_down();
        }
    }

    /// Send heartbeat (AppendEntries) to all peers.
    /// Returns (successes, total_peers).
    pub async fn send_heartbeat(&self, peer_client: &PeerClient) -> (u64, u64) {
        if !self.is_leader() {
            return (0, 0);
        }

        let term = self.current_term();
        let last_idx = self.log.last_index();

        let (successes, total) = peer_client
            .broadcast_heartbeat(
                &self.config.node_id,
                term,
                last_idx,
                0, // last_log_term (simplified)
                last_idx, // leader_commit
            )
            .await;

        debug!(
            node_id = %self.config.node_id,
            successes = successes,
            total = total,
            "Heartbeat broadcast complete"
        );

        (successes, total)
    }

    /// Replicate a new log entry to all peers.
    pub async fn replicate_entry(&self, peer_client: &PeerClient, data: String) -> Result<u64> {
        let idx = self.append_entry(data.clone())?;
        let term = self.current_term();

        let (successes, total) = peer_client
            .broadcast_heartbeat(
                &self.config.node_id,
                term,
                idx - 1, // prev_log_index
                0,        // prev_log_term
                idx,       // leader_commit
            )
            .await;

        info!(
            node_id = %self.config.node_id,
            index = idx,
            replicated_to = successes,
            total_peers = total,
            "Entry replicated"
        );

        Ok(idx)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_creation() {
        let config = ConsensusConfig::default();
        let engine = ConsensusEngine::new(config).unwrap();
        assert_eq!(engine.role(), NodeRole::Follower);
        assert_eq!(engine.current_term(), 0);
    }

    #[test]
    fn test_append_requires_leader() {
        let engine = ConsensusEngine::new(ConsensusConfig::default()).unwrap();
        assert!(engine.append_entry("test".into()).is_err());
    }

    #[test]
    fn test_election_becomes_leader_with_single_node() {
        let config = ConsensusConfig {
            peers: vec![],
            ..Default::default()
        };
        let engine = ConsensusEngine::new(config).unwrap();
        engine.start_election();
        // Single node: self-vote = majority of 1
        assert!(engine.handle_vote_response(true));
        assert!(engine.is_leader());
    }

    #[test]
    fn test_request_vote_granted() {
        let config = ConsensusConfig::default();
        let engine = ConsensusEngine::new(config).unwrap();
        let granted = engine.handle_request_vote("candidate-1", 1, 0);
        assert!(granted);
    }

    #[test]
    fn test_request_vote_rejects_stale_term() {
        let config = ConsensusConfig::default();
        let engine = ConsensusEngine::new(config).unwrap();
        engine.current_term.store(5, Ordering::SeqCst);
        let granted = engine.handle_request_vote("candidate-1", 3, 0);
        assert!(!granted);
    }

    #[test]
    fn test_handle_heartbeat() {
        let config = ConsensusConfig::default();
        let engine = ConsensusEngine::new(config).unwrap();
        let accepted = engine.handle_append_entries("leader-1", 1, vec!["entry-1".into()]);
        assert!(accepted);
        assert_eq!(engine.leader_id(), Some("leader-1".into()));
    }

    #[test]
    fn test_step_down() {
        let config = ConsensusConfig::default();
        let engine = ConsensusEngine::new(config).unwrap();
        engine.start_election();
        engine.handle_vote_response(true);
        assert!(engine.is_leader());
        engine.step_down();
        assert_eq!(engine.role(), NodeRole::Follower);
    }
}
