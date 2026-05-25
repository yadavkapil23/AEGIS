// Safe Rust wrapper around llama.cpp FFI bindings

use crate::llama_cpp_sys as sys;
use anyhow::{anyhow, Result};
use anyhow::Context as AnyhowContext;
use std::ffi::CString;
use std::os::raw::c_char;
use std::sync::Arc;
use parking_lot::Mutex;
use tracing::{debug, info};

/// Safe wrapper for LlamaModel
pub struct Model {
    ptr: *mut sys::LlamaModel,
}

impl Model {
    /// Load model from GGUF file
    pub fn load(path: &str, n_gpu_layers: i32) -> Result<Self> {
        info!("Loading llama.cpp model from: {}", path);

        let path_cstr = CString::new(path)
            .with_context(|| anyhow::anyhow!("Failed to convert path to CString"))?;

        let mut params = unsafe { sys::llama_model_default_params() };
        params.n_gpu_layers = n_gpu_layers;

        let ptr = unsafe { sys::llama_model_load_from_file(path_cstr.as_ptr(), params) };

        if ptr.is_null() {
            return Err(anyhow!("Failed to load model from {}", path));
        }

        info!("Model loaded successfully");
        Ok(Self { ptr })
    }

    /// Get vocabulary size
    pub fn vocab_size(&self) -> i32 {
        unsafe { sys::llama_model_n_vocab(self.ptr) }
    }

    /// Tokenize text
    pub fn tokenize(&self, text: &str, add_bos: bool) -> Result<Vec<i32>> {
        let text_cstr = CString::new(text)
            .with_context(|| anyhow::anyhow!("Failed to convert text to CString"))?;

        let max_tokens = text.len() as i32 * 2; // Safe upper bound
        let mut tokens = vec![0; max_tokens as usize];

        let n_tokens = unsafe {
            sys::llama_tokenize(
                self.ptr,
                text_cstr.as_ptr(),
                tokens.as_mut_ptr(),
                max_tokens,
                add_bos,
            )
        };

        if n_tokens < 0 {
            return Err(anyhow!("Tokenization failed"));
        }

        tokens.truncate(n_tokens as usize);
        Ok(tokens)
    }

    /// Decode token to text
    pub fn token_to_piece(&self, token: i32) -> Result<String> {
        let mut buf = vec![0u8; 128];

        let n = unsafe {
            sys::llama_token_to_piece(
                self.ptr,
                token,
                buf.as_mut_ptr() as *mut c_char,
                buf.len() as i32,
            )
        };

        if n < 0 {
            return Err(anyhow!("Failed to decode token"));
        }

        buf.truncate(n as usize);
        let s = String::from_utf8_lossy(&buf).into_owned();
        Ok(s)
    }

    pub fn as_ptr(&self) -> *mut sys::LlamaModel {
        self.ptr
    }
}

impl Drop for Model {
    fn drop(&mut self) {
        unsafe {
            sys::llama_model_free(self.ptr);
        }
    }
}

unsafe impl Send for Model {}
unsafe impl Sync for Model {}

/// Safe wrapper for LlamaContext
pub struct Context {
    ptr: *mut sys::LlamaContext,
    model: Arc<Model>,
    n_ctx: i32,
}

impl Context {
    /// Create context from model
    pub fn new(model: Arc<Model>, n_ctx: u32, n_batch: u32, n_threads: i32) -> Result<Self> {
        info!("Creating llama.cpp context with n_ctx={}", n_ctx);

        let mut params = unsafe { sys::llama_context_default_params() };
        params.n_context = n_ctx;
        params.n_batch = n_batch;
        params.n_threads = n_threads;
        params.n_threads_batch = (n_threads + 1) / 2; // Half threads for batch

        let ptr = unsafe {
            sys::llama_new_context_with_model(model.as_ptr(), params)
        };

        if ptr.is_null() {
            return Err(anyhow!("Failed to create context"));
        }

        let n_ctx = unsafe { sys::llama_n_ctx(ptr) };

        info!("Context created successfully (n_ctx={})", n_ctx);

        Ok(Self {
            ptr,
            model,
            n_ctx,
        })
    }

    /// Run inference on tokens and get logits
    pub fn eval(&mut self, tokens: &[i32], _n_threads: i32) -> Result<()> {
        debug!("Evaluating {} tokens", tokens.len());

        let mut batch = unsafe { sys::llama_batch_init(tokens.len() as i32, 0, 1) };

        unsafe {
            sys::llama_batch_clear(&mut batch);

            for (i, &token) in tokens.iter().enumerate() {
                sys::llama_batch_add(
                    &mut batch,
                    token,
                    i as i32,
                    std::ptr::null_mut(),
                    0,
                    i == tokens.len() - 1, // Last token gets logits
                );
            }
        }

        let result = unsafe {
            sys::llama_decode(self.ptr, &mut batch)
        };

        unsafe {
            sys::llama_batch_free(batch);
        }

        if result != 0 {
            return Err(anyhow!("Inference failed with code {}", result));
        }

        Ok(())
    }

