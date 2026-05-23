/// Inference Request Handler
/// Handles incoming inference requests with validation

use actix_web::{web, HttpResponse, post, get};
use serde::{Deserialize, Serialize};
use tracing::{info, error};
use crate::backend_manager::BackendManager;
use crate::metrics::PrometheusMetrics;
use crate::llm_backend::LLMBackend;
use crate::database::DbPool;
use std::time::Instant;

#[derive(Debug, Serialize, Deserialize, Clone)]
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

/// POST /infer - Execute inference
#[post("/infer")]
pub async fn infer_handler(
    req: web::Json<InferenceRequest>,
    _manager: web::Data<BackendManager>,
    metrics: web::Data<PrometheusMetrics>,
    llm_backend: web::Data<LLMBackend>,
    db: web::Data<DbPool>,
) -> HttpResponse {
    let start = Instant::now();

    // Validate request
    match validate_request(&req) {
        Err(e) => {
            error!("Invalid request: {}", e);
            metrics.record_inference_error("validation_error");
            return HttpResponse::BadRequest().json(InferenceError {
                error: e,
                error_code: "invalid_request".to_string(),
            });
        }
        Ok(_) => {}
    }

    info!(
        "Inference request: model={}, prompt_len={}, max_tokens={}",
        req.model,
        req.prompt.len(),
        req.max_tokens
    );

    // Call real LLM backend (vLLM with fallback to llama.cpp)
    match llm_backend
        .infer(
            &req.model,
            &req.prompt,
            req.max_tokens,
            req.temperature,
            req.top_p,
        )
        .await
    {
        Ok(result) => {
            let latency_ms = start.elapsed().as_millis() as u32;

            // Record metrics
            metrics.record_inference_success(
                &req.model,
                latency_ms,
                result.tokens_generated,
            );

            // Log to database (async, non-blocking)
            let db_clone = db.clone();
            let model = req.model.clone();
            let backend = result.backend.clone();
            let tokens = result.tokens_generated;
            tokio::spawn(async move {
                if let Err(e) = crate::database::log_inference(
                    &db_clone,
                    &model,
                    "success",
                    latency_ms as i32,
                    Some(tokens as i32),
                    Some(&backend),
                    None,
                ).await {
                    error!("Failed to log inference to database: {}", e);
                }
            });

            info!(
                "Inference succeeded: model={}, backend={}, tokens={}, latency_ms={}",
                req.model, result.backend, result.tokens_generated, latency_ms
            );

            HttpResponse::Ok().json(InferenceResponse {
                success: true,
                output: Some(result.output),
                tokens_generated: result.tokens_generated,
                latency_ms,
                error: None,
            })
        }
        Err(e) => {
            let latency_ms = start.elapsed().as_millis() as u32;

            error!("Inference failed: {}", e);
            metrics.record_inference_error("inference_failed");

            // Log failure to database (async, non-blocking)
            let db_clone = db.clone();
            let model = req.model.clone();
            let error_msg = e.clone();
            tokio::spawn(async move {
                if let Err(db_err) = crate::database::log_inference(
                    &db_clone,
                    &model,
                    "failure",
                    latency_ms as i32,
                    None,
                    None,
                    Some(&error_msg),
                ).await {
                    error!("Failed to log inference failure to database: {}", db_err);
                }
            });

            HttpResponse::InternalServerError().json(InferenceError {
                error: format!("Inference failed: {}", e),
                error_code: "inference_error".to_string(),
            })
        }
    }
}

/// GET /health/ready - Readiness probe
#[get("/health/ready")]
pub async fn health_ready(
    _manager: web::Data<BackendManager>,
    llm_backend: web::Data<LLMBackend>,
) -> HttpResponse {
    let vllm_healthy = llm_backend.check_vllm_health().await;
    let llamacpp_healthy = llm_backend.check_llamacpp_health().await;
    let ollama_healthy = llm_backend.check_ollama_health().await;
    let hf_healthy = llm_backend.check_hf_health().await;

    // Ready if at least one backend is healthy
    let ready = vllm_healthy || llamacpp_healthy || ollama_healthy || hf_healthy;

    let status_code = if ready {
        actix_web::http::StatusCode::OK
    } else {
        actix_web::http::StatusCode::SERVICE_UNAVAILABLE
    };

    let status = if ready { "ready" } else { "not_ready" };

    HttpResponse::build(status_code).json(serde_json::json!({
        "status": status,
        "timestamp": chrono::Utc::now(),
        "backends": {
            "vllm": vllm_healthy,
            "llamacpp": llamacpp_healthy,
            "ollama": ollama_healthy,
            "huggingface": hf_healthy
        }
    }))
}

/// GET /backends/status - Get detailed backend status
#[get("/backends/status")]
pub async fn backends_status(llm_backend: web::Data<LLMBackend>) -> HttpResponse {
    let status = llm_backend.get_backend_status().await;
    HttpResponse::Ok().json(status)
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

/// GET /metrics - Prometheus metrics
#[get("/metrics")]
pub async fn metrics_handler(_metrics: web::Data<PrometheusMetrics>) -> HttpResponse {
    HttpResponse::Ok()
        .content_type("text/plain; version=0.0.4; charset=utf-8")
        .body("# AEGIS Gateway Metrics\n")
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
