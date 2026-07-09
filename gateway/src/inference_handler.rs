/// Inference Request Handler
/// Handles incoming inference requests with validation, metrics, and audit logging

use actix_web::{web, HttpResponse, post, get};
use actix_web::web::Bytes;
use futures_util::StreamExt;
use serde::{Deserialize, Serialize};
use tracing::{info, error};
use std::time::Instant;
use crate::middleware::GatewayState;

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
    pub backend: Option<String>,
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
    state: web::Data<GatewayState>,
) -> HttpResponse {
    let start = Instant::now();

    // Validate request
    if let Err(e) = validate_request(&req) {
        error!("Invalid request: {}", e);
        state.metrics.record_inference_error("validation_error");
        return HttpResponse::BadRequest().json(InferenceError {
            error: e,
            error_code: "invalid_request".to_string(),
        });
    }

    // Check circuit breaker
    if let Err(e) = state.backend_manager.check_circuit_breaker() {
        state.metrics.record_inference_error("circuit_breaker_open");
        return HttpResponse::ServiceUnavailable().json(InferenceError {
            error: format!("Service temporarily unavailable: {}", e),
            error_code: "circuit_breaker_open".to_string(),
        });
    }

    // Acquire bulkhead slot
    if !state.backend_manager.try_acquire_bulkhead() {
        state.metrics.record_inference_error("bulkhead_rejected");
        return HttpResponse::ServiceUnavailable().json(InferenceError {
            error: "Too many concurrent requests".to_string(),
            error_code: "bulkhead_rejected".to_string(),
        });
    }

    info!(
        "Inference request: model={}, prompt_len={}, max_tokens={}",
        req.model,
        req.prompt.len(),
        req.max_tokens
    );

    // Call LLM backend (vLLM with fallback to llama.cpp, Ollama, HuggingFace)
    let result = state.llm_backend
        .infer(
            &req.model,
            &req.prompt,
            req.max_tokens,
            req.temperature,
            req.top_p,
        )
        .await;

    // Release bulkhead slot
    state.backend_manager.release_bulkhead();

    match result {
        Ok(result) => {
            let latency_ms = start.elapsed().as_millis() as u32;

            // Record success in circuit breaker
            state.backend_manager.record_success();

            // Record Prometheus metrics
            state.metrics.record_inference_success(
                &req.model,
                latency_ms,
                result.tokens_generated,
            );

            // Log to database (async, non-blocking)
            let db = state.db_pool.clone();
            let model = req.model.clone();
            let backend = result.backend.clone();
            let tokens = result.tokens_generated;
            tokio::spawn(async move {
                if let Err(e) = crate::database::log_inference(
                    &db,
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

            // Audit log (async, non-blocking)
            let db_audit = state.db_pool.clone();
            let audit_model = req.model.clone();
            let audit_backend = result.backend.clone();
            tokio::spawn(async move {
                if let Err(e) = crate::database::log_audit(
                    &db_audit,
                    "inference",
                    Some("model"),
                    Some(&audit_model),
                    None,
                    Some(&format!("backend={},tokens={}", audit_backend, tokens)),
                    "success",
                ).await {
                    error!("Failed to write audit log: {}", e);
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
                backend: Some(result.backend),
                error: None,
            })
        }
        Err(e) => {
            let latency_ms = start.elapsed().as_millis() as u32;

            // Record failure in circuit breaker
            state.backend_manager.record_failure();

            error!("Inference failed: {}", e);
            state.metrics.record_inference_error("inference_failed");

            // Log failure to database (async, non-blocking)
            let db = state.db_pool.clone();
            let model = req.model.clone();
            let error_msg = e.clone();
            tokio::spawn(async move {
                if let Err(db_err) = crate::database::log_inference(
                    &db,
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

            // Audit log (async, non-blocking)
            let db_audit = state.db_pool.clone();
            let audit_model = req.model.clone();
            let audit_error = e.clone();
            tokio::spawn(async move {
                if let Err(ae) = crate::database::log_audit(
                    &db_audit,
                    "inference",
                    Some("model"),
                    Some(&audit_model),
                    None,
                    Some(&format!("error={}", audit_error)),
                    "failure",
                ).await {
                    error!("Failed to write audit log: {}", ae);
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
pub async fn health_ready(state: web::Data<GatewayState>) -> HttpResponse {
    let vllm_healthy = state.llm_backend.check_vllm_health().await;
    let llamacpp_healthy = state.llm_backend.check_llamacpp_health().await;
    let ollama_healthy = state.llm_backend.check_ollama_health().await;
    let hf_healthy = state.llm_backend.check_hf_health().await;

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
pub async fn backends_status(state: web::Data<GatewayState>) -> HttpResponse {
    let status = state.llm_backend.get_backend_status().await;
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

// ── OpenAI-compatible Chat Completions ─────────────────────────

#[derive(Debug, Serialize, Deserialize)]
pub struct ChatMessage {
    pub role: String,
    pub content: String,
}

#[derive(Debug, Deserialize)]
pub struct ChatCompletionRequest {
    pub model: String,
    pub messages: Vec<ChatMessage>,
    #[serde(default)]
    pub max_tokens: Option<u32>,
    #[serde(default)]
    pub temperature: Option<f32>,
    #[serde(default)]
    pub top_p: Option<f32>,
    #[serde(default)]
    pub stream: Option<bool>,
}

#[derive(Debug, Serialize)]
pub struct ChatCompletionResponse {
    pub id: String,
    pub object: String,
    pub created: i64,
    pub model: String,
    pub choices: Vec<ChatChoice>,
    pub usage: ChatUsage,
}

#[derive(Debug, Serialize)]
pub struct ChatChoice {
    pub index: u32,
    pub message: ChatMessage,
    pub finish_reason: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct ChatUsage {
    pub prompt_tokens: u32,
    pub completion_tokens: u32,
    pub total_tokens: u32,
}

/// POST /v1/chat/completions - OpenAI-compatible chat completions
#[post("/v1/chat/completions")]
pub async fn chat_completions_handler(
    req: web::Json<ChatCompletionRequest>,
    state: web::Data<GatewayState>,
) -> HttpResponse {
    let start = Instant::now();

    if req.messages.is_empty() {
        state.metrics.record_inference_error("validation_error");
        return HttpResponse::BadRequest().json(InferenceError {
            error: "messages cannot be empty".to_string(),
            error_code: "invalid_request".to_string(),
        });
    }

    // Convert messages to a single prompt string
    let prompt: String = req.messages
        .iter()
        .map(|m| format!("{}: {}", m.role, m.content))
        .collect::<Vec<_>>()
        .join("\n");

    let max_tokens = req.max_tokens.unwrap_or(1024);

    // Validate
    if prompt.is_empty() || prompt.len() > 100000 {
        state.metrics.record_inference_error("validation_error");
        return HttpResponse::BadRequest().json(InferenceError {
            error: "invalid messages".to_string(),
            error_code: "invalid_request".to_string(),
        });
    }

    // Check circuit breaker
    if let Err(e) = state.backend_manager.check_circuit_breaker() {
        state.metrics.record_inference_error("circuit_breaker_open");
        return HttpResponse::ServiceUnavailable().json(InferenceError {
            error: format!("Service temporarily unavailable: {}", e),
            error_code: "circuit_breaker_open".to_string(),
        });
    }

    // Acquire bulkhead
    if !state.backend_manager.try_acquire_bulkhead() {
        state.metrics.record_inference_error("bulkhead_rejected");
        return HttpResponse::ServiceUnavailable().json(InferenceError {
            error: "Too many concurrent requests".to_string(),
            error_code: "bulkhead_rejected".to_string(),
        });
    }

    let result = state.llm_backend
        .infer(&req.model, &prompt, max_tokens, req.temperature, req.top_p)
        .await;

    state.backend_manager.release_bulkhead();

    match result {
        Ok(result) => {
            let latency_ms = start.elapsed().as_millis() as u32;
            state.backend_manager.record_success();
            state.metrics.record_inference_success(&req.model, latency_ms, result.tokens_generated);

            let db = state.db_pool.clone();
            let model = req.model.clone();
            let backend = result.backend.clone();
            let tokens = result.tokens_generated;
            tokio::spawn(async move {
                let _ = crate::database::log_inference(
                    &db, &model, "success", latency_ms as i32,
                    Some(tokens as i32), Some(&backend), None,
                ).await;
            });

            let response_id = format!("chatcmpl-{}", uuid::Uuid::new_v4());
            HttpResponse::Ok().json(ChatCompletionResponse {
                id: response_id,
                object: "chat.completion".to_string(),
                created: chrono::Utc::now().timestamp(),
                model: req.model.clone(),
                choices: vec![ChatChoice {
                    index: 0,
                    message: ChatMessage {
                        role: "assistant".to_string(),
                        content: result.output,
                    },
                    finish_reason: Some("stop".to_string()),
                }],
                usage: ChatUsage {
                    prompt_tokens: result.prompt_tokens,
                    completion_tokens: result.tokens_generated,
                    total_tokens: result.total_tokens,
                },
            })
        }
        Err(e) => {
            state.backend_manager.record_failure();
            state.metrics.record_inference_error("inference_failed");
            HttpResponse::InternalServerError().json(InferenceError {
                error: format!("Inference failed: {}", e),
                error_code: "inference_error".to_string(),
            })
        }
    }
}

/// GET /metrics - Prometheus metrics (real export)
#[get("/metrics")]
pub async fn metrics_handler(state: web::Data<GatewayState>) -> HttpResponse {
    match state.metrics.export() {
        Ok(metrics_text) => HttpResponse::Ok()
            .content_type("text/plain; version=0.0.4; charset=utf-8")
            .body(metrics_text),
        Err(e) => {
            error!("Failed to export metrics: {}", e);
            HttpResponse::InternalServerError()
                .body(format!("Failed to export metrics: {}", e))
        }
    }
}

/// Validate inference request
fn validate_request(req: &InferenceRequest) -> Result<(), String> {
    if req.model.is_empty() {
        return Err("model cannot be empty".to_string());
    }
    if !req.model.chars().all(|c| c.is_alphanumeric() || c == '-' || c == '_') {
        return Err("model name contains invalid characters".to_string());
    }
    if req.prompt.is_empty() {
        return Err("prompt cannot be empty".to_string());
    }
    if req.prompt.len() > 100000 {
        return Err("prompt is too long (max 100,000 characters)".to_string());
    }
    if req.max_tokens < 1 || req.max_tokens > 32000 {
        return Err("max_tokens must be between 1 and 32000".to_string());
    }
    if let Some(temp) = req.temperature {
        if !(0.0..=2.0).contains(&temp) {
            return Err("temperature must be between 0.0 and 2.0".to_string());
        }
    }
    if let Some(top_p) = req.top_p {
        if !(0.0..=1.0).contains(&top_p) {
            return Err("top_p must be between 0.0 and 1.0".to_string());
        }
    }
    Ok(())
}

/// POST /infer/stream - Execute inference with streaming SSE response
#[post("/infer/stream")]
pub async fn infer_stream_handler(
    req: web::Json<InferenceRequest>,
    state: web::Data<GatewayState>,
) -> HttpResponse {
    let start = Instant::now();

    if let Err(e) = validate_request(&req) {
        state.metrics.record_inference_error("validation_error");
        return HttpResponse::BadRequest().json(InferenceError {
            error: e,
            error_code: "invalid_request".into(),
        });
    }

    let model = req.model.clone();
    let prompt = req.prompt.clone();
    let max_tokens = req.max_tokens;
    let temperature = req.temperature;
    let top_p = req.top_p;

    info!(model = %model, max_tokens = max_tokens, "Streaming inference started");

    // Build streaming request to vLLM
    let vllm_url = format!("{}/v1/completions", state.llm_backend.vllm_endpoint());
    let stream_request = serde_json::json!({
        "model": model,
        "prompt": prompt,
        "max_tokens": max_tokens,
        "temperature": temperature,
        "top_p": top_p,
        "stream": true,
    });

    let client = reqwest::Client::new();
    let response = match client
        .post(&vllm_url)
        .json(&stream_request)
        .timeout(std::time::Duration::from_secs(120))
        .send()
        .await
    {
        Ok(r) => r,
        Err(e) => {
            error!(error = %e, "Streaming request failed");
            state.metrics.record_inference_error("stream_connect_failed");
            state.backend_manager.record_failure();
            return HttpResponse::BadGateway().json(InferenceError {
                error: format!("Backend connection failed: {}", e),
                error_code: "backend_error".into(),
            });
        }
    };

    if !response.status().is_success() {
        let status = response.status();
        let body = response.text().await.unwrap_or_default();
        error!(status = %status, body = %body, "Backend returned error");
        state.metrics.record_inference_error("backend_error");
        state.backend_manager.record_failure();
        return HttpResponse::BadGateway().json(InferenceError {
            error: format!("Backend error {}: {}", status, body),
            error_code: "backend_error".into(),
        });
    }

    // Stream the response as Server-Sent Events
    let byte_stream = response.bytes_stream();
    let metrics_clone = state.metrics.clone();
    let model_clone = model.clone();
    let db_clone = state.db_pool.clone();

    state.backend_manager.record_success();

    let sse_stream = byte_stream.filter_map(move |chunk| {
        let metrics = metrics_clone.clone();
        let _model = model_clone.clone();
        async move {
            match chunk {
                Ok(bytes) => {
                    let text = String::from_utf8_lossy(&bytes);
                    let mut sse_data = String::new();
                    for line in text.lines() {
                        let line = line.trim();
                        if let Some(json_str) = line.strip_prefix("data: ") {
                            if json_str == "[DONE]" {
                                sse_data.push_str("data: [DONE]\n\n");
                                continue;
                            }
                            sse_data.push_str(&format!("data: {}\n\n", json_str));
                        }
                    }
                    if sse_data.is_empty() {
                        None
                    } else {
                        Some(Ok::<Bytes, actix_web::Error>(Bytes::from(sse_data)))
                    }
                }
                Err(e) => {
                    error!(error = %e, "Stream read error");
                    metrics.record_inference_error("stream_read_error");
                    None
                }
            }
        }
    });

    let latency_ms = start.elapsed().as_millis() as u32;
    let tokens_est = max_tokens; // Estimate for streaming
    state.metrics.record_inference_success(&model, latency_ms, tokens_est);

    // Audit log for streaming request (clone model before moving into closure)
    let model_for_audit = model.clone();
    tokio::spawn(async move {
        let _ = crate::database::log_audit(
            &db_clone,
            "inference_stream",
            Some("model"),
            Some(&model_for_audit),
            None,
            None,
            "success",
        ).await;
    });

    info!(model = %model, latency_ms = latency_ms, "Streaming connection established");

    HttpResponse::Ok()
        .insert_header(("content-type", "text/event-stream"))
        .insert_header(("cache-control", "no-cache"))
        .insert_header(("connection", "keep-alive"))
        .streaming(sse_stream)
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
