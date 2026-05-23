// Configuration management for scheduler
// Supports YAML files, environment variables, and defaults

use serde::{Deserialize, Serialize};
use std::fs;
use anyhow::Result;
use std::path::Path;

/// Scheduler configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SchedulerConfig {
    /// Server configuration
    pub server: ServerConfig,

    /// Cache configuration
    pub cache: CacheConfig,

    /// Consensus configuration
    pub consensus: ConsensusConfig,

    /// Persistence configuration
    pub persistence: PersistenceConfig,

    /// Observability configuration
    pub observability: ObservabilityConfig,
}

/// Server configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ServerConfig {
    /// Server host to bind to
    #[serde(default = "default_host")]
    pub host: String,

    /// Server port
    #[serde(default = "default_port")]
    pub port: u16,

    /// gRPC port
    #[serde(default = "default_grpc_port")]
    pub grpc_port: u16,

    /// Node ID
    #[serde(default = "default_node_id")]
    pub node_id: String,

    /// Graceful shutdown timeout (seconds)
    #[serde(default = "default_shutdown_timeout")]
    pub shutdown_timeout_secs: u64,
}

/// Cache configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CacheConfig {
    /// Total cache size in bytes
    #[serde(default = "default_cache_size")]
    pub total_bytes: usize,

    /// Block size in bytes
    #[serde(default = "default_block_size")]
    pub block_size_bytes: usize,

    /// Eviction policy (lru, lfu)
    #[serde(default = "default_eviction_policy")]
    pub eviction_policy: String,

    /// Enable predictive allocation
    #[serde(default = "default_predictive")]
    pub enable_predictive: bool,
}

/// Consensus configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConsensusConfig {
    /// Cluster nodes (comma-separated or list)
    #[serde(default = "default_cluster_nodes")]
    pub cluster_nodes: Vec<String>,

    /// Election timeout (milliseconds)
    #[serde(default = "default_election_timeout")]
    pub election_timeout_ms: u64,

    /// Heartbeat interval (milliseconds)
    #[serde(default = "default_heartbeat_interval")]
    pub heartbeat_interval_ms: u64,
}

/// Persistence configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PersistenceConfig {
    /// Enable persistence
    #[serde(default = "default_persistence_enabled")]
    pub enabled: bool,

    /// Data directory path
    #[serde(default = "default_data_dir")]
    pub data_dir: String,

    /// Snapshot interval (number of commands)
    #[serde(default = "default_snapshot_interval")]
    pub snapshot_interval: usize,

    /// Sync mode (async, sync)
    #[serde(default = "default_sync_mode")]
    pub sync_mode: String,
}

/// Observability configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ObservabilityConfig {
    /// Log level (trace, debug, info, warn, error)
    #[serde(default = "default_log_level")]
    pub log_level: String,

    /// Enable metrics export
    #[serde(default = "default_metrics_enabled")]
    pub metrics_enabled: bool,

    /// Metrics export port
    #[serde(default = "default_metrics_port")]
    pub metrics_port: u16,

    /// Enable distributed tracing
    #[serde(default = "default_tracing_enabled")]
    pub tracing_enabled: bool,

    /// Tracing endpoint (OTLP)
    #[serde(default = "default_tracing_endpoint")]
    pub tracing_endpoint: String,
}

// Default values
fn default_host() -> String { "0.0.0.0".to_string() }
fn default_port() -> u16 { 50051 }
fn default_grpc_port() -> u16 { 50052 }
fn default_node_id() -> String { "node-1".to_string() }
fn default_shutdown_timeout() -> u64 { 30 }

fn default_cache_size() -> usize { 8 * 1024 * 1024 * 1024 } // 8GB
fn default_block_size() -> usize { 16 * 1024 } // 16KB
fn default_eviction_policy() -> String { "lru".to_string() }
fn default_predictive() -> bool { true }

fn default_cluster_nodes() -> Vec<String> {
    vec!["node-1".to_string()]
}
fn default_election_timeout() -> u64 { 150 }
fn default_heartbeat_interval() -> u64 { 50 }

fn default_persistence_enabled() -> bool { true }
fn default_data_dir() -> String { "./data".to_string() }
fn default_snapshot_interval() -> usize { 10000 }
fn default_sync_mode() -> String { "async".to_string() }

fn default_log_level() -> String { "info".to_string() }
fn default_metrics_enabled() -> bool { true }
fn default_metrics_port() -> u16 { 9090 }
fn default_tracing_enabled() -> bool { false }
fn default_tracing_endpoint() -> String { "http://localhost:4317".to_string() }

impl SchedulerConfig {
    /// Load configuration from YAML file
    pub fn from_file<P: AsRef<Path>>(path: P) -> Result<Self> {
        let content = fs::read_to_string(path)?;
        let config = serde_yaml::from_str(&content)?;
        Ok(config)
    }

