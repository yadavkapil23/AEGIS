// Inference service implementation: gRPC handler for inference requests

use crate::metrics::GatewayMetrics;
use crate::request_queue::RequestQueue;
use aegis_proto::{InferenceRequest, InferenceResponse, Token, InferenceMetrics, HealthCheckRequest, HealthCheckResponse};
use anyhow::Result;
use std::sync::Arc;
use std::time::Instant;
use tracing::{info, warn, instrument};
use uuid::Uuid;

/// InferenceService: handles inference requests
pub struct InferenceService {
    queue: RequestQueue,
    metrics: Arc<GatewayMetrics>,
    max_concurrent: usize,
}

impl InferenceService {
    pub fn new(
        max_concurrent: usize,
        timeout_ms: u64,
        metrics: Arc<GatewayMetrics>,
    ) -> Self {
        let queue = RequestQueue::new(max_concurrent, timeout_ms);

        Self {
            queue,
            metrics,
            max_concurrent,
        }
    }

    /// Process an inference request
    #[instrument(skip(self, request), fields(request_id = %request.request_id))]
    pub async fn infer(&self, request: InferenceRequest) -> Result<Vec<InferenceResponse>> {
        let start = Instant::now();
        let request_id = request.request_id.clone();

        // Rate limiting check
        if !self.metrics.rate_limiter.allow_request() {
            self.metrics.record_rate_limited();
            warn!("Request rate limited");
            return Err(anyhow::anyhow!("Rate limited"));
        }

        // Queue the request
        let _queued = self.queue.enqueue(&request)?;
        self.metrics.record_queued();

        // Simulate inference pipeline
        let responses = self.generate_tokens(&request).await?;

        self.queue.complete(&request_id)?;

        let latency_ms = start.elapsed().as_secs_f64() * 1000.0;
        self.metrics.record_latency(latency_ms);
        self.metrics.record_completed();

        info!(
            request_id = %request_id,
            tokens_generated = responses.len(),
            latency_ms = latency_ms,
            "Inference complete"
        );

        Ok(responses)
    }

    /// Generate tokens with realistic simulation
    async fn generate_tokens(&self, request: &InferenceRequest) -> Result<Vec<InferenceResponse>> {
        let mut responses = Vec::new();
        let trace_id = Uuid::new_v4().to_string();
        let start = Instant::now();

        // Generate realistic number of tokens (no arbitrary limit)
        let num_tokens = request.max_tokens as usize;

        for i in 0..num_tokens {
            let elapsed_ms = start.elapsed().as_secs_f64() * 1000.0;
            let tokens_per_sec = if elapsed_ms > 0.0 {
                ((i + 1) as f64) / (elapsed_ms / 1000.0)
            } else {
                0.0
            };

            // Simulate realistic token generation with variance
            let token_text = match i {
                0 => "The".to_string(),
                1 => " distributed".to_string(),
                2 => " system".to_string(),
                3 => " works".to_string(),
                4 => " efficiently".to_string(),
                _ => format!(" token{}", i),
            };

            let token = Token {
                id: i as i32,
                text: token_text,
                logprob: -0.5 - (i as f32 * 0.1), // Decreasing probability
                accepted: true,
                trace_id: trace_id.clone(),
            };

            // Calculate realistic cache metrics
            let cache_hits = (i as i32) * 2; // Grow with position
            let cache_misses = if i > 5 { (i - 5) as i32 } else { 0 };

            let response = InferenceResponse {
                request_id: request.request_id.clone(),
                token: Some(token),
                position: i as i32,
                status: if i == num_tokens - 1 {
                    "COMPLETE".to_string()
                } else {
                    "GENERATING".to_string()
                },
                stop_reasons: if i == num_tokens - 1 {
                    vec!["max_tokens".to_string()]
                } else {
                    vec![]
                },
                metrics: Some(InferenceMetrics {
                    elapsed_ms: elapsed_ms as f32,
                    tokens_per_second: tokens_per_sec as f32,
                    kv_cache_hits: cache_hits,
                    kv_cache_misses: cache_misses,
                    speculative_tokens_tried: 0,
                    speculative_tokens_accepted: 0,
                    cache_fragmentation: (cache_misses as f32) / std::cmp::max(1, cache_hits + cache_misses) as f32,
                    hardware_node: "scheduler-0".to_string(),
                }),
                error: "".to_string(),
            };

            responses.push(response);
        }

        info!(
            request_id = %request.request_id,
            tokens_generated = num_tokens,
            latency_ms = start.elapsed().as_secs_f64() * 1000.0,
            "Token generation complete"
        );

        Ok(responses)
    }

    /// Health check
    pub async fn health_check(&self, _request: HealthCheckRequest) -> Result<HealthCheckResponse> {
        Ok(HealthCheckResponse {
            status: "SERVING".to_string(),
            details: Default::default(),
        })
    }

    /// Get current queue depth
    pub fn queue_depth(&self) -> usize {
        self.queue.depth()
    }

    /// Get active stream count
    pub fn active_streams(&self) -> usize {
        self.queue.active_streams()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use aegis_telemetry::metrics::RateLimiter;

    #[tokio::test]
    async fn test_infer_basic() {
        let metrics = Arc::new(GatewayMetrics::new());
        let service = InferenceService::new(100, 5000, metrics);

        let request = InferenceRequest {
            request_id: "test-1".to_string(),
            prompt: "Hello, world!".to_string(),
            max_tokens: 5,
            temperature: 0.7,
            top_p: 0.9,
            stop_tokens: vec![],
            seed: 42,
            enable_speculation: false,
            draft_length: 0,
            auth_token: "token".to_string(),
            metadata: Default::default(),
        };

        let responses = service.infer(request).await;
        assert!(responses.is_ok());
        let responses = responses.unwrap();
        assert_eq!(responses.len(), 5);
    }

    #[tokio::test]
    async fn test_health_check() {
        let metrics = Arc::new(GatewayMetrics::new());
        let service = InferenceService::new(100, 5000, metrics);

        let request = HealthCheckRequest {
            service: "inference".to_string(),
        };

        let response = service.health_check(request).await;
        assert!(response.is_ok());
        let response = response.unwrap();
        assert_eq!(response.status, "SERVING");
    }
}
