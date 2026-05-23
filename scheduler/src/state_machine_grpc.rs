// gRPC service for state machine replication
// Handles RPC communication between consensus nodes

use crate::state_machine_coordinator::StateMachineCoordinator;
use crate::state_machine_replication::StateMachineReplication;
use crate::consensus::Vote;
use crate::replicated_log::LogEntry;
use std::sync::Arc;
use parking_lot::Mutex;
use anyhow::{anyhow, Result};
use tracing::{debug, info, warn};

/// RPC request types
#[derive(Clone, Debug)]
pub struct ReplicateEntriesRequest {
    pub leader_id: String,
    pub term: u64,
    pub entries: Vec<LogEntry>,
    pub leader_commit: u64,
}

#[derive(Clone, Debug)]
pub struct ReplicateEntriesResponse {
    pub follower_id: String,
    pub term: u64,
    pub success: bool,
    pub last_lsn: u64,
}

#[derive(Clone, Debug)]
pub struct RequestVoteRequest {
    pub candidate_id: String,
    pub term: u64,
    pub last_log_lsn: Option<u64>,
    pub last_log_term: u64,
}

#[derive(Clone, Debug)]
pub struct RequestVoteResponse {
    pub voter_id: String,
    pub term: u64,
    pub vote_granted: bool,
}

#[derive(Clone, Debug)]
pub struct AppendEntriesRequest {
    pub leader_id: String,
    pub term: u64,
    pub prev_log_lsn: u64,
    pub prev_log_term: u64,
    pub entries: Vec<LogEntry>,
    pub leader_commit: u64,
}

#[derive(Clone, Debug)]
pub struct AppendEntriesResponse {
    pub follower_id: String,
    pub term: u64,
    pub success: bool,
    pub match_lsn: u64,
}

/// gRPC Service Handler for State Machine Replication
pub struct StateMachineGrpcService {
    coordinator: Arc<StateMachineCoordinator>,
    replication: Arc<StateMachineReplication>,
}

impl StateMachineGrpcService {
    /// Create new gRPC service
    pub fn new(
        coordinator: Arc<StateMachineCoordinator>,
        replication: Arc<StateMachineReplication>,
    ) -> Self {
        Self {
            coordinator,
            replication,
        }
    }

    /// Handle ReplicateEntries RPC (deprecated, use AppendEntries instead)
    pub fn replicate_entries(&self, req: ReplicateEntriesRequest) -> Result<ReplicateEntriesResponse> {
        debug!(
            "ReplicateEntries from {}: {} entries at term {}",
            req.leader_id,
            req.entries.len(),
            req.term
        );

        let my_id = self.coordinator.consensus().node_id().clone();

        // Append entries to log
        let mut last_lsn = 0;
        for entry in req.entries {
            last_lsn = entry.lsn;
            self.coordinator.log().append(entry).ok();
        }

        // Update commit index if leader has committed further
        if req.leader_commit > self.coordinator.commit_index() {
            let new_commit = std::cmp::min(req.leader_commit, last_lsn);
            self.coordinator.log().commit(new_commit).ok();
            self.coordinator.apply_pending().ok();
        }

        Ok(ReplicateEntriesResponse {
            follower_id: my_id,
            term: self.coordinator.current_term(),
            success: true,
            last_lsn,
        })
    }

    /// Handle RequestVote RPC (for leader election)
    pub fn request_vote(&self, req: RequestVoteRequest) -> Result<RequestVoteResponse> {
        debug!(
            "RequestVote from {} for term {}",
            req.candidate_id, req.term
        );

        let my_id = self.coordinator.consensus().node_id().clone();
        let current_term = self.coordinator.current_term();

        // If request term is older, reject
        if req.term < current_term {
            return Ok(RequestVoteResponse {
                voter_id: my_id,
                term: current_term,
                vote_granted: false,
            });
        }

        // If request term is newer, update our term
        if req.term > current_term {
            // Step down if leader
            if self.coordinator.is_leader() {
                self.coordinator.consensus().become_follower().ok();
            }
        }

        // Record vote for candidate
        let vote_result = self.coordinator.consensus().receive_vote(&req.candidate_id, Vote::Yes);

        Ok(RequestVoteResponse {
            voter_id: my_id,
            term: self.coordinator.current_term(),
            vote_granted: vote_result.is_ok(),
        })
    }

