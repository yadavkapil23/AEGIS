// State machine replication: Apply log entries to system state
// Ensures all nodes reach consistent state

use crate::replicated_log::{LogOperation, LogEntry};
use std::sync::Arc;
use parking_lot::Mutex;
use anyhow::Result;
use tracing::debug;

/// State machine operation result
#[derive(Clone, Debug, PartialEq)]
pub enum OperationResult {
    Success,
    AlreadyApplied,
    Failed(String),
}

/// Allocation tracking for state machine
#[derive(Clone, Debug)]
pub struct AllocationRecord {
    pub request_id: String,
    pub num_blocks: usize,
    pub applied_at: u64,
}

/// Peer registration tracking
#[derive(Clone, Debug)]
pub struct PeerRecord {
    pub peer_id: String,
    pub peer_addr: String,
    pub capacity: usize,
    pub registered_at: u64,
}

/// State machine for replicated operations
pub struct StateMachine {
    allocations: Arc<Mutex<Vec<AllocationRecord>>>,
    peers: Arc<Mutex<Vec<PeerRecord>>>,
    applied_count: Arc<Mutex<u64>>,
}

impl StateMachine {
    /// Create new state machine
    pub fn new() -> Self {
        Self {
            allocations: Arc::new(Mutex::new(Vec::new())),
            peers: Arc::new(Mutex::new(Vec::new())),
            applied_count: Arc::new(Mutex::new(0)),
        }
    }

    /// Apply log entry to state machine
    pub fn apply_entry(&self, entry: &LogEntry) -> Result<OperationResult> {
        match &entry.operation {
            LogOperation::Allocate {
                request_id,
                num_blocks,
            } => {
                // Check if already applied
                let allocations = self.allocations.lock();
                if allocations.iter().any(|a| a.request_id == *request_id) {
                    return Ok(OperationResult::AlreadyApplied);
                }
                drop(allocations);

                // Record allocation
                let record = AllocationRecord {
                    request_id: request_id.clone(),
                    num_blocks: *num_blocks,
                    applied_at: chrono::Utc::now().timestamp_millis() as u64,
                };

                self.allocations.lock().push(record);
                *self.applied_count.lock() += 1;

                debug!(
                    "Applied allocation: request_id={}, blocks={}",
                    request_id, num_blocks
                );
                Ok(OperationResult::Success)
            }

            LogOperation::Deallocate {
                request_id,
                blocks,
            } => {
                // Find and remove allocation
                let mut allocations = self.allocations.lock();
                if let Some(pos) = allocations.iter().position(|a| a.request_id == *request_id) {
                    allocations.remove(pos);
                    *self.applied_count.lock() += 1;

                    debug!(
                        "Applied deallocation: request_id={}, blocks={}",
                        request_id,
                        blocks.len()
                    );
                    Ok(OperationResult::Success)
                } else {
                    Ok(OperationResult::AlreadyApplied)
                }
            }

            LogOperation::RegisterPeer {
                peer_id,
                peer_addr,
                capacity,
            } => {
                // Check if already registered
                let peers = self.peers.lock();
                if peers.iter().any(|p| p.peer_id == *peer_id) {
                    return Ok(OperationResult::AlreadyApplied);
                }
                drop(peers);

                // Register peer
                let record = PeerRecord {
                    peer_id: peer_id.clone(),
                    peer_addr: peer_addr.clone(),
                    capacity: *capacity,
                    registered_at: chrono::Utc::now().timestamp_millis() as u64,
                };

                self.peers.lock().push(record);
                *self.applied_count.lock() += 1;

                debug!("Applied peer registration: peer_id={}", peer_id);
                Ok(OperationResult::Success)
            }
        }
    }

    /// Get allocation record
    pub fn get_allocation(&self, request_id: &str) -> Option<AllocationRecord> {
        self.allocations
            .lock()
            .iter()
            .find(|a| a.request_id == request_id)
            .cloned()
    }

    /// Get all allocations
    pub fn allocations(&self) -> Vec<AllocationRecord> {
        self.allocations.lock().clone()
    }

    /// Get peer record
    pub fn get_peer(&self, peer_id: &str) -> Option<PeerRecord> {
        self.peers
            .lock()
            .iter()
            .find(|p| p.peer_id == peer_id)
            .cloned()
    }

    /// Get all peers
    pub fn peers(&self) -> Vec<PeerRecord> {
        self.peers.lock().clone()
    }

    /// Get applied count
    pub fn applied_count(&self) -> u64 {
        *self.applied_count.lock()
    }

    /// Clear state (for reset/recovery)
    pub fn clear(&self) {
        self.allocations.lock().clear();
        self.peers.lock().clear();
        *self.applied_count.lock() = 0;
    }

    /// Get state hash for consistency checking
    pub fn state_hash(&self) -> blake3::Hash {
        let mut hasher = blake3::Hasher::new();

        // Hash allocations
        for alloc in self.allocations.lock().iter() {
            hasher.update(alloc.request_id.as_bytes());
            hasher.update(&alloc.num_blocks.to_le_bytes());
        }

        // Hash peers
        for peer in self.peers.lock().iter() {
            hasher.update(peer.peer_id.as_bytes());
            hasher.update(peer.peer_addr.as_bytes());
            hasher.update(&peer.capacity.to_le_bytes());
        }

        // Hash applied count
        hasher.update(&self.applied_count.lock().to_le_bytes());

        hasher.finalize()
    }
}

impl Default for StateMachine {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_state_machine_creation() {
        let sm = StateMachine::new();
        assert_eq!(sm.applied_count(), 0);
        assert_eq!(sm.allocations().len(), 0);
        assert_eq!(sm.peers().len(), 0);
    }

