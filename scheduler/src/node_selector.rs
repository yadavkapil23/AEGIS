// Node selection algorithm for distributed allocation
// Chooses best node based on capacity, latency, and load

use anyhow::{anyhow, Result};
use std::collections::HashMap;
use std::sync::Arc;
use parking_lot::Mutex;
use tracing::{debug, info};

/// Node capacity and availability info
#[derive(Debug, Clone)]
pub struct NodeCapacity {
    pub total_blocks: usize,
    pub free_blocks: usize,
    pub allocated_blocks: usize,
}

impl NodeCapacity {
    pub fn capacity_ratio(&self) -> f32 {
        if self.total_blocks == 0 {
            return 0.0;
        }
        (self.free_blocks as f32) / (self.total_blocks as f32)
    }

    pub fn is_available(&self, required_blocks: usize) -> bool {
        self.free_blocks >= required_blocks
    }
}

/// Node metrics for scoring
#[derive(Debug, Clone)]
pub struct NodeMetrics {
    pub node_id: String,
    pub latency_ms: f32,
    pub load_percent: f32,
    pub capacity: NodeCapacity,
}

impl NodeMetrics {
    /// Score this node for allocation (higher is better)
    pub fn score(&self) -> f32 {
        let capacity_score = self.capacity.capacity_ratio();
        let latency_score = 1.0 - (self.latency_ms.min(100.0) / 100.0);
        let load_score = 1.0 - (self.load_percent.min(100.0) / 100.0);

        // Weighted scoring: capacity (50%), latency (30%), load (20%)
        (capacity_score * 0.5) + (latency_score * 0.3) + (load_score * 0.2)
    }
}

/// Selects best node for allocation
pub struct NodeSelector {
    /// Node metrics cache
    node_metrics: Arc<Mutex<HashMap<String, NodeMetrics>>>,

    /// Last selection time (for load balancing)
    last_selection: Arc<Mutex<Option<String>>>,
}

impl NodeSelector {
    pub fn new() -> Self {
        Self {
            node_metrics: Arc::new(Mutex::new(HashMap::new())),
            last_selection: Arc::new(Mutex::new(None)),
        }
    }

    /// Register a node with its metrics
    pub fn register_node(&self, node_id: String, capacity: NodeCapacity) -> Result<()> {
        debug!("Registering node: {} with {} free blocks", node_id, capacity.free_blocks);

        let mut metrics = self.node_metrics.lock();
        metrics.insert(
            node_id.clone(),
            NodeMetrics {
                node_id,
                latency_ms: 0.0,
                load_percent: 0.0,
                capacity,
            },
        );

        Ok(())
    }

    /// Update node metrics
    pub fn update_metrics(
        &self,
        node_id: &str,
        capacity: NodeCapacity,
        latency_ms: f32,
        load_percent: f32,
    ) -> Result<()> {
        let mut metrics = self.node_metrics.lock();

        if let Some(node_metric) = metrics.get_mut(node_id) {
            node_metric.capacity = capacity;
            node_metric.latency_ms = latency_ms;
            node_metric.load_percent = load_percent;
            debug!(
                "Updated metrics for {}: latency={:.1}ms, load={:.1}%",
                node_id, latency_ms, load_percent
            );
            Ok(())
        } else {
            Err(anyhow!("Node {} not registered", node_id))
        }
    }

    /// Select best node for allocation
    pub fn select_node(&self, required_blocks: usize) -> Result<String> {
        let metrics = self.node_metrics.lock();

        if metrics.is_empty() {
            return Err(anyhow!("No nodes registered"));
        }

        // Filter: must have capacity
        let candidates: Vec<_> = metrics
            .values()
            .filter(|m| m.capacity.is_available(required_blocks))
            .collect();

        if candidates.is_empty() {
            return Err(anyhow!(
                "No node with {} free blocks available",
                required_blocks
            ));
        }

        // Score and sort
        let mut scored: Vec<_> = candidates
            .iter()
            .map(|m| (m.node_id.clone(), m.score()))
            .collect();

        scored.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));

        let selected = scored[0].0.clone();
        info!(
            "Selected node {} for {} blocks (score: {:.3})",
            selected, required_blocks, scored[0].1
        );

        Ok(selected)
    }

    /// Select node with round-robin on ties
    pub fn select_node_round_robin(&self, required_blocks: usize) -> Result<String> {
        let metrics = self.node_metrics.lock();

        if metrics.is_empty() {
            return Err(anyhow!("No nodes registered"));
        }

        // Filter by capacity
        let mut candidates: Vec<_> = metrics
            .iter()
            .filter(|(_, m)| m.capacity.is_available(required_blocks))
            .map(|(_, m)| m.clone())
            .collect();

        if candidates.is_empty() {
            return Err(anyhow!(
                "No node with {} free blocks available",
                required_blocks
            ));
        }

        // Sort by score
        candidates.sort_by(|a, b| {
            b.score().partial_cmp(&a.score()).unwrap_or(std::cmp::Ordering::Equal)
        });

        // Round-robin among top 3
        let last = self.last_selection.lock().clone();
        let top_n = candidates.iter().take(3).collect::<Vec<_>>();

        let selected = if let Some(last_id) = last {
            // Pick next in rotation
            if let Some(pos) = top_n.iter().position(|m| m.node_id == last_id) {
                top_n[(pos + 1) % top_n.len()].clone()
            } else {
                top_n[0].clone()
            }
        } else {
            top_n[0].clone()
        };

        *self.last_selection.lock() = Some(selected.node_id.clone());

        Ok(selected.node_id)
    }

    /// Get all registered nodes
    pub fn get_nodes(&self) -> Vec<String> {
        self.node_metrics
            .lock()
            .keys()
            .cloned()
            .collect()
    }

    /// Get node metrics
    pub fn get_metrics(&self, node_id: &str) -> Result<NodeMetrics> {
        self.node_metrics
            .lock()
            .get(node_id)
            .cloned()
            .ok_or_else(|| anyhow!("Node {} not found", node_id))
    }

    /// Remove a node
    pub fn remove_node(&self, node_id: &str) -> Result<()> {
        self.node_metrics.lock().remove(node_id);
        Ok(())
    }

    /// Get best node without selecting it
    pub fn best_node(&self, required_blocks: usize) -> Result<String> {
        let metrics = self.node_metrics.lock();

        let best = metrics
            .values()
            .filter(|m| m.capacity.is_available(required_blocks))
            .max_by(|a, b| {
                a.score()
                    .partial_cmp(&b.score())
                    .unwrap_or(std::cmp::Ordering::Equal)
            });

        best.map(|m| m.node_id.clone()).ok_or_else(|| {
            anyhow!("No suitable node for {} blocks", required_blocks)
        })
    }

    /// Get all nodes with available capacity
    pub fn get_available_nodes(&self, required_blocks: usize) -> Vec<String> {
        self.node_metrics
            .lock()
            .values()
            .filter(|m| m.capacity.is_available(required_blocks))
            .map(|m| m.node_id.clone())
            .collect()
    }
}

