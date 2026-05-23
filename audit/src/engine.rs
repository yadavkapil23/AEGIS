// Audit engine: append-only cryptographic trail

use crate::metrics::AuditMetrics;
use crate::trail::ExecutionTrail;
use anyhow::{anyhow, Result};
use blake3;
use parking_lot::Mutex;
use std::sync::Arc;
use tracing::info;

/// AuditEvent: a single event to be recorded
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct AuditEvent {
    pub event_id: String,
    pub request_id: String,
    pub event_type: String,
    pub payload: String,
    pub timestamp_ns: u64,
}

/// AuditEngine: maintains cryptographically verifiable audit trail
pub struct AuditEngine {
    trail: Arc<Mutex<ExecutionTrail>>,
    metrics: Arc<AuditMetrics>,
}

impl AuditEngine {
    pub fn new(metrics: Arc<AuditMetrics>) -> Result<Self> {
        let trail = Arc::new(Mutex::new(ExecutionTrail::new()));

        Ok(Self {
            trail,
            metrics,
        })
    }

    /// Record an audit event
    pub fn record(&self, event: AuditEvent) -> Result<String> {
        let start = std::time::Instant::now();

        // Compute BLAKE3 hash of event
        let event_bytes = serde_json::to_vec(&event)?;
        let hash = blake3::hash(&event_bytes);
        let hash_hex = hash.to_hex();

        // Add to trail
        let mut trail = self.trail.lock();
        trail.append(event, hash_hex.to_string())?;

        let elapsed_us = start.elapsed().as_micros() as u64;
        self.metrics.record_hash_latency_us(elapsed_us);
        self.metrics.record_event();

        Ok(hash_hex.to_string())
    }

    /// Get current trail hash
    pub fn current_hash(&self) -> Result<String> {
        let trail = self.trail.lock();
        Ok(trail.current_hash())
    }

    /// Verify the integrity of the trail
    pub fn verify(&self) -> Result<bool> {
        let trail = self.trail.lock();
        Ok(trail.verify())
    }

    pub fn metrics(&self) -> Arc<AuditMetrics> {
        self.metrics.clone()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_record_event() {
        let metrics = Arc::new(AuditMetrics::new());
        let engine = AuditEngine::new(metrics).unwrap();

        let event = AuditEvent {
            event_id: "ev-1".to_string(),
            request_id: "req-1".to_string(),
            event_type: "TOKEN_GENERATED".to_string(),
            payload: "token_data".to_string(),
            timestamp_ns: 1000,
        };

        let result = engine.record(event);
        assert!(result.is_ok());
    }

    #[test]
    fn test_trail_verification() {
        let metrics = Arc::new(AuditMetrics::new());
        let engine = AuditEngine::new(metrics).unwrap();

        let event1 = AuditEvent {
            event_id: "ev-1".to_string(),
            request_id: "req-1".to_string(),
            event_type: "TOKEN_GENERATED".to_string(),
            payload: "token_1".to_string(),
            timestamp_ns: 1000,
        };

        let event2 = AuditEvent {
            event_id: "ev-2".to_string(),
            request_id: "req-1".to_string(),
            event_type: "TOKEN_GENERATED".to_string(),
            payload: "token_2".to_string(),
            timestamp_ns: 2000,
        };

        engine.record(event1).unwrap();
        engine.record(event2).unwrap();

        let verified = engine.verify().unwrap();
        assert!(verified);
    }
}
