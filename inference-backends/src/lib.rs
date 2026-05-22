//! Inference Backends Module
//!
//! Provides abstraction layer for multiple inference backends:
//! - Hugging Face Inference API (cloud-based)
//! - vLLM (self-hosted distributed)
//!
//! Features automatic routing, fallback, and health checking

pub mod config;
pub mod error;
pub mod huggingface;
pub mod models;
pub mod router;
pub mod traits;
pub mod vllm;

pub use config::BackendConfig;
pub use error::{BackendError, Result};
pub use huggingface::HuggingFaceBackend;
pub use models::{BackendPreference, InferenceRequest, InferenceResponse};
pub use router::BackendRouter;
pub use traits::InferenceBackend;
pub use vllm::VLLMBackend;

/// Re-export common types
pub mod prelude {
    pub use crate::{
        BackendConfig, BackendError, BackendPreference, BackendRouter, HuggingFaceBackend,
        InferenceBackend, InferenceRequest, InferenceResponse, Result, VLLMBackend,
    };
}
