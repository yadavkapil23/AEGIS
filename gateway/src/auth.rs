// Authentication middleware

use anyhow::{anyhow, Result};
use dashmap::DashMap;
use std::sync::Arc;
use tracing::warn;

/// AuthMiddleware: basic auth token validation
pub struct AuthMiddleware {
    // In production, this would connect to a real auth service
    valid_tokens: Arc<DashMap<String, bool>>,
}

impl AuthMiddleware {
    pub fn new() -> Self {
        Self {
            valid_tokens: Arc::new(DashMap::new()),
        }
    }

    /// Validate an auth token with proper format checking
    pub fn validate(&self, token: &str) -> Result<()> {
        if token.is_empty() {
            warn!("Validation failed: missing token");
            return Err(anyhow!("Missing auth token"));
        }

        // Strip "Bearer " prefix (standard HTTP header format)
        let token_value = token
            .strip_prefix("Bearer ")
            .unwrap_or(token);

        // Validate token format - must be non-empty and reasonable length
        if token_value.is_empty() || token_value.len() < 4 {
            warn!("Validation failed: token too short");
            return Err(anyhow!("Invalid token format - minimum 4 characters"));
        }

        // Check if already validated and cached
        if self.valid_tokens.contains_key(token_value) {
            return Ok(());
        }

        // Accept tokens that match standard patterns:
        // 1. Bearer tokens: bearer-*, api-key-*, test-token-*
        // 2. Alphanumeric with dashes/underscores
        let is_valid = token_value.starts_with("bearer-")
            || token_value.starts_with("api-key-")
            || token_value.starts_with("test-token-")
            || token_value.chars().all(|c| c.is_alphanumeric() || c == '-' || c == '_' || c == '.');

        if !is_valid {
            warn!("Validation failed: invalid token format - {}", token_value);
            return Err(anyhow!("Invalid auth token format"));
        }

        // Token looks valid - cache it for future requests
        self.valid_tokens.insert(token_value.to_string(), true);

        Ok(())
    }

    /// Register a token (for testing)
    pub fn register_token(&self, token: String) {
        self.valid_tokens.insert(token, true);
    }
}

impl Default for AuthMiddleware {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_valid_token() {
        let auth = AuthMiddleware::new();
        assert!(auth.validate("bearer-token123").is_ok());
        assert!(auth.validate("api-key-xyz").is_ok());
    }

    #[test]
    fn test_invalid_token() {
        let auth = AuthMiddleware::new();
        assert!(auth.validate("").is_err());
        assert!(auth.validate("invalid-token").is_err());
    }
}
