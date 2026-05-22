/// HTTP credential extraction from requests

use actix_web::HttpRequest;
use uuid::Uuid;

/// Extract client IP from request
pub fn extract_client_ip(req: &HttpRequest) -> String {
    req.connection_info()
        .peer_addr()
        .unwrap_or("unknown")
        .to_string()
}

/// Extract or generate request ID
pub fn extract_request_id(req: &HttpRequest) -> String {
    if let Some(req_id) = req.headers().get("x-request-id") {
        if let Ok(id_str) = req_id.to_str() {
            return id_str.to_string();
        }
    }
    Uuid::new_v4().to_string()
}

/// Credential wrapper
#[derive(Clone, Debug)]
pub struct Credential {
    pub client_id: String,
    pub request_id: String,
    pub api_key: Option<String>,
}

impl Credential {
    pub fn from_request(req: &HttpRequest) -> Self {
        Self {
            client_id: extract_client_ip(req),
            request_id: extract_request_id(req),
            api_key: req
                .headers()
                .get("x-api-key")
                .and_then(|h| h.to_str().ok())
                .map(|s| s.to_string()),
        }
    }
}