    /// Remove a sequence from the KV cache
    pub fn kv_cache_rm(&mut self, seq_id: i32, p0: i32, p1: i32) {
        unsafe {
            sys::llama_kv_cache_seq_rm(self.ptr, seq_id, p0, p1);
        }
    }

    /// Copy a sequence in the KV cache
    pub fn kv_cache_cp(&mut self, seq_id_src: i32, seq_id_dst: i32, p0: i32, p1: i32) {
        unsafe {
            sys::llama_kv_cache_seq_cp(self.ptr, seq_id_src, seq_id_dst, p0, p1);
        }
    }

    /// Keep only the specified sequence in the KV cache, removing all others
    pub fn kv_cache_keep(&mut self, seq_id: i32) {
        unsafe {
            sys::llama_kv_cache_seq_keep(self.ptr, seq_id);
        }
    }

    /// Get context size
    pub fn n_ctx(&self) -> i32 {
        self.n_ctx
    }

    pub fn as_ptr(&self) -> *mut sys::LlamaContext {
        self.ptr
    }
}

impl Drop for Context {
    fn drop(&mut self) {
        unsafe {
            sys::llama_free(self.ptr);
        }
    }
}

unsafe impl Send for Context {}
unsafe impl Sync for Context {}

/// Complete llama.cpp inference session
pub struct Session {
    model: Arc<Model>,
    context: Arc<Mutex<Context>>,
    temperature: f32,
    top_p: f32,
    top_k: i32,
}

impl Session {
    /// Create a new inference session
    pub fn new(
        model_path: &str,
        n_ctx: u32,
        n_batch: u32,
        n_threads: i32,
        n_gpu_layers: i32,
        temperature: f32,
        top_p: f32,
        top_k: i32,
    ) -> Result<Self> {
        info!("Creating llama.cpp session: {}", model_path);

        let model = Arc::new(Model::load(model_path, n_gpu_layers)?);
        let context = Arc::new(Mutex::new(Context::new(
            model.clone(),
            n_ctx,
            n_batch,
            n_threads,
        )?));

        Ok(Self {
            model,
            context,
            temperature,
            top_p,
            top_k,
        })
    }

    /// Generate tokens from prompt
    pub fn generate(
        &mut self,
        prompt: &str,
        max_tokens: usize,
        n_threads: i32,
    ) -> Result<Vec<(i32, String)>> {
        info!("Generating {} tokens from prompt", max_tokens);

        // Tokenize prompt
        let mut tokens = self.model.tokenize(prompt, true)?;
        debug!("Prompt tokenized to {} tokens", tokens.len());

        let mut generated = Vec::new();

        // Run inference
        for _ in 0..max_tokens {
            {
                let mut ctx = self.context.lock();
                ctx.eval(&tokens, n_threads)?;
            }

            // Sample next token (simplified: just use top token for now)
            // In real implementation, use llama_sampling_sample() with temperature/top_p
            let next_token = tokens[tokens.len() - 1] + 1; // Dummy sampling
            let text = self.model.token_to_piece(next_token)?;

            generated.push((next_token, text.clone()));
            tokens.push(next_token);

            // Stop if we hit EOT
            if next_token == 2 {
                break;
            }
        }

        Ok(generated)
    }

    /// Get model vocabulary size
    pub fn vocab_size(&self) -> i32 {
        self.model.vocab_size()
    }

    /// Remove a sequence from the KV cache
    pub fn kv_cache_rm(&self, seq_id: i32, p0: i32, p1: i32) {
        let mut ctx = self.context.lock();
        ctx.kv_cache_rm(seq_id, p0, p1);
    }

    /// Copy a sequence in the KV cache
    pub fn kv_cache_cp(&self, seq_id_src: i32, seq_id_dst: i32, p0: i32, p1: i32) {
        let mut ctx = self.context.lock();
        ctx.kv_cache_cp(seq_id_src, seq_id_dst, p0, p1);
    }

    /// Keep only the specified sequence in the KV cache
    pub fn kv_cache_keep(&self, seq_id: i32) {
        let mut ctx = self.context.lock();
        ctx.kv_cache_keep(seq_id);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_model_load_nonexistent() {
        let result = Model::load("/nonexistent/model.gguf", 0);
        assert!(result.is_err());
    }

    #[test]
    fn test_send_sync() {
        fn assert_send_sync<T: Send + Sync>() {}
        assert_send_sync::<Model>();
        assert_send_sync::<Context>();
        assert_send_sync::<Session>();
    }
}