    /// Load configuration from environment
    pub fn from_env() -> Self {
        Self {
            server: ServerConfig {
                host: std::env::var("SCHEDULER_HOST").unwrap_or_else(|_| default_host()),
                port: std::env::var("SCHEDULER_PORT")
                    .ok()
                    .and_then(|p| p.parse().ok())
                    .unwrap_or(default_port()),
                grpc_port: std::env::var("SCHEDULER_GRPC_PORT")
                    .ok()
                    .and_then(|p| p.parse().ok())
                    .unwrap_or(default_grpc_port()),
                node_id: std::env::var("SCHEDULER_NODE_ID").unwrap_or_else(|_| default_node_id()),
                shutdown_timeout_secs: std::env::var("SCHEDULER_SHUTDOWN_TIMEOUT")
                    .ok()
                    .and_then(|t| t.parse().ok())
                    .unwrap_or(default_shutdown_timeout()),
            },
            cache: CacheConfig {
                total_bytes: std::env::var("SCHEDULER_CACHE_SIZE")
                    .ok()
                    .and_then(|s| s.parse().ok())
                    .unwrap_or(default_cache_size()),
                block_size_bytes: std::env::var("SCHEDULER_BLOCK_SIZE")
                    .ok()
                    .and_then(|s| s.parse().ok())
                    .unwrap_or(default_block_size()),
                eviction_policy: std::env::var("SCHEDULER_EVICTION_POLICY")
                    .unwrap_or_else(|_| default_eviction_policy()),
                enable_predictive: std::env::var("SCHEDULER_PREDICTIVE")
                    .ok()
                    .and_then(|v| v.parse().ok())
                    .unwrap_or(default_predictive()),
            },
            consensus: ConsensusConfig {
                cluster_nodes: std::env::var("SCHEDULER_CLUSTER_NODES")
                    .ok()
                    .map(|nodes| nodes.split(',').map(|n| n.to_string()).collect())
                    .unwrap_or_else(|| default_cluster_nodes()),
                election_timeout_ms: std::env::var("SCHEDULER_ELECTION_TIMEOUT")
                    .ok()
                    .and_then(|t| t.parse().ok())
                    .unwrap_or(default_election_timeout()),
                heartbeat_interval_ms: std::env::var("SCHEDULER_HEARTBEAT_INTERVAL")
                    .ok()
                    .and_then(|t| t.parse().ok())
                    .unwrap_or(default_heartbeat_interval()),
            },
            persistence: PersistenceConfig {
                enabled: std::env::var("SCHEDULER_PERSISTENCE_ENABLED")
                    .ok()
                    .and_then(|v| v.parse().ok())
                    .unwrap_or(default_persistence_enabled()),
                data_dir: std::env::var("SCHEDULER_DATA_DIR").unwrap_or_else(|_| default_data_dir()),
                snapshot_interval: std::env::var("SCHEDULER_SNAPSHOT_INTERVAL")
                    .ok()
                    .and_then(|i| i.parse().ok())
                    .unwrap_or(default_snapshot_interval()),
                sync_mode: std::env::var("SCHEDULER_SYNC_MODE").unwrap_or_else(|_| default_sync_mode()),
            },
            observability: ObservabilityConfig {
                log_level: std::env::var("SCHEDULER_LOG_LEVEL").unwrap_or_else(|_| default_log_level()),
                metrics_enabled: std::env::var("SCHEDULER_METRICS_ENABLED")
                    .ok()
                    .and_then(|v| v.parse().ok())
                    .unwrap_or(default_metrics_enabled()),
                metrics_port: std::env::var("SCHEDULER_METRICS_PORT")
                    .ok()
                    .and_then(|p| p.parse().ok())
                    .unwrap_or(default_metrics_port()),
                tracing_enabled: std::env::var("SCHEDULER_TRACING_ENABLED")
                    .ok()
                    .and_then(|v| v.parse().ok())
                    .unwrap_or(default_tracing_enabled()),
                tracing_endpoint: std::env::var("SCHEDULER_TRACING_ENDPOINT")
                    .unwrap_or_else(|_| default_tracing_endpoint()),
            },
        }
    }

    /// Default configuration
    pub fn default() -> Self {
        Self {
            server: ServerConfig {
                host: default_host(),
                port: default_port(),
                grpc_port: default_grpc_port(),
                node_id: default_node_id(),
                shutdown_timeout_secs: default_shutdown_timeout(),
            },
            cache: CacheConfig {
                total_bytes: default_cache_size(),
                block_size_bytes: default_block_size(),
                eviction_policy: default_eviction_policy(),
                enable_predictive: default_predictive(),
            },
            consensus: ConsensusConfig {
                cluster_nodes: default_cluster_nodes(),
                election_timeout_ms: default_election_timeout(),
                heartbeat_interval_ms: default_heartbeat_interval(),
            },
            persistence: PersistenceConfig {
                enabled: default_persistence_enabled(),
                data_dir: default_data_dir(),
                snapshot_interval: default_snapshot_interval(),
                sync_mode: default_sync_mode(),
            },
            observability: ObservabilityConfig {
                log_level: default_log_level(),
                metrics_enabled: default_metrics_enabled(),
                metrics_port: default_metrics_port(),
                tracing_enabled: default_tracing_enabled(),
                tracing_endpoint: default_tracing_endpoint(),
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_config() {
        let config = SchedulerConfig::default();
        assert_eq!(config.server.port, 50051);
        assert_eq!(config.server.node_id, "node-1");
        assert!(config.persistence.enabled);
    }

    #[test]
    fn test_env_config() {
        std::env::set_var("SCHEDULER_NODE_ID", "test-node");
        std::env::set_var("SCHEDULER_PORT", "9999");

        let config = SchedulerConfig::from_env();
        assert_eq!(config.server.node_id, "test-node");
        assert_eq!(config.server.port, 9999);
    }
}
