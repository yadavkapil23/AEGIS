use serde::{Deserialize, Serialize};
use reqwest::Client;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use tracing::{info, warn, error, debug};
use std::time::{Instant, Duration};

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

/// HuggingFace Inference API request
#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct HFRequest {
    pub inputs: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub parameters: Option<HFParameters>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct HFParameters {
    pub max_length: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub temperature: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub top_p: Option<f32>,
}

/// HuggingFace Inference API response
#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct HFResponse {
    #[serde(default)]
    pub generated_text: Option<String>,
    #[serde(default)]
    pub summary_text: Option<String>,
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

/// Circuit Breaker State
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum CircuitState {
    Closed,      // Normal operation
    Open,        // Failing, reject requests
    HalfOpen,    // Testing if service recovered
}

/// Circuit Breaker for backend failure handling
pub struct CircuitBreaker {
    failure_count: AtomicUsize,
    success_count: AtomicUsize,
    state: Arc<parking_lot::Mutex<CircuitState>>,
    failure_threshold: usize,
    success_threshold: usize,
}

impl CircuitBreaker {
    pub fn new(failure_threshold: usize, success_threshold: usize) -> Self {
        Self {
            failure_count: AtomicUsize::new(0),
            success_count: AtomicUsize::new(0),
            state: Arc::new(parking_lot::Mutex::new(CircuitState::Closed)),
            failure_threshold,
            success_threshold,
        }
    }

    pub fn record_success(&self) {
        self.failure_count.store(0, Ordering::Relaxed);
        let mut state = self.state.lock();

        if *state == CircuitState::HalfOpen {
            self.success_count.fetch_add(1, Ordering::Relaxed);
            if self.success_count.load(Ordering::Relaxed) >= self.success_threshold {
                *state = CircuitState::Closed;
                self.success_count.store(0, Ordering::Relaxed);
                info!("Circuit breaker closed (service recovered)");
            }
        }
    }

    pub fn record_failure(&self) {
        let failures = self.failure_count.fetch_add(1, Ordering::Relaxed) + 1;
        let mut state = self.state.lock();

        if failures >= self.failure_threshold && *state == CircuitState::Closed {
            *state = CircuitState::Open;
            warn!("Circuit breaker opened (too many failures)");
        } else if *state == CircuitState::HalfOpen {
            *state = CircuitState::Open;
            warn!("Circuit breaker reopened (recovery failed)");
        }
    }

    pub fn allow_request(&self) -> bool {
        let state = self.state.lock();
        match *state {
            CircuitState::Closed => true,
            CircuitState::Open => false,
            CircuitState::HalfOpen => true, // Allow test request
        }
    }

    pub fn test_recovery(&self) {
        let mut state = self.state.lock();
        if *state == CircuitState::Open {
            *state = CircuitState::HalfOpen;
            self.success_count.store(0, Ordering::Relaxed);
            info!("Circuit breaker entering half-open state");
        }
    }

    pub fn get_state(&self) -> CircuitState {
        *self.state.lock()
    }
}

/// LLM Backend Client with real HTTP integration
pub struct LLMBackend {
    vllm_endpoint: String,
    llamacpp_endpoint: String,
    ollama_endpoint: String,
    hf_api_endpoint: String,
    hf_api_key: Option<String>,
    client: Arc<Client>,
    timeout_secs: u64,
    vllm_circuit_breaker: Arc<CircuitBreaker>,
    llamacpp_circuit_breaker: Arc<CircuitBreaker>,
    ollama_circuit_breaker: Arc<CircuitBreaker>,
    hf_circuit_breaker: Arc<CircuitBreaker>,
}

