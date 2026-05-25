//! Inference Backends Module
//!
//! Provides abstraction layer for multiple inference backends:
//! - Hugging Face Inference API (cloud-based)
//! - vLLM (self-hosted distributed)
//! - llama.cpp (lightweight local)
//! - Mock Backend (testing only - generates fake tokens)
//!
//! Features automatic routing, fallback, and health checking

pub mod config;
pub mod error;
pub mod huggingface;
pub mod llamacpp;
pub mod mock;
pub mod models;
pub mod production_manager;
pub mod router;
pub mod traits;
pub mod vllm;
pub mod llama_cpp_sys;
pub mod llama_cpp_safe;

pub use config::BackendConfig;
pub use error::{BackendError, Result};
pub use huggingface::HuggingFaceBackend;
pub use llamacpp::LlamaCppBackend;
pub use mock::MockBackend;
pub use models::{BackendPreference, InferenceRequest, InferenceResponse};
pub use production_manager::{CircuitBreaker, CircuitBreakerConfig, ProductionBackendManager, RateLimiter, Bulkhead, RetryConfig};
pub use router::BackendRouter;
pub use traits::InferenceBackend;
pub use vllm::VLLMBackend;

/// Re-export common types
pub mod prelude {
    pub use crate::{
        BackendConfig, BackendError, BackendPreference, BackendRouter, HuggingFaceBackend,
        InferenceBackend, InferenceRequest, InferenceResponse, LlamaCppBackend, MockBackend,
        Result, VLLMBackend,
    };
}
