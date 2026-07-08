// Safety module: runtime policy enforcement

pub mod monitor;
pub mod policy;
pub mod metrics;

pub use monitor::SafetyMonitor;
pub use policy::{Policy, PolicyAction};
pub use metrics::SafetyMetrics;


/// SafetyConfig: configuration for safety enforcement
#[derive(Debug, Clone)]
pub struct SafetyConfig {
    pub enabled: bool,
    pub strict_mode: bool,
    pub fallback_strategy: String,
}

impl Default for SafetyConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            strict_mode: false,
            fallback_strategy: "reject".to_string(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_config() {
        let cfg = SafetyConfig::default();
        assert!(cfg.enabled);
    }
}
