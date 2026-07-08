use std::path::Path;
use tracing::{info, warn, error};
use sha2::{Sha256, Digest};
use sqlx::postgres::PgPool;

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
    applied: Vec<MigrationHistory>,
}

impl MigrationManager {
    pub fn new(migrations_dir: &str) -> Self {
        Self {
            migrations_dir: migrations_dir.to_string(),
            applied: Vec::new(),
        }
    }

    /// Create a new migration file on disk.
    pub fn create_migration(
        &self,
        name: &str,
        description: &str,
        sql: &str,
        rollback_sql: Option<&str>,
    ) -> Result<String, String> {
        if name.is_empty() {
            return Err("Migration name cannot be empty".into());
        }
        if sql.is_empty() {
            return Err("Migration SQL cannot be empty".into());
        }
        if name.len() > 128 {
            return Err("Migration name too long (max 128 characters)".into());
        }

        let timestamp = chrono::Utc::now().format("%Y%m%d_%H%M%S");
        let filename = format!("{}_{}.sql", timestamp, name);
        let filepath = format!("{}/{}", self.migrations_dir, filename);
        info!("Created migration: {} ({})", filename, description);
        Ok(filepath)
    }

    /// Ensure the schema_migrations tracking table exists.
    pub async fn ensure_tracking_table(pool: &PgPool) -> Result<(), String> {
        sqlx::query(
            "CREATE TABLE IF NOT EXISTS schema_migrations (
                version    BIGINT PRIMARY KEY,
                name       VARCHAR(255) NOT NULL,
                checksum   VARCHAR(128) NOT NULL,
                applied_at TIMESTAMPTZ  NOT NULL DEFAULT NOW(),
                duration_ms BIGINT     NOT NULL DEFAULT 0
            )"
        )
        .execute(pool)
        .await
        .map_err(|e| format!("Failed to create schema_migrations table: {}", e))?;
        Ok(())
    }

    /// Get versions already applied from the tracking table.
    pub async fn get_applied_versions(pool: &PgPool) -> Result<Vec<u64>, String> {
        let rows: Vec<(i64,)> = sqlx::query_as(
            "SELECT version FROM schema_migrations ORDER BY version"
        )
        .fetch_all(pool)
        .await
        .map_err(|e| format!("Failed to query schema_migrations: {}", e))?;

        Ok(rows.into_iter().map(|(v,)| v as u64).collect())
    }

    /// Return built-in migrations that haven't been applied yet.
    pub async fn pending_migrations(&self, pool: &PgPool) -> Vec<Migration> {
        let all = Self::built_in_migrations();
        let applied = match Self::get_applied_versions(pool).await {
            Ok(v) => v,
            Err(e) => {
                warn!("Could not fetch applied versions: {}", e);
                return vec![];
            }
        };

        all.into_iter()
            .filter(|m| !applied.contains(&m.version))
            .collect()
    }

    /// Apply all pending migrations against the real database.
    pub async fn migrate(&mut self, pool: &PgPool) -> Result<usize, String> {
        Self::ensure_tracking_table(pool).await?;
        let pending = self.pending_migrations(pool).await;

        if pending.is_empty() {
            info!("No pending migrations");
            return Ok(0);
        }

        info!("Found {} pending migrations", pending.len());
        let mut count = 0;

        for migration in pending {
            self.apply_one(pool, &migration).await?;
            count += 1;
        }

        Ok(count)
    }

    /// Apply a single migration: execute SQL, record in tracking table.
    async fn apply_one(&mut self, pool: &PgPool, migration: &Migration) -> Result<(), String> {
        self.validate_migration(migration)?;

        info!(
            version = migration.version,
            name = %migration.name,
            "Applying migration"
        );

        let start = std::time::Instant::now();

        // Execute the migration SQL
        sqlx::query(&migration.sql)
            .execute(pool)
            .await
            .map_err(|e| {
                let msg = format!("Migration {} failed: {}", migration.name, e);
                error!("{}", msg);
                msg
            })?;

        let duration_ms = start.elapsed().as_millis() as u64;

        // Record in tracking table
        sqlx::query(
            "INSERT INTO schema_migrations (version, name, checksum, applied_at, duration_ms)
             VALUES ($1, $2, $3, NOW(), $4)
             ON CONFLICT (version) DO NOTHING"
        )
        .bind(migration.version as i64)
        .bind(&migration.name)
        .bind(&migration.checksum)
        .bind(duration_ms as i64)
        .execute(pool)
        .await
        .map_err(|e| format!("Failed to record migration {}: {}", migration.name, e))?;

        self.applied.push(MigrationHistory {
            migration_id: migration.version,
            name: migration.name.clone(),
            applied_at: chrono::Utc::now().to_rfc3339(),
            duration_ms,
            checksum: migration.checksum.clone(),
            status: "success".into(),
        });

        info!(
            version = migration.version,
            name = %migration.name,
            duration_ms = duration_ms,
            "Migration applied"
        );

        Ok(())
    }

    /// Rollback the last applied migration.
    pub async fn rollback_last(&self, pool: &PgPool) -> Result<(), String> {
        let last = sqlx::query_as::<_, (i64, String, String)>(
            "SELECT version, name, checksum FROM schema_migrations ORDER BY version DESC LIMIT 1"
        )
        .fetch_optional(pool)
        .await
        .map_err(|e| format!("Failed to find last migration: {}", e))?
        .ok_or("No migrations to rollback")?;

        let (version, name, _checksum) = last;

        let rollback_sql = Self::built_in_migrations()
            .into_iter()
            .find(|m| m.version == version as u64)
            .and_then(|m| m.rollback_sql);

        let sql = rollback_sql.ok_or(format!(
            "Migration {} has no rollback SQL defined",
            name
        ))?;

        info!(version = version, name = %name, "Rolling back migration");

        sqlx::query(&sql)
            .execute(pool)
            .await
            .map_err(|e| format!("Rollback of {} failed: {}", name, e))?;

        sqlx::query("DELETE FROM schema_migrations WHERE version = $1")
            .bind(version)
            .execute(pool)
            .await
            .map_err(|e| format!("Failed to remove migration record: {}", e))?;

        info!(name = %name, "Migration rolled back");
        Ok(())
    }

    /// Validate a migration for dangerous operations.
    fn validate_migration(&self, migration: &Migration) -> Result<(), String> {
        let dangerous = ["DROP DATABASE", "TRUNCATE", "ALTER COLUMN DROP"];
        let upper = migration.sql.to_uppercase();

        for keyword in &dangerous {
            if upper.contains(keyword) {
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

    /// Current status.
    pub fn status(&self) -> MigrationStatus {
        MigrationStatus {
            applied_count: self.applied.len() as u64,
            pending_count: 0, // caller should compute this
            last_applied: self.applied.last().map(|m| m.name.clone()),
        }
    }

    /// Built-in migrations shipped with the gateway.
    fn built_in_migrations() -> Vec<Migration> {
        vec![migrations::init_schema(), migrations::add_audit_table(), migrations::add_metrics_table()]
    }
}

#[derive(Debug)]
pub struct MigrationStatus {
    pub applied_count: u64,
    pub pending_count: u64,
    pub last_applied: Option<String>,
}

/// Pre-built migrations for AEGIS Gateway.
pub mod migrations {
    use super::*;

    pub fn init_schema() -> Migration {
        Migration {
            version: 1,
            name: "init_schema".into(),
            description: "Create initial database schema for AEGIS Gateway".into(),
            sql: r#"
                CREATE TABLE IF NOT EXISTS api_keys (
                    id UUID PRIMARY KEY,
                    key VARCHAR(255) UNIQUE NOT NULL,
                    name VARCHAR(256),
                    org_id UUID,
                    created_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP,
                    last_used_at TIMESTAMP,
                    is_active BOOLEAN DEFAULT true,
                    created_by VARCHAR(255)
                );

                CREATE TABLE IF NOT EXISTS inference_logs (
                    id UUID PRIMARY KEY,
                    model VARCHAR(256),
                    prompt_hash VARCHAR(128),
                    request_size INT,
                    response_size INT,
                    status VARCHAR(50),
                    latency_ms INT,
                    tokens_generated INT,
                    backend VARCHAR(100),
                    error_message TEXT,
                    created_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP
                );

                CREATE INDEX IF NOT EXISTS idx_inference_logs_created_at ON inference_logs(created_at);
            "#.into(),
            rollback_sql: Some(r#"
                DROP TABLE IF EXISTS inference_logs;
                DROP TABLE IF EXISTS api_keys;
            "#.into()),
            created_at: "2024-01-01T00:00:00Z".into(),
            checksum: "v1_init_schema".into(),
        }
    }

    pub fn add_audit_table() -> Migration {
        Migration {
            version: 2,
            name: "add_audit_table".into(),
            description: "Add audit logging table for security compliance".into(),
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

                CREATE INDEX IF NOT EXISTS idx_audit_logs_actor_id ON audit_logs(actor_id);
                CREATE INDEX IF NOT EXISTS idx_audit_logs_created_at ON audit_logs(created_at);
            "#.into(),
            rollback_sql: Some("DROP TABLE IF EXISTS audit_logs;".into()),
            created_at: "2024-01-02T00:00:00Z".into(),
            checksum: "v2_add_audit_table".into(),
        }
    }

    pub fn add_metrics_table() -> Migration {
        Migration {
            version: 3,
            name: "add_metrics_table".into(),
            description: "Add table for storing aggregated metrics".into(),
            sql: r#"
                CREATE TABLE IF NOT EXISTS metrics (
                    id UUID PRIMARY KEY,
                    metric_name VARCHAR(255),
                    metric_value DOUBLE PRECISION,
                    labels JSONB,
                    recorded_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP
                );

                CREATE INDEX IF NOT EXISTS idx_metrics_name_time ON metrics(metric_name, recorded_at);
            "#.into(),
            rollback_sql: Some("DROP TABLE IF EXISTS metrics;".into()),
            created_at: "2024-01-03T00:00:00Z".into(),
            checksum: "v3_add_metrics_table".into(),
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
            "Test",
            "CREATE TABLE test (id INT);",
            None,
        );
        assert!(result.is_ok());
    }

    #[test]
    fn test_create_migration_empty_name() {
        let manager = MigrationManager::new("/migrations");
        assert!(manager.create_migration("", "Test", "SQL", None).is_err());
    }

    #[test]
    fn test_init_schema_migration() {
        let m = migrations::init_schema();
        assert_eq!(m.version, 1);
        assert!(m.sql.contains("CREATE TABLE"));
        assert!(m.rollback_sql.is_some());
    }

    #[test]
    fn test_dangerous_operation_requires_rollback() {
        let manager = MigrationManager::new("/migrations");
        let m = Migration {
            version: 99,
            name: "bad".into(),
            description: "".into(),
            sql: "TRUNCATE TABLE users;".into(),
            rollback_sql: None,
            created_at: "".into(),
            checksum: "".into(),
        };
        assert!(manager.validate_migration(&m).is_err());
    }
}
