//! JWT token handling

use crate::auth::{AuthenticationProvider, Credential, Principal, AuthMethod};
use crate::error::{Result, SecurityError};
use async_trait::async_trait;
use chrono::{Duration, Utc};
use serde::{Deserialize, Serialize};
use jsonwebtoken::{decode, encode, DecodingKey, EncodingKey, Header, Validation};

/// JWT claims
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TokenClaims {
    /// Subject (user ID)
    pub sub: String,

    /// Issuer
    pub iss: String,

    /// Issued at
    pub iat: i64,

    /// Expiration time
    pub exp: i64,

    /// User name
    pub name: Option<String>,

    /// Organization ID
    pub org_id: Option<String>,

    /// Permissions
    pub permissions: Vec<String>,

    /// Scopes
    pub scopes: Vec<String>,

    /// API key ID (if applicable)
    pub api_key_id: Option<String>,
}

/// JWT configuration
#[derive(Debug, Clone)]
pub struct JwtConfig {
    /// Secret key for signing
    pub secret: String,

    /// Token issuer
    pub issuer: String,

    /// Token expiration in seconds
    pub expiration_secs: i64,

    /// Allow clock skew in seconds
    pub clock_skew_secs: u32,
}

impl Default for JwtConfig {
    fn default() -> Self {
        Self {
            secret: "your-secret-key".to_string(),
            issuer: "aegis".to_string(),
            expiration_secs: 3600, // 1 hour
            clock_skew_secs: 60,
        }
    }
}

/// JWT provider
pub struct JwtProvider {
    config: JwtConfig,
    encoding_key: EncodingKey,
    decoding_key: DecodingKey,
}

impl JwtProvider {
    /// Create new JWT provider
    pub fn new(config: JwtConfig) -> Result<Self> {
        let encoding_key = EncodingKey::from_secret(config.secret.as_bytes());
        let decoding_key = DecodingKey::from_secret(config.secret.as_bytes());

        Ok(Self {
            config,
            encoding_key,
            decoding_key,
        })
    }

    /// Create token from claims
    pub fn create_token(&self, claims: TokenClaims) -> Result<String> {
        encode(&Header::default(), &claims, &self.encoding_key)
            .map_err(|e| SecurityError::CryptographicError(e.to_string()))
    }

    /// Create token for principal
    pub fn issue_token(&self, principal: &Principal) -> Result<String> {
        let now = Utc::now();
        let exp = (now + Duration::seconds(self.config.expiration_secs)).timestamp();

        let claims = TokenClaims {
            sub: principal.id.clone(),
            iss: self.config.issuer.clone(),
            iat: now.timestamp(),
            exp,
            name: Some(principal.name.clone()),
            org_id: principal.org_id.clone(),
            permissions: principal.permissions.clone(),
            scopes: principal.scopes.clone(),
            api_key_id: principal.api_key_id.clone(),
        };

        self.create_token(claims)
    }

    /// Validate and decode token
    pub fn validate_token(&self, token: &str) -> Result<TokenClaims> {
        let mut validation = Validation::default();
        validation.set_issuer(&[self.config.issuer.clone()]);
        validation.leeway = self.config.clock_skew_secs as u64;

        decode::<TokenClaims>(token, &self.decoding_key, &validation)
            .map(|data| data.claims)
            .map_err(|e| {
                if e.to_string().contains("ExpiredSignature") {
                    SecurityError::TokenExpired
                } else if e.to_string().contains("InvalidSignature") {
                    SecurityError::SignatureInvalid
                } else {
                    SecurityError::InvalidToken(e.to_string())
                }
            })
    }

    /// Refresh token
    pub fn refresh_token(&self, token: &str) -> Result<String> {
        let claims = self.validate_token(token)?;

        let now = Utc::now();
        let exp = (now + Duration::seconds(self.config.expiration_secs)).timestamp();

        let new_claims = TokenClaims {
            iat: now.timestamp(),
            exp,
            ..claims
        };

        self.create_token(new_claims)
    }

    /// Verify token expiration
    pub fn is_expired(&self, token: &str) -> Result<bool> {
        match self.validate_token(token) {
            Ok(claims) => Ok(Utc::now().timestamp() > claims.exp),
            Err(SecurityError::TokenExpired) => Ok(true),
            Err(e) => Err(e),
        }
    }

    /// Get token expiration time
    pub fn expiration_time(&self, token: &str) -> Result<i64> {
        self.validate_token(token).map(|claims| claims.exp)
    }
}

#[async_trait]
impl AuthenticationProvider for JwtProvider {
    async fn authenticate(&self, credential: &Credential) -> Result<Principal> {
        match credential {
            Credential::Bearer(token) => {
                let claims = self.validate_token(token)?;

                Ok(Principal {
                    id: claims.sub,
                    name: claims.name.unwrap_or_default(),
                    api_key_id: claims.api_key_id,
                    permissions: claims.permissions,
                    org_id: claims.org_id,
                    scopes: claims.scopes,
                    metadata: Default::default(),
                })
            }
            _ => Err(SecurityError::AuthenticationRequired),
        }
    }

    async fn validate(&self, principal: &Principal) -> Result<()> {
        // For JWT, validation happens at authentication time
        // Here we just check principal is valid
        if principal.id.is_empty() {
            Err(SecurityError::InvalidToken("Empty principal ID".to_string()))
        } else {
            Ok(())
        }
    }

    async fn refresh(&self, principal: &Principal) -> Result<Credential> {
        let token = self.issue_token(principal)?;
        Ok(Credential::Bearer(token))
    }

    fn supported_methods(&self) -> Vec<AuthMethod> {
        vec![AuthMethod::Bearer]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_jwt_creation_and_validation() {
        let config = JwtConfig::default();
        let provider = JwtProvider::new(config).unwrap();

        let principal = Principal {
            id: "user1".to_string(),
            name: "User One".to_string(),
            api_key_id: None,
            permissions: vec!["read".to_string()],
            org_id: Some("org1".to_string()),
            scopes: vec![],
            metadata: Default::default(),
        };

        let token = provider.issue_token(&principal).unwrap();
        assert!(!token.is_empty());

        let claims = provider.validate_token(&token).unwrap();
        assert_eq!(claims.sub, "user1");
        assert_eq!(claims.org_id, Some("org1".to_string()));
    }

    #[test]
    fn test_jwt_invalid_token() {
        let config = JwtConfig::default();
        let provider = JwtProvider::new(config).unwrap();

        let result = provider.validate_token("invalid.token.here");
        assert!(result.is_err());
    }

    #[test]
    fn test_jwt_token_refresh() {
        let config = JwtConfig::default();
        let provider = JwtProvider::new(config).unwrap();

        let principal = Principal {
            id: "user1".to_string(),
            name: "User One".to_string(),
            api_key_id: None,
            permissions: vec![],
            org_id: None,
            scopes: vec![],
            metadata: Default::default(),
        };

        let token1 = provider.issue_token(&principal).unwrap();
        let token2 = provider.refresh_token(&token1).unwrap();

        // Both should be valid
        assert!(provider.validate_token(&token1).is_ok());
        assert!(provider.validate_token(&token2).is_ok());

        // token2 should have later iat/exp
        let claims1 = provider.validate_token(&token1).unwrap();
        let claims2 = provider.validate_token(&token2).unwrap();
        assert!(claims2.iat > claims1.iat);
    }
}
