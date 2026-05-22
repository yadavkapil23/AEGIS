/// Request cache - Simplified

use lru::LruCache;
use std::num::NonZeroUsize;
use parking_lot::Mutex;
use serde_json::Value;

/// Request cache
pub struct RequestCache {
    cache: Mutex<LruCache<String, Value>>,
}

impl RequestCache {
    pub fn new(capacity: usize) -> Self {
        let size = NonZeroUsize::new(capacity).unwrap_or_else(|| NonZeroUsize::new(100).unwrap());
        Self {
            cache: Mutex::new(LruCache::new(size)),
        }
    }

    pub fn get(&self, request_id: &str) -> Option<Value> {
        self.cache.lock().get(request_id).cloned()
    }

    pub fn put(&self, request_id: String, response: Value) {
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
        let response = serde_json::json!({
            "request_id": "req-1",
            "success": true,
            "block_ids": [1, 2, 3],
            "latency_ms": 10
        });

        cache.put("req-1".to_string(), response.clone());
        assert_eq!(cache.get("req-1"), Some(response));
    }

    #[test]
    fn test_cache_lru_eviction() {
        let cache = RequestCache::new(2);

        let resp1 = serde_json::json!({"request_id": "req-1", "data": "value1"});
        let resp2 = serde_json::json!({"request_id": "req-2", "data": "value2"});
        let resp3 = serde_json::json!({"request_id": "req-3", "data": "value3"});

        cache.put("req-1".to_string(), resp1);
        cache.put("req-2".to_string(), resp2);
        cache.put("req-3".to_string(), resp3);

        // req-1 should be evicted due to LRU
        assert_eq!(cache.get("req-1"), None);
        assert_eq!(cache.len(), 2);
    }
}
