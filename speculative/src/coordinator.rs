// Speculative coordinator: manages draft/verify pipeline

use crate::branch::ExecutionBranch;
use crate::metrics::SpeculativeMetrics;
use anyhow::{anyhow, Result};
use dashmap::DashMap;
use parking_lot::Mutex;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use tracing::{info, warn};

/// Token: a single generated token
#[derive(Debug, Clone)]
pub struct Token {
    pub id: u32,
    pub text: String,
    pub logprob: f32,
}

/// SpeculativeCoordinator: orchestrates speculative decoding
pub struct SpeculativeCoordinator {
    branches: Arc<DashMap<String, Mutex<ExecutionBranch>>>,
    metrics: Arc<SpeculativeMetrics>,
    max_draft_length: usize,
    current_draft_length: Mutex<usize>,
}

impl SpeculativeCoordinator {
    pub fn new(max_draft_length: usize, metrics: Arc<SpeculativeMetrics>) -> Self {
        Self {
            branches: Arc::new(DashMap::new()),
            metrics,
            max_draft_length,
            current_draft_length: Mutex::new(4), // Start with 4 tokens
        }
    }

    /// Create a new speculative branch for a request
    pub fn create_branch(&self, request_id: &str) -> Result<()> {
        let branch = ExecutionBranch::new(
            request_id.to_string(),
            *self.current_draft_length.lock(),
        );
        self.branches.insert(request_id.to_string(), Mutex::new(branch));
        Ok(())
    }

    /// Generate draft tokens
    pub fn generate_draft(&self, request_id: &str, num_tokens: usize) -> Result<Vec<Token>> {
        let draft_length = std::cmp::min(num_tokens, self.max_draft_length);

        // Simulate draft token generation
        let mut tokens = Vec::new();
        for i in 0..draft_length {
            tokens.push(Token {
                id: i as u32,
                text: format!("draft_token_{}", i),
                logprob: -0.5,
            });
        }

        self.metrics.record_draft_length(draft_length);

        if let Some(mut branch) = self.branches.get_mut(request_id) {
            let mut exec_branch = branch.lock();
            exec_branch.add_draft_tokens(tokens.clone())?;
        }

        Ok(tokens)
    }

    /// Verify draft tokens (simulate verification)
    pub fn verify(&self, request_id: &str, token_ids: &[u32]) -> Result<Vec<bool>> {
        // Simulate verifier acceptance/rejection
        let mut acceptances = Vec::new();

        for _ in token_ids {
            // 80% acceptance rate in simulation
            acceptances.push(rand::random::<f32>() < 0.8);
        }

        let acceptance_count = acceptances.iter().filter(|&&a| a).count();
        let rate = if !acceptances.is_empty() {
            (acceptance_count as f32) / (acceptances.len() as f32)
        } else {
            0.0
        };

        self.metrics.record_acceptance_rate(rate);

        Ok(acceptances)
    }

    /// Handle rollback on verification failure
    pub fn rollback(&self, request_id: &str, to_token: u32) -> Result<()> {
        if let Some(mut branch) = self.branches.get_mut(request_id) {
            let mut exec_branch = branch.lock();
            exec_branch.rollback_to(to_token as usize)?;
            self.metrics.record_rollback();
            info!(request_id = request_id, to_token = to_token, "Rollback");
        }
        Ok(())
    }

    /// Commit accepted tokens
    pub fn commit(&self, request_id: &str, num_tokens: usize) -> Result<()> {
        if let Some(mut branch) = self.branches.get_mut(request_id) {
            let mut exec_branch = branch.lock();
            exec_branch.commit(num_tokens)?;
        }
        Ok(())
    }

    /// Get metrics
    pub fn metrics(&self) -> Arc<SpeculativeMetrics> {
        self.metrics.clone()
    }

    /// Adapt draft length based on acceptance rate
    fn adapt_draft_length(&self, current_rate: f64) {
        let mut draft_len = self.current_draft_length.lock();

        if current_rate > 0.85 {
            *draft_len = std::cmp::min(*draft_len + 1, self.max_draft_length);
        } else if current_rate < 0.7 {
            *draft_len = std::cmp::max(*draft_len - 1, 1);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_create_branch() {
        let metrics = Arc::new(SpeculativeMetrics::new());
        let coord = SpeculativeCoordinator::new(16, metrics);

        let result = coord.create_branch("req-1");
        assert!(result.is_ok());
    }

    #[test]
    fn test_generate_draft() {
        let metrics = Arc::new(SpeculativeMetrics::new());
        let coord = SpeculativeCoordinator::new(16, metrics);

        coord.create_branch("req-1").unwrap();
        let tokens = coord.generate_draft("req-1", 5);

        assert!(tokens.is_ok());
        let tokens = tokens.unwrap();
        assert_eq!(tokens.len(), 5);
    }
}
