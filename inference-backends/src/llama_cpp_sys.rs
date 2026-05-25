// Raw FFI bindings to llama.cpp C library
// This file contains the unsafe C bindings - they're wrapped safely below

use std::os::raw::{c_char, c_float, c_int};

// Opaque C types
#[repr(C)]
pub struct LlamaModel {
    _opaque: [u8; 0],
}

#[repr(C)]
pub struct LlamaContext {
    _opaque: [u8; 0],
}

#[repr(C)]
pub struct LlamaBatch {
    _opaque: [u8; 0],
}

// Token type
pub type LlamaToken = i32;

// Model parameters
#[repr(C)]
pub struct LlamaModelParams {
    pub n_gpu_layers: c_int,
    pub main_gpu: c_int,
    pub tensor_split: *const c_float,
    pub progress_callback: Option<extern "C" fn(f32, *mut std::ffi::c_void)>,
    pub progress_callback_user_data: *mut std::ffi::c_void,
    pub vocab_only: bool,
    pub use_mmap: bool,
    pub use_mlock: bool,
}

// Context parameters
#[repr(C)]
pub struct LlamaContextParams {
    pub n_context: u32,
    pub n_batch: u32,
    pub n_ubatch: u32,
    pub n_seq_max: u32,
    pub n_threads: i32,
    pub n_threads_batch: i32,
    pub rope_scaling_type: i32,
    pub rope_freq_base: f32,
    pub rope_freq_scale: f32,
    pub yarn_ext_factor: f32,
    pub yarn_attn_factor: f32,
    pub yarn_beta_fast: f32,
    pub yarn_beta_slow: f32,
    pub yarn_orig_ctx: u32,
    pub defrag_thold: f32,
    pub cb_eval: Option<extern "C" fn(*mut LlamaContext, *mut LlamaBatch, c_int)>,
    pub cb_eval_user_data: *mut std::ffi::c_void,
    pub type_k: i32,
    pub type_v: i32,
    pub logits_all: bool,
    pub embeddings: bool,
    pub offload_kqv: bool,
    pub abort_callback: Option<extern "C" fn(*mut std::ffi::c_void) -> bool>,
    pub abort_callback_data: *mut std::ffi::c_void,
}

// Sampling parameters
#[repr(C)]
pub struct LlamaSamplingParams {
    pub n_prev: i32,
    pub n_probs: i32,
    pub top_k: i32,
    pub top_p: c_float,
    pub min_p: c_float,
    pub tfs_z: c_float,
    pub typical_p: c_float,
    pub temp: c_float,
    pub dynatemp_range: c_float,
    pub dynatemp_exponent: c_float,
    pub penalty_last_n: i32,
    pub penalty_repeat: c_float,
    pub penalty_freq: c_float,
    pub penalty_present: c_float,
    pub mirostat: i32,
    pub mirostat_tau: c_float,
    pub mirostat_eta: c_float,
    pub penalize_nl: bool,
    pub seed: u32,
}

// Raw C functions (unsafe, wrapped in safe layer below)
#[link(name = "llama")]
extern "C" {
    // Model loading
    pub fn llama_model_default_params() -> LlamaModelParams;
    pub fn llama_model_load_from_file(
        fname: *const c_char,
        params: LlamaModelParams,
    ) -> *mut LlamaModel;
    pub fn llama_model_free(model: *mut LlamaModel);

    // Context creation
    pub fn llama_context_default_params() -> LlamaContextParams;
    pub fn llama_new_context_with_model(
        model: *mut LlamaModel,
        params: LlamaContextParams,
    ) -> *mut LlamaContext;
    pub fn llama_free(ctx: *mut LlamaContext);

    // Tokenization
    pub fn llama_tokenize(
        model: *mut LlamaModel,
        text: *const c_char,
        tokens: *mut LlamaToken,
        n_max_tokens: c_int,
        add_bos: bool,
    ) -> c_int;

    pub fn llama_token_to_piece(
        model: *mut LlamaModel,
        token: LlamaToken,
        buf: *mut c_char,
        length: c_int,
    ) -> c_int;

    // Inference
    pub fn llama_batch_init(n_tokens: i32, embd: i32, n_seq_max: i32) -> LlamaBatch;
    pub fn llama_batch_free(batch: LlamaBatch);

    pub fn llama_batch_add(
        batch: *mut LlamaBatch,
        id: LlamaToken,
        pos: i32,
        seq_ids: *mut i32,
        n_seq_id: i32,
        logits: bool,
    );

    pub fn llama_decode(ctx: *mut LlamaContext, batch: *mut LlamaBatch) -> c_int;
    pub fn llama_batch_clear(batch: *mut LlamaBatch);

    // KV Cache management
    pub fn llama_kv_cache_seq_rm(ctx: *mut LlamaContext, seq_id: i32, p0: i32, p1: i32);
    pub fn llama_kv_cache_seq_cp(ctx: *mut LlamaContext, seq_id_src: i32, seq_id_dst: i32, p0: i32, p1: i32);
    pub fn llama_kv_cache_seq_keep(ctx: *mut LlamaContext, seq_id: i32);

    // Sampling
    pub fn llama_sampling_init(params: *const LlamaSamplingParams) -> *mut std::ffi::c_void;
    pub fn llama_sampling_free(state: *mut std::ffi::c_void);
    pub fn llama_sampling_sample(
        ctx: *mut LlamaContext,
        state: *mut std::ffi::c_void,
        idx: i32,
    ) -> LlamaToken;
    pub fn llama_sampling_accept(
        ctx: *mut LlamaContext,
        state: *mut std::ffi::c_void,
        id: LlamaToken,
        apply_grammar: bool,
    );

    // Model info
    pub fn llama_model_n_vocab(model: *mut LlamaModel) -> i32;
    pub fn llama_n_ctx(ctx: *mut LlamaContext) -> i32;

    // Utility
    pub fn llama_print_system_info();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_struct_sizes() {
        // Verify struct sizes match C
        assert!(std::mem::size_of::<LlamaModelParams>() > 0);
        assert!(std::mem::size_of::<LlamaContextParams>() > 0);
        assert!(std::mem::size_of::<LlamaSamplingParams>() > 0);
    }
}
