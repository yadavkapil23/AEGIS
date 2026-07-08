// Eviction policy: LRU, LFU strategies


/// EvictionPolicy: strategy for cache eviction
#[derive(Debug, Clone)]
pub enum EvictionPolicy {
    LRU,  // Least Recently Used
    LFU,  // Least Frequently Used
    FIFO, // First In First Out
}

impl EvictionPolicy {
    /// Select a block to evict from a list of candidates
    pub fn select_victim(&self, candidates: &[(usize, usize, usize)]) -> Option<usize> {
        // candidates: (block_id, last_accessed_timestamp, access_count)

        match self {
            EvictionPolicy::LRU => {
                // Evict block with smallest last_accessed_timestamp
                candidates
                    .iter()
                    .min_by_key(|(_, last_accessed, _)| last_accessed)
                    .map(|(block_id, _, _)| *block_id)
            }
            EvictionPolicy::LFU => {
                // Evict block with smallest access_count
                candidates
                    .iter()
                    .min_by_key(|(_, _, access_count)| access_count)
                    .map(|(block_id, _, _)| *block_id)
            }
            EvictionPolicy::FIFO => {
                // Evict the first block (assuming candidates are ordered)
                candidates.first().map(|(block_id, _, _)| *block_id)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_lru_eviction() {
        let policy = EvictionPolicy::LRU;

        // (block_id, last_accessed_timestamp, access_count)
        let candidates = vec![
            (0, 100, 5),
            (1, 50, 3),  // Least recently used
            (2, 200, 10),
        ];

        let victim = policy.select_victim(&candidates);
        assert_eq!(victim, Some(1));
    }

    #[test]
    fn test_lfu_eviction() {
        let policy = EvictionPolicy::LFU;

        let candidates = vec![
            (0, 100, 5),
            (1, 50, 2),  // Least frequently used
            (2, 200, 10),
        ];

        let victim = policy.select_victim(&candidates);
        assert_eq!(victim, Some(1));
    }

    #[test]
    fn test_fifo_eviction() {
        let policy = EvictionPolicy::FIFO;

        let candidates = vec![
            (0, 100, 5), // First
            (1, 50, 2),
            (2, 200, 10),
        ];

        let victim = policy.select_victim(&candidates);
        assert_eq!(victim, Some(0));
    }
}
