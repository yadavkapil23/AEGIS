/// Database Migration Framework for AEGIS Gateway
/// Manages schema evolution with safety checks

use std::path::Path;
use tracing::{info, warn, error};

/// Migration metadata
#[derive(Debug, Clone)]
pub struct Migration {
    pub version: u64,
    pub name: String,
    pub description: String,
    pub sql: String,
    pub rollback_sql: Option<String>,
    pub created_at: String,
    pub checksum: String,
}

/// Migration history tracking
#[derive(Debug, Clone)]
pub struct MigrationHistory {
    pub migration_id: u64,
    pub name: String,
    pub applied_at: String,
    pub duration_ms: u64,
    pub checksum: String,
    pub status: String,
}

/// Migration executor
pub struct MigrationManager {
    migrations_dir: String,
    applied_migrations: Vec<MigrationHistory>,
}

impl MigrationManager {
    pub fn new(migrations_dir: &str) -> Self {
        Self {
            migrations_dir: migrations_dir.to_string(),
            applied_migrations: Vec::new(),
        }
    }

    /// Create migration file
    pub fn create_migration(
        &self,
        name: &str,
        description: &str,
        sql: &str,
        rollback_sql: Option<&str>,
    ) -> Result<String, String> {
        let timestamp = chrono::Utc::now().format("%Y%m%d_%H%M%S");
        let version = timestamp.to_string();
        let filename = format!("{}_{}.sql", version, name);
        let filepath = format!("{}/{}", self.migrations_dir, filename);

        info!("Creating migration: {} ({})", filename, description);

        // In production, actually write to filesystem
        // For now, just validate
        if name.is_empty() {
            return Err("Migration name cannot be empty".to_string());
        }

        if sql.is_empty() {
            return Err("Migration SQL cannot be empty".to_string());
        }

        if name.len() > 128 {
            return Err("Migration name is too long (max 128 characters)".to_string());
        }

        Ok(filepath)
    }

    /// List pending migrations
    pub fn pending_migrations(&self) -> Vec<Migration> {
        // In production, scan migrations_dir and check against applied_migrations
        vec![]
    }

    /// Apply migrations
    pub async fn migrate(&mut self) -> Result<usize, String> {
        let pending = self.pending_migrations();

        if pending.is_empty() {
            info!("No pending migrations");
            return Ok(0);
        }

        info!("Found {} pending migrations", pending.len());
        let mut applied_count = 0;

        for migration in pending {
            match self.apply_migration(&migration).await {
                Ok(_) => {
                    applied_count += 1;
                    self.applied_migrations.push(MigrationHistory {
                        migration_id: migration.version,
                        name: migration.name.clone(),
                        applied_at: chrono::Utc::now().to_rfc3339(),
                        duration_ms: 0,
                        checksum: migration.checksum.clone(),
                        status: "success".to_string(),
                    });
                    info!("Applied migration: {}", migration.name);
                }
                Err(e) => {
                    error!("Failed to apply migration {}: {}", migration.name, e);
                    // In production, decide whether to rollback or halt
                    return Err(format!("Migration {} failed: {}", migration.name, e));
                }
            }
        }

        Ok(applied_count)
    }

    /// Apply single migration
    async fn apply_migration(&self, migration: &Migration) -> Result<(), String> {
        info!("Applying migration: {} (v{})", migration.name, migration.version);

        // Validate migration before applying
        self.validate_migration(migration)?;

        // In production, execute SQL against actual database
        // This is a dry-run validation
        info!(
            "Would execute SQL ({}): {}",
            migration.version,
            &migration.sql[..migration.sql.len().min(100)]
        );

        Ok(())
    }

    /// Validate migration safety
    fn validate_migration(&self, migration: &Migration) -> Result<(), String> {
        // Check for dangerous operations
        let dangerous_keywords = [
            "DROP DATABASE",
            "DROP TABLE",
            "DELETE FROM",
            "TRUNCATE",
            "ALTER COLUMN DROP",
        ];

        for keyword in &dangerous_keywords {
            if migration.sql.to_uppercase().contains(keyword) {
                if migration.rollback_sql.is_none() {
                    warn!("Dangerous operation '{}' without rollback SQL", keyword);
                    return Err(format!(
                        "Dangerous operation '{}' requires rollback_sql",
                        keyword
                    ));
                }
            }
        }

        Ok(())
    }

    /// Rollback last migration
    pub async fn rollback(&mut self) -> Result<(), String> {
        if let Some(last_migration) = self.applied_migrations.last() {
            warn!(
                "Rolling back migration: {}",
                last_migration.name
            );
            // In production, execute rollback SQL
            self.applied_migrations.pop();
            Ok(())
        } else {
            Err("No migrations to rollback".to_string())
        }
    }

    /// Get migration status
    pub fn status(&self) -> MigrationStatus {
        MigrationStatus {
            applied_count: self.applied_migrations.len() as u64,
            pending_count: self.pending_migrations().len() as u64,
            last_applied: self.applied_migrations.last().map(|m| m.name.clone()),
        }
    }
}

