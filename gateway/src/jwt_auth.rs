/// JWT Token Authentication for Actix-web
/// Validates Bearer tokens and API keys

use actix_web::{
    dev::{forward_ready, Service, ServiceRequest, ServiceResponse, Transform},
    Error, HttpMessage, HttpResponse,
};
use futures_util::future::LocalBoxFuture;
use serde::{Deserialize, Serialize};
use std::rc::Rc;
use tracing::{error, warn, info};

/// JWT Claims structure
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Claims {
    pub sub: String,                    // Subject (user ID)
    pub iss: String,                    // Issuer
    pub aud: String,                    // Audience
    pub exp: i64,                       // Expiration time
    pub iat: i64,                       // Issued at
    pub org_id: Option<String>,         // Organization ID
    pub permissions: Vec<String>,       // User permissions
}

/// Authenticated principal
#[derive(Debug, Clone)]
pub struct AuthenticatedUser {
    pub user_id: String,
    pub org_id: Option<String>,
    pub permissions: Vec<String>,
}

/// API Key configuration
#[derive(Clone)]
pub struct ApiKeyValidator {
    valid_keys: Vec<String>,  // In production, load from database
    jwt_secret: String,
}

impl ApiKeyValidator {
    pub fn new(jwt_secret: String, valid_keys: Vec<String>) -> Self {
        Self {
            valid_keys,
            jwt_secret,
        }
    }

    /// Validate JWT token
    pub fn validate_jwt(&self, token: &str) -> Result<Claims, String> {
        // In production, use jsonwebtoken crate
        // This is a simplified version for demonstration

        // For now, just validate token is not empty
        if token.is_empty() {
            return Err("Token is empty".to_string());
        }

        // Check if token format is valid (3 parts separated by dots)
        let parts: Vec<&str> = token.split('.').collect();
        if parts.len() != 3 {
            return Err("Invalid token format".to_string());
        }

        // Decode payload (base64 second part)
        match base64::decode(parts[1]) {
            Ok(payload) => {
                match serde_json::from_slice::<Claims>(&payload) {
                    Ok(mut claims) => {
                        // Check expiration
                        let now = chrono::Utc::now().timestamp();
                        if claims.exp < now {
                            return Err("Token has expired".to_string());
                        }
                        Ok(claims)
                    }
                    Err(e) => Err(format!("Failed to decode claims: {}", e)),
                }
            }
            Err(e) => Err(format!("Failed to decode token: {}", e)),
        }
    }

    /// Validate API key
    pub fn validate_api_key(&self, key: &str) -> Result<AuthenticatedUser, String> {
        if !self.valid_keys.contains(&key.to_string()) {
            warn!("Invalid API key attempted: {}", &key[..key.len().min(10)]);
            return Err("Invalid API key".to_string());
        }

        // In production, look up key metadata from database
        Ok(AuthenticatedUser {
            user_id: format!("api_user_{}", key.split('-').next().unwrap_or("unknown")),
            org_id: None,
            permissions: vec!["infer".to_string()],
        })
    }

    /// Extract and validate Bearer token from header
    pub fn extract_bearer_token(&self, auth_header: &str) -> Result<AuthenticatedUser, String> {
        if let Some(token) = auth_header.strip_prefix("Bearer ") {
            let claims = self.validate_jwt(token)?;
            Ok(AuthenticatedUser {
                user_id: claims.sub,
                org_id: claims.org_id,
                permissions: claims.permissions,
            })
        } else {
            Err("Invalid Bearer token format".to_string())
        }
    }
}

