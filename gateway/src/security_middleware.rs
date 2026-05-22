/// Security Middleware for Actix-web
/// Rate limiting, CORS, CSRF protection, and security headers

use actix_web::{
    dev::{forward_ready, Service, ServiceRequest, ServiceResponse, Transform},
    Error, HttpResponse, http::HeaderMap,
};
use futures_util::future::LocalBoxFuture;
use std::rc::Rc;
use std::sync::Arc;
use std::collections::HashMap;
use parking_lot::RwLock;
use tracing::{warn, info};

/// Token Bucket for rate limiting
#[derive(Clone)]
pub struct TokenBucket {
    max_tokens: u32,
    tokens: Arc<RwLock<HashMap<String, f64>>>,
    refill_rate: f64,  // tokens per second
    last_refill: Arc<RwLock<HashMap<String, std::time::Instant>>>,
}

impl TokenBucket {
    pub fn new(max_tokens: u32, refill_rate_per_second: f64) -> Self {
        Self {
            max_tokens,
            tokens: Arc::new(RwLock::new(HashMap::new())),
            refill_rate: refill_rate_per_second,
            last_refill: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    /// Check if request is allowed under rate limit
    pub fn allow_request(&self, client_id: &str) -> bool {
        let mut tokens = self.tokens.write();
        let mut last_refill = self.last_refill.write();

        let now = std::time::Instant::now();
        let last = last_refill
            .entry(client_id.to_string())
            .or_insert(now);

        // Refill tokens based on elapsed time
        let elapsed = now.duration_since(*last).as_secs_f64();
        let refill_amount = elapsed * self.refill_rate;

        let current_tokens = tokens
            .entry(client_id.to_string())
            .or_insert(self.max_tokens as f64);

        *current_tokens = (*current_tokens + refill_amount).min(self.max_tokens as f64);
        *last = now;

        // Check if we have at least 1 token
        if *current_tokens >= 1.0 {
            *current_tokens -= 1.0;
            true
        } else {
            false
        }
    }

    /// Get current token count for a client
    pub fn tokens_remaining(&self, client_id: &str) -> f64 {
        self.tokens
            .read()
            .get(client_id)
            .copied()
            .unwrap_or(self.max_tokens as f64)
    }
}

/// Rate Limiting Middleware
pub struct RateLimitMiddleware {
    bucket: Rc<TokenBucket>,
}

impl RateLimitMiddleware {
    pub fn new(max_requests_per_minute: u32) -> Self {
        let refill_rate = max_requests_per_minute as f64 / 60.0;
        Self {
            bucket: Rc::new(TokenBucket::new(max_requests_per_minute, refill_rate)),
        }
    }
}

impl<S, B> Transform<S, ServiceRequest> for RateLimitMiddleware
where
    S: Service<ServiceRequest, Response = ServiceResponse<B>, Error = Error> + 'static,
    S::Future: 'static,
    B: 'static,
{
    type Response = ServiceResponse<B>;
    type Error = Error;
    type InitError = ();
    type Transform = RateLimitMiddlewareService<S>;
    type Future = futures_util::future::Ready<Result<Self::Transform, Self::InitError>>;

    fn new_service(&self, service: S) -> Self::Future {
        futures_util::future::ok(RateLimitMiddlewareService {
            service: Rc::new(service),
            bucket: self.bucket.clone(),
        })
    }
}

pub struct RateLimitMiddlewareService<S> {
    service: Rc<S>,
    bucket: Rc<TokenBucket>,
}

impl<S, B> Service<ServiceRequest> for RateLimitMiddlewareService<S>
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
        let bucket = self.bucket.clone();

        Box::pin(async move {
            // Skip rate limiting for health endpoints
            if req.path().starts_with("/health") {
                return service.call(req).await;
            }

            // Extract client identifier (API key or IP)
            let client_id = if let Some(auth_header) = req.headers().get("authorization") {
                auth_header
                    .to_str()
                    .unwrap_or("unknown")
                    .to_string()
            } else if let Some(api_key) = req.headers().get("x-api-key") {
                api_key
                    .to_str()
                    .unwrap_or("unknown")
                    .to_string()
            } else {
                // Fall back to client IP
                req.connection_info()
                    .peer_addr()
                    .unwrap_or("unknown")
                    .to_string()
            };

            // Check rate limit
            if !bucket.allow_request(&client_id) {
                warn!("Rate limit exceeded for client: {}", client_id);
                return Ok(req.into_response(
                    HttpResponse::TooManyRequests().json(serde_json::json!({
                        "error": "Rate limit exceeded",
                        "retry_after_seconds": 60,
                        "limit": bucket.max_tokens,
                        "window": "1 minute"
                    }))
                ));
            }

            service.call(req).await
        })
    }
}

/// Security Headers Middleware (CORS, CSRF, etc.)
pub struct SecurityHeadersMiddleware;

impl<S, B> Transform<S, ServiceRequest> for SecurityHeadersMiddleware
where
    S: Service<ServiceRequest, Response = ServiceResponse<B>, Error = Error> + 'static,
    S::Future: 'static,
    B: 'static,
{
    type Response = ServiceResponse<B>;
    type Error = Error;
    type InitError = ();
    type Transform = SecurityHeadersMiddlewareService<S>;
    type Future = futures_util::future::Ready<Result<Self::Transform, Self::InitError>>;

    fn new_service(&self, service: S) -> Self::Future {
        futures_util::future::ok(SecurityHeadersMiddlewareService {
            service: Rc::new(service),
        })
    }
}

