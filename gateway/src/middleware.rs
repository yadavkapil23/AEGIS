//! API Gateway middleware for authentication, authorization, and rate limiting

use axum::{
    middleware::Next,
    extract::State,
    response::Response,
    http::Request,
    body::Body,
};
use security::{Principal, SecurityError, AuthenticationProvider, RateLimiter};
use observability::{METRICS, tracing::create_span};
use std::sync::Arc;
use tracing::Instrument;

use crate::credentials::{extract_credential, extract_client_ip, extract_request_id};

/// Gateway application state
#[derive(Clone)]
pub struct GatewayState {
    /// Authentication provider (API Key + JWT)
    pub auth: Arc<dyn AuthenticationProvider>,

    /// Rate limiter
    pub rate_limiter: Arc<RateLimiter>,
}

/// Authentication middleware
///
/// Extracts credentials from request headers and authenticates the principal.
/// Stores the authenticated principal in request extensions for downstream handlers.
pub async fn auth_middleware(
    State(state): State<GatewayState>,
    mut req: Request<Body>,
    next: Next,
) -> Result<Response, SecurityError> {
    let request_id = extract_request_id(req.headers());
    let client_ip = extract_client_ip(req.headers());

    // Span for this request
    let _span = create_span("auth_middleware", &[
        ("request_id", &request_id),
        ("ip", &client_ip),
    ]);

    tracing::debug!(
        request_id = %request_id,
        ip = %client_ip,
        "Processing authentication"
    );

    // Extract credential from headers
    let credential = extract_credential(req.headers()).map_err(|e| {
        tracing::warn!(
            request_id = %request_id,
            error = ?e,
            "Failed to extract credential"
        );
        e
    })?;

    // Authenticate
    let principal = state.auth.authenticate(&credential).await.map_err(|e| {
        tracing::warn!(
            request_id = %request_id,
            error = ?e,
            "Authentication failed"
        );

        // Record metric
        // METRICS.record_auth_failure("invalid_credentials");

        e
    })?;

    tracing::info!(
        request_id = %request_id,
        user = %principal.id,
        org = ?principal.org_id,
        "Authentication successful"
    );

    // Store principal and metadata in request extensions for later use
    req.extensions_mut().insert(principal.clone());
    req.extensions_mut().insert(request_id);
    req.extensions_mut().insert(client_ip);

    Ok(next.run(req).await)
}

/// Rate limiting middleware
///
/// Checks rate limits for authenticated API keys and IP addresses.
/// Rejects requests if rate limit is exceeded.
pub async fn rate_limit_middleware(
    State(state): State<GatewayState>,
    mut req: Request<Body>,
    next: Next,
) -> Result<Response, SecurityError> {
    // Extract metadata from extensions (set by auth middleware)
    let request_id = req
        .extensions()
        .get::<String>()
        .cloned()
        .unwrap_or_else(|| extract_request_id(req.headers()));

    let principal = req
        .extensions()
        .get::<Principal>()
        .cloned();

    let client_ip = req
        .extensions()
        .get::<String>()
        .cloned()
        .unwrap_or_else(|| extract_client_ip(req.headers()));

    // Extract API key ID if available
    let api_key_id = principal.as_ref().and_then(|p| p.api_key_id.clone());

    tracing::debug!(
        request_id = %request_id,
        api_key = ?api_key_id,
        ip = %client_ip,
        "Checking rate limit"
    );

    // Check rate limits
    state
        .rate_limiter
        .check(api_key_id.as_deref(), Some(&client_ip))
        .map_err(|e| {
            tracing::warn!(
                request_id = %request_id,
                api_key = ?api_key_id,
                ip = %client_ip,
                error = ?e,
                "Rate limit exceeded"
            );

            // Record metric
            // METRICS.record_rate_limit_rejection(api_key_id.as_deref());

            e
        })?;

    Ok(next.run(req).await)
}

