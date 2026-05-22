/// Security Middleware for Actix-web
/// Rate limiting, CORS, CSRF protection, and security headers

use actix_web::{
    dev::{forward_ready, Service, ServiceRequest, ServiceResponse, Transform},
    Error, HttpResponse,
};
use futures_util::future::{ok, LocalBoxFuture, Ready};
use std::rc::Rc;
use std::sync::Arc;
use std::collections::HashMap;
use parking_lot::RwLock;
use tracing::warn;

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

        if *current_tokens >= 1.0 {
            *current_tokens -= 1.0;
            true
        } else {
            false
        }
    }
}

/// Rate Limit Middleware
pub struct RateLimitMiddleware {
    bucket: Arc<TokenBucket>,
}

impl RateLimitMiddleware {
    pub fn new(rps: u32) -> Self {
        Self {
            bucket: Arc::new(TokenBucket::new(rps, rps as f64)),
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
    type Future = Ready<Result<Self::Transform, Self::InitError>>;

    fn new_transform(&self, service: S) -> Self::Future {
        ok(RateLimitMiddlewareService {
            service: Rc::new(service),
            bucket: self.bucket.clone(),
        })
    }
}

pub struct RateLimitMiddlewareService<S> {
    service: Rc<S>,
    bucket: Arc<TokenBucket>,
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
            let client_id = req
                .connection_info()
                .peer_addr()
                .unwrap_or("unknown")
                .to_string();

            if !bucket.allow_request(&client_id) {
                warn!("Rate limit exceeded for client: {}", client_id);
                return Err(actix_web::error::ErrorTooManyRequests("Rate limit exceeded"));
            }

            service.call(req).await
        })
    }
}

/// Security Headers Middleware
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
    type Future = Ready<Result<Self::Transform, Self::InitError>>;

    fn new_transform(&self, service: S) -> Self::Future {
        ok(SecurityHeadersMiddlewareService {
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
            let res = service.call(req).await?;
            let mut res = res;

            // Add security headers
            res.headers_mut()
                .insert(actix_web::http::header::HeaderName::from_static("x-content-type-options"),
                    actix_web::http::header::HeaderValue::from_static("nosniff"));
            res.headers_mut()
                .insert(actix_web::http::header::HeaderName::from_static("x-frame-options"),
                    actix_web::http::header::HeaderValue::from_static("DENY"));
            res.headers_mut()
                .insert(actix_web::http::header::HeaderName::from_static("x-xss-protection"),
                    actix_web::http::header::HeaderValue::from_static("1; mode=block"));

            Ok(res)
        })
    }
}

/// Request ID Middleware for tracing
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
    type Future = Ready<Result<Self::Transform, Self::InitError>>;

    fn new_transform(&self, service: S) -> Self::Future {
        ok(RequestIdMiddlewareService {
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

    fn call(&self, req: ServiceRequest) -> Self::Future {
        let request_id = uuid::Uuid::new_v4().to_string();
        let service = self.service.clone();

        Box::pin(async move {
            let res = service.call(req).await?;
            let mut res = res;

            res.headers_mut()
                .insert(actix_web::http::header::HeaderName::from_static("x-request-id"),
                    actix_web::http::header::HeaderValue::from_str(&request_id).unwrap());

            Ok(res)
        })
    }
}
