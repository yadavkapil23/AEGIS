// KV Cache allocator: memory management and fragmentation tracking

use anyhow::{anyhow, Result};
use dashmap::DashMap;
use parking_lot::Mutex;
use std::collections::{HashMap, VecDeque};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use tracing::{info, warn};
use inference_backends::llama_cpp_safe::Session;

/// KVBlock: a unit of KV cache memory
#[derive(Debug, Clone)]
pub struct KVBlock {
    pub id: usize,
    pub size_bytes: usize,
    pub allocated: bool,
    pub owner: Option<String>, // request_id that owns this block
    pub created_at: std::time::Instant,
}

/// KVCacheAllocator: manages KV cache blocks with LRU eviction and physical backend management
pub struct KVCacheAllocator {
    total_bytes: usize,
    block_size: usize,
    blocks: Arc<DashMap<usize, Mutex<KVBlock>>>,
    free_list: Mutex<VecDeque<usize>>,
    block_counter: AtomicUsize,
    
    // Optional physical backend binding
    target_session: Option<Arc<Mutex<Session>>>,

    // Metrics
    total_allocated: AtomicUsize,
    total_evicted: AtomicUsize,
    fragmentation: Mutex<f64>,
}

#[derive(Debug, Clone)]
pub struct CacheStats {
    pub total_blocks: usize,
    pub allocated_blocks: usize,
    pub free_blocks: usize,
    pub total_allocated_bytes: usize,
    pub total_free_bytes: usize,
    pub fragmentation_ratio: f64,
    pub hit_rate: f64,
}

impl KVCacheAllocator {
    pub fn new(total_bytes: usize, block_size: usize) -> Result<Self> {
        if total_bytes < block_size {
            return Err(anyhow!("Total cache size must be >= block size"));
        }

        let num_blocks = total_bytes / block_size;
        let blocks = Arc::new(DashMap::new());
        let mut free_list = VecDeque::new();

        // Initialize blocks
        for i in 0..num_blocks {
            let block = KVBlock {
                id: i,
                size_bytes: block_size,
                allocated: false,
                owner: None,
                created_at: std::time::Instant::now(),
            };
            blocks.insert(i, Mutex::new(block));
            free_list.push_back(i);
        }

        info!(
            total_blocks = num_blocks,
            block_size = block_size,
            total_bytes = total_bytes,
            "Initialized KV cache allocator"
        );

        Ok(Self {
            total_bytes,
            block_size,
            blocks,
            free_list: Mutex::new(free_list),
            block_counter: AtomicUsize::new(num_blocks),
            target_session: None,
            total_allocated: AtomicUsize::new(0),
            total_evicted: AtomicUsize::new(0),
            fragmentation: Mutex::new(0.0),
        })
    }

    /// Set the physical llama.cpp session to manage its internal KV state
    pub fn set_session(&mut self, session: Arc<Mutex<Session>>) {
        self.target_session = Some(session);
    }

    /// Allocate KV blocks
    pub fn allocate(&self, num_blocks: usize) -> Result<Vec<usize>> {
        let mut free_list = self.free_list.lock();

        if free_list.len() < num_blocks {
            return Err(anyhow!(
                "Insufficient free blocks: {} requested, {} available",
                num_blocks,
                free_list.len()
            ));
        }

        let mut allocated = Vec::new();

        for _ in 0..num_blocks {
            if let Some(block_id) = free_list.pop_front() {
                if let Some(mut block) = self.blocks.get_mut(&block_id) {
                    block.lock().allocated = true;
                    allocated.push(block_id);
                }
            }
        }

        self.total_allocated.fetch_add(num_blocks, Ordering::SeqCst);
        self.update_fragmentation();

        Ok(allocated)
    }

    /// Deallocate KV blocks and physically remove from backend
    pub fn deallocate(&self, block_ids: &[usize]) -> Result<()> {
        let mut free_list = self.free_list.lock();

        for &block_id in block_ids {
            if let Some(mut block) = self.blocks.get_mut(&block_id) {
                let mut kb = block.lock();
                kb.allocated = false;
                kb.owner = None;
                drop(kb);
                free_list.push_back(block_id);
                
                // Perform physical eviction if backend is bound
                if let Some(session) = &self.target_session {
                    let s = session.lock();
                    s.kv_cache_rm(block_id as i32, -1, -1);
                }
            }
        }

        self.update_fragmentation();

        Ok(())
    }

    /// Get cache statistics
    pub fn stats(&self) -> CacheStats {
        let allocated_blocks = self.blocks
            .iter()
            .filter(|entry| entry.value().lock().allocated)
            .count();

        let free_list = self.free_list.lock();
        let free_blocks = free_list.len();
        let total_blocks = self.block_counter.load(Ordering::SeqCst);

        let total_allocated_bytes = allocated_blocks * self.block_size;
        let total_free_bytes = free_blocks * self.block_size;
        let fragmentation = *self.fragmentation.lock();

        CacheStats {
            total_blocks,
            allocated_blocks,
            free_blocks,
            total_allocated_bytes,
            total_free_bytes,
            fragmentation_ratio: fragmentation,
            hit_rate: 0.0, // Will be computed by higher-level components
        }
    }

    /// Update fragmentation metric
    fn update_fragmentation(&self) {
        let free_list = self.free_list.lock();
        let free_blocks = free_list.len();
        let total_blocks = self.block_counter.load(Ordering::SeqCst);

        let frag = if total_blocks > 0 {
            (free_blocks as f64) / (total_blocks as f64)
        } else {
            0.0
        };

        *self.fragmentation.lock() = frag;
    }

    /// Mark block as owned by a request
    pub fn mark_owner(&self, block_id: usize, request_id: String) -> Result<()> {
        if let Some(mut block) = self.blocks.get_mut(&block_id) {
            let mut kb = block.lock();
            kb.owner = Some(request_id);
            Ok(())
        } else {
            Err(anyhow!("Block not found"))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_allocator_init() {
        let allocator = KVCacheAllocator::new(1024 * 1024, 16 * 1024).unwrap();
        let stats = allocator.stats();
        assert_eq!(stats.total_blocks, 64);
        assert_eq!(stats.allocated_blocks, 0);
        assert_eq!(stats.free_blocks, 64);
    }

    #[test]
    fn test_allocate_blocks() {
        let allocator = KVCacheAllocator::new(1024 * 1024, 16 * 1024).unwrap();

        let blocks = allocator.allocate(10).unwrap();
        assert_eq!(blocks.len(), 10);

        let stats = allocator.stats();
        assert_eq!(stats.allocated_blocks, 10);
        assert_eq!(stats.free_blocks, 54);
    }

    #[test]
    fn test_deallocate_blocks() {
        let allocator = KVCacheAllocator::new(1024 * 1024, 16 * 1024).unwrap();

        let blocks = allocator.allocate(10).unwrap();
        allocator.deallocate(&blocks).unwrap();

        let stats = allocator.stats();
        assert_eq!(stats.allocated_blocks, 0);
        assert_eq!(stats.free_blocks, 64);
    }

    #[test]
    fn test_insufficient_blocks() {
        let allocator = KVCacheAllocator::new(1024 * 1024, 16 * 1024).unwrap();

        let result = allocator.allocate(100);
        assert!(result.is_err());
    }
}
