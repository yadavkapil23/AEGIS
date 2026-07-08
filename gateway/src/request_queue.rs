use dashmap::DashMap;
use std::collections::VecDeque;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};
use parking_lot::Mutex;
use tokio::sync::Notify;
use tracing::warn;

/// Error returned when the queue is at capacity.
#[derive(Debug)]
pub struct QueueFullError;

impl std::fmt::Display for QueueFullError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "request queue is full")
    }
}

impl std::error::Error for QueueFullError {}

/// Error returned when acquiring a queue slot times out.
#[derive(Debug)]
pub struct QueueTimeoutError;

impl std::fmt::Display for QueueTimeoutError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "timed out waiting for queue slot")
    }
}

impl std::error::Error for QueueTimeoutError {}

/// RAII guard released when the request completes.
pub struct QueuePermit {
    queue: Arc<RequestQueueInner>,
}

impl Drop for QueuePermit {
    fn drop(&mut self) {
        self.queue.release_slot();
    }
}

/// Internal shared state behind the public `RequestQueue`.
struct RequestQueueInner {
    entries: DashMap<String, QueuedRequestEntry>,
    order: Mutex<VecDeque<String>>,
    max_concurrent: usize,
    timeout_ms: u64,
    active_count: AtomicUsize,
    notify: Notify,
}

struct QueuedRequestEntry {
    queued_at: Instant,
    timeout_ms: u64,
}

/// FIFO request queue with timeout support and async slot acquisition.
pub struct RequestQueue {
    inner: Arc<RequestQueueInner>,
}

impl RequestQueue {
    pub fn new(max_concurrent: usize, timeout_ms: u64) -> Self {
        Self {
            inner: Arc::new(RequestQueueInner {
                entries: DashMap::new(),
                order: Mutex::new(VecDeque::new()),
                max_concurrent,
                timeout_ms,
                active_count: AtomicUsize::new(0),
                notify: Notify::new(),
            }),
        }
    }

    /// Non-blocking enqueue.  Returns the queue position (1-indexed) or
    /// `QueueFullError` when the queue is at capacity.
    pub fn enqueue(&self, request_id: String) -> Result<usize, QueueFullError> {
        let active = self.inner.active_count.load(Ordering::SeqCst);
        let waiting = self.inner.order.lock().len();
        if active + waiting >= self.inner.max_concurrent + self.inner.max_concurrent {
            return Err(QueueFullError);
        }

        self.inner.entries.insert(
            request_id.clone(),
            QueuedRequestEntry {
                queued_at: Instant::now(),
                timeout_ms: self.inner.timeout_ms,
            },
        );
        self.inner.order.lock().push_back(request_id);
        Ok(waiting + 1)
    }

    /// Remove and return the oldest queued request (FIFO).
    pub fn dequeue(&self) -> Option<String> {
        let id = self.inner.order.lock().pop_front()?;
        self.inner.entries.remove(&id);
        Some(id)
    }

    /// Async wait for a processing slot.  Returns a `QueuePermit` whose
    /// `Drop` impl releases the slot.  Fails with `QueueTimeoutError` if
    /// no slot becomes available within the configured timeout.
    pub async fn acquire_slot(&self) -> Result<QueuePermit, QueueTimeoutError> {
        let deadline = Instant::now() + Duration::from_millis(self.inner.timeout_ms);

        loop {
            let prev = self.inner.active_count.fetch_add(1, Ordering::SeqCst);
            if prev < self.inner.max_concurrent {
                return Ok(QueuePermit {
                    queue: Arc::clone(&self.inner),
                });
            }
            // Over limit — revert and wait.
            self.inner.active_count.fetch_sub(1, Ordering::SeqCst);

            if Instant::now() >= deadline {
                return Err(QueueTimeoutError);
            }

            let remaining = deadline.duration_since(Instant::now());
            tokio::select! {
                _ = tokio::time::sleep(remaining) => {},
                _ = self.inner.notify.notified() => {},
            }
        }
    }

    /// Release a processing slot and wake one waiter.
    fn release_slot(&self) {
        self.inner.active_count.fetch_sub(1, Ordering::SeqCst);
        self.inner.notify.notify_one();
    }

    /// Scan for requests that have exceeded their timeout and remove them.
    pub fn check_timeouts(&self) -> Vec<String> {
        let now = Instant::now();
        let mut timed_out = Vec::new();

        let expired: Vec<String> = self
            .inner
            .entries
            .iter()
            .filter(|entry| now.duration_since(entry.value().queued_at).as_millis() as u64 > entry.value().timeout_ms)
            .map(|entry| entry.key().clone())
            .collect();

        for id in expired {
            self.inner.entries.remove(&id);
            self.inner.order.lock().retain(|x| x != &id);
            timed_out.push(id);
        }

        timed_out
    }

    /// Remove all queued requests (for graceful shutdown).
    pub fn drain(&self) -> Vec<String> {
        let mut ids: Vec<String> = self.inner.order.lock().drain(..).collect();
        for id in &ids {
            self.inner.entries.remove(id);
        }
        ids
    }

    /// Number of requests waiting in queue.
    pub fn depth(&self) -> usize {
        self.inner.order.lock().len()
    }

    /// Number of requests currently being processed.
    pub fn active(&self) -> usize {
        self.inner.active_count.load(Ordering::SeqCst)
    }

    /// Total queued + active.
    pub fn size(&self) -> usize {
        self.depth() + self.active()
    }

    /// Whether the processing slots are all occupied.
    pub fn is_full(&self) -> bool {
        self.inner.active_count.load(Ordering::SeqCst) >= self.inner.max_concurrent
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn enqueue_dequeue_fifo() {
        let q = RequestQueue::new(10, 5000);
        q.enqueue("req-1".into()).unwrap();
        q.enqueue("req-2".into()).unwrap();
        assert_eq!(q.dequeue(), Some("req-1".into()));
        assert_eq!(q.dequeue(), Some("req-2".into()));
        assert_eq!(q.dequeue(), None);
    }

    #[test]
    fn depth_tracks_correctly() {
        let q = RequestQueue::new(10, 5000);
        assert_eq!(q.depth(), 0);
        q.enqueue("a".into()).unwrap();
        q.enqueue("b".into()).unwrap();
        assert_eq!(q.depth(), 2);
        q.dequeue();
        assert_eq!(q.depth(), 1);
    }

    #[test]
    fn drain_empties_queue() {
        let q = RequestQueue::new(10, 5000);
        q.enqueue("a".into()).unwrap();
        q.enqueue("b".into()).unwrap();
        let drained = q.drain();
        assert_eq!(drained.len(), 2);
        assert_eq!(q.depth(), 0);
    }

    #[test]
    fn check_timeouts_removes_expired() {
        let q = RequestQueue::new(10, 0); // 0ms timeout = immediately expired
        q.enqueue("expired".into()).unwrap();
        let timed_out = q.check_timeouts();
        assert_eq!(timed_out.len(), 1);
        assert_eq!(q.depth(), 0);
    }
}
