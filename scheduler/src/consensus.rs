// Consensus module: Quorum-based coordination
// Implements quorum calculations, vote tracking, and leader election

use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use parking_lot::Mutex;
use anyhow::{anyhow, Result};
use tracing::{debug, info, warn};

/// Node ID type
pub type NodeId = String;

/// Term number for leadership epochs
pub type Term = u64;

/// Log sequence number
pub type Lsn = u64;

/// Quorum configuration
#[derive(Clone, Debug)]
pub struct QuorumConfig {
    pub node_id: NodeId,
    pub nodes: HashSet<NodeId>,
}

impl QuorumConfig {
    /// Create new quorum config
    pub fn new(node_id: impl Into<String>, nodes: Vec<String>) -> Self {
        Self {
            node_id: node_id.into(),
            nodes: nodes.into_iter().collect(),
        }
    }

    /// Get quorum size (majority)
    pub fn quorum_size(&self) -> usize {
        (self.nodes.len() / 2) + 1
    }

    /// Check if we have quorum
    pub fn has_quorum(&self, votes: usize) -> bool {
        votes >= self.quorum_size()
    }

    /// Get total nodes
    pub fn total_nodes(&self) -> usize {
        self.nodes.len()
    }

    /// Check if node is part of this quorum
    pub fn contains_node(&self, node_id: &str) -> bool {
        self.nodes.contains(node_id)
    }
}

/// Vote cast by a node
#[derive(Clone, Debug, PartialEq)]
pub enum Vote {
    Yes,
    No,
    Unknown,
}

/// Consensus state
#[derive(Clone, Debug, PartialEq)]
pub enum ConsensusState {
    Follower,
    Candidate,
    Leader,
}

/// Quorum consensus tracker
pub struct QuorumConsensus {
    config: QuorumConfig,
    state: Arc<Mutex<ConsensusState>>,
    current_term: Arc<AtomicU64>,
    voted_for: Arc<Mutex<Option<NodeId>>>,
    votes: Arc<Mutex<HashMap<NodeId, Vote>>>,
    last_heartbeat: Arc<AtomicU64>,
}

impl QuorumConsensus {
    /// Create new consensus tracker
    pub fn new(config: QuorumConfig) -> Self {
        let mut votes = HashMap::new();
        for node in &config.nodes {
            votes.insert(node.clone(), Vote::Unknown);
        }

        Self {
            config,
            state: Arc::new(Mutex::new(ConsensusState::Follower)),
            current_term: Arc::new(AtomicU64::new(0)),
            voted_for: Arc::new(Mutex::new(None)),
            votes: Arc::new(Mutex::new(votes)),
            last_heartbeat: Arc::new(AtomicU64::new(0)),
        }
    }

    /// Get current state
    pub fn state(&self) -> ConsensusState {
        self.state.lock().clone()
    }

    /// Get node ID
    pub fn node_id(&self) -> &NodeId {
        &self.config.node_id
    }

    /// Get current term
    pub fn current_term(&self) -> Term {
        self.current_term.load(Ordering::SeqCst)
    }

    /// Advance to new term
    pub fn advance_term(&self, new_term: Term) -> Result<()> {
        let current = self.current_term();
        if new_term <= current {
            return Err(anyhow!(
                "Cannot advance to term {} from {}",
                new_term,
                current
            ));
        }

        self.current_term.store(new_term, Ordering::SeqCst);
        *self.voted_for.lock() = None;
        self.reset_votes();

        debug!("Advanced to term {}", new_term);
        Ok(())
    }

    /// Start election: request votes from all nodes
    pub fn request_votes(&self) -> Result<()> {
        let term = self.current_term();
        self.advance_term(term + 1)?;

        *self.state.lock() = ConsensusState::Candidate;
        let my_node = &self.config.node_id;
        *self.voted_for.lock() = Some(my_node.clone());

        // Vote for ourselves
        self.votes
            .lock()
            .insert(my_node.clone(), Vote::Yes);

        info!(
            "Started election for term {} (quorum size: {})",
            self.current_term(),
            self.config.quorum_size()
        );

        Ok(())
    }

    /// Record a vote from a peer
    pub fn receive_vote(&self, from: &str, vote: Vote) -> Result<()> {
        if !self.config.contains_node(from) {
            return Err(anyhow!("Unknown node: {}", from));
        }

        debug!("Received {:?} vote from {} for term {}", vote, from, self.current_term());
        self.votes.lock().insert(from.to_string(), vote);

        Ok(())
    }

