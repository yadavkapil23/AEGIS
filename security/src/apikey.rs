//! API key management and validation

use crate::auth::{AuthenticationProvider, Credential, Principal, AuthMethod};
use crate::error::{Result, SecurityError};
use async_trait::async_trait;
use chrono::{DateTime, Utc, Duration};
use dashmap::DashMap;
use serde::{Deserialize, Serialize};
use sha2::{Sha256, Digest};
use std::sync::Arc;

/// API key metadata
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ApiKeyMetadata {
    /// Unique key ID
    pub id: String,

    /// Key name for identification
    pub name: String,

    /// Owner/creator
    pub owner: String,

    /// Organization ID
    pub org_id: String,

    /// Permissions granted
    pub permissions: Vec<String>,

    /// When key was created
    pub created_at: DateTime<Utc>,

    /// When key expires (None = never)
    pub expires_at: Option<DateTime<Utc>>,

    /// Last used timestamp
    pub last_used: Option<DateTime<Utc>>,

    /// Is key revoked
    pub revoked: bool,

    /// Revocation reason
    pub revocation_reason: Option<String>,

    /// API key prefix (for display, e.g., "sk-abc123...")
    pub prefix: String,

    /// Usage quota per minute (None = unlimited)
    pub quota_per_minute: Option<u32>,
}

/// API key with hash (internal storage)
#[derive(Clone)]
pub struct StoredApiKey {
    pub metadata: ApiKeyMetadata,
    pub hash: String,
}

/// API Key Provider implementation
pub struct ApiKeyProvider {
    keys: Arc<DashMap<String, StoredApiKey>>,
}

impl ApiKeyProvider {
    /// Create new API key provider
    pub fn new() -> Self {
        Self {
            keys: Arc::new(DashMap::new()),
        }
    }

    /// Hash API key for storage
    fn hash_key(key: &str) -> String {
        let mut hasher = Sha256::new();
        hasher.update(key.as_bytes());
        format!("{:x}", hasher.finalize())
    }

    /// Generate new API key
    pub fn generate_key(
        &self,
        owner: String,
        org_id: String,
        permissions: Vec<String>,
        expires_in_days: Option<i64>,
    ) -> Result<(String, ApiKeyMetadata)> {
        // Generate random key
        use rand::Rng;
        let mut rng = rand::thread_rng();
        let random_bytes: Vec<u8> = (0..32).map(|_| rng.gen()).collect();
        let key = format!("sk-{}", hex::encode(&random_bytes));

        let key_id = format!("key-{}", uuid::Uuid::new_v4());
        let prefix = key.chars().take(10).collect::<String>();
        let created_at = Utc::now();
        let expires_at = expires_in_days.map(|days| created_at + Duration::days(days));

        let metadata = ApiKeyMetadata {
            id: key_id.clone(),
            name: format!("API Key {}", prefix),
            owner,
            org_id,
            permissions,
            created_at,
            expires_at,
            last_used: None,
            revoked: false,
            revocation_reason: None,
            prefix,
            quota_per_minute: None,
        };

        let stored = StoredApiKey {
            metadata: metadata.clone(),
            hash: Self::hash_key(&key),
        };

        self.keys.insert(key_id, stored);

        Ok((key, metadata))
    }

    /// Validate API key
    pub fn validate_key(&self, key: &str) -> Result<ApiKeyMetadata> {
        // Extract key ID from key (would be embedded in real implementation)
        // For now, search by hash
        let key_hash = Self::hash_key(key);

        for entry in self.keys.iter() {
            if entry.value().hash == key_hash {
                let mut metadata = entry.value().metadata.clone();

                // Check if revoked
                if metadata.revoked {
                    return Err(SecurityError::InvalidApiKey(
                        format!("Key {} is revoked", metadata.id),
                    ));
                }

                // Check if expired
                if let Some(expires_at) = metadata.expires_at {
                    if Utc::now() > expires_at {
                        return Err(SecurityError::TokenExpired);
                    }
                }

                // Update last used
                metadata.last_used = Some(Utc::now());
                return Ok(metadata);
            }
        }

        Err(SecurityError::InvalidApiKey("Unknown key".to_string()))
    }

    /// Get key metadata by ID
    pub fn get_key(&self, key_id: &str) -> Result<ApiKeyMetadata> {
        self.keys
            .get(key_id)
            .map(|entry| entry.value().metadata.clone())
            .ok_or_else(|| SecurityError::ApiKeyNotFound(key_id.to_string()))
    }

