//! Inference Backends Module
//!
//! Provides abstraction layer for multiple inference backends:
//! - Hugging Face Inference API (cloud-based)
//! - Ollama (self-hosted, e.g. Qwen2.5-0.5B)
//! - llama.cpp (lightweight local, native FFI behind the `native-llama` feature)
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
pub mod ollama;

#[cfg(feature = "native-llama")]
pub mod llama_cpp_sys;
#[cfg(feature = "native-llama")]
pub mod llama_cpp_safe;
#[cfg(not(feature = "native-llama"))]
pub mod llama_cpp_safe {
    //! Stub present when the `native-llama` feature is disabled (the default).
    //! Keeps `Session`'s type name and KV-cache hooks available to dependents
    //! (e.g. `scheduler`) that hold an opaque handle to it, without requiring
    //! the llama.cpp FFI toolchain to be built/linked.
    pub struct Session {
        _private: (),
    }

    impl Session {
        /// No-op: no native session is bound without the `native-llama` feature.
        pub fn kv_cache_rm(&self, _seq_id: i32, _p0: i32, _p1: i32) {}
    }
}

pub use config::BackendConfig;
pub use error::{BackendError, Result};
pub use huggingface::HuggingFaceBackend;
pub use llamacpp::LlamaCppBackend;
pub use mock::MockBackend;
pub use models::{BackendPreference, InferenceRequest, InferenceResponse};
pub use production_manager::{CircuitBreaker, CircuitBreakerConfig, ProductionBackendManager, RateLimiter, Bulkhead, RetryConfig};
pub use router::BackendRouter;
pub use traits::InferenceBackend;
pub use ollama::OllamaBackend;

/// Re-export common types
pub mod prelude {
    pub use crate::{
        BackendConfig, BackendError, BackendPreference, BackendRouter, HuggingFaceBackend,
        InferenceBackend, InferenceRequest, InferenceResponse, LlamaCppBackend, MockBackend,
        Result, OllamaBackend,
    };
}