impl LLMBackend {
    pub fn new(vllm_endpoint: String, llamacpp_endpoint: String, timeout_secs: u64) -> Self {
        let hf_api_key = std::env::var("HUGGINGFACE_API_KEY").ok();
        let hf_api_endpoint = std::env::var("HUGGINGFACE_ENDPOINT")
            .unwrap_or_else(|_| "https://api-inference.huggingface.co/models".to_string());
        let ollama_endpoint = std::env::var("OLLAMA_ENDPOINT")
            .unwrap_or_else(|_| "http://aegis-ollama:11434".to_string());

        info!("Initializing LLM Backend");
        info!("  vLLM endpoint: {}", vllm_endpoint);
        info!("  llama.cpp endpoint: {}", llamacpp_endpoint);
        info!("  Ollama endpoint: {}", ollama_endpoint);
        info!("  HuggingFace API: {} ({})",
            hf_api_endpoint,
            if hf_api_key.is_some() { "configured" } else { "not configured" }
        );
        info!("  Timeout: {} seconds", timeout_secs);

        Self {
            vllm_endpoint,
            llamacpp_endpoint,
            ollama_endpoint,
            hf_api_endpoint,
            hf_api_key,
            client: Arc::new(Client::new()),
            timeout_secs,
            vllm_circuit_breaker: Arc::new(CircuitBreaker::new(5, 3)), // 5 failures, 3 successes to recover
            llamacpp_circuit_breaker: Arc::new(CircuitBreaker::new(5, 3)),
            ollama_circuit_breaker: Arc::new(CircuitBreaker::new(5, 3)),
            hf_circuit_breaker: Arc::new(CircuitBreaker::new(5, 3)),
        }
    }

    /// Retry logic with exponential backoff
    async fn retry_request<F, Fut, T>(
        &self,
        mut f: F,
        max_retries: usize,
    ) -> Result<T, String>
    where
        F: FnMut() -> Fut,
        Fut: std::future::Future<Output = Result<T, String>>,
    {
        let mut retry_count = 0;
        let mut backoff_ms = 100u64; // Start with 100ms

        loop {
            match f().await {
                Ok(result) => return Ok(result),
                Err(e) => {
                    retry_count += 1;
                    if retry_count >= max_retries {
                        return Err(format!("Max retries ({}) exceeded: {}", max_retries, e));
                    }

                    warn!(
                        "Request failed (attempt {}/{}), retrying in {}ms: {}",
                        retry_count, max_retries, backoff_ms, e
                    );

                    tokio::time::sleep(Duration::from_millis(backoff_ms)).await;
                    backoff_ms = (backoff_ms * 2).min(5000); // Exponential backoff, max 5 seconds
                }
            }
        }
    }

