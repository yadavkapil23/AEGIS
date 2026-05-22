/// Inference Service - simplified implementation
/// Handles basic inference request processing

use crate::metrics::GatewayMetrics;
use std::sync::Arc;
use tracing::info;

/// InferenceService: handles inference requests
pub struct InferenceService {
    metrics: Arc<GatewayMetrics>,
}

impl InferenceService {
    pub fn new(
        _max_concurrent: usize,
        _timeout_ms: u64,
        metrics: Arc<GatewayMetrics>,
    ) -> Self {
        info!("Initializing InferenceService");
        Self { metrics }
    }

    /// Get metrics
    pub fn metrics(&self) -> Arc<GatewayMetrics> {
        self.metrics.clone()
    }
}
