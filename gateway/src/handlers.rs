// HTTP request handlers for the gateway

use crate::{
    GatewayState, AllocationRequest, AllocationResponse, DeallocationRequest,
    DeallocationResponse, CacheStats, ClusterHealth,
};
use actix_web::{web, HttpResponse};
use uuid::Uuid;
use serde_json::json;
use tracing::{info, debug, error};

/// Health check endpoint
pub async fn health_check() -> HttpResponse {
    HttpResponse::Ok().json(json!({"status": "healthy"}))
}

/// Readiness check endpoint
pub async fn readiness_check(state: web::Data<GatewayState>) -> HttpResponse {
    match state.client.get_cluster_health().await {
        Ok((healthy, _, _, _)) if healthy => {
            HttpResponse::Ok().json(json!({"ready": true}))
        }
        _ => HttpResponse::ServiceUnavailable()
            .json(json!({"ready": false, "error": "Cluster not healthy"})),
    }
}

/// Allocate blocks
pub async fn allocate(
    state: web::Data<GatewayState>,
    mut req: web::Json<AllocationRequest>,
) -> HttpResponse {
    // Generate request ID if not provided
    if req.request_id.is_empty() {
        req.request_id = Uuid::new_v4().to_string();
    }

    debug!(request_id = req.request_id, "Allocate request received");

    // Check cache first
    if let Some(cached) = state.cache.get(&req.request_id) {
        info!(request_id = req.request_id, "Returning cached allocation");
        return HttpResponse::Ok().json(cached);
    }

    // Allocate blocks via gRPC
    match state
        .client
        .allocate_blocks(
            req.request_id.clone(),
            req.num_blocks,
            req.owner.clone(),
        )
        .await
    {
        Ok((block_ids, latency_ms, node_id)) => {
            let response = AllocationResponse {
                request_id: req.request_id.clone(),
                success: true,
                block_ids,
                error: None,
                latency_ms,
                node_id,
            };

            // Cache response
            state.cache.put(req.request_id.clone(), response.clone());

            info!(
                request_id = req.request_id,
                blocks_count = response.block_ids.len(),
                "Allocation successful"
            );

            HttpResponse::Ok().json(response)
        }
        Err(e) => {
            error!(
                request_id = req.request_id,
                error = %e,
                "Allocation failed"
            );

            let response = AllocationResponse {
                request_id: req.request_id,
                success: false,
                block_ids: vec![],
                error: Some(e.to_string()),
                latency_ms: 0,
                node_id: String::new(),
            };

            HttpResponse::InternalServerError().json(response)
        }
    }
}

/// Deallocate blocks
pub async fn deallocate(
    state: web::Data<GatewayState>,
    mut req: web::Json<DeallocationRequest>,
) -> HttpResponse {
    // Generate request ID if not provided
    if req.request_id.is_empty() {
        req.request_id = Uuid::new_v4().to_string();
    }

    debug!(
        request_id = req.request_id,
        block_count = req.block_ids.len(),
        "Deallocate request received"
    );

    // Deallocate blocks via gRPC
    match state
        .client
        .deallocate_blocks(req.request_id.clone(), req.block_ids.clone())
        .await
    {
        Ok((count, latency_ms, node_id)) => {
            let response = DeallocationResponse {
                request_id: req.request_id.clone(),
                success: true,
                count,
                error: None,
                latency_ms,
            };

            info!(
                request_id = req.request_id,
                blocks_deallocated = count,
                "Deallocation successful"
            );

            HttpResponse::Ok().json(response)
        }
        Err(e) => {
            error!(
                request_id = req.request_id,
                error = %e,
                "Deallocation failed"
            );

            let response = DeallocationResponse {
                request_id: req.request_id,
                success: false,
                count: 0,
                error: Some(e.to_string()),
                latency_ms: 0,
            };

            HttpResponse::InternalServerError().json(response)
        }
    }
}

/// Get cache statistics
pub async fn get_stats(state: web::Data<GatewayState>) -> HttpResponse {
    match state.client.get_stats().await {
        Ok((total_blocks, allocated_blocks, utilization)) => {
            let stats = CacheStats {
                total_blocks,
                allocated_blocks,
                free_blocks: total_blocks - allocated_blocks,
                utilization_percent: utilization,
                total_allocations: 0,
                total_deallocations: 0,
                avg_latency_ms: 0,
                node_id: "cluster".to_string(),
            };

            HttpResponse::Ok().json(stats)
        }
        Err(e) => {
            error!(error = %e, "Failed to get stats");
            HttpResponse::InternalServerError()
                .json(json!({"error": e.to_string()}))
        }
    }
}

/// Get cluster health
pub async fn get_cluster_health(state: web::Data<GatewayState>) -> HttpResponse {
    match state.client.get_cluster_health().await {
        Ok((healthy, total_nodes, healthy_nodes, leader_id)) => {
            let health = ClusterHealth {
                healthy,
                total_nodes,
                healthy_nodes,
                leader_id,
                quorum_status: if healthy_nodes >= (total_nodes + 1) / 2 {
                    "quorum".to_string()
                } else {
                    "no-quorum".to_string()
                },
            };

            HttpResponse::Ok().json(health)
        }
        Err(e) => {
            error!(error = %e, "Failed to get cluster health");
            HttpResponse::InternalServerError()
                .json(json!({"error": e.to_string()}))
        }
    }
}

/// Prometheus metrics endpoint
pub async fn metrics(state: web::Data<GatewayState>) -> HttpResponse {
    // TODO: Implement proper Prometheus metrics
    let metrics = format!(
        "# HELP gateway_allocations_total Total number of allocations\n\
         # TYPE gateway_allocations_total counter\n\
         gateway_allocations_total 0\n\
         # HELP gateway_allocation_latency_ms Allocation latency in milliseconds\n\
         # TYPE gateway_allocation_latency_ms histogram\n\
         gateway_allocation_latency_ms_bucket{{le=\"10\"}} 0\n\
         gateway_allocation_latency_ms_bucket{{le=\"50\"}} 0\n\
         gateway_allocation_latency_ms_bucket{{le=\"100\"}} 0\n\
         gateway_allocation_latency_ms_bucket{{le=\"+Inf\"}} 0\n"
    );

    HttpResponse::Ok()
        .content_type("text/plain")
        .body(metrics)
}
