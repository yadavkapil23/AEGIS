// Distributed tracing with OpenTelemetry (stub)
// Propagates trace context across nodes via gRPC headers

use std::collections::HashMap;

use parking_lot::Mutex;
use std::sync::Arc;

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

    pub fn with_baggage(mut self, key: String, value: String) -> Self {
        self.baggage.insert(key, value);
        self
    }

    pub fn to_headers(&self) -> Vec<(String, String)> {
        vec![
            ("x-trace-id".to_string(), self.trace_id.clone()),
            ("x-span-id".to_string(), self.span_id.clone()),
        ]
    }
}

#[derive(Clone, Debug)]
pub struct SpanRecorder {
    pub name: String,
    pub attributes: HashMap<String, String>,
}

impl SpanRecorder {
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            attributes: HashMap::new(),
        }
    }

    pub fn with_attribute(mut self, key: String, value: String) -> Self {
        self.attributes.insert(key, value);
        self
    }

    pub fn record(&self, _ctx: &DistributedTraceContext) {}
    pub fn record_success(&self, _ctx: &DistributedTraceContext, _duration_ms: u64) {}
    pub fn record_error(&self, _ctx: &DistributedTraceContext, _error: &str) {}
}

#[derive(Clone, Debug, Default)]
pub struct TracingMetrics {
    pub total_spans: Arc<Mutex<u64>>,
    pub completed_spans: Arc<Mutex<u64>>,
    pub failed_spans: Arc<Mutex<u64>>,
}

impl TracingMetrics {
    pub fn record_span(&self) {
        *self.total_spans.lock() += 1;
    }
    pub fn record_completion(&self, _duration_ms: u64) {
        *self.completed_spans.lock() += 1;
    }
    pub fn record_error(&self) {
        *self.failed_spans.lock() += 1;
    }
    pub fn success_rate(&self) -> f64 {
        let total = *self.total_spans.lock() as f64;
        if total == 0.0 {
            1.0
        } else {
            *self.completed_spans.lock() as f64 / total
        }
    }
}
