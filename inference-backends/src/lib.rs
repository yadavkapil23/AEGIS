// Inference backend abstraction layer
// Supports: llama.cpp, vLLM, TensorRT-LLM, etc.

pub mod traits;
pub mod llama_cpp;
pub mod llama_cpp_safe;
pub mod llama_cpp_sys;
pub mod mock;
pub mod metrics;

pub use traits::{InferenceBackend, GenerationParams, Token, GenerationResponse};
pub use llama_cpp::LlamaCppBackend;
pub use llama_cpp_safe::Session;
pub use mock::MockBackend;
pub use metrics::BackendMetrics;

use std::sync::Arc;

/// Factory for creating inference backends
pub struct BackendFactory;

impl BackendFactory {
    /// Create a backend by name
    pub async fn create(
        backend_type: &str,
        config: BackendConfig,
    ) -> anyhow::Result<Arc<dyn InferenceBackend>> {
        match backend_type {
            #[cfg(feature = "llama-cpp")]
            "llama-cpp" => {
                let backend = LlamaCppBackend::new(config).await?;
                Ok(Arc::new(backend))
            }
            #[cfg(feature = "mock")]
            "mock" => {
                let backend = MockBackend::new(config);
                Ok(Arc::new(backend))
            }
            _ => Err(anyhow::anyhow!(
                "Unknown backend type: {}. Available: llama-cpp, mock",
                backend_type
            )),
        }
    }

    /// List available backends
    pub fn available_backends() -> Vec<&'static str> {
        let mut backends = vec![];

        #[cfg(feature = "llama-cpp")]
        backends.push("llama-cpp");

        #[cfg(feature = "mock")]
        backends.push("mock");

        backends
    }
}

/// Configuration for backend initialization
#[derive(Debug, Clone)]
pub struct BackendConfig {
    pub model_path: String,
    pub context_size: usize,
    pub batch_size: usize,
    pub num_gpu_layers: i32,  // -1 = all, 0 = CPU only
}

impl Default for BackendConfig {
    fn default() -> Self {
        Self {
            model_path: "models/llama-7b.gguf".to_string(),
            context_size: 4096,
            batch_size: 32,
            num_gpu_layers: 0,  // CPU by default
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_available_backends() {
        let backends = BackendFactory::available_backends();
        assert!(!backends.is_empty());
        println!("Available backends: {:?}", backends);
    }

    #[tokio::test]
    async fn test_create_mock_backend() {
        let config = BackendConfig::default();
        let backend = BackendFactory::create("mock", config).await;
        assert!(backend.is_ok());
    }
}
