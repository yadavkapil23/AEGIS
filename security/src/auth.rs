//! Authentication trait and middleware

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use crate::error::Result;

/// Authentication method
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AuthMethod {
    /// API key in header
    ApiKey,
    /// JWT bearer token
    Bearer,
    /// mTLS certificate
    MutualTls,
}

/// Authenticated principal information
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Principal {
    /// Unique identifier for the principal
    pub id: String,

    /// Name/alias
    pub name: String,

    /// API key ID (if API key auth)
    pub api_key_id: Option<String>,

    /// Permissions this principal has
    pub permissions: Vec<String>,

    /// Organization/tenant ID
    pub org_id: Option<String>,

    /// Token scopes (if JWT)
    pub scopes: Vec<String>,

    /// Metadata
    pub metadata: std::collections::HashMap<String, String>,
}

impl Principal {
    /// Check if principal has a specific permission
    pub fn has_permission(&self, permission: &str) -> bool {
        self.permissions.contains(&permission.to_string())
    }

    /// Check if principal has all specified permissions
    pub fn has_all_permissions(&self, permissions: &[&str]) -> bool {
        permissions.iter().all(|p| self.has_permission(p))
    }

    /// Check if principal has any of the specified permissions
    pub fn has_any_permission(&self, permissions: &[&str]) -> bool {
        permissions.iter().any(|p| self.has_permission(p))
    }
}

/// Credential types
#[derive(Debug, Clone)]
pub enum Credential {
    /// API key credential
    ApiKey(String),

    /// Bearer token
    Bearer(String),

    /// Certificate (for mTLS)
    Certificate(Vec<u8>),
}

/// Authentication provider trait
#[async_trait]
pub trait AuthenticationProvider: Send + Sync {
    /// Authenticate using credentials
    async fn authenticate(&self, credential: &Credential) -> Result<Principal>;

    /// Validate authentication of principal
    async fn validate(&self, principal: &Principal) -> Result<()>;

    /// Refresh credentials (if applicable)
    async fn refresh(&self, principal: &Principal) -> Result<Credential>;

    /// Supported auth methods
    fn supported_methods(&self) -> Vec<AuthMethod>;

    /// Check if this provider supports a method
    fn supports_method(&self, method: AuthMethod) -> bool {
        self.supported_methods().contains(&method)
    }
}

/// Multi-auth provider supporting multiple authentication methods
pub struct MultiAuthProvider {
    api_key_provider: Option<std::sync::Arc<dyn AuthenticationProvider>>,
    jwt_provider: Option<std::sync::Arc<dyn AuthenticationProvider>>,
    mtls_provider: Option<std::sync::Arc<dyn AuthenticationProvider>>,
}

impl MultiAuthProvider {
    /// Create new multi-auth provider
    pub fn new() -> Self {
        Self {
            api_key_provider: None,
            jwt_provider: None,
            mtls_provider: None,
        }
    }

    /// Set API key provider
    pub fn with_api_key(mut self, provider: std::sync::Arc<dyn AuthenticationProvider>) -> Self {
        self.api_key_provider = Some(provider);
        self
    }

    /// Set JWT provider
    pub fn with_jwt(mut self, provider: std::sync::Arc<dyn AuthenticationProvider>) -> Self {
        self.jwt_provider = Some(provider);
        self
    }

    /// Set mTLS provider
    pub fn with_mtls(mut self, provider: std::sync::Arc<dyn AuthenticationProvider>) -> Self {
        self.mtls_provider = Some(provider);
        self
    }
}

#[async_trait]
impl AuthenticationProvider for MultiAuthProvider {
    async fn authenticate(&self, credential: &Credential) -> Result<Principal> {
        match credential {
            Credential::ApiKey(_) => {
                if let Some(provider) = &self.api_key_provider {
                    provider.authenticate(credential).await
                } else {
                    Err(crate::error::SecurityError::AuthenticationRequired)
                }
            }
            Credential::Bearer(_) => {
                if let Some(provider) = &self.jwt_provider {
                    provider.authenticate(credential).await
                } else {
                    Err(crate::error::SecurityError::AuthenticationRequired)
                }
            }
            Credential::Certificate(_) => {
                if let Some(provider) = &self.mtls_provider {
                    provider.authenticate(credential).await
                } else {
                    Err(crate::error::SecurityError::AuthenticationRequired)
                }
            }
        }
    }

    async fn validate(&self, principal: &Principal) -> Result<()> {
        // Validate based on which auth method was used
        if let Some(_key_id) = &principal.api_key_id {
            if let Some(provider) = &self.api_key_provider {
                return provider.validate(principal).await;
            }
        }
        Ok(())
    }

    async fn refresh(&self, principal: &Principal) -> Result<Credential> {
        if let Some(_key_id) = &principal.api_key_id {
            if let Some(provider) = &self.api_key_provider {
                return provider.refresh(principal).await;
            }
        }
        Err(crate::error::SecurityError::AuthenticationRequired)
    }

    fn supported_methods(&self) -> Vec<AuthMethod> {
        let mut methods = Vec::new();
        if self.api_key_provider.is_some() {
            methods.push(AuthMethod::ApiKey);
        }
        if self.jwt_provider.is_some() {
            methods.push(AuthMethod::Bearer);
        }
        if self.mtls_provider.is_some() {
            methods.push(AuthMethod::MutualTls);
        }
        methods
    }
}

impl Default for MultiAuthProvider {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_principal_permissions() {
        let mut principal = Principal {
            id: "user1".to_string(),
            name: "User One".to_string(),
            api_key_id: None,
            permissions: vec!["read".to_string(), "write".to_string()],
            org_id: None,
            scopes: vec![],
            metadata: Default::default(),
        };

        assert!(principal.has_permission("read"));
        assert!(!principal.has_permission("delete"));
        assert!(principal.has_all_permissions(&["read", "write"]));
        assert!(!principal.has_all_permissions(&["read", "delete"]));
    }

    #[test]
    fn test_multi_auth_provider() {
        let provider = MultiAuthProvider::new();
        assert_eq!(provider.supported_methods().len(), 0);

        let with_methods = provider.supported_methods();
        assert!(with_methods.is_empty());
    }
}
