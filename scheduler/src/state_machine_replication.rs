// State Machine Replication: RPC-based log replication between nodes
// Implements leader-driven replication with heartbeats

use crate::state_machine_coordinator::StateMachineCoordinator;
use crate::replicated_log::LogEntry;
use std::sync::Arc;
use parking_lot::Mutex;
use anyhow::{anyhow, Result};
use tracing::debug;
use std::collections::HashMap;

/// Replication state for a follower
#[derive(Clone, Debug)]
pub struct FollowerState {
    pub follower_id: String,
    pub next_lsn: u64,    // Next LSN to send to this follower
    pub match_lsn: u64,   // Highest LSN known to be replicated
    pub heartbeat_at_ms: u64,
}

impl FollowerState {
    pub fn new(follower_id: String) -> Self {
        Self {
            follower_id,
            next_lsn: 1,
            match_lsn: 0,
            heartbeat_at_ms: chrono::Utc::now().timestamp_millis() as u64,
        }
    }
}

/// State Machine Replication Manager
pub struct StateMachineReplication {
    coordinator: Arc<StateMachineCoordinator>,
    followers: Arc<Mutex<HashMap<String, FollowerState>>>,
}

impl StateMachineReplication {
    /// Create new replication manager
    pub fn new(coordinator: Arc<StateMachineCoordinator>) -> Self {
        Self {
            coordinator,
            followers: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    /// Register a follower node
    pub fn register_follower(&self, follower_id: &str) -> Result<()> {
        let mut followers = self.followers.lock();
        if followers.contains_key(follower_id) {
            return Err(anyhow!("Follower already registered: {}", follower_id));
        }

        followers.insert(
            follower_id.to_string(),
            FollowerState::new(follower_id.to_string()),
        );

        debug!("Registered follower: {}", follower_id);
        Ok(())
    }

    /// Unregister a follower
    pub fn unregister_follower(&self, follower_id: &str) -> Result<()> {
        let mut followers = self.followers.lock();
        followers.remove(follower_id).ok_or_else(|| {
            anyhow!("Follower not found: {}", follower_id)
        })?;

        debug!("Unregistered follower: {}", follower_id);
        Ok(())
    }

    /// Get follower state
    pub fn get_follower(&self, follower_id: &str) -> Option<FollowerState> {
        self.followers.lock().get(follower_id).cloned()
    }

    /// Get all followers
    pub fn followers(&self) -> Vec<FollowerState> {
        self.followers.lock().values().cloned().collect()
    }

    /// Get next entries to send to follower
    pub fn get_entries_for_follower(&self, follower_id: &str) -> Result<Vec<LogEntry>> {
        let follower = self
            .get_follower(follower_id)
            .ok_or_else(|| anyhow!("Follower not found: {}", follower_id))?;

        let from_lsn = follower.next_lsn;
        let log = self.coordinator.log();
        let last_lsn = log.last_lsn().unwrap_or(0);

        if from_lsn > last_lsn {
            return Ok(vec![]);
        }

        let entries = log.get_range(from_lsn, last_lsn);
        Ok(entries)
    }

    /// Update follower state after successful replication
    pub fn acknowledge_replication(&self, follower_id: &str, replicated_lsn: u64) -> Result<()> {
        let mut followers = self.followers.lock();
        let follower = followers
            .get_mut(follower_id)
            .ok_or_else(|| anyhow!("Follower not found: {}", follower_id))?;

        follower.next_lsn = replicated_lsn + 1;
        follower.match_lsn = replicated_lsn;
        follower.heartbeat_at_ms = chrono::Utc::now().timestamp_millis() as u64;

        debug!(
            "Acknowledged replication: follower={}, match_lsn={}",
            follower_id, replicated_lsn
        );
        Ok(())
    }

    /// Update follower state after failed replication
    pub fn acknowledge_replication_failure(&self, follower_id: &str) -> Result<()> {
        let mut followers = self.followers.lock();
        let follower = followers
            .get_mut(follower_id)
            .ok_or_else(|| anyhow!("Follower not found: {}", follower_id))?;

        // Back off
        if follower.next_lsn > 1 {
            follower.next_lsn -= 1;
        }

        debug!(
            "Acknowledged replication failure: follower={}, next_lsn={}",
            follower_id, follower.next_lsn
        );
        Ok(())
    }

    /// Check if majority of followers have replicated up to LSN
    pub fn has_quorum_replication(&self, lsn: u64) -> bool {
        let followers = self.followers.lock();
        let total_nodes = followers.len() + 1; // +1 for leader
        let replicated_count = 1 + followers.values().filter(|f| f.match_lsn >= lsn).count();

        let quorum_size = (total_nodes / 2) + 1;
        replicated_count >= quorum_size
    }

    /// Get highest LSN replicated to all followers
    pub fn min_match_lsn(&self) -> u64 {
        let followers = self.followers.lock();
        if followers.is_empty() {
            return 0;
        }

        followers
            .values()
            .map(|f| f.match_lsn)
            .min()
            .unwrap_or(0)
    }

    /// Advance commit index based on replication
    pub fn advance_commit_index(&self) -> Result<()> {
        let log = self.coordinator.log();
        let current_commit = log.commit_index();
        let min_match = self.min_match_lsn();

        if min_match > current_commit {
            log.commit(min_match)?;
            debug!("Advanced commit index to {}", min_match);
        }

        Ok(())
    }

    /// Get replication status
    pub fn replication_status(&self) -> ReplicationStatus {
        let followers = self.followers.lock();
        let log = self.coordinator.log();

        let total_followers = followers.len();
        let replicated_count = followers
            .values()
            .filter(|f| f.match_lsn >= log.commit_index())
            .count();

        ReplicationStatus {
            total_followers,
            replicated_followers: replicated_count,
            last_lsn: log.last_lsn().unwrap_or(0),
            commit_index: log.commit_index(),
            min_match_lsn: self.min_match_lsn(),
        }
    }

    /// Check if heartbeat is needed for follower
    pub fn needs_heartbeat(&self, follower_id: &str, heartbeat_interval_ms: u64) -> bool {
        if let Some(follower) = self.get_follower(follower_id) {
            let now = chrono::Utc::now().timestamp_millis() as u64;
            now - follower.heartbeat_at_ms >= heartbeat_interval_ms
        } else {
            false
        }
    }
}

/// Replication status snapshot
#[derive(Clone, Debug)]
pub struct ReplicationStatus {
    pub total_followers: usize,
    pub replicated_followers: usize,
    pub last_lsn: u64,
    pub commit_index: u64,
    pub min_match_lsn: u64,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::consensus::QuorumConfig;

    fn create_coordinator() -> Arc<StateMachineCoordinator> {
        let config = QuorumConfig::new("node-1", vec![
            "node-1".to_string(),
            "node-2".to_string(),
            "node-3".to_string(),
        ]);
        Arc::new(StateMachineCoordinator::new(config, 100))
    }

    #[test]
    fn test_replication_manager_creation() {
        let coordinator = create_coordinator();
        let replication = StateMachineReplication::new(coordinator);

        assert_eq!(replication.followers().len(), 0);
    }

    #[test]
    fn test_register_follower() {
        let coordinator = create_coordinator();
        let replication = StateMachineReplication::new(coordinator);

        assert!(replication.register_follower("node-2").is_ok());
        assert_eq!(replication.followers().len(), 1);

        let follower = replication.get_follower("node-2").unwrap();
        assert_eq!(follower.follower_id, "node-2");
        assert_eq!(follower.next_lsn, 1);
        assert_eq!(follower.match_lsn, 0);
    }

    #[test]
    fn test_duplicate_follower_registration() {
        let coordinator = create_coordinator();
        let replication = StateMachineReplication::new(coordinator);

        assert!(replication.register_follower("node-2").is_ok());
        assert!(replication.register_follower("node-2").is_err());
    }

    #[test]
    fn test_unregister_follower() {
        let coordinator = create_coordinator();
        let replication = StateMachineReplication::new(coordinator);

        replication.register_follower("node-2").ok();
        assert_eq!(replication.followers().len(), 1);

        assert!(replication.unregister_follower("node-2").is_ok());
        assert_eq!(replication.followers().len(), 0);
    }

    #[test]
    fn test_acknowledge_replication() {
        let coordinator = create_coordinator();
        let replication = StateMachineReplication::new(coordinator);

        replication.register_follower("node-2").ok();
        assert!(replication.acknowledge_replication("node-2", 5).is_ok());

        let follower = replication.get_follower("node-2").unwrap();
        assert_eq!(follower.next_lsn, 6);
        assert_eq!(follower.match_lsn, 5);
    }

    #[test]
    fn test_acknowledge_replication_failure() {
        let coordinator = create_coordinator();
        let replication = StateMachineReplication::new(coordinator);

        replication.register_follower("node-2").ok();
        replication.acknowledge_replication("node-2", 5).ok();

        assert!(replication.acknowledge_replication_failure("node-2").is_ok());

        let follower = replication.get_follower("node-2").unwrap();
        assert_eq!(follower.next_lsn, 5); // Backed off
    }

    #[test]
    fn test_quorum_replication() {
        let coordinator = create_coordinator();
        let replication = StateMachineReplication::new(coordinator);

        // Register 2 followers (3 nodes total)
        replication.register_follower("node-2").ok();
        replication.register_follower("node-3").ok();

        // No quorum yet
        assert!(!replication.has_quorum_replication(1));

        // Acknowledge from one follower (2/3 = quorum)
        replication.acknowledge_replication("node-2", 1).ok();
        assert!(replication.has_quorum_replication(1));
    }

    #[test]
    fn test_min_match_lsn() {
        let coordinator = create_coordinator();
        let replication = StateMachineReplication::new(coordinator);

        replication.register_follower("node-2").ok();
        replication.register_follower("node-3").ok();

        replication.acknowledge_replication("node-2", 10).ok();
        replication.acknowledge_replication("node-3", 5).ok();

        assert_eq!(replication.min_match_lsn(), 5);
    }

    #[test]
    fn test_advance_commit_index() {
        let coordinator = create_coordinator();
        let replication = StateMachineReplication::new(coordinator.clone());

        // Register follower
        replication.register_follower("node-2").ok();

        // Simulate entries in log
        let log = coordinator.log();
        for i in 1..=5 {
            let entry = crate::replicated_log::LogEntry::new(
                i,
                1,
                crate::replicated_log::LogOperation::Allocate {
                    request_id: format!("req-{}", i),
                    num_blocks: 10,
                },
            );
            log.append(entry).ok();
        }

        // Acknowledge replication
        replication.acknowledge_replication("node-2", 5).ok();

        // Advance commit
        assert!(replication.advance_commit_index().is_ok());
        assert_eq!(log.commit_index(), 5);
    }

    #[test]
    fn test_replication_status() {
        let coordinator = create_coordinator();
        let replication = StateMachineReplication::new(coordinator.clone());

        replication.register_follower("node-2").ok();
        replication.register_follower("node-3").ok();

        replication.acknowledge_replication("node-2", 3).ok();
        replication.acknowledge_replication("node-3", 2).ok();

        let status = replication.replication_status();
        assert_eq!(status.total_followers, 2);
        assert_eq!(status.min_match_lsn, 2);
    }

    #[test]
    fn test_needs_heartbeat() {
        let coordinator = create_coordinator();
        let replication = StateMachineReplication::new(coordinator);

        replication.register_follower("node-2").ok();

        // Should need heartbeat immediately
        assert!(replication.needs_heartbeat("node-2", 100));

        // After acknowledgment, still might need it (depends on time)
        replication.acknowledge_replication("node-2", 1).ok();
        // In tests, time passes quickly, so likely still needs it
        assert!(replication.needs_heartbeat("node-2", 1000)); // 1 second interval
    }
}