impl Default for NodeSelector {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn create_capacity(total: usize, free: usize) -> NodeCapacity {
        NodeCapacity {
            total_blocks: total,
            free_blocks: free,
            allocated_blocks: total - free,
        }
    }

    #[test]
    fn test_register_node() {
        let selector = NodeSelector::new();
        let capacity = create_capacity(1000, 500);
        assert!(selector.register_node("node-1".to_string(), capacity).is_ok());
        assert_eq!(selector.get_nodes().len(), 1);
    }

    #[test]
    fn test_select_node_basic() {
        let selector = NodeSelector::new();
        selector
            .register_node("node-1".to_string(), create_capacity(1000, 500))
            .unwrap();
        selector
            .register_node("node-2".to_string(), create_capacity(1000, 300))
            .unwrap();

        let selected = selector.select_node(100).unwrap();
        assert_eq!(selected, "node-1"); // node-1 has more free blocks
    }

    #[test]
    fn test_select_node_insufficient_capacity() {
        let selector = NodeSelector::new();
        selector
            .register_node("node-1".to_string(), create_capacity(1000, 50))
            .unwrap();

        let result = selector.select_node(100);
        assert!(result.is_err());
    }

    #[test]
    fn test_select_node_no_nodes() {
        let selector = NodeSelector::new();
        let result = selector.select_node(100);
        assert!(result.is_err());
    }

    #[test]
    fn test_update_metrics() {
        let selector = NodeSelector::new();
        selector
            .register_node("node-1".to_string(), create_capacity(1000, 500))
            .unwrap();

        let result = selector.update_metrics("node-1", create_capacity(1000, 400), 10.0, 60.0);
        assert!(result.is_ok());

        let metrics = selector.get_metrics("node-1").unwrap();
        assert_eq!(metrics.latency_ms, 10.0);
        assert_eq!(metrics.load_percent, 60.0);
    }

    #[test]
    fn test_node_score() {
        let metrics1 = NodeMetrics {
            node_id: "node-1".to_string(),
            latency_ms: 10.0,
            load_percent: 20.0,
            capacity: create_capacity(1000, 800),
        };

        let metrics2 = NodeMetrics {
            node_id: "node-2".to_string(),
            latency_ms: 50.0,
            load_percent: 80.0,
            capacity: create_capacity(1000, 400),
        };

        assert!(metrics1.score() > metrics2.score());
    }

    #[test]
    fn test_best_node() {
        let selector = NodeSelector::new();
        selector
            .register_node("node-1".to_string(), create_capacity(1000, 300))
            .unwrap();
        selector
            .register_node("node-2".to_string(), create_capacity(1000, 700))
            .unwrap();

        let best = selector.best_node(100).unwrap();
        assert_eq!(best, "node-2");
    }

    #[test]
    fn test_round_robin() {
        let selector = NodeSelector::new();
        selector
            .register_node("node-1".to_string(), create_capacity(1000, 500))
            .unwrap();
        selector
            .register_node("node-2".to_string(), create_capacity(1000, 500))
            .unwrap();
        selector
            .register_node("node-3".to_string(), create_capacity(1000, 500))
            .unwrap();

        let first = selector.select_node_round_robin(100).unwrap();
        let second = selector.select_node_round_robin(100).unwrap();
        let third = selector.select_node_round_robin(100).unwrap();

        // Should cycle through top nodes
        assert_ne!(first, second);
        assert_ne!(second, third);
    }

    #[test]
    fn test_get_available_nodes() {
        let selector = NodeSelector::new();
        selector
            .register_node("node-1".to_string(), create_capacity(1000, 500))
            .unwrap();
        selector
            .register_node("node-2".to_string(), create_capacity(1000, 50))
            .unwrap();

        let available = selector.get_available_nodes(100);
        assert_eq!(available.len(), 1);
        assert!(available.contains(&"node-1".to_string()));
    }
}
