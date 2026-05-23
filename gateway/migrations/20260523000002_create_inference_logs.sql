-- Migration: Create Inference Logs Table
-- Purpose: Store all inference requests for monitoring and analytics
-- Created: May 23, 2026

CREATE TABLE IF NOT EXISTS inference_logs (
    -- Primary key
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),

    -- Request details
    model VARCHAR(255) NOT NULL,
    prompt_hash VARCHAR(255),  -- Hash of prompt (not full prompt for privacy)
    request_size INT,          -- Request size in bytes
    response_size INT,         -- Response size in bytes

    -- Results
    status VARCHAR(50) NOT NULL,  -- 'success', 'failure', 'timeout', etc
    latency_ms INT NOT NULL,      -- Total request latency
    tokens_generated INT,         -- Number of tokens generated
    backend VARCHAR(50),          -- Which backend handled it ('vLLM', 'llama.cpp')
    error_message TEXT,           -- Error details if status != 'success'

    -- Timestamp
    created_at TIMESTAMP NOT NULL DEFAULT CURRENT_TIMESTAMP,

    -- Constraints
    CHECK (latency_ms >= 0),
    CHECK (status IN ('success', 'failure', 'timeout', 'validation_error', 'auth_error', 'rate_limited'))
);

-- Create indexes for fast queries
CREATE INDEX IF NOT EXISTS idx_inference_logs_model_time
    ON inference_logs(model, created_at DESC);

CREATE INDEX IF NOT EXISTS idx_inference_logs_status_time
    ON inference_logs(status, created_at DESC);

CREATE INDEX IF NOT EXISTS idx_inference_logs_created_at
    ON inference_logs(created_at DESC);

CREATE INDEX IF NOT EXISTS idx_inference_logs_backend
    ON inference_logs(backend);

-- Add comment
COMMENT ON TABLE inference_logs IS 'Inference request logs for monitoring and analytics (async batch writes)';
COMMENT ON COLUMN inference_logs.prompt_hash IS 'Hash of prompt for privacy (do not store full prompt)';
COMMENT ON COLUMN inference_logs.status IS 'Request status: success, failure, timeout, etc';
COMMENT ON COLUMN inference_logs.latency_ms IS 'Total latency from request receipt to response sent';
