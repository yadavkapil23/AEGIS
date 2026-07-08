// Gateway module: HTTP/gRPC entry point for AEGIS inference

pub mod allocation_client;
pub mod api_key_handlers;
pub mod auth;
pub mod backup;
pub mod backend_manager;
pub mod cache;
pub mod config;
pub mod credentials;
pub mod database;
pub mod db_migrations;
pub mod handlers;
pub mod inference_handler;
pub mod jwt_auth;
pub mod llm_backend;
pub mod metrics;
pub mod middleware;
pub mod rate_limiter;
pub mod request_queue;
pub mod request_validator;
pub mod security_middleware;
pub mod service;
pub mod telemetry;

pub use config::GatewayConfig;
pub use middleware::GatewayState;
pub use request_queue::RequestQueue;
pub use metrics::GatewayMetrics;
pub use service::InferenceService;
pub use llm_backend::LLMBackend;
pub use database::DbPool;