#[derive(Debug)]
pub struct MigrationStatus {
    pub applied_count: u64,
    pub pending_count: u64,
    pub last_applied: Option<String>,
}

/// Pre-built migrations for AEGIS Gateway
pub mod migrations {
    use super::*;

    /// Initial schema creation
    pub fn init_schema() -> Migration {
        Migration {
            version: 1,
            name: "init_schema".to_string(),
            description: "Create initial database schema for AEGIS Gateway".to_string(),
            sql: r#"
                CREATE TABLE IF NOT EXISTS api_keys (
                    id UUID PRIMARY KEY,
                    key VARCHAR(255) UNIQUE NOT NULL,
                    name VARCHAR(256),
                    org_id UUID,
                    created_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP,
                    last_used_at TIMESTAMP,
                    is_active BOOLEAN DEFAULT true
                );

                CREATE TABLE IF NOT EXISTS inference_logs (
                    id UUID PRIMARY KEY,
                    request_id VARCHAR(255),
                    user_id VARCHAR(255),
                    model VARCHAR(256),
                    prompt_length INT,
                    tokens_generated INT,
                    latency_ms INT,
                    status VARCHAR(50),
                    error_code VARCHAR(50),
                    created_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP
                );

                CREATE TABLE IF NOT EXISTS rate_limit_counters (
                    api_key_id UUID PRIMARY KEY,
                    requests_this_minute INT DEFAULT 0,
                    requests_this_hour INT DEFAULT 0,
                    reset_minute_at TIMESTAMP,
                    reset_hour_at TIMESTAMP
                );

                CREATE INDEX idx_inference_logs_user_id ON inference_logs(user_id);
                CREATE INDEX idx_inference_logs_created_at ON inference_logs(created_at);
                CREATE INDEX idx_api_keys_org_id ON api_keys(org_id);
            "#.to_string(),
            rollback_sql: Some(r#"
                DROP TABLE IF EXISTS rate_limit_counters;
                DROP TABLE IF EXISTS inference_logs;
                DROP TABLE IF EXISTS api_keys;
            "#.to_string()),
            created_at: "2024-01-01T00:00:00Z".to_string(),
            checksum: "v1_init_schema".to_string(),
        }
    }

    /// Add audit logging table
    pub fn add_audit_table() -> Migration {
        Migration {
            version: 2,
            name: "add_audit_table".to_string(),
            description: "Add audit logging table for security compliance".to_string(),
            sql: r#"
                CREATE TABLE IF NOT EXISTS audit_logs (
                    id UUID PRIMARY KEY,
                    event_type VARCHAR(100),
                    actor_id VARCHAR(255),
                    resource VARCHAR(255),
                    action VARCHAR(100),
                    status VARCHAR(50),
                    details TEXT,
                    created_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP
                );

                CREATE INDEX idx_audit_logs_actor_id ON audit_logs(actor_id);
                CREATE INDEX idx_audit_logs_created_at ON audit_logs(created_at);
            "#.to_string(),
            rollback_sql: Some("DROP TABLE IF EXISTS audit_logs;".to_string()),
            created_at: "2024-01-02T00:00:00Z".to_string(),
            checksum: "v2_add_audit_table".to_string(),
        }
    }

    /// Add performance metrics table
    pub fn add_metrics_table() -> Migration {
        Migration {
            version: 3,
            name: "add_metrics_table".to_string(),
            description: "Add table for storing aggregated metrics".to_string(),
            sql: r#"
                CREATE TABLE IF NOT EXISTS metrics (
                    id UUID PRIMARY KEY,
                    metric_name VARCHAR(255),
                    metric_value FLOAT,
                    labels JSONB,
                    recorded_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP
                );

                CREATE INDEX idx_metrics_name_time ON metrics(metric_name, recorded_at);
            "#.to_string(),
            rollback_sql: Some("DROP TABLE IF EXISTS metrics;".to_string()),
            created_at: "2024-01-03T00:00:00Z".to_string(),
            checksum: "v3_add_metrics_table".to_string(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_migration_manager_creation() {
        let manager = MigrationManager::new("/migrations");
        assert_eq!(manager.migrations_dir, "/migrations");
    }

    #[test]
    fn test_create_migration_valid() {
        let manager = MigrationManager::new("/migrations");
        let result = manager.create_migration(
            "test_migration",
            "Test migration",
            "CREATE TABLE test (id INT);",
            None,
        );
        assert!(result.is_ok());
    }

    #[test]
    fn test_create_migration_empty_name() {
        let manager = MigrationManager::new("/migrations");
        let result = manager.create_migration("", "Test", "SQL", None);
        assert!(result.is_err());
    }

    #[test]
    fn test_migration_status() {
        let manager = MigrationManager::new("/migrations");
        let status = manager.status();
        assert_eq!(status.applied_count, 0);
    }

    #[test]
    fn test_init_schema_migration() {
        let migration = migrations::init_schema();
        assert_eq!(migration.version, 1);
        assert!(migration.sql.contains("CREATE TABLE"));
        assert!(migration.rollback_sql.is_some());
    }
}
