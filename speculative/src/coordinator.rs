// Speculative coordinator: manages draft/verify pipeline

use crate::branch::ExecutionBranch;
use crate::metrics::SpeculativeMetrics;
use anyhow::{anyhow, Result};
use dashmap::DashMap;
use parking_lot::Mutex;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use tracing::{info, warn, debug};
use inference_backends::llama_cpp_safe::Session;

/// Token: a single generated token with log-probability
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
    min_acceptance_ratio: f64,
    current_draft_length: Mutex<usize>,

    draft_session: Option<Arc<Mutex<Session>>>,
    target_session: Option<Arc<Mutex<Session>>>,
}

impl SpeculativeCoordinator {
    pub fn new(max_draft_length: usize, metrics: Arc<SpeculativeMetrics>) -> Self {
        Self {
            branches: Arc::new(DashMap::new()),
            metrics,
            max_draft_length,
            min_acceptance_ratio: 0.7,
            current_draft_length: Mutex::new(4),
            draft_session: None,
            target_session: None,
        }
    }

    pub fn set_sessions(&mut self, draft: Arc<Mutex<Session>>, target: Arc<Mutex<Session>>) {
        self.draft_session = Some(draft);
        self.target_session = Some(target);
    }

    /// Create a new speculative branch for a request.
    pub fn create_branch(&self, request_id: &str) -> Result<()> {
        let branch = ExecutionBranch::new(
            request_id.to_string(),
            *self.current_draft_length.lock(),
        );
        self.branches.insert(request_id.to_string(), Mutex::new(branch));
        Ok(())
    }

    /// Generate draft tokens using the small, fast draft model.
    ///
    /// The draft model produces `draft_length` tokens. Each token carries
    /// its log-probability for later acceptance comparison.
    pub fn generate_draft(
        &self,
        request_id: &str,
        prompt: &str,
        num_tokens: usize,
    ) -> Result<Vec<Token>> {
        let draft_length = num_tokens.min(self.max_draft_length);

        let session = self.draft_session.as_ref()
            .ok_or_else(|| anyhow!("Draft session not initialized"))?;

        let mut s = session.lock();
        let generated = s.generate(prompt, draft_length, 4)?;

        let tokens: Vec<Token> = generated
            .into_iter()
            .map(|(id, text)| Token {
                id: id as u32,
                text,
                logprob: 0.0, // Will be populated by real sampling in Phase 15
            })
            .collect();

        self.metrics.record_draft_length(tokens.len());

        if let Some(mut branch) = self.branches.get_mut(request_id) {
            branch.lock().add_draft_tokens(tokens.clone())?;
        }

        debug!(request_id = request_id, count = tokens.len(), "Draft tokens generated");
        Ok(tokens)
    }

    /// Verify draft tokens against the large target model.
    ///
    /// Strategy: feed the target model the prompt + draft sequence, compare
    /// each draft token against what the target would have produced at that
    /// position. Accept consecutive matches, reject at the first divergence.
    pub fn verify(
        &self,
        request_id: &str,
        prompt: &str,
        draft_tokens: &[Token],
    ) -> Result<Vec<bool>> {
        if draft_tokens.is_empty() {
            return Ok(vec![]);
        }

        let session = self.target_session.as_ref()
            .ok_or_else(|| anyhow!("Target session not initialized"))?;

        // Build the full prompt: original prompt + all draft tokens joined
        let draft_text: String = draft_tokens.iter().map(|t| t.text.as_str()).collect();
        let full_prompt = format!("{}{}", prompt, draft_text);

        // Target model generates the same number of tokens for comparison
        let mut s = session.lock();
        let target_generated = s.generate(&full_prompt, draft_tokens.len(), 4)?;

        let mut acceptances = Vec::new();
        for (i, token) in draft_tokens.iter().enumerate() {
            if i < target_generated.len() && token.id == target_generated[i].0 as u32 {
                acceptances.push(true);
            } else {
                acceptances.push(false);
                // Standard speculative decoding: stop at first rejection
                break;
            }
        }

        let accepted_count = acceptances.iter().filter(|&&a| a).count();
        let rate = accepted_count as f32 / draft_tokens.len() as f32;

        self.metrics.record_acceptance_rate(rate);
        self.adapt_draft_length(rate as f64);

        info!(
            request_id = request_id,
            draft_len = draft_tokens.len(),
            accepted = accepted_count,
            acceptance_rate = rate,
            "Verification complete"
        );

        Ok(acceptances)
    }