    /// Execute inference with vLLM, fallback to llama.cpp, then ollama
    /// Uses circuit breaker pattern and retry logic
    pub async fn infer(
        &self,
        model: &str,
        prompt: &str,
        max_tokens: u32,
        temperature: Option<f32>,
        top_p: Option<f32>,
    ) -> Result<InferenceResult, String> {
        let start = Instant::now();

        debug!("Starting inference: model={}, prompt_len={}", model, prompt.len());

        // Try vLLM first (primary backend)
        if self.vllm_circuit_breaker.allow_request() {
            match self.vllm_infer(model, prompt, max_tokens, temperature, top_p).await {
                Ok(result) => {
                    self.vllm_circuit_breaker.record_success();
                    info!(
                        "vLLM inference succeeded: model={}, tokens={}, latency_ms={}",
                        model, result.tokens_generated, result.latency_ms
                    );
                    return Ok(result);
                }
                Err(e) => {
                    self.vllm_circuit_breaker.record_failure();
                    warn!("vLLM inference failed: {}, falling back to llama.cpp", e);
                }
            }
        } else {
            warn!("vLLM circuit breaker is open, skipping to fallback");
            self.vllm_circuit_breaker.test_recovery();
        }

        // Fallback to llama.cpp
        if self.llamacpp_circuit_breaker.allow_request() {
            match self.llamacpp_infer(prompt, max_tokens, temperature, top_p).await {
                Ok(mut result) => {
                    self.llamacpp_circuit_breaker.record_success();
                    result.latency_ms = start.elapsed().as_millis() as u64;
                    info!(
                        "llama.cpp inference succeeded: tokens={}, latency_ms={}",
                        result.tokens_generated, result.latency_ms
                    );
                    return Ok(result);
                }
                Err(e) => {
                    self.llamacpp_circuit_breaker.record_failure();
                    warn!("llama.cpp inference failed: {}, falling back to ollama", e);
                }
            }
        } else {
            warn!("llama.cpp circuit breaker is open, trying ollama");
            self.llamacpp_circuit_breaker.test_recovery();
        }

        // Fallback to Ollama (OpenAI-compatible API)
        if self.ollama_circuit_breaker.allow_request() {
            match self.ollama_infer(model, prompt, max_tokens, temperature, top_p).await {
                Ok(mut result) => {
                    self.ollama_circuit_breaker.record_success();
                    result.latency_ms = start.elapsed().as_millis() as u64;
                    info!(
                        "Ollama inference succeeded: tokens={}, latency_ms={}",
                        result.tokens_generated, result.latency_ms
                    );
                    return Ok(result);
                }
                Err(e) => {
                    self.ollama_circuit_breaker.record_failure();
                    warn!("Ollama inference failed: {}, falling back to HuggingFace API", e);
                }
            }
        } else {
            warn!("Ollama circuit breaker is open, trying HuggingFace API");
            self.ollama_circuit_breaker.test_recovery();
        }

        // Fallback to HuggingFace Inference API (cloud-based)
        if self.hf_api_key.is_some() && self.hf_circuit_breaker.allow_request() {
            match self.hf_infer(model, prompt, max_tokens, temperature, top_p).await {
                Ok(mut result) => {
                    self.hf_circuit_breaker.record_success();
                    result.latency_ms = start.elapsed().as_millis() as u64;
                    info!(
                        "HuggingFace API inference succeeded: tokens={}, latency_ms={}",
                        result.tokens_generated, result.latency_ms
                    );
                    return Ok(result);
                }
                Err(e) => {
                    self.hf_circuit_breaker.record_failure();
                    error!("HuggingFace API inference failed: {}", e);
                }
            }
        } else if self.hf_api_key.is_none() {
            debug!("HuggingFace API not configured (HUGGINGFACE_API_KEY not set)");
        } else {
            warn!("HuggingFace API circuit breaker is open");
            self.hf_circuit_breaker.test_recovery();
        }

        error!("All inference backends failed or unavailable");
        Err("All inference backends failed or circuit breakers are open".to_string())
    }

    /// Call vLLM backend (OpenAI-compatible API) with retry logic
    async fn vllm_infer(
        &self,
        model: &str,
        prompt: &str,
        max_tokens: u32,
        temperature: Option<f32>,
        top_p: Option<f32>,
    ) -> Result<InferenceResult, String> {
        let start = Instant::now();

        // Validate inputs
        if prompt.is_empty() {
            return Err("Prompt cannot be empty".to_string());
        }
        if max_tokens == 0 {
            return Err("max_tokens must be greater than 0".to_string());
        }

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
        debug!("vLLM Request: model={}, max_tokens={}, has_temperature={}", model, max_tokens, temperature.is_some());

        // Retry logic with exponential backoff
        let result = self.retry_request(
            || async {
                debug!("Sending request to vLLM...");
                let response = self
                    .client
                    .post(&url)
                    .json(&request)
                    .timeout(Duration::from_secs(self.timeout_secs))
                    .send()
                    .await
                    .map_err(|e| {
                        if e.is_timeout() {
                            format!("vLLM request timeout ({} seconds)", self.timeout_secs)
                        } else {
                            format!("vLLM request failed: {}", e)
                        }
                    })?;

                let status = response.status();
                if !status.is_success() {
                    let error_text = response.text().await.unwrap_or_else(|_| "Unknown error".to_string());
                    return Err(format!("vLLM returned status {}: {}", status, error_text));
                }

                debug!("vLLM returned success status");
                let vllm_response: VLLMResponse = response
                    .json()
                    .await
                    .map_err(|e| format!("Failed to parse vLLM response: {}", e))?;

                // Extract output from first choice
                let output = vllm_response
                    .choices
                    .first()
                    .and_then(|choice| choice.text.clone())
                    .ok_or_else(|| {
                        format!(
                            "No output in vLLM response (choices: {})",
                            vllm_response.choices.len()
                        )
                    })?;

                if output.is_empty() {
                    return Err("vLLM returned empty output".to_string());
                }

                let latency_ms = start.elapsed().as_millis() as u64;

                Ok(InferenceResult {
                    output,
                    tokens_generated: vllm_response.usage.completion_tokens,
                    prompt_tokens: vllm_response.usage.prompt_tokens,
                    total_tokens: vllm_response.usage.total_tokens,
                    backend: "vLLM".to_string(),
                    latency_ms,
                })
            },
            3, // Max 3 retries
        )
        .await;

        result
    }