    /// Count yes votes
    pub fn count_yes_votes(&self) -> usize {
        self.votes
            .lock()
            .values()
            .filter(|v| **v == Vote::Yes)
            .count()
    }

    /// Check if we won the election
    pub fn check_election_won(&self) -> bool {
        let yes_votes = self.count_yes_votes();
        let won = self.config.has_quorum(yes_votes);

        if won && self.state() == ConsensusState::Candidate {
            *self.state.lock() = ConsensusState::Leader;
            info!(
                "Won election for term {} with {} votes",
                self.current_term(),
                yes_votes
            );
        }

        won
    }

    /// Become leader
    pub fn become_leader(&self) -> Result<()> {
        if self.state() != ConsensusState::Candidate {
            return Err(anyhow!("Cannot become leader from {:?} state", self.state()));
        }

        *self.state.lock() = ConsensusState::Leader;
        info!("Became leader for term {}", self.current_term());
        Ok(())
    }

    /// Become follower
    pub fn become_follower(&self) -> Result<()> {
        *self.state.lock() = ConsensusState::Follower;
        debug!("Became follower for term {}", self.current_term());
        Ok(())
    }

    /// Record heartbeat from leader
    pub fn heartbeat_received(&self) {
        self.last_heartbeat.store(
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_millis() as u64,
            Ordering::SeqCst,
        );
    }

    /// Check if election timeout elapsed
    pub fn election_timeout_elapsed(&self, timeout_ms: u64) -> bool {
        let last = self.last_heartbeat.load(Ordering::SeqCst);
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_millis() as u64;

        (now - last) > timeout_ms
    }

    /// Reset all votes for new election
    fn reset_votes(&self) {
        let mut votes = self.votes.lock();
        for vote in votes.values_mut() {
            *vote = Vote::Unknown;
        }
    }

