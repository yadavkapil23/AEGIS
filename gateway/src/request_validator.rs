/// Request Validation Layer for Actix-web
/// Centralized input validation for inference requests

use actix_web::{error::JsonPayloadError, web, HttpResponse};
use serde::{Deserialize, Serialize};
use tracing::{warn, error};

/// Validation error response
#[derive(Debug, Serialize)]
pub struct ValidationError {
    pub error: String,
    pub field: String,
    pub code: String,
}

/// Validated inference request
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ValidatedInferenceRequest {
    pub model: String,
    pub prompt: String,
    pub max_tokens: u32,
    pub temperature: Option<f32>,
    pub top_p: Option<f32>,
}

/// Input validation constraints
pub struct ValidationRules {
    pub max_prompt_length: usize,
    pub min_prompt_length: usize,
    pub max_tokens_limit: u32,
    pub min_tokens_limit: u32,
    pub max_temperature: f32,
    pub min_temperature: f32,
    pub max_top_p: f32,
    pub min_top_p: f32,
}

impl Default for ValidationRules {
    fn default() -> Self {
        Self {
            max_prompt_length: 100_000,
            min_prompt_length: 1,
            max_tokens_limit: 32_000,
            min_tokens_limit: 1,
            max_temperature: 2.0,
            min_temperature: 0.0,
            max_top_p: 1.0,
            min_top_p: 0.0,
        }
    }
}

/// Validator for inference requests
pub struct InferenceValidator {
    rules: ValidationRules,
}

impl InferenceValidator {
    pub fn new(rules: ValidationRules) -> Self {
        Self { rules }
    }

    pub fn default() -> Self {
        Self::new(ValidationRules::default())
    }

    /// Validate model name
    fn validate_model(&self, model: &str) -> Result<(), ValidationError> {
        if model.is_empty() {
            return Err(ValidationError {
                error: "Model name cannot be empty".to_string(),
                field: "model".to_string(),
                code: "empty_model".to_string(),
            });
        }

        if model.len() > 256 {
            return Err(ValidationError {
                error: "Model name is too long (max 256 characters)".to_string(),
                field: "model".to_string(),
                code: "model_too_long".to_string(),
            });
        }

        // Alphanumeric, hyphens, underscores, slashes, and dots only
        if !model
            .chars()
            .all(|c| c.is_alphanumeric() || c == '-' || c == '_' || c == '/' || c == '.')
        {
            return Err(ValidationError {
                error: "Model name contains invalid characters".to_string(),
                field: "model".to_string(),
                code: "invalid_model_chars".to_string(),
            });
        }

        Ok(())
    }

    /// Validate prompt
    fn validate_prompt(&self, prompt: &str) -> Result<(), ValidationError> {
        if prompt.is_empty() {
            return Err(ValidationError {
                error: "Prompt cannot be empty".to_string(),
                field: "prompt".to_string(),
                code: "empty_prompt".to_string(),
            });
        }

        if prompt.len() < self.rules.min_prompt_length {
            return Err(ValidationError {
                error: format!(
                    "Prompt is too short (min {} characters)",
                    self.rules.min_prompt_length
                ),
                field: "prompt".to_string(),
                code: "prompt_too_short".to_string(),
            });
        }

        if prompt.len() > self.rules.max_prompt_length {
            return Err(ValidationError {
                error: format!(
                    "Prompt is too long (max {} characters)",
                    self.rules.max_prompt_length
                ),
                field: "prompt".to_string(),
                code: "prompt_too_long".to_string(),
            });
        }

        Ok(())
    }

    /// Validate max_tokens
    fn validate_max_tokens(&self, max_tokens: u32) -> Result<(), ValidationError> {
        if max_tokens < self.rules.min_tokens_limit {
            return Err(ValidationError {
                error: format!(
                    "max_tokens must be at least {}",
                    self.rules.min_tokens_limit
                ),
                field: "max_tokens".to_string(),
                code: "tokens_too_low".to_string(),
            });
        }

        if max_tokens > self.rules.max_tokens_limit {
            return Err(ValidationError {
                error: format!(
                    "max_tokens must not exceed {}",
                    self.rules.max_tokens_limit
                ),
                field: "max_tokens".to_string(),
                code: "tokens_too_high".to_string(),
            });
        }

        Ok(())
    }

    /// Validate temperature
    fn validate_temperature(&self, temperature: f32) -> Result<(), ValidationError> {
        if temperature < self.rules.min_temperature || temperature > self.rules.max_temperature {
            return Err(ValidationError {
                error: format!(
                    "Temperature must be between {} and {}",
                    self.rules.min_temperature, self.rules.max_temperature
                ),
                field: "temperature".to_string(),
                code: "invalid_temperature".to_string(),
            });
        }

        Ok(())
    }

