/// Real LLM Backend Integration
/// Supports vLLM (primary) and llama.cpp (fallback)

use serde::{Deserialize, Serialize};
use reqwest::Client;
use std::sync::Arc;
use tracing::{info, warn, error};
use std::time::Instant;

/// vLLM completion request
#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct VLLMRequest {
    pub prompt: String,
    pub max_tokens: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub temperature: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub top_p: Option<f32>,
    pub model: Option<String>,
    pub stream: bool,
}

/// vLLM completion response
#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct VLLMResponse {
    pub id: String,
    pub object: String,
    pub created: u64,
    pub model: String,
    pub choices: Vec<VLLMChoice>,
    pub usage: VLLMUsage,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct VLLMChoice {
    pub index: u32,
    pub message: Option<VLLMMessage>,
    pub text: Option<String>,
    pub finish_reason: String,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct VLLMMessage {
    pub role: String,
    pub content: String,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct VLLMUsage {
    pub prompt_tokens: u32,
    pub completion_tokens: u32,
    pub total_tokens: u32,
}

/// llama.cpp completion request
#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct LlamaCppRequest {
    pub prompt: String,
    pub n_predict: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub temperature: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub top_p: Option<f32>,
    pub stream: bool,
}

/// llama.cpp completion response
#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct LlamaCppResponse {
    pub content: String,
    pub stop: bool,
    pub generation_settings: Option<LlamaCppSettings>,
    pub tokens_predicted: u32,
    pub tokens_evaluated: u32,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct LlamaCppSettings {
    pub n_ctx: u32,
    pub n_predict: u32,
    pub temperature: f32,
    pub top_p: f32,
}

/// Unified LLM response
#[derive(Debug, Clone)]
pub struct InferenceResult {
    pub output: String,
    pub tokens_generated: u32,
    pub prompt_tokens: u32,
    pub total_tokens: u32,
    pub backend: String,
    pub latency_ms: u64,
}

/// LLM Backend Client
pub struct LLMBackend {
    vllm_endpoint: String,
    llamacpp_endpoint: String,
    client: Arc<Client>,
    timeout_secs: u64,
}

impl LLMBackend {
    pub fn new(vllm_endpoint: String, llamacpp_endpoint: String, timeout_secs: u64) -> Self {
        Self {
            vllm_endpoint,
            llamacpp_endpoint,
            client: Arc::new(Client::new()),
            timeout_secs,
        }
    }

    /// Execute inference with vLLM, fallback to llama.cpp
    pub async fn infer(
        &self,
        model: &str,
        prompt: &str,
        max_tokens: u32,
        temperature: Option<f32>,
        top_p: Option<f32>,
    ) -> Result<InferenceResult, String> {
        let start = Instant::now();

        // Try vLLM first (primary backend)
        match self.vllm_infer(model, prompt, max_tokens, temperature, top_p).await {
            Ok(result) => {
                info!(
                    "vLLM inference succeeded: model={}, tokens={}, latency_ms={}",
                    model, result.tokens_generated, result.latency_ms
                );
                return Ok(result);
            }
            Err(e) => {
                warn!("vLLM inference failed: {}, falling back to llama.cpp", e);
            }
        }

        // Fallback to llama.cpp
        match self.llamacpp_infer(prompt, max_tokens, temperature, top_p).await {
            Ok(mut result) => {
                result.latency_ms = start.elapsed().as_millis() as u64;
                info!(
                    "llama.cpp inference succeeded: tokens={}, latency_ms={}",
                    result.tokens_generated, result.latency_ms
                );
                Ok(result)
            }
            Err(e) => {
                error!("Both backends failed: vLLM and llama.cpp - {}", e);
                Err(format!("All inference backends failed: {}", e))
            }
        }
    }

    /// Call vLLM backend (OpenAI-compatible API)
    async fn vllm_infer(
        &self,
        model: &str,
        prompt: &str,
        max_tokens: u32,
        temperature: Option<f32>,
        top_p: Option<f32>,
    ) -> Result<InferenceResult, String> {
        let start = Instant::now();

        let request = VLLMRequest {
            prompt: prompt.to_string(),
            max_tokens,
            temperature,
            top_p,
            model: Some(model.to_string()),
            stream: false,
        };

        let url = format!("{}/v1/completions", self.vllm_endpoint);

        info!("Calling vLLM: {}", url);

        let response = self
            .client
            .post(&url)
            .json(&request)
            .timeout(std::time::Duration::from_secs(self.timeout_secs))
            .send()
            .await
            .map_err(|e| format!("vLLM request failed: {}", e))?;

        let status = response.status();
        if !status.is_success() {
            return Err(format!("vLLM returned status {}: {}", status, response.text().await.unwrap_or_default()));
        }

        let vllm_response: VLLMResponse = response
            .json()
            .await
            .map_err(|e| format!("Failed to parse vLLM response: {}", e))?;

        // Extract output from first choice
        let output = vllm_response
            .choices
            .first()
            .and_then(|choice| choice.text.clone())
            .ok_or("No output in vLLM response".to_string())?;

        let latency_ms = start.elapsed().as_millis() as u64;

        Ok(InferenceResult {
            output,
            tokens_generated: vllm_response.usage.completion_tokens,
            prompt_tokens: vllm_response.usage.prompt_tokens,
            total_tokens: vllm_response.usage.total_tokens,
            backend: "vLLM".to_string(),
            latency_ms,
        })
    }

    /// Call llama.cpp backend
    async fn llamacpp_infer(
        &self,
        prompt: &str,
        max_tokens: u32,
        temperature: Option<f32>,
        top_p: Option<f32>,
    ) -> Result<InferenceResult, String> {
        let start = Instant::now();

        let request = LlamaCppRequest {
            prompt: prompt.to_string(),
            n_predict: max_tokens,
            temperature,
            top_p,
            stream: false,
        };

        let url = format!("{}/completion", self.llamacpp_endpoint);

        info!("Calling llama.cpp: {}", url);

        let response = self
            .client
            .post(&url)
            .json(&request)
            .timeout(std::time::Duration::from_secs(self.timeout_secs))
            .send()
            .await
            .map_err(|e| format!("llama.cpp request failed: {}", e))?;

        let status = response.status();
        if !status.is_success() {
            return Err(format!("llama.cpp returned status {}: {}", status, response.text().await.unwrap_or_default()));
        }

        let llama_response: LlamaCppResponse = response
            .json()
            .await
            .map_err(|e| format!("Failed to parse llama.cpp response: {}", e))?;

        let latency_ms = start.elapsed().as_millis() as u64;

        Ok(InferenceResult {
            output: llama_response.content,
            tokens_generated: llama_response.tokens_predicted,
            prompt_tokens: llama_response.tokens_evaluated,
            total_tokens: llama_response.tokens_predicted + llama_response.tokens_evaluated,
            backend: "llama.cpp".to_string(),
            latency_ms,
        })
    }

    /// Check if vLLM is healthy
    pub async fn check_vllm_health(&self) -> bool {
        let url = format!("{}/health", self.vllm_endpoint);
        match self
            .client
            .get(&url)
            .timeout(std::time::Duration::from_secs(5))
            .send()
            .await
        {
            Ok(resp) => {
                let healthy = resp.status().is_success();
                if healthy {
                    info!("vLLM health check: OK");
                } else {
                    warn!("vLLM health check failed: {}", resp.status());
                }
                healthy
            }
            Err(e) => {
                warn!("vLLM health check error: {}", e);
                false
            }
        }
    }

    /// Check if llama.cpp is healthy
    pub async fn check_llamacpp_health(&self) -> bool {
        let url = format!("{}/health", self.llamacpp_endpoint);
        match self
            .client
            .get(&url)
            .timeout(std::time::Duration::from_secs(5))
            .send()
            .await
        {
            Ok(resp) => {
                let healthy = resp.status().is_success();
                if healthy {
                    info!("llama.cpp health check: OK");
                } else {
                    warn!("llama.cpp health check failed: {}", resp.status());
                }
                healthy
            }
            Err(e) => {
                warn!("llama.cpp health check error: {}", e);
                false
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_vllm_request_serialization() {
        let req = VLLMRequest {
            prompt: "What is AI?".to_string(),
            max_tokens: 100,
            temperature: Some(0.7),
            top_p: Some(0.9),
            model: Some("llama-7b".to_string()),
            stream: false,
        };

        let json = serde_json::to_string(&req).unwrap();
        assert!(json.contains("What is AI?"));
        assert!(json.contains("100"));
    }

    #[test]
    fn test_llamacpp_request_serialization() {
        let req = LlamaCppRequest {
            prompt: "Hello world".to_string(),
            n_predict: 50,
            temperature: Some(0.8),
            top_p: None,
            stream: false,
        };

        let json = serde_json::to_string(&req).unwrap();
        assert!(json.contains("Hello world"));
        assert!(json.contains("50"));
    }
}
