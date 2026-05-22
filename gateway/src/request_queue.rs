/// Request Queue - simplified implementation
/// FIFO queue for managing concurrent requests

use dashmap::DashMap;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Instant;

/// QueuedRequest: internal tracking of queued request
struct QueuedRequest {
    request_id: String,
    queued_at: Instant,
    timeout_ms: u64,
}

/// RequestQueue: FIFO request queue with timeout tracking
pub struct RequestQueue {
    queue: DashMap<String, QueuedRequest>,
    max_concurrent: usize,
    active_count: AtomicUsize,
}

impl RequestQueue {
    pub fn new(max_concurrent: usize, _timeout_ms: u64) -> Self {
        Self {
            queue: DashMap::new(),
            max_concurrent,
            active_count: AtomicUsize::new(0),
        }
    }

    /// Get current queue size
    pub fn size(&self) -> usize {
        self.active_count.load(Ordering::SeqCst)
    }

    /// Check if queue is full
    pub fn is_full(&self) -> bool {
        self.active_count.load(Ordering::SeqCst) >= self.max_concurrent
    }
}
