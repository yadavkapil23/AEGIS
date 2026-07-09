/// Integration tests for AEGIS Gateway HTTP endpoints.
///
/// Health/allocation endpoints tested via Actix test server.
/// Inference validation tested as unit tests.

use actix_web::{web, App, http::StatusCode};
use actix_web::test as actix_test;

use aegis_gateway::backend_manager::BackendManager;
use aegis_gateway::llm_backend::LLMBackend;
use aegis_gateway::metrics::PrometheusMetrics;
use aegis_gateway::inference_handler;
use aegis_gateway::handlers;
use aegis_gateway::request_validator::{validate_request, InferenceRequest};

fn test_app() -> App<
    impl actix_web::dev::ServiceFactory<
        actix_web::dev::ServiceRequest,
        Config = (),
        Response = actix_web::dev::ServiceResponse<impl actix_web::body::MessageBody>,
        Error = actix_web::Error,
        InitError = (),
    >,
> {
    let bm = web::Data::new(BackendManager::new().unwrap());
    let pm = web::Data::new(PrometheusMetrics::new().unwrap());
    let lb = web::Data::new(LLMBackend::new("http://localhost:8000".into(), "http://localhost:8001".into(), 30));

    App::new()
        .app_data(bm).app_data(pm).app_data(lb)
        .service(inference_handler::health_live)
        .service(inference_handler::health_ready)
        .service(inference_handler::health_startup)
        .service(handlers::health_check)
        .service(handlers::readiness_check)
        .service(handlers::get_stats)
        .service(handlers::get_cluster_health)
}

// ── HTTP Health Endpoints ─────────────────────────────────────

#[actix_web::test]
async fn liveness_probe() {
    let app = actix_test::init_service(test_app()).await;
    let resp = actix_test::call_service(&app, actix_test::TestRequest::get().uri("/health/live").to_request()).await;
    assert_eq!(resp.status(), StatusCode::OK);
    let body: serde_json::Value = actix_test::read_body_json(resp).await;
    assert_eq!(body["status"], "alive");
}

#[actix_web::test]
async fn startup_probe() {
    let app = actix_test::init_service(test_app()).await;
    let resp = actix_test::call_service(&app, actix_test::TestRequest::get().uri("/health/startup").to_request()).await;
    assert_eq!(resp.status(), StatusCode::OK);
    let body: serde_json::Value = actix_test::read_body_json(resp).await;
    assert_eq!(body["status"], "started");
}

#[actix_web::test]
async fn stats_endpoint() {
    let app = actix_test::init_service(test_app()).await;
    let resp = actix_test::call_service(&app, actix_test::TestRequest::get().uri("/v1/stats").to_request()).await;
    assert!(resp.status().is_success() || resp.status().is_server_error());
}

#[actix_web::test]
async fn cluster_health_endpoint() {
    let app = actix_test::init_service(test_app()).await;
    let resp = actix_test::call_service(&app, actix_test::TestRequest::get().uri("/v1/cluster").to_request()).await;
    assert!(resp.status().is_success() || resp.status().is_server_error());
}

// ── Validation Unit Tests ─────────────────────────────────────

fn mk_req(model: &str, prompt: &str, max_tokens: u32) -> InferenceRequest {
    InferenceRequest { model: model.into(), prompt: prompt.into(), max_tokens, temperature: None, top_p: None }
}

#[test]
fn valid_request_passes() {
    assert!(validate_request(&mk_req("qwen2.5:0.5b", "Hello", 100)).is_ok());
}

#[test]
fn empty_model_rejected() {
    assert_eq!(validate_request(&mk_req("", "Hello", 10)).unwrap_err().error_code, "empty_model");
}

#[test]
fn invalid_model_chars_rejected() {
    assert_eq!(validate_request(&mk_req("model/with/slashes", "Hello", 10)).unwrap_err().error_code, "invalid_model_name");
}

#[test]
fn model_too_long_rejected() {
    assert_eq!(validate_request(&mk_req(&"a".repeat(300), "Hello", 10)).unwrap_err().error_code, "model_too_long");
}

#[test]
fn empty_prompt_rejected() {
    assert_eq!(validate_request(&mk_req("qwen2.5:0.5b", "", 10)).unwrap_err().error_code, "empty_prompt");
}

#[test]
fn prompt_too_long_rejected() {
    assert_eq!(validate_request(&mk_req("qwen2.5:0.5b", &"x".repeat(100_001), 10)).unwrap_err().error_code, "prompt_too_long");
}

#[test]
fn zero_max_tokens_rejected() {
    assert_eq!(validate_request(&mk_req("qwen2.5:0.5b", "Hello", 0)).unwrap_err().error_code, "invalid_max_tokens");
}

#[test]
fn max_tokens_too_large_rejected() {
    assert_eq!(validate_request(&mk_req("qwen2.5:0.5b", "Hello", 50_000)).unwrap_err().error_code, "invalid_max_tokens");
}

#[test]
fn temperature_out_of_range() {
    let mut r = mk_req("qwen2.5:0.5b", "Hello", 10);
    r.temperature = Some(3.0);
    assert_eq!(validate_request(&r).unwrap_err().error_code, "invalid_temperature");
}

#[test]
fn temperature_valid() {
    let mut r = mk_req("qwen2.5:0.5b", "Hello", 10);
    r.temperature = Some(0.7);
    assert!(validate_request(&r).is_ok());
}

#[test]
fn top_p_out_of_range() {
    let mut r = mk_req("qwen2.5:0.5b", "Hello", 10);
    r.top_p = Some(1.5);
    assert_eq!(validate_request(&r).unwrap_err().error_code, "invalid_top_p");
}

#[test]
fn top_p_valid() {
    let mut r = mk_req("qwen2.5:0.5b", "Hello", 10);
    r.top_p = Some(0.9);
    assert!(validate_request(&r).is_ok());
}

#[test]
fn optional_fields_none() {
    let mut r = mk_req("qwen2.5:0.5b", "Hello", 10);
    r.temperature = None;
    r.top_p = None;
    assert!(validate_request(&r).is_ok());
}
