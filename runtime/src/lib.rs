// Runtime module: orchestrates AEGIS execution pipeline

use aegis_audit::{AuditEngine, AuditMetrics};
use aegis_consensus::{ConsensusEngine, ConsensusConfig};
use aegis_gateway::GatewayConfig;
use aegis_safety::{SafetyMonitor, SafetyMetrics};
use aegis_scheduler::{KVScheduler, SchedulerConfig};
use anyhow::Result;
use std::sync::Arc;
use tracing::info;

/// AEGISRuntime: main execution orchestrator
pub struct AEGISRuntime {
    scheduler: Arc<KVScheduler>,
    safety: Arc<SafetyMonitor>,
    audit: Arc<AuditEngine>,
    consensus: Arc<ConsensusEngine>,
}

/// RuntimeConfig: unified configuration
#[derive(Debug, Clone)]
pub struct RuntimeConfig {
    pub scheduler: SchedulerConfig,
}

impl Default for RuntimeConfig {
    fn default() -> Self {
        Self {
            scheduler: SchedulerConfig::default(),
        }
    }
}

impl AEGISRuntime {
    /// Initialize AEGIS runtime
    pub async fn new(config: RuntimeConfig) -> Result<Self> {
        aegis_telemetry::init_telemetry("aegis").await?;

        info!("Initializing AEGIS Runtime");

        let scheduler = Arc::new(KVScheduler::new(config.scheduler)?);

        let safety_metrics = Arc::new(SafetyMetrics::new());
        let safety = Arc::new(SafetyMonitor::new(safety_metrics));

        let audit_metrics = Arc::new(AuditMetrics::new());
        let audit = Arc::new(AuditEngine::new(audit_metrics)?);

        let consensus = Arc::new(ConsensusEngine::new(ConsensusConfig::default())?);

        info!("AEGIS Runtime initialized successfully");

        Ok(Self {
            scheduler,
            safety,
            audit,
            consensus,
        })
    }

    // Accessors
    pub fn scheduler(&self) -> Arc<KVScheduler> {
        self.scheduler.clone()
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
            scheduler: self.scheduler.metrics().summary(),
            safety: self.safety.metrics().summary(),
            audit: self.audit.metrics().summary(),
        }
    }
}

#[derive(Debug)]
pub struct MetricsSummary {
    pub scheduler: aegis_scheduler::metrics::SchedulerMetricsSummary,
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
}
