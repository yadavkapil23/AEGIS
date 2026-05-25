// Speculative decoding benchmark

use aegis_speculative::{SpeculativeCoordinator, SpeculativeMetrics};
use criterion::{black_box, criterion_group, criterion_main, Criterion};
use std::sync::Arc;

fn benchmark_speculative_decoding(c: &mut Criterion) {
    c.bench_function("spec_generate_draft_tokens", |b| {
        let metrics = Arc::new(SpeculativeMetrics::new());
        let coord = SpeculativeCoordinator::new(16, metrics);
        coord.create_branch("req-1").unwrap();

        b.iter(|| {
            let _ = coord.generate_draft("req-1", "Test prompt", black_box(5));
        })
    });

    c.bench_function("spec_verify_tokens", |b| {
        let metrics = Arc::new(SpeculativeMetrics::new());
        let coord = SpeculativeCoordinator::new(16, metrics);
        coord.create_branch("req-1").unwrap();

        let draft = coord.generate_draft("req-1", "Test prompt", 5).unwrap_or_default();

        b.iter(|| {
            let _ = coord.verify("req-1", "Test prompt", black_box(&draft));
        })
    });

    c.bench_function("spec_rollback", |b| {
        let metrics = Arc::new(SpeculativeMetrics::new());
        let coord = SpeculativeCoordinator::new(16, metrics);

        b.iter(|| {
            coord.create_branch("req-bench").ok();
            coord.generate_draft("req-bench", 5).ok();
            let _ = coord.rollback("req-bench", black_box(2));
        })
    });

    c.bench_function("spec_adaptation", |b| {
        let metrics = Arc::new(SpeculativeMetrics::new());
        let coord = SpeculativeCoordinator::new(16, metrics);

        b.iter(|| {
            for i in 0..100 {
                let req_id = format!("req-{}", i);
                coord.create_branch(&req_id).ok();
                let draft = coord.generate_draft(&req_id, "Test prompt", 5).unwrap_or_default();
                coord.verify(&req_id, "Test prompt", &draft).ok();
            }
        })
    });
}

criterion_group!(benches, benchmark_speculative_decoding);
criterion_main!(benches);