    /// Revoke a key
    pub fn revoke_key(&self, key_id: &str, reason: Option<String>) -> Result<()> {
        if let Some(mut entry) = self.keys.get_mut(key_id) {
            entry.metadata.revoked = true;
            entry.metadata.revocation_reason = reason;
            Ok(())
        } else {
            Err(SecurityError::ApiKeyNotFound(key_id.to_string()))
        }
    }

    /// List all keys for an organization
    pub fn list_keys(&self, org_id: &str) -> Vec<ApiKeyMetadata> {
        self.keys
            .iter()
            .filter(|entry| entry.value().metadata.org_id == org_id)
            .map(|entry| entry.value().metadata.clone())
            .collect()
    }

    /// Rotate a key (revoke old, create new)
    pub fn rotate_key(&self, old_key_id: &str, new_permissions: Option<Vec<String>>) -> Result<(String, ApiKeyMetadata)> {
        let old_metadata = self.get_key(old_key_id)?;

        // Revoke old key
        self.revoke_key(old_key_id, Some("Rotated".to_string()))?;

        // Create new key with same properties
        let permissions = new_permissions.unwrap_or(old_metadata.permissions);
        let (new_key, new_metadata) = self.generate_key(
            old_metadata.owner,
            old_metadata.org_id,
            permissions,
            old_metadata.expires_at.map(|exp| {
                (exp - Utc::now()).num_days()
            }),
        )?;

        Ok((new_key, new_metadata))
    }
}

impl Default for ApiKeyProvider {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl AuthenticationProvider for ApiKeyProvider {
    async fn authenticate(&self, credential: &Credential) -> Result<Principal> {
        match credential {
            Credential::ApiKey(key) => {
                let metadata = self.validate_key(key)?;

                Ok(Principal {
                    id: metadata.owner.clone(),
                    name: metadata.name.clone(),
                    api_key_id: Some(metadata.id),
                    permissions: metadata.permissions.clone(),
                    org_id: Some(metadata.org_id.clone()),
                    scopes: vec![],
                    metadata: Default::default(),
                })
            }
            _ => Err(SecurityError::AuthenticationRequired),
        }
    }

    async fn validate(&self, principal: &Principal) -> Result<()> {
        if let Some(key_id) = &principal.api_key_id {
            self.get_key(key_id).map(|_| ())
        } else {
            Err(SecurityError::AuthenticationRequired)
        }
    }

    async fn refresh(&self, principal: &Principal) -> Result<Credential> {
        if let Some(key_id) = &principal.api_key_id {
            let metadata = self.get_key(key_id)?;
            if !metadata.revoked {
                // In real implementation, would generate new credential
                Ok(Credential::ApiKey(format!("sk-refreshed-{}", key_id)))
            } else {
                Err(SecurityError::InvalidApiKey("Key is revoked".to_string()))
            }
        } else {
            Err(SecurityError::AuthenticationRequired)
        }
    }

    fn supported_methods(&self) -> Vec<AuthMethod> {
        vec![AuthMethod::ApiKey]
    }
}

// Re-export for convenience
pub use hex;

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_api_key_generation() {
        let provider = ApiKeyProvider::new();
        let (key, metadata) = provider
            .generate_key(
                "user1".to_string(),
                "org1".to_string(),
                vec!["read".to_string()],
                Some(30),
            )
            .unwrap();

        assert!(key.starts_with("sk-"));
        assert_eq!(metadata.owner, "user1");
        assert!(!metadata.revoked);
    }

    #[tokio::test]
    async fn test_api_key_validation() {
        let provider = ApiKeyProvider::new();
        let (key, _) = provider
            .generate_key(
                "user1".to_string(),
                "org1".to_string(),
                vec!["read".to_string()],
                Some(30),
            )
            .unwrap();

        let metadata = provider.validate_key(&key).unwrap();
        assert_eq!(metadata.owner, "user1");
    }

    #[tokio::test]
    async fn test_api_key_revocation() {
        let provider = ApiKeyProvider::new();
        let (key, metadata) = provider
            .generate_key(
                "user1".to_string(),
                "org1".to_string(),
                vec!["read".to_string()],
                Some(30),
            )
            .unwrap();

        provider.revoke_key(&metadata.id, Some("Testing".to_string())).unwrap();

        assert!(provider.validate_key(&key).is_err());
    }
}