    #[test]
    fn test_apply_allocation() {
        let sm = StateMachine::new();
        let op = LogOperation::Allocate {
            request_id: "req-1".to_string(),
            num_blocks: 100,
        };
        let entry = LogEntry::new(1, 1, op);

        let result = sm.apply_entry(&entry).unwrap();
        assert_eq!(result, OperationResult::Success);
        assert_eq!(sm.applied_count(), 1);
        assert_eq!(sm.allocations().len(), 1);

        let alloc = sm.get_allocation("req-1").unwrap();
        assert_eq!(alloc.request_id, "req-1");
        assert_eq!(alloc.num_blocks, 100);
    }

    #[test]
    fn test_idempotent_allocation() {
        let sm = StateMachine::new();
        let op = LogOperation::Allocate {
            request_id: "req-1".to_string(),
            num_blocks: 100,
        };
        let entry = LogEntry::new(1, 1, op);

        // Apply first time
        let result1 = sm.apply_entry(&entry).unwrap();
        assert_eq!(result1, OperationResult::Success);

        // Apply again
        let result2 = sm.apply_entry(&entry).unwrap();
        assert_eq!(result2, OperationResult::AlreadyApplied);

        // Should only have one allocation
        assert_eq!(sm.applied_count(), 1);
        assert_eq!(sm.allocations().len(), 1);
    }

    #[test]
    fn test_apply_deallocation() {
        let sm = StateMachine::new();

        // First allocate
        let alloc_op = LogOperation::Allocate {
            request_id: "req-1".to_string(),
            num_blocks: 100,
        };
        let alloc_entry = LogEntry::new(1, 1, alloc_op);
        sm.apply_entry(&alloc_entry).ok();

        assert_eq!(sm.allocations().len(), 1);

        // Then deallocate
        let dealloc_op = LogOperation::Deallocate {
            request_id: "req-1".to_string(),
            blocks: vec![0, 1, 2],
        };
        let dealloc_entry = LogEntry::new(2, 1, dealloc_op);
        let result = sm.apply_entry(&dealloc_entry).unwrap();

        assert_eq!(result, OperationResult::Success);
        assert_eq!(sm.allocations().len(), 0);
        assert_eq!(sm.applied_count(), 2);
    }

    #[test]
    fn test_register_peer() {
        let sm = StateMachine::new();
        let op = LogOperation::RegisterPeer {
            peer_id: "node-2".to_string(),
            peer_addr: "localhost:50052".to_string(),
            capacity: 1024,
        };
        let entry = LogEntry::new(1, 1, op);

        let result = sm.apply_entry(&entry).unwrap();
        assert_eq!(result, OperationResult::Success);
        assert_eq!(sm.peers().len(), 1);

        let peer = sm.get_peer("node-2").unwrap();
        assert_eq!(peer.peer_id, "node-2");
        assert_eq!(peer.peer_addr, "localhost:50052");
        assert_eq!(peer.capacity, 1024);
    }

    #[test]
    fn test_multiple_operations() {
        let sm = StateMachine::new();

        // Add 3 allocations
        for i in 1..=3 {
            let op = LogOperation::Allocate {
                request_id: format!("req-{}", i),
                num_blocks: i * 10,
            };
            let entry = LogEntry::new(i as u64, 1, op);
            sm.apply_entry(&entry).ok();
        }

        // Add 2 peers
        for i in 1..=2 {
            let op = LogOperation::RegisterPeer {
                peer_id: format!("node-{}", i + 1),
                peer_addr: format!("localhost:5005{}", i),
                capacity: 512,
            };
            let entry = LogEntry::new((3 + i) as u64, 1, op);
            sm.apply_entry(&entry).ok();
        }

        assert_eq!(sm.allocations().len(), 3);
        assert_eq!(sm.peers().len(), 2);
        assert_eq!(sm.applied_count(), 5);
    }

    #[test]
    fn test_state_hash_consistency() {
        let sm1 = StateMachine::new();
        let sm2 = StateMachine::new();

        // Apply same operations to both
        let op = LogOperation::Allocate {
            request_id: "req-1".to_string(),
            num_blocks: 100,
        };
        let entry = LogEntry::new(1, 1, op);

        sm1.apply_entry(&entry).ok();
        sm2.apply_entry(&entry).ok();

        // Hashes should match
        let hash1 = sm1.state_hash();
        let hash2 = sm2.state_hash();
        assert_eq!(hash1, hash2);
    }

    #[test]
    fn test_state_hash_differs_with_different_state() {
        let sm1 = StateMachine::new();
        let sm2 = StateMachine::new();

        // Apply to sm1
        let op1 = LogOperation::Allocate {
            request_id: "req-1".to_string(),
            num_blocks: 100,
        };
        let entry1 = LogEntry::new(1, 1, op1);
        sm1.apply_entry(&entry1).ok();

        // Apply different to sm2
        let op2 = LogOperation::Allocate {
            request_id: "req-2".to_string(),
            num_blocks: 200,
        };
        let entry2 = LogEntry::new(1, 1, op2);
        sm2.apply_entry(&entry2).ok();

        // Hashes should differ
        let hash1 = sm1.state_hash();
        let hash2 = sm2.state_hash();
        assert_ne!(hash1, hash2);
    }

    #[test]
    fn test_clear_state() {
        let sm = StateMachine::new();

        // Add some operations
        let op = LogOperation::Allocate {
            request_id: "req-1".to_string(),
            num_blocks: 100,
        };
        let entry = LogEntry::new(1, 1, op);
        sm.apply_entry(&entry).ok();

        assert!(sm.applied_count() > 0);

        // Clear
        sm.clear();

        assert_eq!(sm.applied_count(), 0);
        assert_eq!(sm.allocations().len(), 0);
        assert_eq!(sm.peers().len(), 0);
    }
}
