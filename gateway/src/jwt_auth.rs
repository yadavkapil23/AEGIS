/// JWT Authentication Middleware for Actix-web

use actix_web::{
    dev::{forward_ready, Service, ServiceRequest, ServiceResponse, Transform},
    body::{BoxBody, MessageBody},
    Error, HttpMessage, HttpResponse, http::header,
};
use futures_util::future::{ok, LocalBoxFuture, Ready};
use serde::{Deserialize, Serialize};
use std::rc::Rc;
use tracing::{error, warn};
use base64::Engine;

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

/// API Key validator
#[derive(Clone)]
pub struct ApiKeyValidator {
    valid_keys: Vec<String>,
    jwt_secret: String,
}

impl ApiKeyValidator {
    pub fn new(jwt_secret: String, valid_keys: Vec<String>) -> Self {
        Self {
            valid_keys,
            jwt_secret,
        }
    }

    pub fn validate_jwt(&self, token: &str) -> Result<Claims, String> {
        if token.is_empty() {
            return Err("Token is empty".to_string());
        }

        let parts: Vec<&str> = token.split('.').collect();
        if parts.len() != 3 {
            return Err("Invalid token format".to_string());
        }

        let engine = base64::engine::general_purpose::STANDARD;
        match engine.decode(parts[1]) {
            Ok(payload) => {
                match serde_json::from_slice::<Claims>(&payload) {
                    Ok(claims) => {
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

    pub fn validate_api_key(&self, key: &str) -> Result<AuthenticatedUser, String> {
        if !self.valid_keys.contains(&key.to_string()) {
            warn!("Invalid API key attempted");
            return Err("Invalid API key".to_string());
        }

        Ok(AuthenticatedUser {
            user_id: format!("api_user_{}", key.split('-').next().unwrap_or("unknown")),
            org_id: None,
            permissions: vec!["infer".to_string()],
        })
    }

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
            let path = req.path();
            if path.starts_with("/health") || path == "/metrics" {
                let res = service.call(req).await?;
                return Ok(res.map_into_boxed_body());
            }

            let auth_header = match req.headers().get(header::AUTHORIZATION) {
                Some(h) => match h.to_str() {
                    Ok(h_str) => h_str,
                    Err(_) => {
                        error!("Invalid Authorization header format");
                        return Ok(req.into_response(
                            HttpResponse::BadRequest().json(serde_json::json!({
                                "error": "Invalid Authorization header"
                            }))
                        ).map_into_boxed_body());
                    }
                },
                None => {
                    warn!("Missing Authorization header for {}", path);
                    return Ok(req.into_response(
                        HttpResponse::Unauthorized().json(serde_json::json!({
                            "error": "Authorization required"
                        }))
                    ).map_into_boxed_body());
                }
            };

            if auth_header.starts_with("Bearer ") {
                if let Ok(user) = validator.extract_bearer_token(auth_header) {
                    req.extensions_mut().insert(user);
                    let res = service.call(req).await?;
                    return Ok(res.map_into_boxed_body());
                }
            }

            if let Some(api_key_header) = req.headers().get("x-api-key") {
                if let Ok(api_key) = api_key_header.to_str() {
                    if let Ok(user) = validator.validate_api_key(api_key) {
                        req.extensions_mut().insert(user);
                        let res = service.call(req).await?;
                        return Ok(res.map_into_boxed_body());
                    }
                }
            }

            Ok(req.into_response(
                HttpResponse::Unauthorized().json(serde_json::json!({
                    "error": "Invalid credentials"
                }))
            ).map_into_boxed_body())
        })
    }
}