    /// Call llama.cpp backend with retry logic
    async fn llamacpp_infer(
        &self,
        prompt: &str,
        max_tokens: u32,
        temperature: Option<f32>,
        top_p: Option<f32>,
    ) -> Result<InferenceResult, String> {
        let start = Instant::now();

        // Validate inputs
        if prompt.is_empty() {
            return Err("Prompt cannot be empty".to_string());
        }
        if max_tokens == 0 {
            return Err("max_tokens must be greater than 0".to_string());
        }

        let request = LlamaCppRequest {
            prompt: prompt.to_string(),
            n_predict: max_tokens,
            temperature,
            top_p,
            stream: false,
        };

        let url = format!("{}/completion", self.llamacpp_endpoint);

        info!("Calling llama.cpp: {}", url);
        debug!("llama.cpp Request: max_tokens={}, has_temperature={}", max_tokens, temperature.is_some());

        // Retry logic with exponential backoff
        let result = self.retry_request(
            || async {
                debug!("Sending request to llama.cpp...");
                let response = self
                    .client
                    .post(&url)
                    .json(&request)
                    .timeout(Duration::from_secs(self.timeout_secs))
                    .send()
                    .await
                    .map_err(|e| {
                        if e.is_timeout() {
                            format!("llama.cpp request timeout ({} seconds)", self.timeout_secs)
                        } else {
                            format!("llama.cpp request failed: {}", e)
                        }
                    })?;

                let status = response.status();
                if !status.is_success() {
                    let error_text = response.text().await.unwrap_or_else(|_| "Unknown error".to_string());
                    return Err(format!("llama.cpp returned status {}: {}", status, error_text));
                }

                debug!("llama.cpp returned success status");
                let llama_response: LlamaCppResponse = response
                    .json()
                    .await
                    .map_err(|e| format!("Failed to parse llama.cpp response: {}", e))?;

                if llama_response.content.is_empty() {
                    return Err("llama.cpp returned empty content".to_string());
                }

                let latency_ms = start.elapsed().as_millis() as u64;

                Ok(InferenceResult {
                    output: llama_response.content,
                    tokens_generated: llama_response.tokens_predicted,
                    prompt_tokens: llama_response.tokens_evaluated,
                    total_tokens: llama_response.tokens_predicted + llama_response.tokens_evaluated,
                    backend: "llama.cpp".to_string(),
                    latency_ms,
                })
            },
            3, // Max 3 retries
        )
        .await;

        result
    }

