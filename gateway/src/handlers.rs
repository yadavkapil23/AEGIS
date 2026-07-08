use actix_web::{web, HttpResponse, get, post};
use serde::{Deserialize, Serialize};
use tracing::{info, error};

use crate::middleware::GatewayState;

// ── Request / Response types ──────────────────────────────────

#[derive(Debug, Deserialize)]
pub struct AllocateRequestBody {
    pub request_id: String,
    pub num_blocks: u32,
    pub owner: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct DeallocateRequestBody {
    pub request_id: String,
    pub block_ids: Vec<u64>,
}

#[derive(Debug, Serialize)]
pub struct AllocateResponse {
    pub success: bool,
    pub block_ids: Vec<u64>,
    pub latency_ms: u32,
    pub node_id: String,
    pub error: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct DeallocateResponse {
    pub success: bool,
    pub count: u32,
    pub latency_ms: u32,
    pub error: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct StatsResponse {
    pub total_blocks: u64,
    pub allocated_blocks: u64,
    pub free_blocks: u32,
}

#[derive(Debug, Serialize)]
pub struct ClusterHealthResponse {
    pub healthy: bool,
    pub total_nodes: u32,
    pub healthy_nodes: u32,
    pub leader_id: String,
}

// ── Handlers ──────────────────────────────────────────────────

/// GET /health — Deep health check across all subsystems.
#[get("/health")]
pub async fn health_check(state: web::Data<GatewayState>) -> HttpResponse {
    let health = state.health_status().await;
    let status_code = if health["status"] == "healthy" {
        actix_web::http::StatusCode::OK
    } else {
        actix_web::http::StatusCode::SERVICE_UNAVAILABLE
    };
    HttpResponse::build(status_code).json(health)
}

/// GET /ready — Readiness probe: at least one backend + DB + scheduler.
#[get("/ready")]
pub async fn readiness_check(state: web::Data<GatewayState>) -> HttpResponse {
    let health = state.health_status().await;
    let ready = health["status"] == "healthy";
    let code = if ready {
        actix_web::http::StatusCode::OK
    } else {
        actix_web::http::StatusCode::SERVICE_UNAVAILABLE
    };
    HttpResponse::build(code).json(serde_json::json!({
        "ready": ready,
        "subsystems": health,
    }))
}

/// POST /v1/allocate — Allocate KV-cache blocks via scheduler gRPC.
#[post("/v1/allocate")]
pub async fn allocate(
    state: web::Data<GatewayState>,
    body: web::Json<AllocateRequestBody>,
) -> HttpResponse {
    let start = std::time::Instant::now();

    match state
        .allocation_client
        .allocate_blocks(
            body.request_id.clone(),
            body.num_blocks,
            body.owner.clone(),
        )
        .await
    {
        Ok((block_ids, latency_ms, node_id)) => {
            info!(
                request_id = %body.request_id,
                blocks = block_ids.len(),
                node = %node_id,
                "Allocation succeeded"
            );
            HttpResponse::Ok().json(AllocateResponse {
                success: true,
                block_ids,
                latency_ms,
                node_id,
                error: None,
            })
        }
        Err(e) => {
            error!(
                request_id = %body.request_id,
                error = %e,
                "Allocation failed"
            );
            HttpResponse::InternalServerError().json(AllocateResponse {
                success: false,
                block_ids: vec![],
                latency_ms: start.elapsed().as_millis() as u32,
                node_id: String::new(),
                error: Some(e.to_string()),
            })
        }
    }
}

/// POST /v1/deallocate — Release KV-cache blocks via scheduler gRPC.
#[post("/v1/deallocate")]
pub async fn deallocate(
    state: web::Data<GatewayState>,
    body: web::Json<DeallocateRequestBody>,
) -> HttpResponse {
    let start = std::time::Instant::now();

    match state
        .allocation_client
        .deallocate_blocks(body.request_id.clone(), body.block_ids.clone())
        .await
    {
        Ok((count, latency_ms, _node_id)) => {
            info!(
                request_id = %body.request_id,
                count = count,
                "Deallocation succeeded"
            );
            HttpResponse::Ok().json(DeallocateResponse {
                success: true,
                count,
                latency_ms,
                error: None,
            })
        }
        Err(e) => {
            error!(
                request_id = %body.request_id,
                error = %e,
                "Deallocation failed"
            );
            HttpResponse::InternalServerError().json(DeallocateResponse {
                success: false,
                count: 0,
                latency_ms: start.elapsed().as_millis() as u32,
                error: Some(e.to_string()),
            })
        }
    }
}

/// GET /v1/stats — Cache statistics from the scheduler.
#[get("/v1/stats")]
pub async fn get_stats(state: web::Data<GatewayState>) -> HttpResponse {
    match state.allocation_client.get_stats().await {
        Ok((total, allocated, free)) => {
            HttpResponse::Ok().json(StatsResponse {
                total_blocks: total,
                allocated_blocks: allocated,
                free_blocks: free,
            })
        }
        Err(e) => {
            error!(error = %e, "Failed to get stats");
            HttpResponse::InternalServerError().json(serde_json::json!({
                "error": e.to_string(),
            }))
        }
    }
}

/// GET /v1/cluster — Cluster health from the scheduler.
#[get("/v1/cluster")]
pub async fn get_cluster_health(state: web::Data<GatewayState>) -> HttpResponse {
    match state.allocation_client.get_cluster_health().await {
        Ok((healthy, total, healthy_nodes, leader_id)) => {
            HttpResponse::Ok().json(ClusterHealthResponse {
                healthy,
                total_nodes: total,
                healthy_nodes,
                leader_id,
            })
        }
        Err(e) => {
            error!(error = %e, "Failed to get cluster health");
            HttpResponse::InternalServerError().json(serde_json::json!({
                "error": e.to_string(),
            }))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn allocate_request_deserializes() {
        let body = r#"{"request_id":"req-1","num_blocks":10}"#;
        let req: AllocateRequestBody = serde_json::from_str(body).unwrap();
        assert_eq!(req.request_id, "req-1");
        assert_eq!(req.num_blocks, 10);
        assert!(req.owner.is_none());
    }

    #[test]
    fn deallocate_request_deserializes() {
        let body = r#"{"request_id":"req-1","block_ids":[1,2,3]}"#;
        let req: DeallocateRequestBody = serde_json::from_str(body).unwrap();
        assert_eq!(req.block_ids, vec![1, 2, 3]);
    }
}
