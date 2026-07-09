// Gateway module: HTTP/gRPC entry point for AEGIS inference

pub mod allocation_client;
pub mod api_key_handlers;
pub mod backend_manager;
pub mod cache;
pub mod config;
pub mod database;
pub mod handlers;
pub mod inference_handler;
pub mod jwt_auth;
pub mod llm_backend;
pub mod metrics;
pub mod middleware;
pub mod request_queue;
pub mod request_validator;
pub mod security_middleware;
pub mod telemetry;

pub use config::GatewayConfig;
pub use middleware::GatewayState;
pub use llm_backend::LLMBackend;
pub use database::DbPool;