    /// Call Ollama backend (OpenAI-compatible API) with retry logic
    async fn ollama_infer(
        &self,
        model: &str,
        prompt: &str,
        max_tokens: u32,
        temperature: Option<f32>,
        top_p: Option<f32>,
    ) -> Result<InferenceResult, String> {
        let start = Instant::now();

        // Validate inputs
        if prompt.is_empty() {
            return Err("Prompt cannot be empty".to_string());
        }
        if max_tokens == 0 {
            return Err("max_tokens must be greater than 0".to_string());
        }

        let request = VLLMRequest {
            prompt: prompt.to_string(),
            max_tokens,
            temperature,
            top_p,
            model: Some(model.to_string()),
            stream: false,
        };

        let url = format!("{}/v1/completions", self.ollama_endpoint);
        info!("Calling Ollama: {}", url);
        debug!("Ollama Request: model={}, max_tokens={}, has_temperature={}", model, max_tokens, temperature.is_some());

        // Retry logic with exponential backoff
        let result = self.retry_request(
            || async {
                debug!("Sending request to Ollama...");
                let response = self
                    .client
                    .post(&url)
                    .json(&request)
                    .timeout(Duration::from_secs(self.timeout_secs))
                    .send()
                    .await
                    .map_err(|e| {
                        if e.is_timeout() {
                            format!("Ollama request timeout ({} seconds)", self.timeout_secs)
                        } else {
                            format!("Ollama request failed: {}", e)
                        }
                    })?;

                let status = response.status();
                if !status.is_success() {
                    let error_text = response.text().await.unwrap_or_else(|_| "Unknown error".to_string());
                    return Err(format!("Ollama returned status {}: {}", status, error_text));
                }

                debug!("Ollama returned success status");
                let ollama_response: VLLMResponse = response
                    .json()
                    .await
                    .map_err(|e| format!("Failed to parse Ollama response: {}", e))?;

                // Extract output from first choice
                let output = ollama_response
                    .choices
                    .first()
                    .and_then(|choice| choice.text.clone())
                    .ok_or_else(|| {
                        format!(
                            "No output in Ollama response (choices: {})",
                            ollama_response.choices.len()
                        )
                    })?;

                if output.is_empty() {
                    return Err("Ollama returned empty output".to_string());
                }

                let latency_ms = start.elapsed().as_millis() as u64;

                Ok(InferenceResult {
                    output,
                    tokens_generated: ollama_response.usage.completion_tokens,
                    prompt_tokens: ollama_response.usage.prompt_tokens,
                    total_tokens: ollama_response.usage.total_tokens,
                    backend: "Ollama".to_string(),
                    latency_ms,
                })
            },
            3, // Max 3 retries
        )
        .await;

        result
    }

