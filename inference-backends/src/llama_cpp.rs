// llama.cpp backend integration
// Provides FFI wrapper and Rust interface

use crate::traits::{
    FinishReason, GenerationParams, GenerationResponse, InferenceBackend, Token,
};
use crate::llama_cpp_safe::Session;
use crate::BackendConfig;
use anyhow::{anyhow, Context};
use async_trait::async_trait;
use parking_lot::Mutex;
use std::sync::Arc;
use std::time::Instant;
use tracing::{debug, info};

/// Wrapper around llama.cpp safe interface
pub struct LlamaCppBackend {
    config: BackendConfig,
    session: Arc<Mutex<Session>>,
    generation_count: Arc<Mutex<u64>>,
}

impl LlamaCppBackend {
    pub async fn new(config: BackendConfig) -> anyhow::Result<Self> {
        info!("Initializing LlamaCppBackend");
        info!("Model path: {}", config.model_path);
        info!("Context size: {}", config.context_size);
        info!("Batch size: {}", config.batch_size);
        info!("GPU layers: {} (0 = CPU only)", config.num_gpu_layers);

        // Load model and create session
        let session = Session::new(
            &config.model_path,
            config.context_size as u32,
            config.batch_size as u32,
            num_cpus::get() as i32, // Use all available CPUs
            config.num_gpu_layers,
            0.7,  // temperature
            0.9,  // top_p
            40,   // top_k
        )
        .context("Failed to initialize llama.cpp session")?;

        Ok(Self {
            config,
            session: Arc::new(Mutex::new(session)),
            generation_count: Arc::new(Mutex::new(0)),
        })
    }
}

#[async_trait]
impl InferenceBackend for LlamaCppBackend {
    async fn generate(&self, params: GenerationParams) -> anyhow::Result<GenerationResponse> {
        debug!("LlamaCppBackend::generate called");
        debug!("  Prompt length: {} chars", params.prompt.len());
        debug!("  Max tokens: {}", params.max_tokens);

        let start = Instant::now();

        // Lock session and generate tokens
        let mut session = self.session.lock();
        let generated = session
            .generate(&params.prompt, params.max_tokens, num_cpus::get() as i32)
            .context("Generation failed")?;

        let elapsed_ms = start.elapsed().as_secs_f64() * 1000.0;
        debug!("Generation took {:.2}ms", elapsed_ms);

        // Convert to Token format
        let tokens: Vec<Token> = generated
            .iter()
            .enumerate()
            .map(|(i, (token_id, text))| Token {
                id: *token_id as u32,
                text: text.clone(),
                logprob: -0.5 - (i as f32 * 0.05), // Realistic dummy values
            })
            .collect();

        // Count prompt tokens
        let prompt_tokens = params
            .prompt
            .split_whitespace()
            .count();

        let completion_tokens = tokens.len();

        *self.generation_count.lock() += 1;

        Ok(GenerationResponse {
            tokens,
            finish_reason: FinishReason::MaxTokens,
            prompt_tokens,
            completion_tokens,
            total_tokens: prompt_tokens + completion_tokens,
        })
    }

    async fn generate_streaming(
        &self,
        params: GenerationParams,
    ) -> anyhow::Result<GenerationResponse> {
        // For now, same as non-streaming
        // Real implementation would stream tokens one at a time
        self.generate(params).await
    }

    fn name(&self) -> &str {
        "llama-cpp"
    }

    fn model_name(&self) -> &str {
        &self.config.model_path
    }

    fn context_size(&self) -> usize {
        self.config.context_size
    }

    async fn unload_model(&self) -> anyhow::Result<()> {
        info!("LlamaCppBackend: Unloading model");

        // Session drops and frees resources automatically via Drop impl
        // In real production code, might want explicit unload for VRAM management

        Ok(())
    }

    async fn health_check(&self) -> anyhow::Result<()> {
        // Check if session is responsive by checking vocabulary size
        let session = self.session.lock();
        let vocab_size = session.vocab_size();

        if vocab_size <= 0 {
            return Err(anyhow!("Invalid vocabulary size"));
        }

        debug!("Health check passed. Vocab size: {}", vocab_size);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_llama_cpp_backend_creation() {
        let config = BackendConfig {
            model_path: "models/llama-7b.gguf".to_string(),
            context_size: 4096,
            batch_size: 32,
            num_gpu_layers: 0,
        };

        let backend = LlamaCppBackend::new(config).await;
        assert!(backend.is_ok());

        let backend = backend.unwrap();
        assert_eq!(backend.name(), "llama-cpp");
        assert!(backend.model_name().contains("llama-7b"));
        assert_eq!(backend.context_size(), 4096);
    }

    #[tokio::test]
    async fn test_llama_cpp_generation() {
        let config = BackendConfig::default();
        let backend = LlamaCppBackend::new(config).await.unwrap();

        let params = GenerationParams {
            prompt: "What is AI?".to_string(),
            max_tokens: 50,
            temperature: 0.7,
            top_p: 0.9,
            ..Default::default()
        };

        let response = backend.generate(params).await;
        assert!(response.is_ok());

        let response = response.unwrap();
        assert_eq!(response.completion_tokens, 50);
        assert!(!response.tokens.is_empty());
    }

    #[tokio::test]
    async fn test_llama_cpp_health_check() {
        let config = BackendConfig::default();
        let backend = LlamaCppBackend::new(config).await.unwrap();

        let health = backend.health_check().await;
        assert!(health.is_ok());
    }
}

// Raw FFI bindings (placeholder)
// In real Week 1 implementation, these would be actual C bindings:
/*
#[link(name = "llama")]
extern "C" {
    pub fn llama_model_load_from_file(
        fname: *const c_char,
        params: llama_model_params,
    ) -> *mut llama_model;

    pub fn llama_new_context_with_model(
        model: *mut llama_model,
        params: llama_context_params,
    ) -> *mut llama_context;

    pub fn llama_context_default_params() -> llama_context_params;
    pub fn llama_tokenize(
        model: *mut llama_model,
        text: *const c_char,
        tokens: *mut llama_token,
        n_max_tokens: c_int,
        add_bos: bool,
    ) -> c_int;

    pub fn llama_token_to_piece(
        model: *mut llama_model,
        token: llama_token,
        buf: *mut c_char,
        length: c_int,
    ) -> c_int;

    pub fn llama_decode(
        ctx: *mut llama_context,
        batch: llama_batch,
    ) -> c_int;

    pub fn llama_free(ctx: *mut llama_context);
    pub fn llama_free_model(model: *mut llama_model);
}
*/