/// JWT Authentication Middleware for Actix-web
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
    B: 'static,
{
    type Response = ServiceResponse<B>;
    type Error = Error;
    type InitError = ();
    type Transform = JwtAuthMiddlewareService<S>;
    type Future = futures_util::future::Ready<Result<Self::Transform, Self::InitError>>;

    fn new_service(&self, service: S) -> Self::Future {
        futures_util::future::ok(JwtAuthMiddlewareService {
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
    B: 'static,
{
    type Response = ServiceResponse<B>;
    type Error = Error;
    type Future = LocalBoxFuture<'static, Result<Self::Response, Self::Error>>;

    forward_ready!(service);

    fn call(&self, req: ServiceRequest) -> Self::Future {
        let service = self.service.clone();
        let validator = self.validator.clone();

        Box::pin(async move {
            // Skip auth for public endpoints
            let path = req.path();
            if path.starts_with("/health") || path == "/metrics" {
                return service.call(req).await;
            }

            // Extract Authorization header
            let auth_header = match req.headers().get("authorization") {
                Some(header) => match header.to_str() {
                    Ok(h) => h,
                    Err(_) => {
                        error!("Invalid Authorization header format");
                        return Ok(req.into_response(
                            HttpResponse::BadRequest().json(serde_json::json!({
                                "error": "Invalid Authorization header"
                            }))
                        ));
                    }
                },
                None => {
                    warn!("Missing Authorization header for {}", path);
                    return Ok(req.into_response(
                        HttpResponse::Unauthorized().json(serde_json::json!({
                            "error": "Authorization required",
                            "code": "missing_auth"
                        }))
                    ));
                }
            };

            // Try Bearer token first
            if auth_header.starts_with("Bearer ") {
                match validator.extract_bearer_token(auth_header) {
                    Ok(user) => {
                        info!("JWT authentication successful for user: {}", user.user_id);
                        req.extensions_mut().insert(user);
                        return service.call(req).await;
                    }
                    Err(e) => {
                        warn!("JWT validation failed: {}", e);
                        return Ok(req.into_response(
                            HttpResponse::Unauthorized().json(serde_json::json!({
                                "error": "Invalid token",
                                "details": e
                            }))
                        ));
                    }
                }
            }

            // Try API key
            if auth_header.starts_with("ApiKey ") {
                let key = auth_header.strip_prefix("ApiKey ").unwrap_or("");
                match validator.validate_api_key(key) {
                    Ok(user) => {
                        info!("API key authentication successful for user: {}", user.user_id);
                        req.extensions_mut().insert(user);
                        return service.call(req).await;
                    }
                    Err(e) => {
                        warn!("API key validation failed: {}", e);
                        return Ok(req.into_response(
                            HttpResponse::Unauthorized().json(serde_json::json!({
                                "error": "Invalid API key"
                            }))
                        ));
                    }
                }
            }

            // Try X-API-Key header
            if let Some(api_key_header) = req.headers().get("x-api-key") {
                if let Ok(key) = api_key_header.to_str() {
                    match validator.validate_api_key(key) {
                        Ok(user) => {
                            info!("API key authentication successful for user: {}", user.user_id);
                            req.extensions_mut().insert(user);
                            return service.call(req).await;
                        }
                        Err(e) => {
                            warn!("API key validation failed: {}", e);
                            return Ok(req.into_response(
                                HttpResponse::Unauthorized().json(serde_json::json!({
                                    "error": "Invalid API key"
                                }))
                            ));
                        }
                    }
                }
            }

            warn!("Unsupported authorization scheme");
            Ok(req.into_response(
                HttpResponse::Unauthorized().json(serde_json::json!({
                    "error": "Unsupported authorization scheme",
                    "supported": ["Bearer <jwt>", "ApiKey <key>", "X-API-Key: <key>"]
                }))
            ))
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_api_key_validator_creation() {
        let validator = ApiKeyValidator::new(
            "secret".to_string(),
            vec!["sk-test123".to_string()],
        );
        assert_eq!(validator.valid_keys.len(), 1);
    }

    #[test]
    fn test_validate_api_key_success() {
        let validator = ApiKeyValidator::new(
            "secret".to_string(),
            vec!["sk-test123".to_string()],
        );
        let result = validator.validate_api_key("sk-test123");
        assert!(result.is_ok());
    }

    #[test]
    fn test_validate_api_key_failure() {
        let validator = ApiKeyValidator::new(
            "secret".to_string(),
            vec!["sk-test123".to_string()],
        );
        let result = validator.validate_api_key("sk-invalid");
        assert!(result.is_err());
    }
}
