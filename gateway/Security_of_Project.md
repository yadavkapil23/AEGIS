# AEGIS Gateway Security Setup

Complete guide for setting up authentication and rate limiting in the AEGIS API Gateway using Axum.

## Quick Start

### 1. Initialize Security Providers in main.rs

```rust
use axum::{Router, routing::{get, post}, middleware};
use security::{ApiKeyProvider, JwtProvider, RateLimiter, MultiAuthProvider, JwtConfig};
use gateway::{
    credentials::extract_credential,
    middleware::{auth_middleware, rate_limit_middleware, GatewayState},
    api_key_handlers::*,
};
use std::sync::Arc;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Initialize observability
    observability::init_tracing(&Default::default());

    // Initialize API key provider
    let api_key_provider = Arc::new(ApiKeyProvider::new());

    // Initialize JWT provider
    let jwt_config = JwtConfig {
        secret: std::env::var("JWT_SECRET")
            .unwrap_or_else(|_| "dev-secret-key-change-in-production".to_string()),
        issuer: "aegis".to_string(),
        expiration_secs: 3600,
        clock_skew_secs: 60,
    };
    let jwt_provider = Arc::new(JwtProvider::new(jwt_config)?);

    // Initialize rate limiter
    let rate_limiter = Arc::new(RateLimiter::new(Default::default()));

    // Create multi-auth provider
    let auth = Arc::new(
        MultiAuthProvider::new()
            .with_api_key(api_key_provider.clone())
            .with_jwt(jwt_provider.clone()),
    );

    // Gateway state
    let gateway_state = GatewayState {
        auth,
        rate_limiter,
    };

    // Create router with protected routes
    let app = create_routes(gateway_state, api_key_provider);

    // Start server
    let listener = tokio::net::TcpListener::bind("0.0.0.0:8000").await?;
    axum::serve(listener, app).await?;

    Ok(())
}
```

### 2. Create Routes with Middleware

```rust
use axum::{Router, routing::{get, post}, middleware};

fn create_routes(
    gateway_state: GatewayState,
    api_key_provider: Arc<ApiKeyProvider>,
) -> Router {
    // Public routes (no auth required)
    let public = Router::new()
        .route("/health", get(health_check))
        .route("/health/live", get(health_live))
        .route("/health/ready", get(health_ready));

    // Protected routes (auth + rate limit)
    let protected = Router::new()
        // Inference endpoint
        .route("/infer", post(handle_inference))
        
        // Model endpoints
        .route("/models", get(list_models))
        .route("/models/:id", get(get_model))
        
        // Admin endpoints (require admin permission)
        .route("/admin/keys", post(create_api_key))
        .route("/admin/keys", get(list_api_keys))
        .route("/admin/keys/revoke", post(revoke_api_key))
        .route("/admin/keys/rotate", post(rotate_api_key))
        
        // Apply authentication middleware
        .layer(middleware::from_fn_with_state(
            gateway_state.clone(),
            auth_middleware,
        ))
        
        // Apply rate limiting middleware
        .layer(middleware::from_fn_with_state(
            gateway_state.clone(),
            rate_limit_middleware,
        ));

    // Combine routes
    Router::new()
        .merge(public)
        .merge(protected)
        .with_state((gateway_state, api_key_provider))
}
```

## Handlers

### Health Check (Public)

```rust
async fn health_check() -> axum::http::StatusCode {
    axum::http::StatusCode::OK
}

async fn health_live() -> Json<serde_json::json!> {
    Json(serde_json::json!({
        "alive": true,
        "timestamp": chrono::Utc::now().to_rfc3339(),
    }))
}

async fn health_ready() -> Json<serde_json::json!> {
    Json(serde_json::json!({
        "ready": true,
        "timestamp": chrono::Utc::now().to_rfc3339(),
    }))
}
```

### Inference (Protected)

```rust
use axum::extract::State;
use security::Principal;
use serde::{Deserialize, Serialize};

#[derive(Deserialize)]
pub struct InferenceRequest {
    pub model: String,
    pub prompt: String,
    pub max_tokens: u32,
}

#[derive(Serialize)]
pub struct InferenceResponse {
    pub result: String,
    pub tokens: u32,
}

async fn handle_inference(
    State((state, _)): State<(GatewayState, Arc<ApiKeyProvider>)>,
    extract::Extension(principal): extract::Extension<Principal>,
    Json(request): Json<InferenceRequest>,
) -> Result<Json<InferenceResponse>, SecurityError> {
    // Check permission
    if !principal.has_permission("infer") {
        return Err(SecurityError::InsufficientPermissions);
    }

    // Simulate inference
    let response = InferenceResponse {
        result: format!("Inference for model: {}", request.model),
        tokens: request.max_tokens,
    };

    tracing::info!(
        user = %principal.id,
        model = %request.model,
        "Inference completed"
    );

    Ok(Json(response))
}
```

### API Key Management (Protected + Admin)