    /// Full speculative decoding step: draft → verify → commit/rollback.
    ///
    /// Returns the list of accepted tokens (subset of draft_tokens).
    pub fn step(
        &self,
        request_id: &str,
        prompt: &str,
    ) -> Result<Vec<Token>> {
        let draft_length = *self.current_draft_length.lock();

        // Draft
        let draft_tokens = self.generate_draft(request_id, prompt, draft_length)?;
        if draft_tokens.is_empty() {
            return Ok(vec![]);
        }

        // Verify
        let acceptances = self.verify(request_id, prompt, &draft_tokens)?;

        // Collect accepted tokens
        let accepted: Vec<Token> = draft_tokens
            .into_iter()
            .zip(acceptances.iter())
            .filter(|(_, &a)| a)
            .map(|(t, _)| t)
            .collect();

        let rejected_count = acceptances.iter().filter(|&&a| !a).count();

        if rejected_count > 0 {
            // Rollback rejected tokens from the branch
            let commit_len = accepted.len();
            self.rollback(request_id, commit_len as u32)?;
            self.metrics.record_rollback();
            info!(
                request_id = request_id,
                rejected = rejected_count,
                "Rollback performed"
            );
        }

        // Commit accepted tokens
        if !accepted.is_empty() {
            self.commit(request_id, accepted.len())?;
        }

        self.metrics.record_verification();

        Ok(accepted)
    }

    /// Rollback the branch to a specific token position.
    /// Physically removes KV-cache entries beyond `to_token`.
    pub fn rollback(&self, request_id: &str, to_token: u32) -> Result<()> {
        if let Some(mut branch) = self.branches.get_mut(request_id) {
            let mut exec_branch = branch.lock();
            exec_branch.rollback_to(to_token as usize)?;

            // Physically remove KV-cache entries beyond the rollback point
            if let Some(session) = &self.target_session {
                let s = session.lock();
                s.kv_cache_rm(0, to_token as i32, -1);
                info!(
                    request_id = request_id,
                    to_token = to_token,
                    "KV-cache physically rolled back"
                );
            }
        }
        Ok(())
    }

    /// Commit accepted tokens to the branch.
    pub fn commit(&self, request_id: &str, num_tokens: usize) -> Result<()> {
        if let Some(mut branch) = self.branches.get_mut(request_id) {
            branch.lock().commit(num_tokens)?;
        }
        Ok(())
    }

    /// Remove a completed branch.
    pub fn remove_branch(&self, request_id: &str) {
        self.branches.remove(request_id);
    }

    pub fn metrics(&self) -> Arc<SpeculativeMetrics> {
        self.metrics.clone()
    }

    /// Adapt draft length based on recent acceptance rate.
    /// High acceptance → longer drafts; low acceptance → shorter drafts.
    fn adapt_draft_length(&self, current_rate: f64) {
        let mut draft_len = self.current_draft_length.lock();

        if current_rate > 0.85 && *draft_len < self.max_draft_length {
            *draft_len += 1;
            debug!(new_len = *draft_len, "Draft length increased");
        } else if current_rate < self.min_acceptance_ratio && *draft_len > 1 {
            *draft_len -= 1;
            debug!(new_len = *draft_len, "Draft length decreased");
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
        assert!(coord.create_branch("req-1").is_ok());
    }

    #[test]
    fn test_adapt_draft_length_up() {
        let metrics = Arc::new(SpeculativeMetrics::new());
        let coord = SpeculativeCoordinator::new(16, metrics);

        // High acceptance should increase draft length
        for _ in 0..5 {
            coord.adapt_draft_length(0.95);
        }
        assert_eq!(*coord.current_draft_length.lock(), 9); // 4 + 5
    }

    #[test]
    fn test_adapt_draft_length_down() {
        let metrics = Arc::new(SpeculativeMetrics::new());
        let coord = SpeculativeCoordinator::new(16, metrics);

        // Low acceptance should decrease draft length
        for _ in 0..5 {
            coord.adapt_draft_length(0.3);
        }
        assert_eq!(*coord.current_draft_length.lock(), 1); // min is 1
    }

    #[test]
    fn test_remove_branch() {
        let metrics = Arc::new(SpeculativeMetrics::new());
        let coord = SpeculativeCoordinator::new(16, metrics);
        coord.create_branch("req-1").unwrap();
        assert!(coord.branches.contains_key("req-1"));
        coord.remove_branch("req-1");
        assert!(!coord.branches.contains_key("req-1"));
    }
}