    /// Handle AppendEntries RPC (heartbeat + log replication)
    pub fn append_entries(&self, req: AppendEntriesRequest) -> Result<AppendEntriesResponse> {
        debug!(
            "AppendEntries from {}: {} entries, commit={}",
            req.leader_id,
            req.entries.len(),
            req.leader_commit
        );

        let my_id = self.coordinator.consensus().node_id().clone();
        let current_term = self.coordinator.current_term();

        // If request term is older, reject
        if req.term < current_term {
            return Ok(AppendEntriesResponse {
                follower_id: my_id,
                term: current_term,
                success: false,
                match_lsn: 0,
            });
        }

        // If request term is newer, update term and become follower
        if req.term > current_term {
            if self.coordinator.is_leader() {
                self.coordinator.consensus().become_follower().ok();
            }
        }

        // Update heartbeat
        self.coordinator.consensus().heartbeat_received();

        // Check log consistency (prev_log_lsn)
        if req.prev_log_lsn > 0 {
            if self.coordinator.log().get(req.prev_log_lsn).is_none() {
                // Log mismatch - ask leader to send earlier entries
                return Ok(AppendEntriesResponse {
                    follower_id: my_id,
                    term: current_term,
                    success: false,
                    match_lsn: 0,
                });
            }
        }

        // Append entries
        let mut last_lsn = req.prev_log_lsn;
        for entry in req.entries {
            last_lsn = entry.lsn;
            self.coordinator.log().append(entry).ok();
        }

        // Update commit index
        if req.leader_commit > self.coordinator.commit_index() {
            let new_commit = std::cmp::min(req.leader_commit, last_lsn);
            self.coordinator.log().commit(new_commit).ok();
            self.coordinator.apply_pending().ok();
        }

        Ok(AppendEntriesResponse {
            follower_id: my_id,
            term: current_term,
            success: true,
            match_lsn: last_lsn,
        })
    }

    /// Handle leader allocation request (leader only)
    pub fn allocate(&self, request_id: &str, num_blocks: usize) -> Result<u64> {
        if !self.coordinator.is_leader() {
            return Err(anyhow!("Not leader"));
        }

        self.coordinator.allocate(request_id, num_blocks)
    }

    /// Handle leader deallocation request (leader only)
    pub fn deallocate(&self, request_id: &str, blocks: Vec<usize>) -> Result<u64> {
        if !self.coordinator.is_leader() {
            return Err(anyhow!("Not leader"));
        }

        self.coordinator.deallocate(request_id, blocks)
    }

    /// Get current state (for debugging)
    pub fn get_state_info(&self) -> StateInfo {
        StateInfo {
            node_id: self.coordinator.consensus().node_id().clone(),
            is_leader: self.coordinator.is_leader(),
            current_term: self.coordinator.current_term(),
            log_len: self.coordinator.log_len(),
            commit_index: self.coordinator.commit_index(),
            last_applied: self.coordinator.last_applied(),
            applied_count: self.coordinator.applied_count(),
            state_hash: self.coordinator.state_hash().to_hex().to_string(),
        }
    }

    /// Get replication status (leader only)
    pub fn get_replication_status(&self) -> Option<crate::state_machine_replication::ReplicationStatus> {
        if self.coordinator.is_leader() {
            Some(self.replication.replication_status())
        } else {
            None
        }
    }

    // Internal accessors for testing
    pub fn coordinator(&self) -> Arc<StateMachineCoordinator> {
        self.coordinator.clone()
    }

    pub fn replication(&self) -> Arc<StateMachineReplication> {
        self.replication.clone()
    }
}

/// State information snapshot
#[derive(Clone, Debug)]
pub struct StateInfo {
    pub node_id: String,
    pub is_leader: bool,
    pub current_term: u64,
    pub log_len: usize,
    pub commit_index: u64,
    pub last_applied: u64,
    pub applied_count: u64,
    pub state_hash: String,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::consensus::QuorumConfig;

    fn create_service() -> StateMachineGrpcService {
        let config = QuorumConfig::new("node-1", vec![
            "node-1".to_string(),
            "node-2".to_string(),
            "node-3".to_string(),
        ]);
        let coordinator = Arc::new(StateMachineCoordinator::new(config, 100));
        let replication = Arc::new(StateMachineReplication::new(coordinator.clone()));

        StateMachineGrpcService::new(coordinator, replication)
    }