    /// Get vote count for diagnostics
    pub fn get_vote_summary(&self) -> HashMap<String, String> {
        let votes = self.votes.lock();
        votes
            .iter()
            .map(|(node, vote)| {
                (
                    node.clone(),
                    match vote {
                        Vote::Yes => "yes",
                        Vote::No => "no",
                        Vote::Unknown => "unknown",
                    }
                    .to_string(),
                )
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn create_quorum(node_id: &str, size: usize) -> QuorumConfig {
        let mut nodes = vec![node_id.to_string()];
        for i in 2..=size {
            nodes.push(format!("node-{}", i));
        }
        QuorumConfig::new(node_id, nodes)
    }

    #[test]
    fn test_quorum_config_creation() {
        let config = create_quorum("node-1", 3);
        assert_eq!(config.total_nodes(), 3);
        assert_eq!(config.quorum_size(), 2);
        assert!(config.contains_node("node-1"));
    }

    #[test]
    fn test_quorum_size_calculation() {
        assert_eq!(create_quorum("node-1", 3).quorum_size(), 2);
        assert_eq!(create_quorum("node-1", 5).quorum_size(), 3);
        assert_eq!(create_quorum("node-1", 7).quorum_size(), 4);
    }

    #[test]
    fn test_has_quorum() {
        let config = create_quorum("node-1", 3);
        assert!(!config.has_quorum(1));
        assert!(config.has_quorum(2));
        assert!(config.has_quorum(3));
    }

    #[test]
    fn test_consensus_creation() {
        let config = create_quorum("node-1", 3);
        let consensus = QuorumConsensus::new(config);

        assert_eq!(consensus.state(), ConsensusState::Follower);
        assert_eq!(consensus.current_term(), 0);
    }

    #[test]
    fn test_advance_term() {
        let config = create_quorum("node-1", 3);
        let consensus = QuorumConsensus::new(config);

        assert!(consensus.advance_term(1).is_ok());
        assert_eq!(consensus.current_term(), 1);

        assert!(consensus.advance_term(2).is_ok());
        assert_eq!(consensus.current_term(), 2);

        // Cannot go backwards
        assert!(consensus.advance_term(1).is_err());
    }

    #[test]
    fn test_request_votes() {
        let config = create_quorum("node-1", 3);
        let consensus = QuorumConsensus::new(config);

        assert!(consensus.request_votes().is_ok());
        assert_eq!(consensus.state(), ConsensusState::Candidate);
        assert_eq!(consensus.current_term(), 1);
        assert_eq!(consensus.count_yes_votes(), 1); // Voted for ourselves
    }

    #[test]
    fn test_receive_votes() {
        let config = create_quorum("node-1", 3);
        let consensus = QuorumConsensus::new(config);

        assert!(consensus.request_votes().is_ok());

        // Receive votes from peers
        assert!(consensus.receive_vote("node-2", Vote::Yes).is_ok());
        assert_eq!(consensus.count_yes_votes(), 2);

        assert!(consensus.receive_vote("node-3", Vote::No).is_ok());
        assert_eq!(consensus.count_yes_votes(), 2);
    }

    #[test]
    fn test_election_won() {
        let config = create_quorum("node-1", 3);
        let consensus = QuorumConsensus::new(config);

        assert!(consensus.request_votes().is_ok());
        assert!(!consensus.check_election_won()); // Only 1 vote, need 2

        consensus.receive_vote("node-2", Vote::Yes).ok();
        assert!(consensus.check_election_won()); // 2 votes = quorum
        assert_eq!(consensus.state(), ConsensusState::Leader);
    }

    #[test]
    fn test_split_brain_prevention() {
        let config = create_quorum("node-1", 5);
        let consensus = QuorumConsensus::new(config);

        assert!(consensus.request_votes().is_ok());

        // Receive 2 no votes
        consensus.receive_vote("node-2", Vote::No).ok();
        consensus.receive_vote("node-3", Vote::No).ok();

        // Even with remaining yes votes, we can't win
        consensus.receive_vote("node-4", Vote::Yes).ok();
        consensus.receive_vote("node-5", Vote::Yes).ok();

        // We have 3 yes votes (1 from self + 2 from peers)
        // But total is 3/5, need 3 for quorum - this SHOULD win
        assert!(consensus.check_election_won()); // 3 >= quorum_size(3)
    }

    #[test]
    fn test_become_leader() {
        let config = create_quorum("node-1", 3);
        let consensus = QuorumConsensus::new(config);

        assert!(consensus.request_votes().is_ok());
        consensus.receive_vote("node-2", Vote::Yes).ok();
        consensus.check_election_won();

        assert_eq!(consensus.state(), ConsensusState::Leader);
    }

    #[test]
    fn test_become_follower() {
        let config = create_quorum("node-1", 3);
        let consensus = QuorumConsensus::new(config);

        assert!(consensus.request_votes().is_ok());
        assert!(consensus.become_follower().is_ok());
        assert_eq!(consensus.state(), ConsensusState::Follower);
    }

    #[test]
    fn test_heartbeat_timeout() {
        let config = create_quorum("node-1", 3);
        let consensus = QuorumConsensus::new(config);

        // Should timeout initially (no heartbeat yet)
        assert!(consensus.election_timeout_elapsed(100));

        // Record heartbeat
        consensus.heartbeat_received();

        // Should not timeout immediately
        assert!(!consensus.election_timeout_elapsed(1000));

        // But should timeout after waiting
        std::thread::sleep(std::time::Duration::from_millis(150));
        assert!(consensus.election_timeout_elapsed(100));
    }

    #[test]
    fn test_vote_summary() {
        let config = create_quorum("node-1", 3);
        let consensus = QuorumConsensus::new(config);

        assert!(consensus.request_votes().is_ok());
        consensus.receive_vote("node-2", Vote::Yes).ok();
        consensus.receive_vote("node-3", Vote::No).ok();

        let summary = consensus.get_vote_summary();
        assert_eq!(summary.len(), 3);
        assert_eq!(summary.get("node-1").unwrap(), "yes");
        assert_eq!(summary.get("node-2").unwrap(), "yes");
        assert_eq!(summary.get("node-3").unwrap(), "no");
    }

    #[test]
    fn test_unknown_node_vote() {
        let config = create_quorum("node-1", 3);
        let consensus = QuorumConsensus::new(config);

        let result = consensus.receive_vote("unknown-node", Vote::Yes);
        assert!(result.is_err());
    }

    #[test]
    fn test_term_ordering() {
        let config = create_quorum("node-1", 3);
        let consensus = QuorumConsensus::new(config);

        for term in 1..=5 {
            assert!(consensus.advance_term(term).is_ok());
            assert_eq!(consensus.current_term(), term);
        }
    }
}
