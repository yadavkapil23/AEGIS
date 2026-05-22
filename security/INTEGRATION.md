# AEGIS Security Module Integration Guide

Step-by-step guide for integrating security (authentication, authorization, rate limiting) into the API gateway and inference system.

## Table of Contents

1. [Setup](#setup)
2. [API Gateway Integration](#api-gateway-integration)
3. [Rate Limiting Integration](#rate-limiting-integration)
4. [API Key Management](#api-key-management)
5. [JWT Token Management](#jwt-token-management)
6. [Inference Backend Protection](#inference-backend-protection)
7. [Monitoring & Audit](#monitoring--audit)
8. [Complete Example](#complete-example)

## Setup

### Step 1: Add security dependency

```toml
# In gateway/Cargo.toml and inference-backends/Cargo.toml
[dependencies]
security = { path = "../security" }
```

### Step 2: Initialize security providers at startup

```rust
// In main.rs
use security::{
    ApiKeyProvider, JwtProvider, RateLimiter,
    RateLimiterConfig, JwtConfig, MultiAuthProvider,
};
use std::sync::Arc;

#[tokio::main]
async fn main() -> Result<()> {
    // Initialize API key provider
    let api_key_provider = Arc::new(ApiKeyProvider::new());

    // Initialize JWT provider
    let jwt_config = JwtConfig {
        secret: std::env::var("JWT_SECRET")
            .unwrap_or_else(|_| "dev-secret-key".to_string()),
        issuer: "aegis".to_string(),
        expiration_secs: 3600,
        clock_skew_secs: 60,
    };
    let jwt_provider = Arc::new(JwtProvider::new(jwt_config)?);

    // Initialize rate limiter
    let rate_config = RateLimiterConfig {
        global_rps: 10000,
        per_key_rps: 1000,
        per_ip_rps: 100,
        burst_size: 100,
        enable_per_key: true,
        enable_per_ip: true,
    };
    let rate_limiter = Arc::new(RateLimiter::new(rate_config));

    // Initialize multi-auth
    let multi_auth = Arc::new(
        MultiAuthProvider::new()
            .with_api_key(api_key_provider.clone())
            .with_jwt(jwt_provider.clone()),
    );

    // Create app state
    let state = AppState {
        api_key_provider,
        jwt_provider,
        rate_limiter,
        multi_auth,
    };

    // Start server
    start_api_gateway(state).await?;

    Ok(())
}

pub struct AppState {
    pub api_key_provider: Arc<ApiKeyProvider>,
    pub jwt_provider: Arc<JwtProvider>,
    pub rate_limiter: Arc<RateLimiter>,
    pub multi_auth: Arc<MultiAuthProvider>,
}
```

## API Gateway Integration

### Step 1: Extract Credentials from HTTP Request

```rust
use security::{Credential, SecurityError};
use axum::http::Request;

pub fn extract_credential(req: &Request<Body>) -> Result<Credential, SecurityError> {
    // Check Authorization header
    if let Some(header) = req.headers().get("Authorization") {
        let header_str = header
            .to_str()
            .map_err(|_| SecurityError::InvalidToken("Invalid header".to_string()))?;

        if let Some(token) = header_str.strip_prefix("Bearer ") {
            // JWT token
            return Ok(Credential::Bearer(token.to_string()));
        }

        if let Some(key) = header_str.strip_prefix("ApiKey ") {
            // API key
            return Ok(Credential::ApiKey(key.to_string()));
        }
    }

    // Check X-API-Key header
    if let Some(header) = req.headers().get("X-API-Key") {
        let key = header
            .to_str()
            .map_err(|_| SecurityError::InvalidApiKey("Invalid header".to_string()))?
            .to_string();
        return Ok(Credential::ApiKey(key));
    }

    Err(SecurityError::AuthenticationRequired)
}

pub fn extract_ip(req: &Request<Body>) -> String {
    // Get from X-Forwarded-For or connection info
    req.headers()
        .get("X-Forwarded-For")
        .and_then(|h| h.to_str().ok())
        .unwrap_or("unknown")
        .to_string()
}
```

### Step 2: Authentication Middleware

```rust
use axum::{
    middleware::Next,
    extract::State,
    response::Response,
};

pub async fn auth_middleware(
    State(state): State<Arc<AppState>>,
    req: Request<Body>,
    next: Next,
) -> Result<Response, SecurityError> {
    // Extract credential
    let credential = extract_credential(&req)?;

    // Authenticate
    let principal = state.multi_auth.authenticate(&credential).await?;

    // Get IP for rate limiting
    let ip = extract_ip(&req);

    // Check rate limit
    state.rate_limiter.check(
        principal.api_key_id.as_deref(),
        Some(&ip),
    )?;

    // Store principal in request extensions for later use
    let mut req = req;
    req.extensions_mut().insert(principal);

    Ok(next.run(req).await)
}
```

### Step 3: Authorization Middleware

```rust
pub async fn authz_middleware(
    State(state): State<Arc<AppState>>,
    req: Request<Body>,
    next: Next,
) -> Result<Response, SecurityError> {
    let principal = req
        .extensions()
        .get::<Principal>()
        .ok_or(SecurityError::AuthenticationRequired)?
        .clone();

    // Check if principal has required permission
    // (This would be route-specific in real implementation)

    Ok(next.run(req).await)
}
```

### Step 4: Setup Routes with Authentication

```rust
use axum::{
    Router,
    routing::{get, post},
    middleware,
};

pub fn create_auth_routes(state: Arc<AppState>) -> Router {
    Router::new()
        // Public routes (no auth)
        .route("/health", get(health_check))
        .route("/health/live", get(liveness_probe))
        .route("/health/ready", get(readiness_probe))

        // Protected routes
        .route("/infer", post(handle_inference))
        .route("/models", get(list_models))
        .route("/admin/keys", post(admin_create_key))

        // Apply authentication middleware
        .layer(middleware::from_fn_with_state(
            state.clone(),
            auth_middleware,
        ))
}
```

## Rate Limiting Integration

### Per-Request Rate Limit Check

```rust
use axum::extract::State;
use security::Result;

pub async fn handle_inference(
    State(state): State<Arc<AppState>>,
    extract::Path(model_id): extract::Path<String>,
    Principal { api_key_id, .. }: Principal,
    Json(request): Json<InferenceRequest>,
) -> Result<Json<InferenceResponse>> {
    // Additional rate limit check if needed
    let ip = "192.168.1.1"; // Extract from request
    state.rate_limiter.check(api_key_id.as_deref(), Some(ip))?;

    // Process inference
    let response = inference_engine.infer(&request).await?;

    Ok(Json(response))
}
```

### Rate Limit Status Endpoint

```rust
pub async fn get_rate_limit_status(
    State(state): State<Arc<AppState>>,
    extract::Extension(principal): extract::Extension<Principal>,
) -> Json<RateLimitStatus> {
    let stats = state.rate_limiter.stats();
    let key_stats = principal.api_key_id
        .as_deref()
        .and_then(|id| state.rate_limiter.key_stats(id));

    Json(RateLimitStatus {
        global_requests: stats.global_requests,
        key_requests: key_stats.map(|s| s.requests),
        rejected_requests: stats.rejected_requests,
    })
}
```

## API Key Management

### Create API Key (Admin Endpoint)

```rust
use axum::extract::State;
use serde::{Deserialize, Serialize};

#[derive(Deserialize)]
pub struct CreateKeyRequest {
    pub owner: String,
    pub org_id: String,
    pub permissions: Vec<String>,
    pub expires_in_days: Option<i64>,
}

#[derive(Serialize)]
pub struct CreateKeyResponse {
    pub api_key: String,
    pub key_id: String,
    pub created_at: String,
}

pub async fn admin_create_key(
    State(state): State<Arc<AppState>>,
    extract::Extension(principal): extract::Extension<Principal>,
    Json(req): Json<CreateKeyRequest>,
) -> Result<Json<CreateKeyResponse>> {
    // Check admin permission
    if !principal.has_permission("admin") {
        return Err(SecurityError::InsufficientPermissions);
    }

    // Generate key
    let (api_key, metadata) = state.api_key_provider.generate_key(
        req.owner,
        req.org_id,
        req.permissions,
        req.expires_in_days,
    )?;

    Ok(Json(CreateKeyResponse {
        api_key,
        key_id: metadata.id,
        created_at: metadata.created_at.to_rfc3339(),
    }))
}
```

### Revoke API Key

```rust
#[derive(Deserialize)]
pub struct RevokeKeyRequest {
    pub key_id: String,
    pub reason: Option<String>,
}

pub async fn admin_revoke_key(
    State(state): State<Arc<AppState>>,
    extract::Extension(principal): extract::Extension<Principal>,
    Json(req): Json<RevokeKeyRequest>,
) -> Result<()> {
    if !principal.has_permission("admin") {
        return Err(SecurityError::InsufficientPermissions);
    }

    state.api_key_provider.revoke_key(&req.key_id, req.reason)?;

    Ok(())
}
```

### Rotate API Key

```rust
#[derive(Deserialize)]
pub struct RotateKeyRequest {
    pub old_key_id: String,
    pub new_permissions: Option<Vec<String>>,
}

pub async fn user_rotate_key(
    State(state): State<Arc<AppState>>,
    extract::Extension(principal): extract::Extension<Principal>,
    Json(req): Json<RotateKeyRequest>,
) -> Result<Json<CreateKeyResponse>> {
    // Verify owner
    let old_key = state.api_key_provider.get_key(&req.old_key_id)?;
    if old_key.owner != principal.id {
        return Err(SecurityError::InsufficientPermissions);
    }

    let (api_key, metadata) = state.api_key_provider.rotate_key(
        &req.old_key_id,
        req.new_permissions,
    )?;

    Ok(Json(CreateKeyResponse {
        api_key,
        key_id: metadata.id,
        created_at: metadata.created_at.to_rfc3339(),
    }))
}
```

## JWT Token Management

### Issue Token

```rust
#[derive(Serialize)]
pub struct TokenResponse {
    pub access_token: String,
    pub token_type: String,
    pub expires_in: i64,
}

pub async fn issue_token(
    State(state): State<Arc<AppState>>,
    extract::Extension(principal): extract::Extension<Principal>,
) -> Result<Json<TokenResponse>> {
    let token = state.jwt_provider.issue_token(&principal)?;
    let expiration = state.jwt_provider.expiration_time(&token)?;

    Ok(Json(TokenResponse {
        access_token: token,
        token_type: "Bearer".to_string(),
        expires_in: expiration - chrono::Utc::now().timestamp(),
    }))
}
```

### Refresh Token

```rust
pub async fn refresh_token(
    State(state): State<Arc<AppState>>,
    extract::Extension(principal): extract::Extension<Principal>,
    Json(req): Json<RefreshTokenRequest>,
) -> Result<Json<TokenResponse>> {
    let new_token = state.jwt_provider.refresh_token(&req.token)?;
    let expiration = state.jwt_provider.expiration_time(&new_token)?;

    Ok(Json(TokenResponse {
        access_token: new_token,
        token_type: "Bearer".to_string(),
        expires_in: expiration - chrono::Utc::now().timestamp(),
    }))
}
```

## Inference Backend Protection

```rust
use security::Principal;
use inference_backends::InferenceBackend;

pub async fn protected_inference(
    principal: Principal,
    backend: Arc<dyn InferenceBackend>,
    request: InferenceRequest,
) -> Result<InferenceResponse> {
    // 1. Check permission
    if !principal.has_permission("infer") {
        tracing::warn!(
            user = %principal.id,
            "Unauthorized inference attempt"
        );
        return Err(SecurityError::InsufficientPermissions);
    }

    // 2. Check rate limit (already done in middleware)

    // 3. Log inference
    tracing::info!(
        user = %principal.id,
        org = ?principal.org_id,
        model = %request.model,
        "Processing inference request"
    );

    // 4. Execute inference
    match backend.infer(&request).await {
        Ok(response) => {
            tracing::info!(
                user = %principal.id,
                tokens = response.tokens.len(),
                "Inference completed successfully"
            );
            Ok(response)
        }
        Err(e) => {
            tracing::error!(
                user = %principal.id,
                error = %e,
                "Inference failed"
            );
            Err(e.into())
        }
    }
}
```

## Monitoring & Audit

### Security Event Logging

```rust
use observability::METRICS;
use security::SecurityError;

pub async fn audit_security_event(
    event_type: &str,
    principal: &Principal,
    result: bool,
    details: Option<&str>,
) {
    tracing::info!(
        event = event_type,
        user = %principal.id,
        org = ?principal.org_id,
        success = result,
        details = details,
        "Security event"
    );

    // Record metrics
    if result {
        // METRICS.record_auth_success(event_type);
    } else {
        // METRICS.record_auth_failure(event_type);
    }
}
```

### Error Response Handling

```rust
use axum::response::IntoResponse;
use axum::http::StatusCode;
use axum::Json;

impl IntoResponse for SecurityError {
    fn into_response(self) -> Response {
        let (status, error_message) = match self {
            SecurityError::AuthenticationRequired => (
                StatusCode::UNAUTHORIZED,
                "Authentication required",
            ),
            SecurityError::InsufficientPermissions => (
                StatusCode::FORBIDDEN,
                "Insufficient permissions",
            ),
            SecurityError::RateLimitExceeded { .. } => (
                StatusCode::TOO_MANY_REQUESTS,
                "Rate limit exceeded",
            ),
            SecurityError::TokenExpired => (
                StatusCode::UNAUTHORIZED,
                "Token has expired",
            ),
            _ => (
                StatusCode::INTERNAL_SERVER_ERROR,
                "Internal server error",
            ),
        };

        tracing::warn!(
            error = ?self,
            "Security error"
        );

        let body = Json(serde_json::json!({
            "error": error_message,
        }));

        (status, body).into_response()
    }
}
```

## Complete Example

Full integration in a simple API:

```rust
use axum::{
    Router,
    routing::{get, post},
    middleware,
    Json,
    extract::State,
    http::StatusCode,
};
use security::{
    ApiKeyProvider, JwtProvider, RateLimiter,
    RateLimiterConfig, JwtConfig, MultiAuthProvider,
    Principal, Credential,
};
use std::sync::Arc;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Initialize providers
    let api_key = Arc::new(ApiKeyProvider::new());
    let jwt = Arc::new(JwtProvider::new(JwtConfig::default())?);
    let limiter = Arc::new(RateLimiter::new(RateLimiterConfig::default()));
    let multi_auth = Arc::new(
        MultiAuthProvider::new()
            .with_api_key(api_key.clone())
            .with_jwt(jwt.clone()),
    );

    // Create app state
    let state = Arc::new(AppState {
        api_key,
        jwt,
        limiter,
        multi_auth,
    });

    // Create routes
    let protected_routes = Router::new()
        .route("/infer", post(handle_infer))
        .route("/models", get(list_models))
        .layer(middleware::from_fn_with_state(
            state.clone(),
            auth_middleware,
        ));

    let app = Router::new()
        .route("/health", get(health))
        .merge(protected_routes)
        .with_state(state);

    // Start server
    let listener = tokio::net::TcpListener::bind("0.0.0.0:8000").await?;
    axum::serve(listener, app).await?;

    Ok(())
}

async fn health() -> StatusCode {
    StatusCode::OK
}

async fn handle_infer(
    State(state): State<Arc<AppState>>,
    extract::Extension(principal): extract::Extension<Principal>,
) -> Json<serde_json::Value> {
    Json(serde_json::json!({
        "user": principal.id,
        "message": "Inference successful"
    }))
}

async fn list_models(
    extract::Extension(principal): extract::Extension<Principal>,
) -> Json<Vec<String>> {
    Json(vec!["gpt-4".to_string(), "llama-7b".to_string()])
}

pub struct AppState {
    api_key: Arc<ApiKeyProvider>,
    jwt: Arc<JwtProvider>,
    limiter: Arc<RateLimiter>,
    multi_auth: Arc<MultiAuthProvider>,
}
```

## Testing Checklist

- [ ] API key generation and validation working
- [ ] JWT token creation and validation working
- [ ] Rate limiting rejecting excess requests
- [ ] Authentication middleware blocking unauthenticated requests
- [ ] Authorization middleware checking permissions
- [ ] Error responses returning correct HTTP status codes
- [ ] API key revocation preventing further access
- [ ] Token refresh generating new tokens
- [ ] Metrics recording security events
- [ ] Audit logs capturing all security events
- [ ] Load test with concurrent requests
- [ ] Verify rate limit accuracy under load

## Deployment Checklist

- [ ] Environment variables set (JWT_SECRET, TLS certificates)
- [ ] TLS certificates installed and valid
- [ ] Rate limiter limits appropriate for expected load
- [ ] API key provider backed by database (not in-memory)
- [ ] Audit logging configured
- [ ] Security monitoring/alerting in place
- [ ] Regular key rotation automated
- [ ] Certificate renewal automated
- [ ] Security headers configured in gateway
- [ ] CORS properly configured
- [ ] Database credentials stored securely
- [ ] Secrets not in logs or error messages

## Performance Testing

```bash
# Test API key validation throughput
cargo bench --bench api_key_validation

# Test JWT validation throughput
cargo bench --bench jwt_validation

# Test rate limiter throughput
cargo bench --bench rate_limiter

# Load test with concurrent requests
ab -n 10000 -c 100 http://localhost:8000/health
```

## Next Steps

1. Integrate with observability for security metrics
2. Set up audit logging to database
3. Implement OAuth/OpenID Connect
4. Add IP whitelisting/blacklisting
5. Implement certificate pinning
6. Add security headers middleware (HSTS, CSP, etc.)
7. Set up automated secret rotation
8. Implement login attempt throttling
9. Add request signing for additional security
10. Create security incident response procedures