```rust
use axum::extract::State;
use security::Principal;

async fn create_api_key(
    State((_, api_key_provider)): State<(GatewayState, Arc<ApiKeyProvider>)>,
    extract::Extension(principal): extract::Extension<Principal>,
    Json(request): Json<CreateApiKeyRequest>,
) -> Result<(StatusCode, Json<CreateApiKeyResponse>), SecurityError> {
    // Check admin permission (middleware will handle via require_permission)
    if !principal.has_permission("admin") {
        return Err(SecurityError::InsufficientPermissions);
    }

    // Generate key
    let (api_key, metadata) = api_key_provider.generate_key(
        request.owner,
        request.org_id,
        request.permissions,
        request.expires_in_days,
    )?;

    tracing::info!(
        admin = %principal.id,
        key_id = %metadata.id,
        "API key created"
    );

    Ok((StatusCode::CREATED, Json(CreateApiKeyResponse {
        api_key,
        key_id: metadata.id,
        created_at: metadata.created_at.to_rfc3339(),
        expires_at: metadata.expires_at.map(|dt| dt.to_rfc3339()),
    })))
}
```

## Testing

### Create API Key

```bash
curl -X POST http://localhost:8000/admin/keys \
  -H "Content-Type: application/json" \
  -H "Authorization: Bearer <admin-jwt-token>" \
  -d '{
    "owner": "user@example.com",
    "org_id": "org1",
    "permissions": ["infer", "read"],
    "expires_in_days": 90
  }'
```

Response:
```json
{
  "api_key": "sk-abc123...",
  "key_id": "key-uuid",
  "created_at": "2026-05-22T...",
  "expires_at": "2026-08-20T..."
}
```

### Use API Key

```bash
curl -X POST http://localhost:8000/infer \
  -H "Content-Type: application/json" \
  -H "X-API-Key: sk-abc123..." \
  -d '{
    "model": "llama-7b",
    "prompt": "Hello",
    "max_tokens": 100
  }'
```

### Use JWT Token

```bash
curl -X POST http://localhost:8000/infer \
  -H "Content-Type: application/json" \
  -H "Authorization: Bearer eyJhbGc..." \
  -d '{
    "model": "llama-7b",
    "prompt": "Hello",
    "max_tokens": 100
  }'
```

### Check Health (Public)

```bash
curl http://localhost:8000/health
curl http://localhost:8000/health/live
curl http://localhost:8000/health/ready
```

## Error Responses

### Missing Authentication
```bash
curl http://localhost:8000/infer
```

Response (401):
```json
{
  "error": "Authentication required",
  "status": 401
}
```

### Invalid API Key
```bash
curl -H "X-API-Key: invalid" http://localhost:8000/infer
```

Response (401):
```json
{
  "error": "Invalid API key",
  "status": 401
}
```

### Insufficient Permissions
```bash
# Using API key with only "read" permission on "infer" endpoint
curl -H "X-API-Key: sk-abc..." http://localhost:8000/infer
```

Response (403):
```json
{
  "error": "Insufficient permissions",
  "status": 403
}
```

### Rate Limit Exceeded
```bash
# After exceeding rate limit
curl -H "X-API-Key: sk-abc..." http://localhost:8000/infer
```

Response (429):
```json
{
  "error": "Rate limit exceeded",
  "status": 429
}
```

## Environment Variables

```bash
# JWT secret for token signing
JWT_SECRET="your-secret-key-min-32-chars"

# Server configuration
LISTEN_ADDR="0.0.0.0"
LISTEN_PORT="8000"

# Rate limiting
GLOBAL_RPS=10000
PER_KEY_RPS=1000
PER_IP_RPS=100

# Logging
RUST_LOG="info,gateway=debug,security=debug"
```

## Middleware Layer Diagram

```
HTTP Request
    ↓
[Trace Middleware] - Log request details
    ↓
[Auth Middleware] - Extract & validate credentials
    ↓
[Rate Limit Middleware] - Check rate limits
    ↓
[Handler] - Process request
    ↓
[Response] - Return result
```

## Security Best Practices

1. **Always use HTTPS in production** - TLS/mTLS with proper certificates
2. **Store JWT secret securely** - Use environment variables or vault
3. **Rotate API keys regularly** - Implement automated rotation
4. **Monitor rate limit rejections** - May indicate attacks
5. **Log all auth failures** - For security auditing
6. **Use strong permissions** - Follow principle of least privilege
7. **Set reasonable rate limits** - Based on expected usage
8. **Implement audit logging** - Track all auth events

## Troubleshooting

### "Authentication required" on public endpoints

Public endpoints should not have auth middleware. Check that routes are defined correctly:

```rust
// ✗ Wrong - applies auth to public routes
Router::new()
    .route("/health", get(health_check))
    .layer(middleware::from_fn_with_state(..., auth_middleware))

// ✓ Correct - only applies auth to protected routes
Router::new()
    .merge(public_routes) // No auth
    .merge(protected_routes // Has auth
        .layer(middleware::from_fn_with_state(..., auth_middleware))
    )
```

### Rate limit rejecting legitimate traffic

Adjust rate limiter config:

```rust
let rate_config = RateLimiterConfig {
    global_rps: 10000,    // Increase if needed
    per_key_rps: 1000,
    per_ip_rps: 100,
    ..
};
```

### JWT tokens not validating

Check:
1. JWT secret matches between issue and validation
2. Token hasn't expired
3. Token issuer matches configured issuer
4. No clock skew issues

## Next Steps

1. Add TLS/mTLS configuration
2. Set up certificate management
3. Add API key audit logging
4. Implement token refresh endpoints
5. Add IP whitelisting
6. Create security monitoring dashboards
7. Set up automated key rotation
8. Implement OAuth/OpenID Connect
