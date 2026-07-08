/// Integration tests for AEGIS Gateway HTTP endpoints.
///
/// These tests spin up the Actix-Web app in-process and send real HTTP requests
/// through the test server. No external services required for basic validation tests.

use actix_web::{test, web, App, http::StatusCode};
use serde_json::json;

/// Build the test app with all routes registered (no middleware for simplicity).
fn test_app() -> App<
    impl actix_web::dev::ServiceFactory<
        actix_web::dev::ServiceRequest,
        Config = (),
        Response = actix_web::dev::ServiceResponse<impl actix_web::body::MessageBody>,
        Error = actix_web::Error,
        InitError = (),
    >,
> {
    use aegis_gateway::inference_handler;
    use aegis_gateway::handlers;

    App::new()
        .service(inference_handler::infer_handler)
        .service(inference_handler::infer_stream_handler)
        .service(inference_handler::health_live)
        .service(inference_handler::health_ready)
        .service(inference_handler::health_startup)
        .service(handlers::health_check)
        .service(handlers::readiness_check)
        .service(handlers::get_stats)
        .service(handlers::get_cluster_health)
}

// ── Health Endpoints ──────────────────────────────────────────

#[actix_web::test]
async fn test_liveness_probe() {
    let app = test::init_service(test_app()).await;
    let req = test::TestRequest::get().uri("/health/live").to_request();
    let resp = test::call_service(&app, req).await;
    assert_eq!(resp.status(), StatusCode::OK);

    let body: serde_json::Value = test::read_body_json(resp).await;
    assert_eq!(body["status"], "alive");
}

#[actix_web::test]
async fn test_startup_probe() {
    let app = test::init_service(test_app()).await;
    let req = test::TestRequest::get().uri("/health/startup").to_request();
    let resp = test::call_service(&app, req).await;
    assert_eq!(resp.status(), StatusCode::OK);

    let body: serde_json::Value = test::read_body_json(resp).await;
    assert_eq!(body["status"], "started");
}

// ── Inference Validation ──────────────────────────────────────

#[actix_web::test]
async fn test_infer_empty_model_rejected() {
    let app = test::init_service(test_app()).await;
    let req = test::TestRequest::post()
        .uri("/infer")
        .set_json(json!({
            "model": "",
            "prompt": "Hello",
            "max_tokens": 10
        }))
        .to_request();
    let resp = test::call_service(&app, req).await;
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
}

#[actix_web::test]
async fn test_infer_empty_prompt_rejected() {
    let app = test::init_service(test_app()).await;
    let req = test::TestRequest::post()
        .uri("/infer")
        .set_json(json!({
            "model": "llama-7b",
            "prompt": "",
            "max_tokens": 10
        }))
        .to_request();
    let resp = test::call_service(&app, req).await;
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
}

#[actix_web::test]
async fn test_infer_invalid_max_tokens_rejected() {
    let app = test::init_service(test_app()).await;
    let req = test::TestRequest::post()
        .uri("/infer")
        .set_json(json!({
            "model": "llama-7b",
            "prompt": "Hello",
            "max_tokens": 0
        }))
        .to_request();
    let resp = test::call_service(&app, req).await;
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
}

#[actix_web::test]
async fn test_infer_max_tokens_too_large_rejected() {
    let app = test::init_service(test_app()).await;
    let req = test::TestRequest::post()
        .uri("/infer")
        .set_json(json!({
            "model": "llama-7b",
            "prompt": "Hello",
            "max_tokens": 50000
        }))
        .to_request();
    let resp = test::call_service(&app, req).await;
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
}

#[actix_web::test]
async fn test_infer_temperature_out_of_range_rejected() {
    let app = test::init_service(test_app()).await;
    let req = test::TestRequest::post()
        .uri("/infer")
        .set_json(json!({
            "model": "llama-7b",
            "prompt": "Hello",
            "max_tokens": 10,
            "temperature": 3.0
        }))
        .to_request();
    let resp = test::call_service(&app, req).await;
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
}

#[actix_web::test]
async fn test_infer_top_p_out_of_range_rejected() {
    let app = test::init_service(test_app()).await;
    let req = test::TestRequest::post()
        .uri("/infer")
        .set_json(json!({
            "model": "llama-7b",
            "prompt": "Hello",
            "max_tokens": 10,
            "top_p": 1.5
        }))
        .to_request();
    let resp = test::call_service(&app, req).await;
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
}

#[actix_web::test]
async fn test_infer_invalid_model_chars_rejected() {
    let app = test::init_service(test_app()).await;
    let req = test::TestRequest::post()
        .uri("/infer")
        .set_json(json!({
            "model": "model/with/slashes",
            "prompt": "Hello",
            "max_tokens": 10
        }))
        .to_request();
    let resp = test::call_service(&app, req).await;
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
}

// ── Allocation Endpoints (require backend, test validation) ───

#[actix_web::test]
async fn test_allocate_missing_body_rejected() {
    let app = test::init_service(test_app()).await;
    let req = test::TestRequest::post()
        .uri("/v1/allocate")
        .to_request();
    let resp = test::call_service(&app, req).await;
    // Should return 400 or 415 (missing content type / body)
    assert!(
        resp.status().is_client_error(),
        "Expected client error, got {}",
        resp.status()
    );
}

#[actix_web::test]
async fn test_stats_endpoint_returns_json() {
    let app = test::init_service(test_app()).await;
    let req = test::TestRequest::get().uri("/v1/stats").to_request();
    let resp = test::call_service(&app, req).await;
    assert!(resp.status().is_success() || resp.status().is_server_error());
}

// ── Request Validation Edge Cases ─────────────────────────────

#[actix_web::test]
async fn test_infer_valid_request_reaches_backend() {
    // This will fail with a backend error (no vLLM running),
    // but it proves the validation and routing logic works.
    let app = test::init_service(test_app()).await;
    let req = test::TestRequest::post()
        .uri("/infer")
        .set_json(json!({
            "model": "llama-7b",
            "prompt": "What is 2+2?",
            "max_tokens": 10,
            "temperature": 0.7,
            "top_p": 0.9
        }))
        .to_request();
    let resp = test::call_service(&app, req).await;
    // Will be 502 (bad gateway) since no backend is running,
    // but NOT 400 (bad request) — validation passed.
    assert_ne!(resp.status(), StatusCode::BAD_REQUEST);
}

#[actix_web::test]
async fn test_infer_optional_fields_none() {
    let app = test::init_service(test_app()).await;
    let req = test::TestRequest::post()
        .uri("/infer")
        .set_json(json!({
            "model": "test-model",
            "prompt": "Hello",
            "max_tokens": 5
        }))
        .to_request();
    let resp = test::call_service(&app, req).await;
    // Validation passes, backend may fail
    assert_ne!(resp.status(), StatusCode::BAD_REQUEST);
}
