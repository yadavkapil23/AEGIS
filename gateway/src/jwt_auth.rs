/// JWT Authentication Middleware for Actix-web
/// Supports JWT (HMAC-SHA256 verified) and API key (database-backed) authentication

use actix_web::{
    dev::{forward_ready, Service, ServiceRequest, ServiceResponse, Transform},
    body::{BoxBody, MessageBody},
    Error, HttpMessage, HttpResponse, http::header,
};
use futures_util::future::{ok, LocalBoxFuture, Ready};
use serde::{Deserialize, Serialize};
use std::rc::Rc;
use tracing::warn;
use base64::Engine;
use hmac::{Hmac, Mac};
use sha2::Sha256;
use crate::database::DbPool;

type HmacSha256 = Hmac<Sha256>;

/// JWT Claims structure
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Claims {
    pub sub: String,
    pub iss: String,
    pub aud: String,
    pub exp: i64,
    pub iat: i64,
    pub org_id: Option<String>,
    pub permissions: Vec<String>,
}

/// Authenticated principal
#[derive(Debug, Clone)]
pub struct AuthenticatedUser {
    pub user_id: String,
    pub org_id: Option<String>,
    pub permissions: Vec<String>,
}

/// API Key validator with optional database backing
#[derive(Clone)]
pub struct ApiKeyValidator {
    jwt_secret: String,
    fallback_keys: Vec<String>,
    db_pool: Option<DbPool>,
}

impl ApiKeyValidator {
    pub fn new(jwt_secret: String, fallback_keys: Vec<String>, db_pool: Option<DbPool>) -> Self {
        Self {
            jwt_secret,
            fallback_keys,
            db_pool,
        }
    }

    /// Validate a JWT token with HMAC-SHA256 signature verification
    pub fn validate_jwt(&self, token: &str) -> Result<Claims, String> {
        if token.is_empty() {
            return Err("Token is empty".to_string());
        }

        let parts: Vec<&str> = token.split('.').collect();
        if parts.len() != 3 {
            return Err("Invalid token format".to_string());
        }

        let header_b64 = parts[0];
        let payload_b64 = parts[1];
        let signature_b64 = parts[2];

        // Verify HMAC-SHA256 signature
        let engine = base64::engine::general_purpose::URL_SAFE_NO_PAD;
        let signing_input = format!("{}.{}", header_b64, payload_b64);

        let signature = engine.decode(signature_b64)
            .map_err(|e| format!("Invalid signature encoding: {}", e))?;

        // Create MAC for signing input verification
        let mut mac = HmacSha256::new_from_slice(self.jwt_secret.as_bytes())
            .map_err(|e| format!("Invalid HMAC key: {}", e))?;
        mac.update(signing_input.as_bytes());
        mac.verify_slice(&signature)
            .map_err(|_| "Invalid JWT signature".to_string())?;

        // Decode and validate claims
        let payload = engine.decode(payload_b64)
            .map_err(|e| format!("Invalid payload encoding: {}", e))?;

        let claims: Claims = serde_json::from_slice(&payload)
            .map_err(|e| format!("Failed to decode claims: {}", e))?;

        let now = chrono::Utc::now().timestamp();
        if claims.exp < now {
            return Err("Token has expired".to_string());
        }

        Ok(claims)
    }

    /// Validate an API key against the database (or fallback keys)
    pub async fn validate_api_key(&self, key: &str) -> Result<AuthenticatedUser, String> {
        if key.is_empty() || key.len() < 5 {
            warn!("Invalid API key format: too short");
            return Err("Invalid API key".to_string());
        }

        // Check database first
        if let Some(ref pool) = self.db_pool {
            if crate::database::validate_api_key(pool, key).await {
                return Ok(AuthenticatedUser {
                    user_id: format!("api_key_{}", &key[..8.min(key.len())]),
                    org_id: None,
                    permissions: vec!["infer".to_string(), "admin".to_string()],
                });
            }
        }

        // Fallback to configured keys
        if self.fallback_keys.contains(&key.to_string()) {
            return Ok(AuthenticatedUser {
                user_id: format!("api_user_{}", &key[..8.min(key.len())]),
                org_id: None,
                permissions: vec!["infer".to_string()],
            });
        }

        warn!("Invalid API key: {}", &key[..8.min(key.len())]);
        Err("Invalid API key".to_string())
    }

