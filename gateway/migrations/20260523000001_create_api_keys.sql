-- Migration: Create API Keys Table
-- Purpose: Store API keys for authentication with in-memory caching
-- Created: May 23, 2026

CREATE TABLE IF NOT EXISTS api_keys (
    -- Primary key
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),

    -- API key value (unique)
    key VARCHAR(255) UNIQUE NOT NULL,

    -- Human-readable name
    name VARCHAR(255),

    -- Timestamps
    created_at TIMESTAMP NOT NULL DEFAULT CURRENT_TIMESTAMP,
    last_used TIMESTAMP,

    -- Status
    is_active BOOLEAN NOT NULL DEFAULT TRUE,

    -- Audit trail
    created_by VARCHAR(255),

    -- Constraints
    CHECK (LENGTH(key) >= 10)
);

-- Create indexes for fast lookups
CREATE INDEX IF NOT EXISTS idx_api_keys_key ON api_keys(key);
CREATE INDEX IF NOT EXISTS idx_api_keys_active ON api_keys(is_active);
CREATE INDEX IF NOT EXISTS idx_api_keys_created_at ON api_keys(created_at);

-- Add comment
COMMENT ON TABLE api_keys IS 'API keys for gateway authentication (cached in memory on startup)';
COMMENT ON COLUMN api_keys.key IS 'API key value, should be hashed in future';
COMMENT ON COLUMN api_keys.is_active IS 'Whether this key is currently active';
