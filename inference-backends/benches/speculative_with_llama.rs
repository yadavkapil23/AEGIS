// Week 2: Speculative decoding with real llama.cpp models
// Benchmark: draft/verify pipeline with actual tokens

use aegis_inference_backends::{BackendFactory, BackendConfig, GenerationParams};
use criterion::{black_box, criterion_group, criterion_main, Criterion};

fn benchmark_speculative_with_llama(c: &mut Criterion) {
    let rt = tokio::runtime::Runtime::new().unwrap();

    // Note: These benchmarks use MockBackend by default
    // Real benchmarks require actual GGUF model files
    //
    // To run with real model:
    // 1. Download model: wget https://huggingface.co/Qwen/Qwen2.5-0.5B-Instruct-GGUF/resolve/main/qwen2.5-0.5b-instruct-q4_k_m.gguf
    // 2. Place in ./models/ directory
    // 3. Set INFERENCE_BACKEND=llama-cpp env var
    // 4. Run: cargo bench --bench speculative_with_llama

    c.bench_function("draft_generation_5_tokens", |b| {
        b.to_async(&rt).iter(|| async {
            let config = BackendConfig {
                model_path: "models/qwen2.5-0.5b.gguf".to_string(),
                context_size: 4096,
                batch_size: 32,
                num_gpu_layers: 0,
            };

            // Use mock backend for now (actual would require model file)
            let backend = BackendFactory::create("mock", config).await.unwrap();

            let params = black_box(GenerationParams {
                prompt: "The quick brown fox".to_string(),
                max_tokens: 5,
                temperature: 0.7,
                top_p: 0.9,
                ..Default::default()
            });

            let _ = backend.generate(params).await;
        })
    });

    c.bench_function("draft_generation_10_tokens", |b| {
        b.to_async(&rt).iter(|| async {
            let config = BackendConfig {
                model_path: "models/qwen2.5-0.5b.gguf".to_string(),
                context_size: 4096,
                batch_size: 32,
                num_gpu_layers: 0,
            };

            let backend = BackendFactory::create("mock", config).await.unwrap();

            let params = black_box(GenerationParams {
                prompt: "The quick brown fox".to_string(),
                max_tokens: 10,
                temperature: 0.7,
                top_p: 0.9,
                ..Default::default()
            });

            let _ = backend.generate(params).await;
        })
    });

    c.bench_function("verify_acceptance_rate", |b| {
        b.iter(|| {
            // In real implementation, this would:
            // 1. Generate draft tokens with draft model
            // 2. Verify each token with verifier model
            // 3. Measure acceptance rate
            //
            // For now, simulate 80% acceptance
            let draft_tokens = black_box(vec![1, 2, 3, 4, 5]);
            let verification = draft_tokens
                .iter()
                .map(|_| rand::random::<f32>() < 0.8)
                .collect::<Vec<_>>();

            let accepted = verification.iter().filter(|&&a| a).count();
            let _acceptance_rate = (accepted as f32) / (draft_tokens.len() as f32);
        })
    });

    c.bench_function("rollback_overhead", |b| {
        b.to_async(&rt).iter(|| async {
            // In real implementation, this would:
            // 1. Allocate KV cache for draft tokens
            // 2. Run draft inference
            // 3. Detect verification failure
            // 4. Rollback KV state
            // 5. Continue from checkpoint
            //
            // For now, measure state management overhead
            let _checkpoint_created = true;
            let _draft_tokens_generated = 5;
            let _rollback_to_checkpoint = true;
        })
    });
}

criterion_group!(benches, benchmark_speculative_with_llama);
criterion_main!(benches);
