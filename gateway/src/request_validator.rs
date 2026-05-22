/// Request Validation - Simplified

use actix_web::HttpResponse;
use serde::Deserialize;

#[derive(Deserialize, Clone, Debug)]
pub struct ValidatedRequest {
    pub data: String,
}

pub fn validate_request(_data: &str) -> Result<(), String> {
    Ok(())
}
