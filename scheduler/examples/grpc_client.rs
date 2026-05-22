// gRPC Client Example
// Shows how to use the Allocation Service

use tonic::transport::Channel;

mod allocation {
    include!(concat!(env!("OUT_DIR"), "/scheduler.allocation.rs"));
}

use allocation::allocation_service_client::AllocationServiceClient;
use allocation::{AllocateRequest, DeallocateRequest, StatsRequest, HealthRequest};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Connect to gRPC server
    let channel = Channel::from_static("http://[::1]:50052")
        .connect()
        .await?;
    
    let mut client = AllocationServiceClient::new(channel);

    // Example 1: Get cluster health
    println!("=== Cluster Health ===");
    let health_request = tonic::Request::new(HealthRequest {});
    match client.get_cluster_health(health_request).await {
        Ok(response) => {
            let health = response.into_inner();
            println!("Healthy: {}", health.healthy);
            println!("Total nodes: {}", health.total_nodes);
            println!("Healthy nodes: {}", health.healthy_nodes);
            println!("Leader: {}", health.leader_id);
        }
        Err(e) => println!("Error: {}", e),
    }

    // Example 2: Get cache stats
    println!("\n=== Cache Statistics ===");
    let stats_request = tonic::Request::new(StatsRequest {});
    match client.get_stats(stats_request).await {
        Ok(response) => {
            let stats = response.into_inner();
            println!("Total blocks: {}", stats.total_blocks);
            println!("Allocated blocks: {}", stats.allocated_blocks);
            println!("Free blocks: {}", stats.free_blocks);
            println!("Utilization: {}%", stats.utilization_percent);
            println!("Total allocations: {}", stats.total_allocations);
            println!("Total deallocations: {}", stats.total_deallocations);
        }
        Err(e) => println!("Error: {}", e),
    }

    // Example 3: Allocate blocks
    println!("\n=== Allocate Blocks ===");
    let allocate_request = tonic::Request::new(AllocateRequest {
        request_id: "req-1".to_string(),
        num_blocks: 5,
        owner: Some("client-1".to_string()),
        priority: 5,
    });
    match client.allocate_blocks(allocate_request).await {
        Ok(response) => {
            let result = response.into_inner();
            println!("Success: {}", result.success);
            println!("Blocks: {:?}", result.block_ids);
            println!("Latency: {}ms", result.latency_ms);
            println!("Node: {}", result.node_id);

            // Example 4: Deallocate blocks
            println!("\n=== Deallocate Blocks ===");
            let deallocate_request = tonic::Request::new(DeallocateRequest {
                request_id: "req-2".to_string(),
                block_ids: result.block_ids,
            });
            match client.deallocate_blocks(deallocate_request).await {
                Ok(response) => {
                    let result = response.into_inner();
                    println!("Success: {}", result.success);
                    println!("Count: {}", result.count);
                    println!("Latency: {}ms", result.latency_ms);
                }
                Err(e) => println!("Error: {}", e),
            }
        }
        Err(e) => println!("Error: {}", e),
    }

    Ok(())
}