    /// Call HuggingFace Inference API with retry logic
    async fn hf_infer(
        &self,
        model: &str,
        prompt: &str,
        max_tokens: u32,
        temperature: Option<f32>,
        top_p: Option<f32>,
    ) -> Result<InferenceResult, String> {
        let start = Instant::now();

        // Validate inputs
        if prompt.is_empty() {
            return Err("Prompt cannot be empty".to_string());
        }
        if max_tokens == 0 {
            return Err("max_tokens must be greater than 0".to_string());
        }

        let request = HFRequest {
            inputs: prompt.to_string(),
            parameters: Some(HFParameters {
                max_length: Some(max_tokens),
                temperature,
                top_p,
            }),
        };

        let url = format!("{}/{}", self.hf_api_endpoint, model);
        info!("Calling HuggingFace API: {}", url);
        debug!("HF Request: model={}, max_tokens={}, has_temperature={}", model, max_tokens, temperature.is_some());

        // Retry logic with exponential backoff
        let result = self.retry_request(
            || async {
                debug!("Sending request to HuggingFace API...");
                let mut request = self
                    .client
                    .post(&url)
                    .json(&request)
                    .timeout(Duration::from_secs(self.timeout_secs));

                // Add Authorization header if API key is present
                if let Some(api_key) = &self.hf_api_key {
                    request = request.bearer_auth(api_key);
                }

                let response = request
                    .send()
                    .await
                    .map_err(|e| {
                        if e.is_timeout() {
                            format!("HuggingFace API request timeout ({} seconds)", self.timeout_secs)
                        } else {
                            format!("HuggingFace API request failed: {}", e)
                        }
                    })?;

                let status = response.status();
                if !status.is_success() {
                    let error_text = response.text().await.unwrap_or_else(|_| "Unknown error".to_string());
                    return Err(format!("HuggingFace API returned status {}: {}", status, error_text));
                }

                debug!("HuggingFace API returned success status");
                let hf_response_array: Vec<HFResponse> = response
                    .json()
                    .await
                    .map_err(|e| format!("Failed to parse HuggingFace API response: {}", e))?;

                // Extract output from first response
                let hf_response = hf_response_array
                    .first()
                    .ok_or_else(|| "No response from HuggingFace API".to_string())?;

                let output = hf_response
                    .generated_text
                    .clone()
                    .or_else(|| hf_response.summary_text.clone())
                    .ok_or_else(|| "No output in HuggingFace API response".to_string())?;

                if output.is_empty() {
                    return Err("HuggingFace API returned empty output".to_string());
                }

                let latency_ms = start.elapsed().as_millis() as u64;

                // Estimate token counts (rough approximation)
                let prompt_tokens = (prompt.len() / 4) as u32;
                let completion_tokens = (output.len() / 4) as u32;

                Ok(InferenceResult {
                    output,
                    tokens_generated: completion_tokens,
                    prompt_tokens,
                    total_tokens: prompt_tokens + completion_tokens,
                    backend: "HuggingFace".to_string(),
                    latency_ms,
                })
            },
            3, // Max 3 retries
        )
        .await;

        result
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
            .timeout(Duration::from_secs(5))
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

    /// Check if Ollama is healthy
    pub async fn check_ollama_health(&self) -> bool {
        let url = format!("{}/api/tags", self.ollama_endpoint);
        match self
            .client
            .get(&url)
            .timeout(Duration::from_secs(5))
            .send()
            .await
        {
            Ok(resp) => {
                let healthy = resp.status().is_success();
                if healthy {
                    info!("Ollama health check: OK");
                } else {
                    warn!("Ollama health check failed: {}", resp.status());
                }
                healthy
            }
            Err(e) => {
                warn!("Ollama health check error: {}", e);
                false
            }
        }
    }

    /// Check if HuggingFace API is accessible
    pub async fn check_hf_health(&self) -> bool {
        // HF API health check just verifies we have an API key
        // (can't really health-check the endpoint without making a request)
        if self.hf_api_key.is_some() {
            info!("HuggingFace API health check: OK (API key configured)");
            true
        } else {
            debug!("HuggingFace API not configured (no API key)");
            false
        }
    }

    /// Get status of all backends
    pub async fn get_backend_status(&self) -> BackendStatus {
        let vllm_health = self.check_vllm_health().await;
        let llamacpp_health = self.check_llamacpp_health().await;
        let ollama_health = self.check_ollama_health().await;
        let hf_health = self.check_hf_health().await;

        BackendStatus {
            vllm: BackendInfo {
                endpoint: self.vllm_endpoint.clone(),
                healthy: vllm_health,
                circuit_breaker_state: self.vllm_circuit_breaker.get_state().to_string(),
            },
            llamacpp: BackendInfo {
                endpoint: self.llamacpp_endpoint.clone(),
                healthy: llamacpp_health,
                circuit_breaker_state: self.llamacpp_circuit_breaker.get_state().to_string(),
            },
            ollama: BackendInfo {
                endpoint: self.ollama_endpoint.clone(),
                healthy: ollama_health,
                circuit_breaker_state: self.ollama_circuit_breaker.get_state().to_string(),
            },
            huggingface: BackendInfo {
                endpoint: self.hf_api_endpoint.clone(),
                healthy: hf_health,
                circuit_breaker_state: self.hf_circuit_breaker.get_state().to_string(),
            },
        }
    }
}

/// Backend status information
#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct BackendStatus {
    pub vllm: BackendInfo,
    pub llamacpp: BackendInfo,
    pub ollama: BackendInfo,
    pub huggingface: BackendInfo,
}

/// Individual backend information
#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct BackendInfo {
    pub endpoint: String,
    pub healthy: bool,
    pub circuit_breaker_state: String,
}

// Serialize CircuitState as string
impl Serialize for CircuitState {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        let state = match self {
            CircuitState::Closed => "closed",
            CircuitState::Open => "open",
            CircuitState::HalfOpen => "half-open",
        };
        serializer.serialize_str(state)
    }
}

impl std::fmt::Display for CircuitState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            CircuitState::Closed => write!(f, "closed"),
            CircuitState::Open => write!(f, "open"),
            CircuitState::HalfOpen => write!(f, "half-open"),
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
