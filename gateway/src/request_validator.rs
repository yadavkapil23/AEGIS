use actix_web::HttpResponse;
use serde::{Deserialize, Serialize};

/// Structured validation error returned to the client.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ValidationError {
    pub error: String,
    pub error_code: String,
    pub field: Option<String>,
}

impl ValidationError {
    fn new(error: impl Into<String>, error_code: impl Into<String>, field: impl Into<String>) -> Self {
        Self {
            error: error.into(),
            error_code: error_code.into(),
            field: Some(field.into()),
        }
    }

    /// Convert to an Actix JSON response.
    pub fn into_response(self) -> HttpResponse {
        HttpResponse::BadRequest().json(self)
    }
}

/// Inference request fields to validate.
#[derive(Debug, Clone, Deserialize)]
pub struct InferenceRequest {
    pub model: String,
    pub prompt: String,
    pub max_tokens: u32,
    #[serde(default)]
    pub temperature: Option<f32>,
    #[serde(default)]
    pub top_p: Option<f32>,
}

/// Validated and sanitised request ready for inference.
#[derive(Debug, Clone)]
pub struct ValidatedRequest {
    pub model: String,
    pub prompt: String,
    pub max_tokens: u32,
    pub temperature: Option<f32>,
    pub top_p: Option<f32>,
}

/// Validate an inference request.  Returns the sanitised form on success
/// or the first encountered `ValidationError`.
pub fn validate_request(req: &InferenceRequest) -> Result<ValidatedRequest, ValidationError> {
    // ── Model name ──────────────────────────────────────────────
    if req.model.is_empty() {
        return Err(ValidationError::new(
            "model cannot be empty",
            "empty_model",
            "model",
        ));
    }
    if req.model.len() > 256 {
        return Err(ValidationError::new(
            "model name too long (max 256 characters)",
            "model_too_long",
            "model",
        ));
    }
    if !req.model.chars().all(|c| c.is_alphanumeric() || c == '-' || c == '_' || c == ':' || c == '.') {
        return Err(ValidationError::new(
            "model name contains invalid characters (alphanumeric, '-', '_', ':', '.' allowed)",
            "invalid_model_name",
            "model",
        ));
    }

    // ── Prompt ──────────────────────────────────────────────────
    if req.prompt.is_empty() {
        return Err(ValidationError::new(
            "prompt cannot be empty",
            "empty_prompt",
            "prompt",
        ));
    }
    if req.prompt.len() > 100_000 {
        return Err(ValidationError::new(
            "prompt is too long (max 100,000 characters)",
            "prompt_too_long",
            "prompt",
        ));
    }

    // ── max_tokens ──────────────────────────────────────────────
    if req.max_tokens == 0 || req.max_tokens > 32_000 {
        return Err(ValidationError::new(
            "max_tokens must be between 1 and 32000",
            "invalid_max_tokens",
            "max_tokens",
        ));
    }

    // ── temperature (optional) ──────────────────────────────────
    if let Some(temp) = req.temperature {
        if !(0.0..=2.0).contains(&temp) {
            return Err(ValidationError::new(
                "temperature must be between 0.0 and 2.0",
                "invalid_temperature",
                "temperature",
            ));
        }
    }

    // ── top_p (optional) ────────────────────────────────────────
    if let Some(top_p) = req.top_p {
        if !(0.0..=1.0).contains(&top_p) {
            return Err(ValidationError::new(
                "top_p must be between 0.0 and 1.0",
                "invalid_top_p",
                "top_p",
            ));
        }
    }

    Ok(ValidatedRequest {
        model: req.model.clone(),
        prompt: req.prompt.clone(),
        max_tokens: req.max_tokens,
        temperature: req.temperature,
        top_p: req.top_p,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn valid_req() -> InferenceRequest {
        InferenceRequest {
            model: "qwen2.5:0.5b".into(),
            prompt: "Hello world".into(),
            max_tokens: 100,
            temperature: Some(0.7),
            top_p: Some(0.9),
        }
    }

    #[test]
    fn valid_request_passes() {
        assert!(validate_request(&valid_req()).is_ok());
    }

    #[test]
    fn empty_model_fails() {
        let mut r = valid_req();
        r.model = "".into();
        let err = validate_request(&r).unwrap_err();
        assert_eq!(err.error_code, "empty_model");
    }

    #[test]
    fn invalid_model_chars_fails() {
        let mut r = valid_req();
        r.model = "model/with/slashes".into();
        assert_eq!(validate_request(&r).unwrap_err().error_code, "invalid_model_name");
    }

    #[test]
    fn empty_prompt_fails() {
        let mut r = valid_req();
        r.prompt = "".into();
        assert_eq!(validate_request(&r).unwrap_err().error_code, "empty_prompt");
    }

    #[test]
    fn prompt_too_long_fails() {
        let mut r = valid_req();
        r.prompt = "x".repeat(100_001);
        assert_eq!(validate_request(&r).unwrap_err().error_code, "prompt_too_long");
    }

    #[test]
    fn zero_max_tokens_fails() {
        let mut r = valid_req();
        r.max_tokens = 0;
        assert_eq!(validate_request(&r).unwrap_err().error_code, "invalid_max_tokens");
    }

    #[test]
    fn high_max_tokens_fails() {
        let mut r = valid_req();
        r.max_tokens = 50_000;
        assert_eq!(validate_request(&r).unwrap_err().error_code, "invalid_max_tokens");
    }

    #[test]
    fn temperature_out_of_range_fails() {
        let mut r = valid_req();
        r.temperature = Some(3.0);
        assert_eq!(validate_request(&r).unwrap_err().error_code, "invalid_temperature");
    }

    #[test]
    fn top_p_out_of_range_fails() {
        let mut r = valid_req();
        r.top_p = Some(1.5);
        assert_eq!(validate_request(&r).unwrap_err().error_code, "invalid_top_p");
    }

    #[test]
    fn optional_fields_none_are_ok() {
        let mut r = valid_req();
        r.temperature = None;
        r.top_p = None;
        assert!(validate_request(&r).is_ok());
    }
}
