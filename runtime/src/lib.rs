// Runtime module: orchestrates AEGIS execution pipeline

use aegis_audit::{AuditEngine, AuditMetrics};
use aegis_consensus::{ConsensusEngine, ConsensusConfig};
use aegis_gateway::{GatewayServer, GatewayConfig};
use aegis_proto::InferenceRequest;
use aegis_safety::{SafetyMonitor, SafetyMetrics};
use aegis_scheduler::{KVScheduler, SchedulerConfig};
use aegis_speculative::{SpeculativeCoordinator, SpeculativeMetrics};
use aegis_telemetry;
use anyhow::Result;
use std::sync::Arc;
use tracing::info;

/// AEGISRuntime: main execution orchestrator
pub struct AEGISRuntime {
    gateway: Arc<GatewayServer>,
    scheduler: Arc<KVScheduler>,
    speculative: Arc<SpeculativeCoordinator>,
    safety: Arc<SafetyMonitor>,
    audit: Arc<AuditEngine>,
    consensus: Arc<ConsensusEngine>,
}

/// RuntimeConfig: unified configuration
#[derive(Debug, Clone)]
pub struct RuntimeConfig {
    pub gateway: GatewayConfig,
    pub scheduler: SchedulerConfig,
}

impl Default for RuntimeConfig {
    fn default() -> Self {
        Self {
            gateway: GatewayConfig::default(),
            scheduler: SchedulerConfig::default(),
        }
    }
}

impl AEGISRuntime {
    /// Initialize AEGIS runtime
    pub async fn new(config: RuntimeConfig) -> Result<Self> {
        // Initialize telemetry
        aegis_telemetry::init_telemetry("aegis").await?;

        info!("Initializing AEGIS Runtime");

        // Create subsystems
        let gateway = Arc::new(GatewayServer::new(config.gateway));
        let scheduler = Arc::new(KVScheduler::new(config.scheduler)?);

        let spec_metrics = Arc::new(SpeculativeMetrics::new());
        let speculative = Arc::new(SpeculativeCoordinator::new(16, spec_metrics));

        let safety_metrics = Arc::new(SafetyMetrics::new());
        let safety = Arc::new(SafetyMonitor::new(safety_metrics));

        let audit_metrics = Arc::new(AuditMetrics::new());
        let audit = Arc::new(AuditEngine::new(audit_metrics)?);

        let consensus = Arc::new(ConsensusEngine::new(ConsensusConfig::default())?);

        info!("AEGIS Runtime initialized successfully");

        Ok(Self {
            gateway,
            scheduler,
            speculative,
            safety,
            audit,
            consensus,
        })
    }

    /// Execute an inference request
    pub async fn execute(&self, request: InferenceRequest) -> Result<Vec<String>> {
        let request_id = request.request_id.clone();
        
        // OpenTelemetry Trace Context (simulated active trace for this execution)
        let trace_id = uuid::Uuid::new_v4().to_string();
        info!(request_id = %request_id, trace_id = %trace_id, "Starting distributed execution trace");

        // Initialize audit trail
        let event = aegis_audit::engine::AuditEvent {
            event_id: uuid::Uuid::new_v4().to_string(),
            request_id: request_id.clone(),
            event_type: "REQUEST_RECEIVED".to_string(),
            payload: serde_json::json!({
                "request_id": request.request_id,
                "max_tokens": request.max_tokens,
                "temperature": request.temperature,
                "enable_speculation": request.enable_speculation,
                "trace_id": trace_id
            }).to_string(),
            timestamp_ns: chrono::Utc::now().timestamp_nanos_opt().unwrap_or(0) as u64,
        };

        self.audit.record(event)?;

        // Initialize safety
        self.safety.initialize_request(&request_id)?;

        // Allocate KV cache
        let blocks = self.scheduler.allocate(&request_id, 10)?;
        info!(request_id = %request_id, blocks = blocks.len(), "Physical KV cache allocated");

        // Initialize speculative branch
        self.speculative.create_branch(&request_id)?;

        let mut output_tokens = Vec::new();
        
        // 1. Generate Draft Tokens
        let draft_tokens = self.speculative.generate_draft(&request_id, &request.prompt, request.max_tokens as usize)?;
        
        // 2. Verify Draft Tokens with Target Model
        let acceptances = self.speculative.verify(&request_id, &request.prompt, &draft_tokens)?;
        
        let mut accepted_count = 0;
        for (i, &accepted) in acceptances.iter().enumerate() {
            if accepted {
                output_tokens.push(draft_tokens[i].text.clone());
                accepted_count += 1;
            } else {
                // 3. Rollback KV Cache upon first rejection
                info!(request_id = %request_id, token_index = i, "Speculative rejection, initiating rollback");
                self.speculative.rollback(&request_id, draft_tokens[i].id)?;
                break;
            }
        }
        
        // 4. Commit accepted tokens to branch history
        self.speculative.commit(&request_id, accepted_count)?;

        // Deallocate cache
        self.scheduler.deallocate(&blocks)?;
        info!(request_id = %request_id, accepted_tokens = accepted_count, "Execution complete, KV cache deallocated");

        Ok(output_tokens)
    }

    // Accessors
    pub fn gateway(&self) -> Arc<GatewayServer> {
        self.gateway.clone()
    }

    pub fn scheduler(&self) -> Arc<KVScheduler> {
        self.scheduler.clone()
    }

    pub fn speculative(&self) -> Arc<SpeculativeCoordinator> {
        self.speculative.clone()
    }

    pub fn safety(&self) -> Arc<SafetyMonitor> {
        self.safety.clone()
    }

    pub fn audit(&self) -> Arc<AuditEngine> {
        self.audit.clone()
    }

    pub fn consensus(&self) -> Arc<ConsensusEngine> {
        self.consensus.clone()
    }

    /// Get summary of all metrics
    pub fn metrics_summary(&self) -> MetricsSummary {
        MetricsSummary {
            gateway: self.gateway.metrics().summary(),
            scheduler: self.scheduler.metrics().summary(),
            speculative: self.speculative.metrics().summary(),
            safety: self.safety.metrics().summary(),
            audit: self.audit.metrics().summary(),
        }
    }
}

#[derive(Debug)]
pub struct MetricsSummary {
    pub gateway: aegis_gateway::metrics::GatewayMetricsSummary,
    pub scheduler: aegis_scheduler::metrics::SchedulerMetricsSummary,
    pub speculative: aegis_speculative::metrics::SpeculativeMetricsSummary,
    pub safety: aegis_safety::metrics::SafetyMetricsSummary,
    pub audit: aegis_audit::metrics::AuditMetricsSummary,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_runtime_creation() {
        let config = RuntimeConfig::default();
        let runtime = AEGISRuntime::new(config).await;
        assert!(runtime.is_ok());
    }

    #[tokio::test]
    async fn test_execute_request() {
        let config = RuntimeConfig::default();
        let runtime = AEGISRuntime::new(config).await.unwrap();

        let request = InferenceRequest {
            request_id: "test-req-1".to_string(),
            prompt: "Hello, world!".to_string(),
            max_tokens: 5,
            temperature: 0.7,
            top_p: 0.9,
            stop_tokens: vec![],
            seed: 42,
            enable_speculation: true,
            draft_length: 4,
            auth_token: "bearer-token".to_string(),
            metadata: Default::default(),
        };

        let result = runtime.execute(request).await;
        assert!(result.is_ok());
        let tokens = result.unwrap();
        assert!(!tokens.is_empty());
    }
}
