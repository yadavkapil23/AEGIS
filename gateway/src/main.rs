// AEGIS Gateway - Main entry point

use aegis_gateway::{GatewayServer, GatewayConfig};
use anyhow::Result;
use tracing::info;
use tracing_subscriber;

#[tokio::main]
async fn main() -> Result<()> {
    // Initialize tracing/logging
    tracing_subscriber::fmt()
        .with_max_level(tracing::Level::INFO)
        .init();

    info!("Starting AEGIS Gateway");

    // Create gateway with default config
    let config = GatewayConfig::default();
    let gateway = GatewayServer::new(config);

    // Run the gateway
    gateway.run().await?;

    Ok(())
}
