// llama.cpp backend integration
// Uses llama_cpp_safe::Session for direct FFI-based inference

use crate::error::Result;
use crate::models::{HealthStatus, InferenceRequest, InferenceResponse};
use crate::llama_cpp_safe::Session;
use crate::traits::InferenceBackend;
use anyhow::Context;
use async_trait::async_trait;
use chrono::Utc;
use parking_lot::Mutex;
use std::sync::Arc;
use std::time::Instant;
use tracing::{debug, info, warn};

/// Backend config for the FFI-based llama.cpp backend
pub struct LlamaCppFfiConfig {
    pub model_path: String,
    pub context_size: u32,
    pub batch_size: u32,
    pub num_gpu_layers: i32,
    pub supported_models: Vec<String>,
}

/// Wrapper around llama.cpp safe interface (direct FFI, not HTTP)
pub struct LlamaCppFfiBackend {
    config: LlamaCppFfiConfig,
    session: Arc<Mutex<Session>>,
}

impl LlamaCppFfiBackend {
    pub async fn new(config: LlamaCppFfiConfig) -> anyhow::Result<Self> {
        info!("Initializing LlamaCppFfiBackend");
        info!("Model path: {}", config.model_path);
        info!("Context size: {}", config.context_size);
        info!("Batch size: {}", config.batch_size);
        info!("GPU layers: {} (0 = CPU only)", config.num_gpu_layers);

        let session = Session::new(
            &config.model_path,
            config.context_size,
            config.batch_size,
            num_cpus::get() as i32,
            config.num_gpu_layers,
            0.7,
            0.9,
            40,
        )
        .context("Failed to initialize llama.cpp session")?;

        Ok(Self {
            config,
            session: Arc::new(Mutex::new(session)),
        })
    }
}

#[async_trait]
impl InferenceBackend for LlamaCppFfiBackend {
    async fn infer(&self, request: InferenceRequest) -> Result<InferenceResponse> {
        debug!("LlamaCppFfiBackend::infer called");
        debug!("  Prompt length: {} chars", request.prompt.len());
        debug!("  Max tokens: {:?}", request.max_tokens);

        let start = Instant::now();

        let max_tokens = request.max_tokens.unwrap_or(256) as usize;

        let mut session = self.session.lock();
        let generated = session
            .generate(&request.prompt, max_tokens, num_cpus::get() as i32)
            .context("Generation failed")?;

        let elapsed_ms = start.elapsed().as_secs_f64() * 1000.0;
        debug!("Generation took {:.2}ms", elapsed_ms);

        let text: String = generated.iter().map(|(_, t)| t.as_str()).collect();

        let tokens_generated = generated.len() as u32;

        Ok(InferenceResponse {
            request_id: request.request_id.clone(),
            text,
            tokens_generated,
            backend_used: format!("llama-cpp-ffi:{}", self.config.model_path),
            processing_time_ms: elapsed_ms as u64,
            token_probabilities: None,
            finish_reason: "stop".to_string(),
            created_at: Utc::now(),
        })
    }

    async fn health_check(&self) -> Result<HealthStatus> {
        let session = self.session.lock();
        let vocab_size = session.vocab_size();

        let healthy = vocab_size > 0;
        let status = if healthy {
            "healthy".to_string()
        } else {
            "unhealthy: invalid vocabulary size".to_string()
        };

        if healthy {
            debug!("Health check passed. Vocab size: {}", vocab_size);
        } else {
            warn!("Health check failed. Vocab size: {}", vocab_size);
        }

        Ok(HealthStatus {
            healthy,
            backend: "llama-cpp-ffi".to_string(),
            status,
            latency_ms: 0.0,
            request_count: 0,
            error_count: 0,
            last_check: Utc::now(),
        })
    }

    fn name(&self) -> &str {
        "llama-cpp-ffi"
    }

    async fn supports_model(&self, model: &str) -> Result<bool> {
        Ok(self
            .config
            .supported_models
            .iter()
            .any(|m| m.contains(model)))
    }

    async fn get_models(&self) -> Result<Vec<String>> {
        Ok(self.config.supported_models.clone())
    }
}