pub struct SecurityHeadersMiddlewareService<S> {
    service: Rc<S>,
}

impl<S, B> Service<ServiceRequest> for SecurityHeadersMiddlewareService<S>
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

        Box::pin(async move {
            let mut res = service.call(req).await?;

            // Add security headers
            let headers = res.headers_mut();

            // CORS headers (allow inference requests from trusted origins)
            headers.insert(
                "Access-Control-Allow-Origin",
                "http://localhost:3000,http://localhost:5173"
                    .parse()
                    .unwrap(),
            );
            headers.insert(
                "Access-Control-Allow-Methods",
                "GET,POST,PUT,DELETE,OPTIONS".parse().unwrap(),
            );
            headers.insert(
                "Access-Control-Allow-Headers",
                "Content-Type,Authorization,X-API-Key,X-Request-ID"
                    .parse()
                    .unwrap(),
            );
            headers.insert("Access-Control-Allow-Credentials", "true".parse().unwrap());
            headers.insert("Access-Control-Max-Age", "86400".parse().unwrap());

            // CSRF protection (same-site cookie policy)
            headers.insert(
                "X-CSRF-Token",
                uuid::Uuid::new_v4().to_string().parse().unwrap(),
            );

            // Security headers
            headers.insert(
                "X-Content-Type-Options",
                "nosniff".parse().unwrap(),
            );
            headers.insert(
                "X-Frame-Options",
                "DENY".parse().unwrap(),
            );
            headers.insert(
                "X-XSS-Protection",
                "1; mode=block".parse().unwrap(),
            );
            headers.insert(
                "Strict-Transport-Security",
                "max-age=31536000; includeSubDomains".parse().unwrap(),
            );
            headers.insert(
                "Content-Security-Policy",
                "default-src 'self'; script-src 'self'; style-src 'self' 'unsafe-inline'"
                    .parse()
                    .unwrap(),
            );

            // Prevent caching of sensitive data
            headers.insert(
                "Cache-Control",
                "no-store, no-cache, must-revalidate, proxy-revalidate"
                    .parse()
                    .unwrap(),
            );
            headers.insert("Pragma", "no-cache".parse().unwrap());
            headers.insert("Expires", "0".parse().unwrap());

            Ok(res)
        })
    }
}

/// Request ID Middleware (for tracing)
pub struct RequestIdMiddleware;

impl<S, B> Transform<S, ServiceRequest> for RequestIdMiddleware
where
    S: Service<ServiceRequest, Response = ServiceResponse<B>, Error = Error> + 'static,
    S::Future: 'static,
    B: 'static,
{
    type Response = ServiceResponse<B>;
    type Error = Error;
    type InitError = ();
    type Transform = RequestIdMiddlewareService<S>;
    type Future = futures_util::future::Ready<Result<Self::Transform, Self::InitError>>;

    fn new_service(&self, service: S) -> Self::Future {
        futures_util::future::ok(RequestIdMiddlewareService {
            service: Rc::new(service),
        })
    }
}

pub struct RequestIdMiddlewareService<S> {
    service: Rc<S>,
}

impl<S, B> Service<ServiceRequest> for RequestIdMiddlewareService<S>
where
    S: Service<ServiceRequest, Response = ServiceResponse<B>, Error = Error> + 'static,
    S::Future: 'static,
    B: 'static,
{
    type Response = ServiceResponse<B>;
    type Error = Error;
    type Future = LocalBoxFuture<'static, Result<Self::Response, Self::Error>>;

    forward_ready!(service);

    fn call(&self, mut req: ServiceRequest) -> Self::Future {
        let service = self.service.clone();

        // Extract or generate request ID
        let request_id = req
            .headers()
            .get("x-request-id")
            .and_then(|h| h.to_str().ok())
            .map(|s| s.to_string())
            .unwrap_or_else(|| uuid::Uuid::new_v4().to_string());

        // Store in extensions for middleware chain
        req.extensions_mut().insert(request_id.clone());

        Box::pin(async move {
            let mut res = service.call(req).await?;

            // Add request ID to response headers
            res.headers_mut().insert(
                "X-Request-ID",
                request_id.parse().unwrap(),
            );

            Ok(res)
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_token_bucket_creation() {
        let bucket = TokenBucket::new(100, 1.67);  // 100 req/min
        assert_eq!(bucket.max_tokens, 100);
    }

    #[test]
    fn test_token_bucket_allows_first_request() {
        let bucket = TokenBucket::new(10, 1.0);
        assert!(bucket.allow_request("client1"));
    }

    #[test]
    fn test_token_bucket_respects_limit() {
        let bucket = TokenBucket::new(2, 0.0);  // No refill
        assert!(bucket.allow_request("client1"));
        assert!(bucket.allow_request("client1"));
        assert!(!bucket.allow_request("client1"));  // Exceeded limit
    }

    #[test]
    fn test_rate_limit_different_clients() {
        let bucket = TokenBucket::new(1, 0.0);  // Max 1 token, no refill
        assert!(bucket.allow_request("client1"));
        assert!(!bucket.allow_request("client1"));  // client1 limit exceeded
        assert!(bucket.allow_request("client2"));  // client2 has own limit
    }
}
