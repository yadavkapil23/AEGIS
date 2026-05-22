//! HTTP credential extraction from headers

use axum::{
    http::{Request, HeaderMap},
    body::Body,
};
use security::{Credential, SecurityError};

/// Extract credential from HTTP request headers
///
/// Supports:
/// - Authorization: Bearer <token> (JWT)
/// - Authorization: ApiKey <key> (API key)
/// - X-API-Key: <key> (API key)
pub fn extract_credential(headers: &HeaderMap) -> Result<Credential, SecurityError> {
    // Check Authorization header
    if let Some(auth_header) = headers.get("authorization") {
        let auth_str = auth_header
            .to_str()
            .map_err(|_| SecurityError::InvalidToken("Invalid authorization header".to_string()))?;

        // Check for Bearer token (JWT)
        if let Some(token) = auth_str.strip_prefix("Bearer ") {
            return Ok(Credential::Bearer(token.to_string()));
        }

        // Check for ApiKey
        if let Some(key) = auth_str.strip_prefix("ApiKey ") {
            return Ok(Credential::ApiKey(key.to_string()));
        }

        return Err(SecurityError::InvalidToken(
            "Authorization header format: 'Bearer <token>' or 'ApiKey <key>'".to_string(),
        ));
    }

    // Check X-API-Key header (convenience for API key)
    if let Some(key_header) = headers.get("x-api-key") {
        let key = key_header
            .to_str()
            .map_err(|_| SecurityError::InvalidApiKey("Invalid X-API-Key header".to_string()))?
            .to_string();

        return Ok(Credential::ApiKey(key));
    }

    // No credential found
    Err(SecurityError::AuthenticationRequired)
}

/// Extract client IP address from request headers
///
/// Checks in order:
/// 1. X-Forwarded-For header (proxy/load balancer)
/// 2. X-Real-IP header (nginx)
/// 3. CF-Connecting-IP header (Cloudflare)
/// 4. Unknown if not found
pub fn extract_client_ip(headers: &HeaderMap) -> String {
    // Check X-Forwarded-For (proxy chain, take first)
    if let Some(forwarded) = headers.get("x-forwarded-for") {
        if let Ok(forwarded_str) = forwarded.to_str() {
            return forwarded_str
                .split(',')
                .next()
                .unwrap_or("unknown")
                .trim()
                .to_string();
        }
    }

    // Check X-Real-IP (nginx)
    if let Some(real_ip) = headers.get("x-real-ip") {
        if let Ok(ip_str) = real_ip.to_str() {
            return ip_str.to_string();
        }
    }

    // Check CF-Connecting-IP (Cloudflare)
    if let Some(cf_ip) = headers.get("cf-connecting-ip") {
        if let Ok(ip_str) = cf_ip.to_str() {
            return ip_str.to_string();
        }
    }

    "unknown".to_string()
}

/// Extract request ID for tracing
pub fn extract_request_id(headers: &HeaderMap) -> String {
    // Check for existing request ID (passed through from client/proxy)
    if let Some(req_id) = headers.get("x-request-id") {
        if let Ok(id_str) = req_id.to_str() {
            return id_str.to_string();
        }
    }

    // Generate new request ID
    uuid::Uuid::new_v4().to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::http::HeaderMap;

    #[test]
    fn test_extract_bearer_token() {
        let mut headers = HeaderMap::new();
        headers.insert(
            "authorization",
            "Bearer eyJhbGciOiJIUzI1NiIsInR5cCI6IkpXVCJ9".parse().unwrap(),
        );

        let credential = extract_credential(&headers).unwrap();
        match credential {
            Credential::Bearer(token) => assert_eq!(token, "eyJhbGciOiJIUzI1NiIsInR5cCI6IkpXVCJ9"),
            _ => panic!("Expected Bearer credential"),
        }
    }

    #[test]
    fn test_extract_api_key_from_auth_header() {
        let mut headers = HeaderMap::new();
        headers.insert("authorization", "ApiKey sk-test123".parse().unwrap());

        let credential = extract_credential(&headers).unwrap();
        match credential {
            Credential::ApiKey(key) => assert_eq!(key, "sk-test123"),
            _ => panic!("Expected ApiKey credential"),
        }
    }

    #[test]
    fn test_extract_api_key_from_header() {
        let mut headers = HeaderMap::new();
        headers.insert("x-api-key", "sk-test456".parse().unwrap());

        let credential = extract_credential(&headers).unwrap();
        match credential {
            Credential::ApiKey(key) => assert_eq!(key, "sk-test456"),
            _ => panic!("Expected ApiKey credential"),
        }
    }

    #[test]
    fn test_no_credential() {
        let headers = HeaderMap::new();
        let result = extract_credential(&headers);
        assert!(matches!(result, Err(SecurityError::AuthenticationRequired)));
    }

    #[test]
    fn test_extract_client_ip_x_forwarded_for() {
        let mut headers = HeaderMap::new();
        headers.insert("x-forwarded-for", "192.168.1.100, 10.0.0.1".parse().unwrap());

        let ip = extract_client_ip(&headers);
        assert_eq!(ip, "192.168.1.100");
    }

    #[test]
    fn test_extract_client_ip_x_real_ip() {
        let mut headers = HeaderMap::new();
        headers.insert("x-real-ip", "203.0.113.45".parse().unwrap());

        let ip = extract_client_ip(&headers);
        assert_eq!(ip, "203.0.113.45");
    }

    #[test]
    fn test_extract_request_id() {
        let mut headers = HeaderMap::new();
        headers.insert("x-request-id", "req-abc123".parse().unwrap());

        let id = extract_request_id(&headers);
        assert_eq!(id, "req-abc123");
    }

    #[test]
    fn test_extract_request_id_generates_new() {
        let headers = HeaderMap::new();
        let id = extract_request_id(&headers);
        assert!(!id.is_empty());
        assert_ne!(id, extract_request_id(&headers)); // Different each time
    }
}
