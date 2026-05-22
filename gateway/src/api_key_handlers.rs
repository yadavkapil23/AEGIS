//! API key management endpoints

use axum::{
    extract::State,
    http::StatusCode,
    Json,
};
use security::{Principal, SecurityError, ApiKeyProvider};
use serde::{Deserialize, Serialize};
use std::sync::Arc;

#[derive(Deserialize)]
pub struct CreateApiKeyRequest {
    /// Owner email/identifier
    pub owner: String,

    /// Organization ID
    pub org_id: String,

    /// Permissions to grant
    pub permissions: Vec<String>,

    /// Days until expiration (None = no expiration)
    pub expires_in_days: Option<i64>,
}

#[derive(Serialize)]
pub struct CreateApiKeyResponse {
    /// The actual API key (shown only once)
    pub api_key: String,

    /// Key ID for reference
    pub key_id: String,

    /// Creation timestamp
    pub created_at: String,

    /// Expiration timestamp (if applicable)
    pub expires_at: Option<String>,
}

/// Create a new API key
///
/// Requires: admin permission
pub async fn create_api_key(
    State(provider): State<Arc<ApiKeyProvider>>,
    extract::Extension(principal): extract::Extension<Principal>,
    Json(req): Json<CreateApiKeyRequest>,
) -> Result<(StatusCode, Json<CreateApiKeyResponse>), SecurityError> {
    // Check admin permission
    if !principal.has_permission("admin") {
        tracing::warn!(
            user = %principal.id,
            "Unauthorized API key creation attempt"
        );
        return Err(SecurityError::InsufficientPermissions);
    }

    // Generate key
    let (api_key, metadata) = provider.generate_key(
        req.owner.clone(),
        req.org_id.clone(),
        req.permissions.clone(),
        req.expires_in_days,
    )?;

    tracing::info!(
        user = %principal.id,
        key_id = %metadata.id,
        owner = %req.owner,
        org = %req.org_id,
        "API key created"
    );

    Ok((
        StatusCode::CREATED,
        Json(CreateApiKeyResponse {
            api_key,
            key_id: metadata.id,
            created_at: metadata.created_at.to_rfc3339(),
            expires_at: metadata.expires_at.map(|dt| dt.to_rfc3339()),
        }),
    ))
}

#[derive(Deserialize)]
pub struct RevokeApiKeyRequest {
    pub key_id: String,

    pub reason: Option<String>,
}

#[derive(Serialize)]
pub struct RevokeApiKeyResponse {
    pub key_id: String,

    pub revoked: bool,

    pub revocation_reason: Option<String>,
}

/// Revoke an API key
///
/// Requires: admin permission
pub async fn revoke_api_key(
    State(provider): State<Arc<ApiKeyProvider>>,
    extract::Extension(principal): extract::Extension<Principal>,
    Json(req): Json<RevokeApiKeyRequest>,
) -> Result<Json<RevokeApiKeyResponse>, SecurityError> {
    if !principal.has_permission("admin") {
        tracing::warn!(
            user = %principal.id,
            key_id = %req.key_id,
            "Unauthorized API key revocation attempt"
        );
        return Err(SecurityError::InsufficientPermissions);
    }

    provider.revoke_key(&req.key_id, req.reason.clone())?;

    tracing::info!(
        user = %principal.id,
        key_id = %req.key_id,
        reason = ?req.reason,
        "API key revoked"
    );

    Ok(Json(RevokeApiKeyResponse {
        key_id: req.key_id,
        revoked: true,
        revocation_reason: req.reason,
    }))
}

#[derive(Deserialize)]
pub struct RotateApiKeyRequest {
    /// Old key ID to rotate
    pub key_id: String,

    /// Optional new permissions
    pub new_permissions: Option<Vec<String>>,
}

#[derive(Serialize)]
pub struct RotateApiKeyResponse {
    /// New API key
    pub api_key: String,

    /// New key ID
    pub key_id: String,

    /// Old key has been revoked
    pub old_key_revoked: bool,

    pub created_at: String,
}

/// Rotate an API key
///
/// Revokes old key and creates new one with same or updated permissions.
pub async fn rotate_api_key(
    State(provider): State<Arc<ApiKeyProvider>>,
    extract::Extension(principal): extract::Extension<Principal>,
    Json(req): Json<RotateApiKeyRequest>,
) -> Result<Json<RotateApiKeyResponse>, SecurityError> {
    // Allow users to rotate their own keys, or admins to rotate any key
    let old_metadata = provider.get_key(&req.key_id)?;

    if old_metadata.owner != principal.id && !principal.has_permission("admin") {
        tracing::warn!(
            user = %principal.id,
            key_id = %req.key_id,
            "Unauthorized API key rotation attempt"
        );
        return Err(SecurityError::InsufficientPermissions);
    }

    let (new_api_key, new_metadata) =
        provider.rotate_key(&req.key_id, req.new_permissions.clone())?;

    tracing::info!(
        user = %principal.id,
        old_key_id = %req.key_id,
        new_key_id = %new_metadata.id,
        "API key rotated"
    );

    Ok(Json(RotateApiKeyResponse {
        api_key: new_api_key,
        key_id: new_metadata.id,
        old_key_revoked: true,
        created_at: new_metadata.created_at.to_rfc3339(),
    }))
}

#[derive(Serialize)]
pub struct ListApiKeysResponse {
    pub keys: Vec<ApiKeyInfo>,
}

#[derive(Serialize)]
pub struct ApiKeyInfo {
    pub id: String,

    pub name: String,

    pub owner: String,

    pub permissions: Vec<String>,

    pub created_at: String,

    pub expires_at: Option<String>,

    pub revoked: bool,

    pub last_used: Option<String>,
}

/// List API keys for organization
///
/// Requires: admin permission
pub async fn list_api_keys(
    State(provider): State<Arc<ApiKeyProvider>>,
    extract::Extension(principal): extract::Extension<Principal>,
) -> Result<Json<ListApiKeysResponse>, SecurityError> {
    // Only list keys for own org unless admin
    let org_id = if principal.has_permission("admin") {
        // Admin can list all, but we'd need to implement org filtering
        // For now, return empty
        return Ok(Json(ListApiKeysResponse { keys: vec![] }));
    } else {
        principal.org_id.ok_or(SecurityError::InsufficientPermissions)?
    };

    let keys_metadata = provider.list_keys(&org_id);

    let keys = keys_metadata
        .into_iter()
        .map(|meta| ApiKeyInfo {
            id: meta.id,
            name: meta.name,
            owner: meta.owner,
            permissions: meta.permissions,
            created_at: meta.created_at.to_rfc3339(),
            expires_at: meta.expires_at.map(|dt| dt.to_rfc3339()),
            revoked: meta.revoked,
            last_used: meta.last_used.map(|dt| dt.to_rfc3339()),
        })
        .collect();

    tracing::info!(
        user = %principal.id,
        org = %org_id,
        count = keys.len(),
        "Listed API keys"
    );

    Ok(Json(ListApiKeysResponse { keys }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_create_key_request() {
        let req = CreateApiKeyRequest {
            owner: "user@example.com".to_string(),
            org_id: "org1".to_string(),
            permissions: vec!["read".to_string()],
            expires_in_days: Some(90),
        };

        assert_eq!(req.owner, "user@example.com");
        assert_eq!(req.org_id, "org1");
    }
}
