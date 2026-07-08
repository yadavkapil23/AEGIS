// Audit module: cryptographically verifiable execution traces

pub mod engine;
pub mod trail;
pub mod metrics;

pub use engine::AuditEngine;
pub use trail::ExecutionTrail;
pub use metrics::AuditMetrics;


/// AuditConfig: configuration for audit trail
#[derive(Debug, Clone)]
pub struct AuditConfig {
    pub enabled: bool,
    pub hash_algorithm: String,  // "blake3"
    pub persist_to_disk: bool,
}

impl Default for AuditConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            hash_algorithm: "blake3".to_string(),
            persist_to_disk: false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_config() {
        let cfg = AuditConfig::default();
        assert!(cfg.enabled);
    }
}
