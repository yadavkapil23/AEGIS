// Replicated log: Durability and consistency across nodes
// Implements log entry replication with commit tracking

use std::collections::VecDeque;
use std::sync::Arc;
use parking_lot::Mutex;
use anyhow::{anyhow, Result};
use tracing::debug;
use serde::{Serialize, Deserialize};

pub use crate::consensus::{Lsn, Term};

/// Log entry operation type
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub enum LogOperation {
    Allocate { request_id: String, num_blocks: usize },
    Deallocate { request_id: String, blocks: Vec<usize> },
    RegisterPeer { peer_id: String, peer_addr: String, capacity: usize },
}

/// Single log entry
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct LogEntry {
    pub lsn: Lsn,
    pub term: Term,
    pub operation: LogOperation,
    pub timestamp_ms: u64,
}

impl LogEntry {
    /// Create new log entry
    pub fn new(lsn: Lsn, term: Term, operation: LogOperation) -> Self {
        Self {
            lsn,
            term,
            operation,
            timestamp_ms: chrono::Utc::now().timestamp_millis() as u64,
        }
    }
}

/// Replicated log state
pub struct ReplicatedLog {
    entries: Arc<Mutex<VecDeque<LogEntry>>>,
    commit_index: Arc<Mutex<Lsn>>,
    last_applied: Arc<Mutex<Lsn>>,
    max_entries: usize,
}

impl ReplicatedLog {
    /// Create new replicated log
    pub fn new(max_entries: usize) -> Self {
        Self {
            entries: Arc::new(Mutex::new(VecDeque::new())),
            commit_index: Arc::new(Mutex::new(0)),
            last_applied: Arc::new(Mutex::new(0)),
            max_entries,
        }
    }

    /// Append entry to log
    pub fn append(&self, entry: LogEntry) -> Result<Lsn> {
        let mut entries = self.entries.lock();

        if entries.len() >= self.max_entries {
            return Err(anyhow!("Log is full"));
        }

        let lsn = entry.lsn;
        entries.push_back(entry);

        debug!("Appended log entry at LSN {}", lsn);
        Ok(lsn)
    }

    /// Get entry at LSN
    pub fn get(&self, lsn: Lsn) -> Option<LogEntry> {
        let entries = self.entries.lock();
        entries
            .iter()
            .find(|e| e.lsn == lsn)
            .cloned()
    }

    /// Get entries in range (inclusive)
    pub fn get_range(&self, from_lsn: Lsn, to_lsn: Lsn) -> Vec<LogEntry> {
        let entries = self.entries.lock();
        entries
            .iter()
            .filter(|e| e.lsn >= from_lsn && e.lsn <= to_lsn)
            .cloned()
            .collect()
    }

    /// Mark entries as committed up to LSN
    pub fn commit(&self, lsn: Lsn) -> Result<()> {
        let current = *self.commit_index.lock();
        if lsn < current {
            return Err(anyhow!("Cannot commit backwards: {} < {}", lsn, current));
        }

        // Verify entry exists
        if !self.entries.lock().iter().any(|e| e.lsn == lsn) {
            return Err(anyhow!("No entry at LSN {}", lsn));
        }

        *self.commit_index.lock() = lsn;
        debug!("Committed log entries up to LSN {}", lsn);
        Ok(())
    }

    /// Get commit index
    pub fn commit_index(&self) -> Lsn {
        *self.commit_index.lock()
    }

    /// Get last applied index
    pub fn last_applied(&self) -> Lsn {
        *self.last_applied.lock()
    }

    /// Mark entries as applied
    pub fn apply(&self, lsn: Lsn) -> Result<()> {
        let current = *self.last_applied.lock();
        if lsn < current {
            return Err(anyhow!("Cannot apply backwards: {} < {}", lsn, current));
        }

        // Verify entry exists
        if !self.entries.lock().iter().any(|e| e.lsn == lsn) {
            return Err(anyhow!("No entry at LSN {}", lsn));
        }

        *self.last_applied.lock() = lsn;
        debug!("Applied log entries up to LSN {}", lsn);
        Ok(())
    }

    /// Get pending entries (committed but not applied)
    pub fn pending_entries(&self) -> Vec<LogEntry> {
        let last_applied = *self.last_applied.lock();
        let commit_index = *self.commit_index.lock();

        let entries = self.entries.lock();
        entries
            .iter()
            .filter(|e| e.lsn > last_applied && e.lsn <= commit_index)
            .cloned()
            .collect()
    }

    /// Get uncommitted entries
    pub fn uncommitted_entries(&self) -> Vec<LogEntry> {
        let commit_index = *self.commit_index.lock();
        let entries = self.entries.lock();
        entries
            .iter()
            .filter(|e| e.lsn > commit_index)
            .cloned()
            .collect()
    }

    /// Get log size
    pub fn len(&self) -> usize {
        self.entries.lock().len()
    }

    /// Get last LSN
    pub fn last_lsn(&self) -> Option<Lsn> {
        self.entries.lock().back().map(|e| e.lsn)
    }

    /// Clear log (for leader election)
    pub fn clear(&self) {
        self.entries.lock().clear();
        *self.commit_index.lock() = 0;
        *self.last_applied.lock() = 0;
    }

