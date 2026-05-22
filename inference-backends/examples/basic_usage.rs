//! Basic usage example for dual-backend inference

use inference_backends::prelude::*;

#[tokio::main]
async fn main() -> Result<()> {
    // Initialize logging
    tracing_subscriber::fmt::init();

    // Load configuration from file
    let config = BackendConfig::default();

    println!("Creating backend router...");
    match BackendRouter::new(config).await {
        Ok(router) => {
            println!("✓ Router initialized");

            println!("Warming up backends...");
            let _ = router.warmup().await;

            // Create a request
            let request = InferenceRequest::new(
                "mistralai/Mistral-7B-Instruct-v0.2",
                "What is the capital of France?",
            )
            .with_max_tokens(100)
            .with_temperature(0.7)
            .with_backend(BackendPreference::Auto);

            println!("\nExecuting inference request...");
            println!("Model: {}", request.model);
            println!("Prompt: {}", request.prompt);

            match router.infer(request).await {
                Ok(response) => {
                    println!("\n✓ Inference successful!");
                    println!("Response: {}", response.text);
                    println!("Backend: {}", response.backend_used);
                    println!("Latency: {}ms", response.processing_time_ms);
                    println!("Tokens: {}", response.tokens_generated);
                }
                Err(e) => {
                    eprintln!("✗ Inference failed: {}", e);
                }
            }

            // Check backend health
            println!("\nChecking backend health...");
            match router.health_check().await {
                Ok(statuses) => {
                    for status in statuses {
                        println!(
                            "  {}: {} (latency: {:.2}ms)",
                            status.backend,
                            if status.healthy {
                                "✓ healthy"
                            } else {
                                "✗ unhealthy"
                            },
                            status.latency_ms
                        );
                    }
                }
                Err(e) => {
                    eprintln!("Health check failed: {}", e);
                }
            }
        }
        Err(e) => {
            eprintln!("✗ Failed to initialize router: {}", e);
        }
    }

    Ok(())
}
