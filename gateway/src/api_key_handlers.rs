use actix_web::{web, HttpResponse, get, post, delete};
use serde::{Deserialize, Serialize};
use tracing::{info, error};
use sha2::Digest;

use crate::database::{self, ApiKeyInfo};
use crate::middleware::GatewayState;

// ── Request / Response types ──────────────────────────────────

#[derive(Debug, Deserialize)]
pub struct CreateApiKeyRequest {
    pub name: String,
    pub created_by: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct CreateApiKeyResponse {
    pub id: String,
    pub key: String,       // Only returned ONCE — the full key
    pub name: String,
    pub created_at: String,
}

#[derive(Debug, Serialize)]
pub struct ApiKeyInfoResponse {
    pub id: String,
    pub key_preview: String, // Masked: "sk-...xxxx"
    pub name: Option<String>,
    pub created_at: String,
    pub last_used: Option<String>,
    pub is_active: bool,
}

#[derive(Debug, Serialize)]
pub struct ApiKeyListResponse {
    pub keys: Vec<ApiKeyInfoResponse>,
    pub total: usize,
}

// ── Helpers ───────────────────────────────────────────────────

/// Generate a cryptographically random API key: `sk-` + 32 hex bytes.
fn generate_api_key() -> String {
    use rand::Rng;
    let mut rng = rand::thread_rng();
    let bytes: Vec<u8> = (0..32).map(|_| rng.gen()).collect();
    let hex: String = bytes.iter().map(|b| format!("{:02x}", b)).collect();
    format!("sk-{}", hex)
}

/// Mask a key for display: show prefix and last 4 chars.
fn mask_key(key: &str) -> String {
    if key.len() <= 12 {
        return key.to_string(); // Too short to mask meaningfully
    }
    let prefix = &key[..7]; // "sk-xxxx"
    let suffix = &key[key.len() - 4..];
    format!("{}...{}", prefix, suffix)
}

fn key_info_to_response(info: &ApiKeyInfo) -> ApiKeyInfoResponse {
    ApiKeyInfoResponse {
        id: info.id.to_string(),
        key_preview: mask_key(&info.key),
        name: info.name.clone(),
        created_at: info.created_at.to_rfc3339(),
        last_used: info.last_used.map(|dt| dt.to_rfc3339()),
        is_active: info.is_active,
    }
}

// ── Handlers ──────────────────────────────────────────────────

/// POST /api/keys — Create a new API key.
#[post("/api/keys")]
pub async fn create_api_key(
    state: web::Data<GatewayState>,
    body: web::Json<CreateApiKeyRequest>,
) -> HttpResponse {
    let raw_key = generate_api_key();

    match database::add_api_key(
        &state.db_pool,
        &raw_key,
        Some(&body.name),
        body.created_by.as_deref(),
    )
    .await
    {
        Ok(info) => {
            info!(name = %body.name, "API key created");
            HttpResponse::Created().json(CreateApiKeyResponse {
                id: info.id.to_string(),
                key: raw_key, // Full key returned only once
                name: body.name.clone(),
                created_at: info.created_at.to_rfc3339(),
            })
        }
        Err(e) => {
            error!(error = %e, "Failed to create API key");
            HttpResponse::InternalServerError().json(serde_json::json!({
                "error": format!("Failed to create API key: {}", e),
            }))
        }
    }
}

/// GET /api/keys — List all API keys (masked).
#[get("/api/keys")]
pub async fn get_api_keys(state: web::Data<GatewayState>) -> HttpResponse {
    let db_keys = database::list_api_keys(&state.db_pool);
    let keys: Vec<ApiKeyInfoResponse> = db_keys.iter().map(key_info_to_response).collect();
    let total = keys.len();
    HttpResponse::Ok().json(ApiKeyListResponse { keys, total })
}

/// GET /api/keys/{key} — Get a single API key by raw key value.
#[get("/api/keys/{key}")]
pub async fn get_api_key(
    state: web::Data<GatewayState>,
    path: web::Path<String>,
) -> HttpResponse {
    let key = path.into_inner();
    match database::get_api_key_info(&state.db_pool, &key) {
        Some(info) => HttpResponse::Ok().json(key_info_to_response(&info)),
        None => HttpResponse::NotFound().json(serde_json::json!({
            "error": "API key not found",
        })),
    }
}

/// DELETE /api/keys/{key} — Revoke (disable) an API key.
#[delete("/api/keys/{key}")]
pub async fn revoke_api_key(
    state: web::Data<GatewayState>,
    path: web::Path<String>,
) -> HttpResponse {
    let key = path.into_inner();
    match database::disable_api_key(&state.db_pool, &key).await {
        Ok(true) => {
            info!("API key revoked");
            HttpResponse::Ok().json(serde_json::json!({
                "success": true,
                "message": "API key revoked",
            }))
        }
        Ok(false) => HttpResponse::NotFound().json(serde_json::json!({
            "error": "API key not found",
        })),
        Err(e) => {
            error!(error = %e, "Failed to revoke API key");
            HttpResponse::InternalServerError().json(serde_json::json!({
                "error": format!("Failed to revoke API key: {}", e),
            }))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn generate_key_has_prefix() {
        let key = generate_api_key();
        assert!(key.starts_with("sk-"));
        assert_eq!(key.len(), 67); // "sk-" (3) + 64 hex chars
    }

    #[test]
    fn mask_key_works() {
        let key = "sk-abcdef1234567890abcdef1234567890abcdef1234567890abcdef1234567890";
        let masked = mask_key(key);
        assert!(masked.contains("..."));
        assert!(masked.starts_with("sk-abc"));
        assert!(masked.ends_with("7890"));
    }

    #[test]
    fn create_request_deserializes() {
        let body = r#"{"name":"test-key","created_by":"admin"}"#;
        let req: CreateApiKeyRequest = serde_json::from_str(body).unwrap();
        assert_eq!(req.name, "test-key");
        assert_eq!(req.created_by.unwrap(), "admin");
    }
}
