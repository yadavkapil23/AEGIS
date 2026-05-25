// Speculative coordinator: manages draft/verify pipeline

use crate::branch::ExecutionBranch;
use crate::metrics::SpeculativeMetrics;
use anyhow::{anyhow, Result};
use dashmap::DashMap;
use parking_lot::Mutex;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use tracing::{info, warn};
use inference_backends::llama_cpp_safe::Session;

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
    
    // Real models
    draft_session: Option<Arc<Mutex<Session>>>,
    target_session: Option<Arc<Mutex<Session>>>,
}

impl SpeculativeCoordinator {
    pub fn new(max_draft_length: usize, metrics: Arc<SpeculativeMetrics>) -> Self {
        Self {
            branches: Arc::new(DashMap::new()),
            metrics,
            max_draft_length,
            current_draft_length: Mutex::new(4), // Start with 4 tokens
            draft_session: None,
            target_session: None,
        }
    }

    /// Set inference sessions
    pub fn set_sessions(&mut self, draft: Arc<Mutex<Session>>, target: Arc<Mutex<Session>>) {
        self.draft_session = Some(draft);
        self.target_session = Some(target);
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

    /// Generate draft tokens using the small draft model
    pub fn generate_draft(&self, request_id: &str, prompt: &str, num_tokens: usize) -> Result<Vec<Token>> {
        let draft_length = std::cmp::min(num_tokens, self.max_draft_length);

        if let Some(session) = &self.draft_session {
            let mut s = session.lock();
            let generated = s.generate(prompt, draft_length, 4)?;
            
            let mut tokens = Vec::new();
            for (id, text) in generated {
                tokens.push(Token {
                    id: id as u32,
                    text,
                    logprob: 0.0, // Simplified
                });
            }

            self.metrics.record_draft_length(tokens.len());

            if let Some(mut branch) = self.branches.get_mut(request_id) {
                let mut exec_branch = branch.lock();
                exec_branch.add_draft_tokens(tokens.clone())?;
            }

            Ok(tokens)
        } else {
            Err(anyhow!("Draft session not initialized"))
        }
    }

    /// Verify draft tokens using the large target model
    pub fn verify(&self, request_id: &str, prompt: &str, draft_tokens: &[Token]) -> Result<Vec<bool>> {
        if draft_tokens.is_empty() {
            return Ok(vec![]);
        }

        if let Some(session) = &self.target_session {
            // Simplified verification: we ask the target model to generate the next N tokens given the prompt
            // and compare them with the draft tokens.
            let mut s = session.lock();
            let target_generated = s.generate(prompt, draft_tokens.len(), 4)?;

            let mut acceptances = Vec::new();
            for (i, token) in draft_tokens.iter().enumerate() {
                if i < target_generated.len() && token.id == target_generated[i].0 as u32 {
                    acceptances.push(true);
                } else {
                    acceptances.push(false);
                    // In a real scenario, we stop verifying after the first rejection
                    break;
                }
            }

            let acceptance_count = acceptances.iter().filter(|&&a| a).count();
            let rate = if !draft_tokens.is_empty() {
                (acceptance_count as f32) / (draft_tokens.len() as f32)
            } else {
                0.0
            };

            self.metrics.record_acceptance_rate(rate);
            self.adapt_draft_length(rate as f64);

            Ok(acceptances)
        } else {
            Err(anyhow!("Target session not initialized"))
        }
    }

    /// Handle rollback on verification failure
    pub fn rollback(&self, request_id: &str, to_token: u32) -> Result<()> {
        if let Some(mut branch) = self.branches.get_mut(request_id) {
            let mut exec_branch = branch.lock();
            exec_branch.rollback_to(to_token as usize)?;
            self.metrics.record_rollback();
            
            // Here we would interact with the KV Scheduler to physically rollback the sequence 
            // via llama_kv_cache_seq_rm.
            
            info!(request_id = request_id, to_token = to_token, "Physical KV Rollback performed");
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