    /// Validate top_p
    fn validate_top_p(&self, top_p: f32) -> Result<(), ValidationError> {
        if top_p < self.rules.min_top_p || top_p > self.rules.max_top_p {
            return Err(ValidationError {
                error: format!(
                    "top_p must be between {} and {}",
                    self.rules.min_top_p, self.rules.max_top_p
                ),
                field: "top_p".to_string(),
                code: "invalid_top_p".to_string(),
            });
        }

        Ok(())
    }

    /// Validate complete inference request
    pub fn validate(&self, req: &crate::inference_handler::InferenceRequest) -> Result<(), ValidationError> {
        self.validate_model(&req.model)?;
        self.validate_prompt(&req.prompt)?;
        self.validate_max_tokens(req.max_tokens)?;

        if let Some(temp) = req.temperature {
            self.validate_temperature(temp)?;
        }

        if let Some(top_p) = req.top_p {
            self.validate_top_p(top_p)?;
        }

        Ok(())
    }
}

/// Extractor for validated requests
pub struct ValidatedRequest<T: serde::de::DeserializeOwned>(pub T);

impl<T> actix_web::FromRequest for ValidatedRequest<T>
where
    T: serde::de::DeserializeOwned + 'static,
{
    type Error = actix_web::Error;
    type Future = futures_util::future::Ready<Result<Self, Self::Error>>;

    fn from_request(
        req: &actix_web::HttpRequest,
        payload: &mut actix_web::dev::Payload,
    ) -> Self::Future {
        futures_util::future::ok(ValidatedRequest(Default::default()))
    }
}

/// Input sanitization helpers
pub mod sanitize {
    /// Remove potentially dangerous characters from input
    pub fn sanitize_string(input: &str, allow_newlines: bool) -> String {
        let mut result = String::new();
        for c in input.chars() {
            match c {
                // Allow alphanumeric and common punctuation
                c if c.is_alphanumeric()
                    || c.is_whitespace() && allow_newlines
                    || c == '-'
                    || c == '_'
                    || c == '.'
                    || c == ','
                    || c == '!'
                    || c == '?'
                    || c == ':'
                    || c == ';'
                    || c == '('
                    || c == ')' =>
                {
                    result.push(c);
                }
                // Skip control characters
                c if c.is_control() => {}
                // Allow other whitespace but convert to space
                c if c.is_whitespace() => result.push(' '),
                // Skip other characters
                _ => {}
            }
        }
        result
    }

    /// Trim and normalize whitespace
    pub fn normalize_whitespace(input: &str) -> String {
        input
            .split_whitespace()
            .collect::<Vec<_>>()
            .join(" ")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_validate_model_empty() {
        let validator = InferenceValidator::default();
        let result = validator.validate_model("");
        assert!(result.is_err());
    }

    #[test]
    fn test_validate_model_valid() {
        let validator = InferenceValidator::default();
        let result = validator.validate_model("llama-7b");
        assert!(result.is_ok());
    }

    #[test]
    fn test_validate_model_with_slash() {
        let validator = InferenceValidator::default();
        let result = validator.validate_model("meta-llama/Llama-2-7b");
        assert!(result.is_ok());
    }

    #[test]
    fn test_validate_prompt_empty() {
        let validator = InferenceValidator::default();
        let result = validator.validate_prompt("");
        assert!(result.is_err());
    }

    #[test]
    fn test_validate_prompt_valid() {
        let validator = InferenceValidator::default();
        let result = validator.validate_prompt("What is AI?");
        assert!(result.is_ok());
    }

    #[test]
    fn test_validate_max_tokens_valid() {
        let validator = InferenceValidator::default();
        let result = validator.validate_max_tokens(100);
        assert!(result.is_ok());
    }

    #[test]
    fn test_validate_temperature_valid() {
        let validator = InferenceValidator::default();
        let result = validator.validate_temperature(0.7);
        assert!(result.is_ok());
    }

    #[test]
    fn test_validate_temperature_invalid() {
        let validator = InferenceValidator::default();
        let result = validator.validate_temperature(5.0);
        assert!(result.is_err());
    }

    #[test]
    fn test_sanitize_string() {
        let input = "Hello\x00World!";
        let output = sanitize::sanitize_string(input, false);
        assert!(!output.contains('\x00'));
    }

    #[test]
    fn test_normalize_whitespace() {
        let input = "Hello   world   \n  test";
        let output = sanitize::normalize_whitespace(input);
        assert_eq!(output, "Hello world test");
    }
}
