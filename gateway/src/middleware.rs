/// Middleware utilities for Actix-web
/// Authentication, authorization, and rate limiting

use std::sync::Arc;

/// Gateway application state
#[derive(Clone)]
pub struct GatewayState {
    /// Simple state holder for gateway
    pub name: String,
}

impl GatewayState {
    pub fn new() -> Self {
        Self {
            name: "AEGIS Gateway".to_string(),
        }
    }
}

impl Default for GatewayState {
    fn default() -> Self {
        Self::new()
    }
}
