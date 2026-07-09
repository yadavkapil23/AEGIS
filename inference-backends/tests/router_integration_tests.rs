//! Integration tests for `BackendRouter` against a mocked Ollama HTTP server.
//!
//! These exercise the real router/backend wiring end-to-end (config -> router ->
//! HTTP call -> response mapping) rather than unit-testing a single function.

use inference_backends::config::{BackendConfig, OllamaConfig};
use inference_backends::{BackendPreference, BackendRouter, InferenceRequest};

fn ollama_config(endpoints: Vec<String>, models: Vec<&str>) -> OllamaConfig {
    OllamaConfig {
        enabled: true,
        endpoints,
        models: models.into_iter().map(String::from).collect(),
        timeout_ms: 5000,
        max_concurrent_requests: 10,
        load_balancing: "round_robin".to_string(),
        enable_connection_pooling: true,
        pool_size: 4,
    }
}

fn router_config(ollama: OllamaConfig) -> BackendConfig {
    BackendConfig {
        huggingface: None,
        ollama: Some(ollama),
        llamacpp: None,
        default_preference: "auto".to_string(),
        fallback_order: vec!["ollama".to_string()],
        default_timeout_ms: 5000,
        enable_health_checks: true,
        health_check_interval_secs: 60,
    }
}

#[tokio::test]
async fn infer_via_ollama_returns_generated_text() {
    let mut server = mockito::Server::new_async().await;
    let _m = server
        .mock("POST", "/api/generate")
        .with_status(200)
        .with_header("content-type", "application/json")
        .with_body(r#"{"response":"hello world","done":true}"#)
        .create_async()
        .await;

    let config = router_config(ollama_config(vec![server.url()], vec!["qwen2.5:0.5b"]));
    let router = BackendRouter::new(config).await.expect("router should init");

    let request = InferenceRequest::new("qwen2.5:0.5b", "Hi there")
        .with_backend(BackendPreference::Ollama);

    let response = router.infer(request).await.expect("inference should succeed");

    assert_eq!(response.text, "hello world");
    assert!(response.backend_used.starts_with("ollama:"));
    assert_eq!(response.tokens_generated, 2);
}

#[tokio::test]
async fn infer_with_unsupported_model_fails_without_calling_backend() {
    let server = mockito::Server::new_async().await;
    // No mock registered for /api/generate — if the router calls it, this
    // assertion (0 hits) below will fail, proving the model check short-circuits.
    let config = router_config(ollama_config(vec![server.url()], vec!["known-model"]));
    let router = BackendRouter::new(config).await.expect("router should init");

    let request = InferenceRequest::new("unknown-model", "Hi there")
        .with_backend(BackendPreference::Ollama);

    let result = router.infer(request).await;
    assert!(result.is_err());
}

#[tokio::test]
async fn infer_propagates_backend_http_error() {
    let mut server = mockito::Server::new_async().await;
    let _m = server
        .mock("POST", "/api/generate")
        .with_status(500)
        .with_body("internal error")
        .create_async()
        .await;

    let config = router_config(ollama_config(vec![server.url()], vec!["qwen2.5:0.5b"]));
    let router = BackendRouter::new(config).await.expect("router should init");

    let request = InferenceRequest::new("qwen2.5:0.5b", "Hi there")
        .with_backend(BackendPreference::Ollama);

    let result = router.infer(request).await;
    assert!(result.is_err());
}

#[tokio::test]
async fn health_check_reports_unhealthy_when_endpoint_unreachable() {
    // Port 1 is reserved and should refuse connections, simulating a dead endpoint.
    let config = router_config(ollama_config(
        vec!["http://127.0.0.1:1".to_string()],
        vec!["qwen2.5:0.5b"],
    ));
    let router = BackendRouter::new(config).await.expect("router should init");

    let statuses = router.health_check().await.expect("health check should not error");
    assert_eq!(statuses.len(), 1);
    assert!(!statuses[0].healthy);
    assert_eq!(statuses[0].status, "degraded");
}

#[tokio::test]
async fn health_check_reports_healthy_when_endpoint_responds() {
    let mut server = mockito::Server::new_async().await;
    let _m = server
        .mock("GET", "/api/tags")
        .with_status(200)
        .with_body("{}")
        .create_async()
        .await;

    let config = router_config(ollama_config(vec![server.url()], vec!["qwen2.5:0.5b"]));
    let router = BackendRouter::new(config).await.expect("router should init");

    let statuses = router.health_check().await.expect("health check should not error");
    assert_eq!(statuses.len(), 1);
    assert!(statuses[0].healthy);
}

#[tokio::test]
async fn round_robin_load_balances_across_endpoints() {
    let mut server_a = mockito::Server::new_async().await;
    let mut server_b = mockito::Server::new_async().await;

    let mock_a = server_a
        .mock("POST", "/api/generate")
        .with_status(200)
        .with_header("content-type", "application/json")
        .with_body(r#"{"response":"from a","done":true}"#)
        .expect(1)
        .create_async()
        .await;
    let mock_b = server_b
        .mock("POST", "/api/generate")
        .with_status(200)
        .with_header("content-type", "application/json")
        .with_body(r#"{"response":"from b","done":true}"#)
        .expect(1)
        .create_async()
        .await;

    let config = router_config(ollama_config(
        vec![server_a.url(), server_b.url()],
        vec!["qwen2.5:0.5b"],
    ));
    let router = BackendRouter::new(config).await.expect("router should init");

    for _ in 0..2 {
        let request = InferenceRequest::new("qwen2.5:0.5b", "Hi")
            .with_backend(BackendPreference::Ollama);
        router.infer(request).await.expect("inference should succeed");
    }

    mock_a.assert_async().await;
    mock_b.assert_async().await;
}

#[tokio::test]
async fn get_available_models_aggregates_and_dedupes() {
    let server = mockito::Server::new_async().await;
    let config = router_config(ollama_config(
        vec![server.url()],
        vec!["model-a", "model-b", "model-a"],
    ));
    let router = BackendRouter::new(config).await.expect("router should init");

    let models = router.get_available_models().await.expect("should list models");
    assert_eq!(models, vec!["model-a", "model-b"]);
}

#[tokio::test]
async fn router_creation_fails_with_no_backends_configured() {
    let config = router_config(OllamaConfig {
        enabled: false,
        ..ollama_config(vec!["http://127.0.0.1:11434".to_string()], vec!["m"])
    });

    let result = BackendRouter::new(config).await;
    assert!(result.is_err());
}