    /// Truncate log from LSN (exclusive)
    pub fn truncate_from(&self, lsn: Lsn) -> Result<()> {
        let mut entries = self.entries.lock();
        entries.retain(|e| e.lsn < lsn);

        // Reset pointers if needed
        let commit_index = self.commit_index.lock();
        if *commit_index >= lsn {
            drop(commit_index);
            *self.commit_index.lock() = lsn.saturating_sub(1);
        }

        debug!("Truncated log from LSN {}", lsn);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn create_allocate_op(request_id: &str, blocks: usize) -> LogOperation {
        LogOperation::Allocate {
            request_id: request_id.to_string(),
            num_blocks: blocks,
        }
    }

    #[test]
    fn test_log_creation() {
        let log = ReplicatedLog::new(100);
        assert_eq!(log.len(), 0);
        assert_eq!(log.last_lsn(), None);
    }

    #[test]
    fn test_append_entry() {
        let log = ReplicatedLog::new(100);

        let entry = LogEntry::new(1, 1, create_allocate_op("req-1", 10));
        let lsn = log.append(entry).unwrap();

        assert_eq!(lsn, 1);
        assert_eq!(log.len(), 1);
        assert_eq!(log.last_lsn(), Some(1));
    }

    #[test]
    fn test_get_entry() {
        let log = ReplicatedLog::new(100);

        let entry = LogEntry::new(1, 1, create_allocate_op("req-1", 10));
        log.append(entry.clone()).ok();

        let retrieved = log.get(1).unwrap();
        assert_eq!(retrieved.lsn, entry.lsn);
        assert_eq!(retrieved.operation, entry.operation);
    }

    #[test]
    fn test_get_range() {
        let log = ReplicatedLog::new(100);

        for i in 1..=5 {
            let entry = LogEntry::new(i, 1, create_allocate_op(&format!("req-{}", i), 10));
            log.append(entry).ok();
        }

        let range = log.get_range(2, 4);
        assert_eq!(range.len(), 3);
        assert_eq!(range[0].lsn, 2);
        assert_eq!(range[2].lsn, 4);
    }

    #[test]
    fn test_commit() {
        let log = ReplicatedLog::new(100);

        let entry = LogEntry::new(1, 1, create_allocate_op("req-1", 10));
        log.append(entry).ok();

        assert_eq!(log.commit_index(), 0);
        assert!(log.commit(1).is_ok());
        assert_eq!(log.commit_index(), 1);
    }

    #[test]
    fn test_cannot_commit_missing_entry() {
        let log = ReplicatedLog::new(100);
        let result = log.commit(99);
        assert!(result.is_err());
    }

    #[test]
    fn test_apply() {
        let log = ReplicatedLog::new(100);

        let entry = LogEntry::new(1, 1, create_allocate_op("req-1", 10));
        log.append(entry).ok();
        log.commit(1).ok();

        assert_eq!(log.last_applied(), 0);
        assert!(log.apply(1).is_ok());
        assert_eq!(log.last_applied(), 1);
    }

    #[test]
    fn test_pending_entries() {
        let log = ReplicatedLog::new(100);

        for i in 1..=5 {
            let entry = LogEntry::new(i, 1, create_allocate_op(&format!("req-{}", i), 10));
            log.append(entry).ok();
        }

        log.commit(3).ok();
        log.apply(1).ok();

        let pending = log.pending_entries();
        assert_eq!(pending.len(), 2); // LSN 2 and 3
        assert_eq!(pending[0].lsn, 2);
        assert_eq!(pending[1].lsn, 3);
    }

    #[test]
    fn test_uncommitted_entries() {
        let log = ReplicatedLog::new(100);

        for i in 1..=5 {
            let entry = LogEntry::new(i, 1, create_allocate_op(&format!("req-{}", i), 10));
            log.append(entry).ok();
        }

        log.commit(3).ok();

        let uncommitted = log.uncommitted_entries();
        assert_eq!(uncommitted.len(), 2); // LSN 4 and 5
        assert_eq!(uncommitted[0].lsn, 4);
        assert_eq!(uncommitted[1].lsn, 5);
    }

    #[test]
    fn test_log_full() {
        let log = ReplicatedLog::new(3);

        for i in 1..=3 {
            let entry = LogEntry::new(i, 1, create_allocate_op(&format!("req-{}", i), 10));
            assert!(log.append(entry).is_ok());
        }

        let entry = LogEntry::new(4, 1, create_allocate_op("req-4", 10));
        let result = log.append(entry);
        assert!(result.is_err());
    }

    #[test]
    fn test_clear_log() {
        let log = ReplicatedLog::new(100);

        for i in 1..=5 {
            let entry = LogEntry::new(i, 1, create_allocate_op(&format!("req-{}", i), 10));
            log.append(entry).ok();
        }

        log.commit(3).ok();
        log.apply(2).ok();

        log.clear();

        assert_eq!(log.len(), 0);
        assert_eq!(log.commit_index(), 0);
        assert_eq!(log.last_applied(), 0);
    }

    #[test]
    fn test_truncate_from() {
        let log = ReplicatedLog::new(100);

        for i in 1..=5 {
            let entry = LogEntry::new(i, 1, create_allocate_op(&format!("req-{}", i), 10));
            log.append(entry).ok();
        }

        log.truncate_from(3).ok();

        assert_eq!(log.len(), 2);
        assert_eq!(log.last_lsn(), Some(2));
    }

    #[test]
    fn test_operation_serialization() {
        let op = create_allocate_op("req-1", 100);
        let json = serde_json::to_string(&op).unwrap();
        let restored: LogOperation = serde_json::from_str(&json).unwrap();
        assert_eq!(op, restored);
    }

    #[test]
    fn test_entry_serialization() {
        let entry = LogEntry::new(1, 1, create_allocate_op("req-1", 100));
        let json = serde_json::to_string(&entry).unwrap();
        let restored: LogEntry = serde_json::from_str(&json).unwrap();
        assert_eq!(entry.lsn, restored.lsn);
        assert_eq!(entry.term, restored.term);
    }
}
