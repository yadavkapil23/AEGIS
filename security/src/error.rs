//! Security module error types

use std::fmt;

/// Error type for security operations
#[derive(Debug, Clone)]
pub enum SecurityError {
    /// API key validation failed
    InvalidApiKey(String),

    /// JWT token validation failed
    InvalidToken(String),

    /// Token expired
    TokenExpired,

    /// Token not yet valid
    TokenNotYetValid,

    /// Signature verification failed
    SignatureInvalid,

    /// Rate limit exceeded
    RateLimitExceeded {
        limit: u32,
        window_secs: u64,
    },

    /// TLS configuration error
    TlsConfigurationError(String),

    /// Key rotation failed
    KeyRotationFailed(String),

    /// API key not found
    ApiKeyNotFound(String),

    /// Insufficient permissions
    InsufficientPermissions,

    /// Authentication required
    AuthenticationRequired,

    /// Cryptographic operation failed
    CryptographicError(String),

    /// Configuration error
    ConfigurationError(String),
}

impl fmt::Display for SecurityError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            SecurityError::InvalidApiKey(msg) => {
                write!(f, "Invalid API key: {}", msg)
            }
            SecurityError::InvalidToken(msg) => {
                write!(f, "Invalid token: {}", msg)
            }
            SecurityError::TokenExpired => {
                write!(f, "Token has expired")
            }
            SecurityError::TokenNotYetValid => {
                write!(f, "Token is not yet valid")
            }
            SecurityError::SignatureInvalid => {
                write!(f, "Token signature is invalid")
            }
            SecurityError::RateLimitExceeded { limit, window_secs } => {
                write!(f, "Rate limit exceeded: {} requests per {} seconds", limit, window_secs)
            }
            SecurityError::TlsConfigurationError(msg) => {
                write!(f, "TLS configuration error: {}", msg)
            }
            SecurityError::KeyRotationFailed(msg) => {
                write!(f, "Key rotation failed: {}", msg)
            }
            SecurityError::ApiKeyNotFound(key) => {
                write!(f, "API key not found: {}", key)
            }
            SecurityError::InsufficientPermissions => {
                write!(f, "Insufficient permissions for this operation")
            }
            SecurityError::AuthenticationRequired => {
                write!(f, "Authentication is required")
            }
            SecurityError::CryptographicError(msg) => {
                write!(f, "Cryptographic error: {}", msg)
            }
            SecurityError::ConfigurationError(msg) => {
                write!(f, "Security configuration error: {}", msg)
            }
        }
    }
}

impl std::error::Error for SecurityError {}

/// Result type for security operations
pub type Result<T> = std::result::Result<T, SecurityError>;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_error_display() {
        let err = SecurityError::InvalidApiKey("test-key".to_string());
        assert_eq!(err.to_string(), "Invalid API key: test-key");
    }

    #[test]
    fn test_rate_limit_error() {
        let err = SecurityError::RateLimitExceeded {
            limit: 100,
            window_secs: 60,
        };
        assert!(err.to_string().contains("100 requests per 60 seconds"));
    }
}