    #[test]
    fn test_service_creation() {
        let service = create_service();
        assert_eq!(service.coordinator().consensus().config().node_id, "node-1");
    }

    #[test]
    fn test_replicate_entries() {
        let service = create_service();

        let entry = LogEntry::new(
            1,
            1,
            crate::replicated_log::LogOperation::Allocate {
                request_id: "req-1".to_string(),
                num_blocks: 100,
            },
        );

        let req = ReplicateEntriesRequest {
            leader_id: "leader".to_string(),
            term: 1,
            entries: vec![entry],
            leader_commit: 1,
        };

        let resp = service.replicate_entries(req).unwrap();
        assert!(resp.success);
        assert_eq!(resp.last_lsn, 1);
    }

    #[test]
    fn test_request_vote() {
        let service = create_service();

        let req = RequestVoteRequest {
            candidate_id: "node-2".to_string(),
            term: 1,
            last_log_lsn: None,
            last_log_term: 0,
        };

        let resp = service.request_vote(req).unwrap();
        assert!(resp.vote_granted);
    }

    #[test]
    fn test_append_entries_heartbeat() {
        let service = create_service();

        let req = AppendEntriesRequest {
            leader_id: "leader".to_string(),
            term: 1,
            prev_log_lsn: 0,
            prev_log_term: 0,
            entries: vec![],
            leader_commit: 0,
        };

        let resp = service.append_entries(req).unwrap();
        assert!(resp.success);
    }

    #[test]
    fn test_allocate_requires_leader() {
        let service = create_service();

        // Not leader, should fail
        let result = service.allocate("req-1", 100);
        assert!(result.is_err());
    }

    #[test]
    fn test_allocate_as_leader() {
        let service = create_service();

        // Become leader
        service.coordinator().consensus().request_votes().ok();
        service
            .coordinator()
            .consensus()
            .receive_vote("node-2", crate::consensus::Vote::Yes)
            .ok();
        service.coordinator().consensus().check_election_won();

        // Now allocate should work
        let result = service.allocate("req-1", 100);
        assert!(result.is_ok());
    }

    #[test]
    fn test_get_state_info() {
        let service = create_service();

        let info = service.get_state_info();
        assert_eq!(info.node_id, "node-1");
        assert!(!info.is_leader);
        assert_eq!(info.log_len, 0);
    }

    #[test]
    fn test_append_entries_with_log_entries() {
        let service = create_service();

        let entry = LogEntry::new(
            1,
            1,
            crate::replicated_log::LogOperation::Allocate {
                request_id: "req-1".to_string(),
                num_blocks: 50,
            },
        );

        let req = AppendEntriesRequest {
            leader_id: "leader".to_string(),
            term: 1,
            prev_log_lsn: 0,
            prev_log_term: 0,
            entries: vec![entry],
            leader_commit: 1,
        };

        let resp = service.append_entries(req).unwrap();
        assert!(resp.success);
        assert_eq!(resp.match_lsn, 1);
        assert_eq!(service.coordinator().log_len(), 1);
    }

    #[test]
    fn test_append_entries_commit_index_update() {
        let service = create_service();

        // Add an entry
        let entry = LogEntry::new(
            1,
            1,
            crate::replicated_log::LogOperation::Allocate {
                request_id: "req-1".to_string(),
                num_blocks: 100,
            },
        );

        let req = AppendEntriesRequest {
            leader_id: "leader".to_string(),
            term: 1,
            prev_log_lsn: 0,
            prev_log_term: 0,
            entries: vec![entry],
            leader_commit: 1,
        };

        service.append_entries(req).unwrap();

        // Verify commit index updated
        assert_eq!(service.coordinator().commit_index(), 1);
    }

    #[test]
    fn test_replicate_entries_idempotent() {
        let service = create_service();

        let entry = LogEntry::new(
            1,
            1,
            crate::replicated_log::LogOperation::Allocate {
                request_id: "req-1".to_string(),
                num_blocks: 100,
            },
        );

        let req = ReplicateEntriesRequest {
            leader_id: "leader".to_string(),
            term: 1,
            entries: vec![entry],
            leader_commit: 1,
        };

        // First call
        service.replicate_entries(req.clone()).unwrap();
        assert_eq!(service.coordinator().log_len(), 1);

        // Second call (idempotent)
        service.replicate_entries(req).unwrap();
        assert_eq!(service.coordinator().log_len(), 1); // Still 1, not 2
    }
}
