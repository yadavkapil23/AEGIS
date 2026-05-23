//! AEGIS Security Module
//!
//! Comprehensive security layer providing authentication, authorization, rate limiting,
//! and encrypted communication for production inference systems.
//!
//! # Features
//!
//! - **Authentication**: API keys, JWT tokens, mTLS certificates
//! - **Authorization**: Role-based access control (RBAC) via permissions
//! - **Rate Limiting**: Global, per-API-key, and per-IP request throttling
//! - **Encryption**: TLS/mTLS for secure communication
//! - **Key Management**: API key generation, rotation, and revocation
//!
//! # Quick Start
//!
//! ```ignore
//! use security::{
//!     ApiKeyProvider, JwtProvider, RateLimiter, TlsConfig,
//!     auth::{AuthenticationProvider, Credential},
//!     error::SecurityError,
//! };
//!
//! // API Key authentication
//! let api_key_provider = ApiKeyProvider::new();
//! let (key, metadata) = api_key_provider
//!     .generate_key("user1".to_string(), "org1".to_string(), vec!["read".to_string()], None)
//!     .unwrap();
//!
//! // Authenticate with key
//! let credential = Credential::ApiKey(key);
//! let principal = api_key_provider.authenticate(&credential).await.unwrap();
//!
//! // Rate limiting
//! let limiter = RateLimiter::new(Default::default());
//! limiter.check(Some(&principal.api_key_id.unwrap()), None).unwrap();
//!
//! // JWT tokens
//! let jwt = JwtProvider::new(Default::default()).unwrap();
//! let token = jwt.issue_token(&principal).unwrap();
//! ```

pub mod auth;
pub mod apikey;
pub mod jwt;
pub mod rate_limiter;
pub mod tls;
pub mod error;

pub use auth::{Principal, AuthenticationProvider, AuthMethod, Credential, MultiAuthProvider};
pub use apikey::ApiKeyProvider;
pub use jwt::{JwtProvider, TokenClaims, JwtConfig};
pub use rate_limiter::{RateLimiter, RateLimiterStats, RateLimitConfig};
pub use tls::{TlsConfig, TlsServerConfig, CertificateValidator, CertificateInfo, TlsMetrics};
pub use error::{SecurityError, Result};

/// Security prelude - commonly used items
pub mod prelude {
    pub use crate::auth::{Principal, AuthenticationProvider, AuthMethod, Credential};
    pub use crate::apikey::ApiKeyProvider;
    pub use crate::jwt::JwtProvider;
    pub use crate::rate_limiter::RateLimiter;
    pub use crate::tls::TlsConfig;
    pub use crate::error::SecurityError;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_api_key_provider_available() {
        let provider = ApiKeyProvider::new();
        let methods = provider.supported_methods();
        assert_eq!(methods, vec![AuthMethod::ApiKey]);
    }

    #[test]
    fn test_jwt_provider_available() {
        let provider = JwtProvider::new(Default::default()).unwrap();
        let methods = provider.supported_methods();
        assert_eq!(methods, vec![AuthMethod::Bearer]);
    }

    #[test]
    fn test_principal_creation() {
        let principal = Principal {
            id: "user1".to_string(),
            name: "User One".to_string(),
            api_key_id: None,
            permissions: vec!["read".to_string()],
            org_id: Some("org1".to_string()),
            scopes: vec![],
            metadata: Default::default(),
        };

        assert!(principal.has_permission("read"));
        assert!(!principal.has_permission("write"));
    }

    #[test]
    fn test_multi_auth_provider() {
        let api_key = ApiKeyProvider::new();
        let jwt = JwtProvider::new(Default::default()).unwrap();

        let multi = MultiAuthProvider::new()
            .with_api_key(std::sync::Arc::new(api_key))
            .with_jwt(std::sync::Arc::new(jwt));

        let methods = multi.supported_methods();
        assert_eq!(methods.len(), 2);
        assert!(methods.contains(&AuthMethod::ApiKey));
        assert!(methods.contains(&AuthMethod::Bearer));
    }
}