/// Authorization middleware
///
/// Verifies that authenticated principal has required permissions.
/// Can be customized per route to require specific permissions.
pub fn require_permission(required: &'static str) -> impl Fn(State<GatewayState>, Request<Body>, Next) -> impl std::future::Future<Output = Result<Response, SecurityError>> + Clone {
    move |_state: State<GatewayState>, req: Request<Body>, next: Next| {
        let principal = req
            .extensions()
            .get::<Principal>()
            .cloned();

        async move {
            let principal = principal.ok_or(SecurityError::AuthenticationRequired)?;

            if !principal.has_permission(required) {
                let request_id = req
                    .extensions()
                    .get::<String>()
                    .cloned()
                    .unwrap_or_default();

                tracing::warn!(
                    request_id = %request_id,
                    user = %principal.id,
                    required = required,
                    "Authorization denied - insufficient permissions"
                );

                // Record metric
                // METRICS.record_auth_denied(&principal.id, required);

                return Err(SecurityError::InsufficientPermissions);
            }

            tracing::debug!(
                user = %principal.id,
                permission = required,
                "Authorization granted"
            );

            Ok(next.run(req).await)
        }
    }
}

/// Request tracing middleware
///
/// Adds request ID and logging for all requests
pub async fn trace_middleware(
    mut req: Request<Body>,
    next: Next,
) -> Response {
    let request_id = extract_request_id(req.headers());
    let method = req.method().clone();
    let path = req.uri().path().to_string();
    let client_ip = extract_client_ip(req.headers());

    // Create span with request context
    let span = tracing::info_span!(
        "http_request",
        request_id = %request_id,
        method = %method,
        path = %path,
        ip = %client_ip,
    );

    async {
        let start = std::time::Instant::now();

        // Store request ID for downstream handlers
        req.extensions_mut().insert(request_id.clone());

        let response = next.run(req).await;
        let elapsed = start.elapsed();

        let status = response.status();

        tracing::info!(
            request_id = %request_id,
            status = status.as_u16(),
            latency_ms = elapsed.as_millis() as u64,
            "Request completed"
        );

        // Record metrics
        // METRICS.record_http_request(&method.to_string(), status.as_u16(), elapsed.as_millis() as f64);

        response
    }
    .instrument(span)
    .await
}

/// Error response middleware
///
/// Converts security errors to appropriate HTTP responses
pub fn security_error_to_response(err: SecurityError) -> Response {
    use axum::{
        http::StatusCode,
        Json,
    };

    let (status, message) = match err {
        SecurityError::AuthenticationRequired => (
            StatusCode::UNAUTHORIZED,
            "Authentication required",
        ),
        SecurityError::InsufficientPermissions => (
            StatusCode::FORBIDDEN,
            "Insufficient permissions",
        ),
        SecurityError::RateLimitExceeded { limit, window_secs } => {
            tracing::warn!(
                limit = limit,
                window_secs = window_secs,
                "Rate limit exceeded"
            );
            (
                StatusCode::TOO_MANY_REQUESTS,
                "Rate limit exceeded",
            )
        }
        SecurityError::TokenExpired => (
            StatusCode::UNAUTHORIZED,
            "Token has expired",
        ),
        SecurityError::InvalidToken(_) => (
            StatusCode::UNAUTHORIZED,
            "Invalid token",
        ),
        SecurityError::InvalidApiKey(_) => (
            StatusCode::UNAUTHORIZED,
            "Invalid API key",
        ),
        _ => (
            StatusCode::INTERNAL_SERVER_ERROR,
            "Internal server error",
        ),
    };

    let body = Json(serde_json::json!({
        "error": message,
        "status": status.as_u16(),
    }));

    (status, body).into_response()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_security_error_responses() {
        let err = SecurityError::AuthenticationRequired;
        let response = security_error_to_response(err);
        assert_eq!(response.status(), axum::http::StatusCode::UNAUTHORIZED);

        let err = SecurityError::InsufficientPermissions;
        let response = security_error_to_response(err);
        assert_eq!(response.status(), axum::http::StatusCode::FORBIDDEN);

        let err = SecurityError::RateLimitExceeded {
            limit: 100,
            window_secs: 60,
        };
        let response = security_error_to_response(err);
        assert_eq!(response.status(), axum::http::StatusCode::TOO_MANY_REQUESTS);
    }
}
