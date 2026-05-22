/// API key management - Minimal implementation

use actix_web::HttpResponse;
use serde::Serialize;

#[derive(Serialize)]
pub struct ApiKeyInfo {
    pub key: String,
}

pub async fn get_api_keys() -> HttpResponse {
    HttpResponse::Ok().json(serde_json::json!({"keys": []}))
}
