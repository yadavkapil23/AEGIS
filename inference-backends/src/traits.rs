use crate::models::{HealthStatus, InferenceRequest, InferenceResponse};
use crate::Result;
use async_trait::async_trait;

/// Trait that all inference backends must implement
#[async_trait]
pub trait InferenceBackend: Send + Sync {
    /// Execute inference request
    async fn infer(&self, request: InferenceRequest) -> Result<InferenceResponse>;

    /// Check backend health
    async fn health_check(&self) -> Result<HealthStatus>;

    /// Get backend name
    fn name(&self) -> &str;

    /// Check if backend supports this model
    async fn supports_model(&self, model: &str) -> Result<bool>;

    /// Get list of supported models
    async fn get_models(&self) -> Result<Vec<String>>;

    /// Warm up the backend (optional)
    async fn warmup(&self) -> Result<()> {
        Ok(())
    }
}