    /// Extract and validate a Bearer token (tries JWT first, then API key)
    pub async fn extract_bearer_token(&self, auth_header: &str) -> Result<AuthenticatedUser, String> {
        if let Some(token) = auth_header.strip_prefix("Bearer ") {
            // Try JWT validation first
            if let Ok(claims) = self.validate_jwt(token) {
                return Ok(AuthenticatedUser {
                    user_id: claims.sub,
                    org_id: claims.org_id,
                    permissions: claims.permissions,
                });
            }
            // Try as API key
            self.validate_api_key(token).await
        } else {
            Err("Invalid Bearer token format".to_string())
        }
    }
}

/// JWT Authentication Middleware
pub struct JwtAuthMiddleware {
    validator: Rc<ApiKeyValidator>,
}

impl JwtAuthMiddleware {
    pub fn new(validator: ApiKeyValidator) -> Self {
        Self {
            validator: Rc::new(validator),
        }
    }
}

impl<S, B> Transform<S, ServiceRequest> for JwtAuthMiddleware
where
    S: Service<ServiceRequest, Response = ServiceResponse<B>, Error = Error> + 'static,
    S::Future: 'static,
    B: MessageBody + 'static,
{
    type Response = ServiceResponse<BoxBody>;
    type Error = Error;
    type InitError = ();
    type Transform = JwtAuthMiddlewareService<S>;
    type Future = Ready<Result<Self::Transform, Self::InitError>>;

    fn new_transform(&self, service: S) -> Self::Future {
        ok(JwtAuthMiddlewareService {
            service: Rc::new(service),
            validator: self.validator.clone(),
        })
    }
}

pub struct JwtAuthMiddlewareService<S> {
    service: Rc<S>,
    validator: Rc<ApiKeyValidator>,
}

impl<S, B> Service<ServiceRequest> for JwtAuthMiddlewareService<S>
where
    S: Service<ServiceRequest, Response = ServiceResponse<B>, Error = Error> + 'static,
    S::Future: 'static,
    B: MessageBody + 'static,
{
    type Response = ServiceResponse<BoxBody>;
    type Error = Error;
    type Future = LocalBoxFuture<'static, Result<Self::Response, Self::Error>>;

    forward_ready!(service);

    fn call(&self, req: ServiceRequest) -> Self::Future {
        let service = self.service.clone();
        let validator = self.validator.clone();

        Box::pin(async move {
            let path = req.path().to_string();

            // Skip auth for health and metrics endpoints
            if path.starts_with("/health") || path == "/metrics" {
                let res = service.call(req).await?;
                return Ok(res.map_into_boxed_body());
            }

            // Try Authorization header (Bearer token)
            if let Some(h) = req.headers().get(header::AUTHORIZATION) {
                if let Ok(h_str) = h.to_str() {
                    if let Ok(user) = validator.extract_bearer_token(h_str).await {
                        req.extensions_mut().insert(user);
                        let res = service.call(req).await?;
                        return Ok(res.map_into_boxed_body());
                    }
                }
            }

            // Try x-api-key header
            for (header_name, header_value) in req.headers().iter() {
                if header_name.as_str().eq_ignore_ascii_case("x-api-key") {
                    if let Ok(api_key) = header_value.to_str() {
                        if let Ok(user) = validator.validate_api_key(api_key).await {
                            tracing::info!("API key auth successful for {}", path);
                            req.extensions_mut().insert(user);
                            let res = service.call(req).await?;
                            return Ok(res.map_into_boxed_body());
                        }
                    }
                }
            }

            warn!("Authentication failed for {}", path);
            Ok(req.into_response(
                HttpResponse::Unauthorized().json(serde_json::json!({
                    "error": "Invalid credentials",
                    "message": "Provide a valid JWT token or API key"
                }))
            ).map_into_boxed_body())
        })
    }
}
