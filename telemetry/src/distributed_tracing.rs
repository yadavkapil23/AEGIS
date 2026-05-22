// Distributed tracing with OpenTelemetry (stub)
// Propagates trace context across nodes via gRPC headers

use std::collections::HashMap;
use tracing::{debug, info};

/// Distributed trace context for cross-node requests
#[derive(Clone, Debug)]
pub struct DistributedTraceContext {
    pub trace_id: String,
    pub span_id: String,
    pub parent_span_id: Option<String>,
    pub baggage: HashMap<String, String>,
}

impl DistributedTraceContext {
    /// Create a new root trace context
    pub fn new(_request_id: impl Into<String>) -> Self {
        Self {
            trace_id: uuid::Uuid::new_v4().to_string(),
            span_id: uuid::Uuid::new_v4().to_string(),
            parent_span_id: None,
            baggage: HashMap::new(),
        }
    }

    /// Create a child span context for the same trace
    pub fn child_span(&self) -> Self {
        Self {
            trace_id: self.trace_id.clone(),
            span_id: uuid::Uuid::new_v4().to_string(),
            parent_span_id: Some(self.span_id.clone()),
            baggage: self.baggage.clone(),
        }
    }

    /// Convert to gRPC traceparent header
    pub fn to_traceparent(&self) -> String {
        format!("00-{}-{}-01", self.trace_id, self.span_id)
    }
}
