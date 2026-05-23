//! Rate limiting for API requests

use crate::error::{Result, SecurityError};
use dashmap::DashMap;
use governor::{Quota, RateLimiter as GovernorRateLimiter};
use governor::state::{InMemoryState, NotKeyed};
use governor::clock::DefaultClock;
use std::num::NonZeroU32;

type DirectRateLimiter = GovernorRateLimiter<NotKeyed, InMemoryState, DefaultClock>;
use std::sync::Arc;
use serde::{Deserialize, Serialize};
use chrono::{DateTime, Utc};

/// Rate limit configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RateLimitConfig {
    /// Requests per second (global)
    pub global_rps: u32,

    /// Requests per second per API key
    pub per_key_rps: u32,

    /// Requests per second per IP address
    pub per_ip_rps: u32,

    /// Burst capacity (tokens available immediately)
    pub burst_size: u32,

    /// Enable per-key limiting
    pub enable_per_key: bool,

    /// Enable per-IP limiting
    pub enable_per_ip: bool,
}

impl Default for RateLimitConfig {
    fn default() -> Self {
        Self {
            global_rps: 10000,
            per_key_rps: 1000,
            per_ip_rps: 100,
            burst_size: 100,
            enable_per_key: true,
            enable_per_ip: true,
        }
    }
}

/// Per-identity rate limiter
struct IdentityLimiter {
    limiter: DirectRateLimiter,
    created_at: DateTime<Utc>,
    request_count: Arc<std::sync::atomic::AtomicU64>,
}

impl IdentityLimiter {
    fn new(rps: u32, burst: u32) -> Self {
        let quota = Quota::per_second(
            NonZeroU32::new(rps).unwrap_or_else(|| NonZeroU32::new(1).unwrap()),
        );
        let limiter = GovernorRateLimiter::direct(quota);

        Self {
            limiter,
            created_at: Utc::now(),
            request_count: Arc::new(std::sync::atomic::AtomicU64::new(0)),
        }
    }

    fn check(&self) -> bool {
        self.limiter.check().is_ok()
    }

    fn record_request(&self) {
        self.request_count
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    }

    fn get_count(&self) -> u64 {
        self.request_count
            .load(std::sync::atomic::Ordering::Relaxed)
    }
}

/// Rate limiting manager
pub struct RateLimiter {
    config: RateLimitConfig,

    // Global rate limiter
    global: IdentityLimiter,

    // Per-API-key limiters
    key_limiters: Arc<DashMap<String, IdentityLimiter>>,

    // Per-IP limiters
    ip_limiters: Arc<DashMap<String, IdentityLimiter>>,

    // Stats
    rejected_requests: Arc<std::sync::atomic::AtomicU64>,
}

impl RateLimiter {
    /// Create new rate limiter
    pub fn new(config: RateLimitConfig) -> Self {
        let global = IdentityLimiter::new(config.global_rps, config.burst_size);

        Self {
            config,
            global,
            key_limiters: Arc::new(DashMap::new()),
            ip_limiters: Arc::new(DashMap::new()),
            rejected_requests: Arc::new(std::sync::atomic::AtomicU64::new(0)),
        }
    }

    /// Check if request is allowed (global only)
    pub fn check_global(&self) -> Result<()> {
        if self.global.check() {
            self.global.record_request();
            Ok(())
        } else {
            self.rejected_requests
                .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            Err(SecurityError::RateLimitExceeded {
                limit: self.config.global_rps,
                window_secs: 1,
            })
        }
    }

    /// Check if request from API key is allowed
    pub fn check_api_key(&self, api_key_id: &str) -> Result<()> {
        // Check global first
        self.check_global()?;

        if !self.config.enable_per_key {
            return Ok(());
        }

        let allowed = {
            let mut entry = self.key_limiters
                .entry(api_key_id.to_string())
                .or_insert_with(|| IdentityLimiter::new(self.config.per_key_rps, self.config.burst_size));

            let is_allowed = entry.limiter.check().is_ok();
            if is_allowed {
                entry.record_request();
            }
            is_allowed
        };

        if allowed {
            Ok(())
        } else {
            self.rejected_requests
                .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            Err(SecurityError::RateLimitExceeded {
                limit: self.config.per_key_rps,
                window_secs: 1,
            })
        }
    }

