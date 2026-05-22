// Request cache for gateway

use crate::AllocationResponse;
use lru::LruCache;
use std::num::NonZeroUsize;
use parking_lot::Mutex;

/// Request cache to avoid duplicate allocations
pub struct RequestCache {
    cache: Mutex<LruCache<String, AllocationResponse>>,
}

impl RequestCache {
    /// Create new request cache
    pub fn new(capacity: usize) -> Self {
        let size = NonZeroUsize::new(capacity).unwrap_or_else(|| NonZeroUsize::new(100).unwrap());
        Self {
            cache: Mutex::new(LruCache::new(size)),
        }
    }

    /// Get cached response
    pub fn get(&self, request_id: &str) -> Option<AllocationResponse> {
        self.cache.lock().get(request_id).cloned()
    }

    /// Put response in cache
    pub fn put(&self, request_id: String, response: AllocationResponse) {
        self.cache.lock().put(request_id, response);
    }

    /// Clear cache
    pub fn clear(&self) {
        self.cache.lock().clear();
    }

    /// Get cache size
    pub fn len(&self) -> usize {
        self.cache.lock().len()
    }

    /// Check if cache is empty
    pub fn is_empty(&self) -> bool {
        self.cache.lock().is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_cache_put_get() {
        let cache = RequestCache::new(10);
        let response = AllocationResponse {
            request_id: "req-1".to_string(),
            success: true,
            block_ids: vec![1, 2, 3],
            error: None,
            latency_ms: 10,
            node_id: "node-1".to_string(),
        };

        cache.put("req-1".to_string(), response.clone());
        assert_eq!(cache.get("req-1"), Some(response));
    }

    #[test]
    fn test_cache_lru_eviction() {
        let cache = RequestCache::new(2);

        let resp1 = AllocationResponse {
            request_id: "req-1".to_string(),
            success: true,
            block_ids: vec![1],
            error: None,
            latency_ms: 10,
            node_id: "node-1".to_string(),
        };

        let resp2 = AllocationResponse {
            request_id: "req-2".to_string(),
            success: true,
            block_ids: vec![2],
            error: None,
            latency_ms: 10,
            node_id: "node-1".to_string(),
        };

        let resp3 = AllocationResponse {
            request_id: "req-3".to_string(),
            success: true,
            block_ids: vec![3],
            error: None,
            latency_ms: 10,
            node_id: "node-1".to_string(),
        };

        cache.put("req-1".to_string(), resp1);
        cache.put("req-2".to_string(), resp2);
        cache.put("req-3".to_string(), resp3);

        // req-1 should be evicted due to LRU
        assert_eq!(cache.get("req-1"), None);
        assert_eq!(cache.len(), 2);
    }
}
