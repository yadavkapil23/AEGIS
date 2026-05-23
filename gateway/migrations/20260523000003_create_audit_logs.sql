-- Migration: Create Audit Logs Table
-- Purpose: Track administrative actions for compliance and debugging
-- Created: May 23, 2026

CREATE TABLE IF NOT EXISTS audit_logs (
    -- Primary key
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),

    -- Action details
    action VARCHAR(255) NOT NULL,        -- e.g., 'api_key_created', 'config_changed'
    resource_type VARCHAR(255),          -- e.g., 'api_key', 'backend', 'config'
    resource_id VARCHAR(255),            -- ID of the resource being modified

    -- Who and when
    user_id VARCHAR(255),                -- User performing the action
    created_at TIMESTAMP NOT NULL DEFAULT CURRENT_TIMESTAMP,

    -- Additional info
    details TEXT,                        -- JSON with details of the change
    status VARCHAR(50) NOT NULL,         -- 'success', 'failure'

    -- Constraints
    CHECK (status IN ('success', 'failure', 'pending'))
);

-- Create indexes for fast queries
CREATE INDEX IF NOT EXISTS idx_audit_logs_action_time
    ON audit_logs(action, created_at DESC);

CREATE INDEX IF NOT EXISTS idx_audit_logs_user_time
    ON audit_logs(user_id, created_at DESC);

CREATE INDEX IF NOT EXISTS idx_audit_logs_resource
    ON audit_logs(resource_type, resource_id);

CREATE INDEX IF NOT EXISTS idx_audit_logs_created_at
    ON audit_logs(created_at DESC);

-- Add comment
COMMENT ON TABLE audit_logs IS 'Audit trail of administrative actions (optional, for compliance)';
COMMENT ON COLUMN audit_logs.action IS 'Type of action performed';
COMMENT ON COLUMN audit_logs.details IS 'JSON object with action details';
COMMENT ON COLUMN audit_logs.status IS 'Whether action succeeded or failed';