    /// Check if request from IP is allowed
    pub fn check_ip(&self, ip_address: &str) -> Result<()> {
        // Check global first
        self.check_global()?;

        if !self.config.enable_per_ip {
            return Ok(());
        }

        let allowed = {
            let mut entry = self.ip_limiters
                .entry(ip_address.to_string())
                .or_insert_with(|| IdentityLimiter::new(self.config.per_ip_rps, self.config.burst_size));

            let is_allowed = entry.limiter.check().is_ok();
            if is_allowed {
                entry.record_request();
            }
            is_allowed
        };

        if allowed {
            Ok(())
        } else {
            self.rejected_requests
                .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            Err(SecurityError::RateLimitExceeded {
                limit: self.config.per_ip_rps,
                window_secs: 1,
            })
        }
    }

    /// Check if request is allowed (combined: global + key + IP)
    pub fn check(&self, api_key_id: Option<&str>, ip_address: Option<&str>) -> Result<()> {
        // Check global
        self.check_global()?;

        // Check API key
        if let Some(key_id) = api_key_id {
            self.check_api_key(key_id)?;
        }

        // Check IP
        if let Some(ip) = ip_address {
            self.check_ip(ip)?;
        }

        Ok(())
    }

    /// Get statistics
    pub fn stats(&self) -> RateLimiterStats {
        RateLimiterStats {
            global_requests: self.global.get_count(),
            key_limiters_count: self.key_limiters.len(),
            ip_limiters_count: self.ip_limiters.len(),
            rejected_requests: self.rejected_requests
                .load(std::sync::atomic::Ordering::Relaxed),
        }
    }

    /// Reset statistics
    pub fn reset_stats(&self) {
        self.rejected_requests
            .store(0, std::sync::atomic::Ordering::Relaxed);
    }

    /// Get per-key statistics
    pub fn key_stats(&self, api_key_id: &str) -> Option<KeyStats> {
        self.key_limiters.get(api_key_id).map(|limiter| KeyStats {
            api_key_id: api_key_id.to_string(),
            requests: limiter.get_count(),
            created_at: limiter.created_at,
        })
    }

    /// Clean up old limiters (called periodically)
    pub fn cleanup_old_limiters(&self, max_age_hours: i64) {
        use chrono::Duration;
        let cutoff = Utc::now() - Duration::hours(max_age_hours);

        // Remove old key limiters
        self.key_limiters.retain(|_, limiter| limiter.created_at > cutoff);

        // Remove old IP limiters
        self.ip_limiters.retain(|_, limiter| limiter.created_at > cutoff);
    }
}

/// Rate limiter statistics
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RateLimiterStats {
    pub global_requests: u64,
    pub key_limiters_count: usize,
    pub ip_limiters_count: usize,
    pub rejected_requests: u64,
}

/// Per-key statistics
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct KeyStats {
    pub api_key_id: String,
    pub requests: u64,
    pub created_at: DateTime<Utc>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_rate_limiter_global() {
        let config = RateLimitConfig {
            global_rps: 10,
            ..Default::default()
        };
        let limiter = RateLimiter::new(config);

        // Should allow initial requests
        for _ in 0..10 {
            assert!(limiter.check_global().is_ok());
        }

        // Should reject excess
        assert!(limiter.check_global().is_err());
    }

    #[test]
    fn test_rate_limiter_per_key() {
        let config = RateLimitConfig {
            global_rps: 1000,
            per_key_rps: 5,
            enable_per_key: true,
            ..Default::default()
        };
        let limiter = RateLimiter::new(config);

        let key = "test-key-1";

        // Should allow per-key limit
        for _ in 0..5 {
            assert!(limiter.check_api_key(key).is_ok());
        }

        // Should reject excess
        assert!(limiter.check_api_key(key).is_err());
    }

    #[test]
    fn test_rate_limiter_per_ip() {
        let config = RateLimitConfig {
            global_rps: 1000,
            per_ip_rps: 5,
            enable_per_ip: true,
            ..Default::default()
        };
        let limiter = RateLimiter::new(config);

        let ip = "192.168.1.1";

        // Should allow per-IP limit
        for _ in 0..5 {
            assert!(limiter.check_ip(ip).is_ok());
        }

        // Should reject excess
        assert!(limiter.check_ip(ip).is_err());
    }

    #[test]
    fn test_stats() {
        let config = RateLimitConfig::default();
        let limiter = RateLimiter::new(config);

        let _ = limiter.check_global();
        let stats = limiter.stats();

        assert!(stats.global_requests > 0);
    }
}
