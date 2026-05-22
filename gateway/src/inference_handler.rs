/// Inference Request Handler with Backend Manager Integration
/// Handles incoming inference requests with all production safety features

use actix_web::{web, HttpResponse, post, get};
use serde::{Deserialize, Serialize};
use tracing::{info, error};
use crate::backend_manager::BackendManager;
use crate::metrics::PrometheusMetrics;
use aegis_inference_backends::models::InferenceRequest as BackendRequest;

#[derive(Debug, Serialize, Deserialize)]
pub struct InferenceRequest {
    pub model: String,
    pub prompt: String,
    pub max_tokens: u32,
    #[serde(default)]
    pub temperature: Option<f32>,
    #[serde(default)]
    pub top_p: Option<f32>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct InferenceResponse {
    pub success: bool,
    pub output: Option<String>,
    pub tokens_generated: u32,
    pub latency_ms: u32,
    pub error: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct InferenceError {
    pub error: String,
    pub error_code: String,
}

/// POST /infer - Execute inference with production safeguards
#[post("/infer")]
pub async fn infer_handler(
    req: web::Json<InferenceRequest>,
    manager: web::Data<BackendManager>,
    metrics: web::Data<PrometheusMetrics>,
) -> HttpResponse {
    // Validate request
    match validate_request(&req) {
        Err(e) => {
            error!("Invalid request: {}", e);
            metrics.record_inference_error("invalid_input");
            return HttpResponse::BadRequest().json(InferenceError {
                error: e,
                error_code: "invalid_request".to_string(),
            });
        }
        Ok(_) => {}
    }

    // Convert to backend request
    let backend_request = BackendRequest {
        model: req.model.clone(),
        prompt: req.prompt.clone(),
        max_tokens: req.max_tokens,
    };

    // Execute inference with all production safeguards
    match manager.infer(backend_request).await {
        Ok(response) => {
            info!(
                "Inference success: model={}, tokens={}, latency={}ms",
                req.model, response.tokens_generated, response.latency_ms
            );

            metrics.record_inference_success(
                &req.model,
                response.latency_ms,
                response.tokens_generated,
            );

            HttpResponse::Ok().json(InferenceResponse {
                success: true,
                output: Some(response.output),
                tokens_generated: response.tokens_generated,
                latency_ms: response.latency_ms,
                error: None,
            })
        }
        Err(aegis_inference_backends::BackendError::RateLimited) => {
            error!("Rate limited");
            metrics.record_rate_limited();
            HttpResponse::TooManyRequests().json(InferenceError {
                error: "Rate limit exceeded. Please try again later.".to_string(),
                error_code: "rate_limited".to_string(),
            })
        }
        Err(aegis_inference_backends::BackendError::CircuitBreakerOpen) => {
            error!("Circuit breaker open");
            metrics.record_circuit_breaker_trip();
            metrics.record_inference_error("circuit_breaker_open");
            HttpResponse::ServiceUnavailable().json(InferenceError {
                error: "Backend temporarily unavailable due to high error rate. Please try again in a few seconds.".to_string(),
                error_code: "circuit_breaker_open".to_string(),
            })
        }
        Err(e) => {
            error!("Inference error: {}", e);
            metrics.record_inference_error("backend_error");
            HttpResponse::BadGateway().json(InferenceError {
                error: format!("Backend error: {}", e),
                error_code: "backend_error".to_string(),
            })
        }
    }
}

/// GET /health/ready - Readiness probe
#[get("/health/ready")]
pub async fn health_ready(manager: web::Data<BackendManager>) -> HttpResponse {
    if manager.health_check().await {
        HttpResponse::Ok().json(serde_json::json!({
            "status": "ready",
            "timestamp": chrono::Utc::now()
        }))
    } else {
        HttpResponse::ServiceUnavailable().json(serde_json::json!({
            "status": "not_ready",
            "reason": "Circuit breaker open or backend unhealthy"
        }))
    }
}

/// GET /health/live - Liveness probe
#[get("/health/live")]
pub async fn health_live() -> HttpResponse {
    HttpResponse::Ok().json(serde_json::json!({
        "status": "alive",
        "pid": std::process::id()
    }))
}

/// GET /health/startup - Startup probe
#[get("/health/startup")]
pub async fn health_startup() -> HttpResponse {
    HttpResponse::Ok().json(serde_json::json!({
        "status": "started",
        "timestamp": chrono::Utc::now()
    }))
}

/// GET /metrics - Prometheus metrics for monitoring
#[get("/metrics")]
pub async fn metrics_handler(metrics: web::Data<PrometheusMetrics>) -> HttpResponse {
    match metrics.export() {
        Ok(prometheus_text) => {
            HttpResponse::Ok()
                .content_type("text/plain; version=0.0.4; charset=utf-8")
                .body(prometheus_text)
        }
        Err(e) => {
            error!("Failed to export metrics: {}", e);
            HttpResponse::InternalServerError().json(serde_json::json!({
                "error": "Failed to export metrics"
            }))
        }
    }
}

/// Validate inference request
fn validate_request(req: &InferenceRequest) -> Result<(), String> {
    // Validate model name
    if req.model.is_empty() {
        return Err("model cannot be empty".to_string());
    }

    if !req.model.chars().all(|c| c.is_alphanumeric() || c == '-' || c == '_') {
        return Err("model name contains invalid characters".to_string());
    }

    // Validate prompt
    if req.prompt.is_empty() {
        return Err("prompt cannot be empty".to_string());
    }

    if req.prompt.len() > 100000 {
        return Err("prompt is too long (max 100,000 characters)".to_string());
    }

    // Validate max_tokens
    if req.max_tokens < 1 || req.max_tokens > 32000 {
        return Err("max_tokens must be between 1 and 32000".to_string());
    }

    // Validate temperature if provided
    if let Some(temp) = req.temperature {
        if temp < 0.0 || temp > 2.0 {
            return Err("temperature must be between 0.0 and 2.0".to_string());
        }
    }

    // Validate top_p if provided
    if let Some(top_p) = req.top_p {
        if top_p < 0.0 || top_p > 1.0 {
            return Err("top_p must be between 0.0 and 1.0".to_string());
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_validate_valid_request() {
        let req = InferenceRequest {
            model: "llama-7b".to_string(),
            prompt: "What is AI?".to_string(),
            max_tokens: 100,
            temperature: Some(0.7),
            top_p: Some(0.9),
        };

        assert!(validate_request(&req).is_ok());
    }

    #[test]
    fn test_validate_empty_model() {
        let req = InferenceRequest {
            model: "".to_string(),
            prompt: "test".to_string(),
            max_tokens: 100,
            temperature: None,
            top_p: None,
        };

        assert!(validate_request(&req).is_err());
    }

    #[test]
    fn test_validate_empty_prompt() {
        let req = InferenceRequest {
            model: "llama".to_string(),
            prompt: "".to_string(),
            max_tokens: 100,
            temperature: None,
            top_p: None,
        };

        assert!(validate_request(&req).is_err());
    }

    #[test]
    fn test_validate_invalid_max_tokens() {
        let req = InferenceRequest {
            model: "llama".to_string(),
            prompt: "test".to_string(),
            max_tokens: 50000,
            temperature: None,
            top_p: None,
        };

        assert!(validate_request(&req).is_err());
    }

    #[test]
    fn test_validate_invalid_temperature() {
        let req = InferenceRequest {
            model: "llama".to_string(),
            prompt: "test".to_string(),
            max_tokens: 100,
            temperature: Some(3.0),
            top_p: None,
        };

        assert!(validate_request(&req).is_err());
    }

    #[test]
    fn test_validate_invalid_top_p() {
        let req = InferenceRequest {
            model: "llama".to_string(),
            prompt: "test".to_string(),
            max_tokens: 100,
            temperature: None,
            top_p: Some(1.5),
        };

        assert!(validate_request(&req).is_err());
    }
}
