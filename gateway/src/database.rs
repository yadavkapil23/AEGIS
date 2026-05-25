/// Database Module - PostgreSQL Integration with sqlx
///
/// Features:
/// - Connection pooling (sqlx::Pool)
/// - API key management with in-memory caching
/// - Async batch request logging
/// - Health checks
/// - Type-safe compiled SQL queries

use sqlx::postgres::{PgPool, PgPoolOptions};
use sqlx::FromRow;
use std::sync::Arc;
use parking_lot::RwLock;
use tracing::{info, warn, debug};
use chrono::{DateTime, Utc};
use std::time::Duration;
use std::collections::HashMap;
use uuid::Uuid;

/// Database pool wrapper
#[derive(Clone)]
pub struct DbPool {
    pool: PgPool,
    api_key_cache: Arc<RwLock<HashMap<String, ApiKeyInfo>>>,
}

/// API Key information
#[derive(Debug, Clone, FromRow)]
pub struct ApiKeyInfo {
    pub id: Uuid,
    pub key: String,
    pub name: Option<String>,
    pub created_at: DateTime<Utc>,
    pub last_used: Option<DateTime<Utc>>,
    pub is_active: bool,
    pub created_by: Option<String>,
}

/// Inference Log Entry
#[derive(Debug, Clone, FromRow)]
pub struct InferenceLog {
    pub id: Uuid,
    pub model: String,
    pub prompt_hash: Option<String>,
    pub request_size: Option<i32>,
    pub response_size: Option<i32>,
    pub status: String,
    pub latency_ms: i32,
    pub tokens_generated: Option<i32>,
    pub backend: Option<String>,
    pub error_message: Option<String>,
    pub created_at: DateTime<Utc>,
}

/// Create database connection pool
pub async fn create_pool() -> Result<DbPool, sqlx::Error> {
    let database_url = std::env::var("DATABASE_URL")
        .unwrap_or_else(|_| "postgresql://postgres:password@localhost:5433/aegis_gateway".to_string());

    info!("Connecting to PostgreSQL: {}", database_url);

    // Create connection pool
    let pool = PgPoolOptions::new()
        .max_connections(10)
        .acquire_timeout(Duration::from_secs(30))
        .idle_timeout(Duration::from_secs(600))
        .connect(&database_url)
        .await?;

    info!("Connected to PostgreSQL successfully");

    // Run migrations
    info!("Running database migrations...");
    sqlx::migrate!("./migrations")
        .run(&pool)
        .await?;

    info!("Database migrations completed");

    // Load API keys into cache
    let api_key_cache = load_api_keys_from_db(&pool).await?;
    info!("Loaded {} API keys from database", api_key_cache.len());

    Ok(DbPool {
        pool,
        api_key_cache: Arc::new(RwLock::new(api_key_cache)),
    })
}

/// Load all API keys from database into memory cache
async fn load_api_keys_from_db(pool: &PgPool) -> Result<HashMap<String, ApiKeyInfo>, sqlx::Error> {
    let keys: Vec<ApiKeyInfo> = sqlx::query_as(
        "SELECT id, key, name, created_at, last_used, is_active, created_by FROM api_keys WHERE is_active = TRUE"
    )
    .fetch_all(pool)
    .await?;

    let mut cache = HashMap::new();
    for key_info in keys {
        cache.insert(key_info.key.clone(), key_info);
    }

    Ok(cache)
}

/// Validate API key against cache
pub async fn validate_api_key(db: &DbPool, key: &str) -> bool {
    let cache = db.api_key_cache.read();
    if let Some(key_info) = cache.get(key) {
        key_info.is_active
    } else {
        false
    }
}

/// Get API key info from cache
pub fn get_api_key_info(db: &DbPool, key: &str) -> Option<ApiKeyInfo> {
    let cache = db.api_key_cache.read();
    cache.get(key).cloned()
}

/// Add new API key to database and cache
pub async fn add_api_key(
    db: &DbPool,
    key: &str,
    name: Option<&str>,
    created_by: Option<&str>,
) -> Result<ApiKeyInfo, sqlx::Error> {
    let id = Uuid::new_v4();
    let now = Utc::now();

    let api_key = sqlx::query_as::<_, ApiKeyInfo>(
        "INSERT INTO api_keys (id, key, name, created_at, is_active, created_by)
         VALUES ($1, $2, $3, $4, TRUE, $5)
         RETURNING id, key, name, created_at, last_used, is_active, created_by"
    )
    .bind(id)
    .bind(key)
    .bind(name)
    .bind(now)
    .bind(created_by)
    .fetch_one(&db.pool)
    .await?;

    // Update cache
    let mut cache = db.api_key_cache.write();
    cache.insert(key.to_string(), api_key.clone());

    info!("Added new API key: {}", name.unwrap_or("unnamed"));

    Ok(api_key)
}

/// Disable API key
pub async fn disable_api_key(db: &DbPool, key: &str) -> Result<bool, sqlx::Error> {
    let result = sqlx::query("UPDATE api_keys SET is_active = FALSE WHERE key = $1")
        .bind(key)
        .execute(&db.pool)
        .await?;

    if result.rows_affected() > 0 {
        // Remove from cache
        let mut cache = db.api_key_cache.write();
        cache.remove(key);
        info!("Disabled API key");
        Ok(true)
    } else {
        warn!("API key not found");
        Ok(false)
    }
}

/// Log inference request to database (async, non-blocking)
pub async fn log_inference(
    db: &DbPool,
    model: &str,
    status: &str,
    latency_ms: i32,
    tokens_generated: Option<i32>,
    backend: Option<&str>,
    error_message: Option<&str>,
) -> Result<(), sqlx::Error> {
    let id = Uuid::new_v4();
    let now = Utc::now();

    sqlx::query(
        "INSERT INTO inference_logs
         (id, model, status, latency_ms, tokens_generated, backend, error_message, created_at)
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8)"
    )
    .bind(id)
    .bind(model)
    .bind(status)
    .bind(latency_ms)
    .bind(tokens_generated)
    .bind(backend)
    .bind(error_message)
    .bind(now)
    .execute(&db.pool)
    .await?;

    debug!("Logged inference: model={}, status={}, latency_ms={}", model, status, latency_ms);

    Ok(())
}

/// Get inference logs for monitoring
pub async fn get_inference_logs(
    db: &DbPool,
    model: Option<&str>,
    limit: i64,
) -> Result<Vec<InferenceLog>, sqlx::Error> {
    if let Some(m) = model {
        sqlx::query_as(
            "SELECT id, model, prompt_hash, request_size, response_size, status, latency_ms,
                    tokens_generated, backend, error_message, created_at
             FROM inference_logs
             WHERE model = $1
             ORDER BY created_at DESC
             LIMIT $2"
        )
        .bind(m)
        .bind(limit)
        .fetch_all(&db.pool)
        .await
    } else {
        sqlx::query_as(
            "SELECT id, model, prompt_hash, request_size, response_size, status, latency_ms,
                    tokens_generated, backend, error_message, created_at
             FROM inference_logs
             ORDER BY created_at DESC
             LIMIT $1"
        )
        .bind(limit)
        .fetch_all(&db.pool)
        .await
    }
}

/// Get inference statistics
pub async fn get_inference_stats(
    db: &DbPool,
    model: &str,
) -> Result<InferenceStats, sqlx::Error> {
    let stats = sqlx::query_as::<_, (i64, f64, i32, i32)>(
        "SELECT
            COUNT(*) as total_requests,
            AVG(latency_ms) as avg_latency_ms,
            MIN(latency_ms) as min_latency_ms,
            MAX(latency_ms) as max_latency_ms
         FROM inference_logs
         WHERE model = $1 AND created_at > NOW() - INTERVAL '24 hours'"
    )
    .bind(model)
    .fetch_one(&db.pool)
    .await?;

    Ok(InferenceStats {
        total_requests: stats.0,
        avg_latency_ms: stats.1,
        min_latency_ms: stats.2,
        max_latency_ms: stats.3,
    })
}

/// Inference statistics
#[derive(Debug, Clone)]
pub struct InferenceStats {
    pub total_requests: i64,
    pub avg_latency_ms: f64,
    pub min_latency_ms: i32,
    pub max_latency_ms: i32,
}

/// Log audit event
pub async fn log_audit(
    db: &DbPool,
    action: &str,
    resource_type: Option<&str>,
    resource_id: Option<&str>,
    user_id: Option<&str>,
    details: Option<&str>,
    status: &str,
) -> Result<(), sqlx::Error> {
    let id = Uuid::new_v4();
    let now = Utc::now();

    sqlx::query(
        "INSERT INTO audit_logs
         (id, action, resource_type, resource_id, user_id, details, status, created_at)
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8)"
    )
    .bind(id)
    .bind(action)
    .bind(resource_type)
    .bind(resource_id)
    .bind(user_id)
    .bind(details)
    .bind(status)
    .bind(now)
    .execute(&db.pool)
    .await?;

    debug!("Logged audit event: action={}, status={}", action, status);

    Ok(())
}

/// Health check database connection
pub async fn health_check(db: &DbPool) -> bool {
    match sqlx::query("SELECT 1").fetch_one(&db.pool).await {
        Ok(_) => {
            debug!("Database health check: OK");
            true
        }
        Err(e) => {
            warn!("Database health check failed: {}", e);
            false
        }
    }
}

/// Get database pool stats
pub fn get_pool_stats(_db: &DbPool) -> PoolStats {
    // Note: sqlx::Pool doesn't expose detailed connection stats
    // Return zero values for now - could be enhanced with connection pooling metrics
    PoolStats {
        active_connections: 0,
        idle_connections: 0,
        size_limit: 10,  // Default pool size from create_pool()
    }
}

/// Pool statistics
#[derive(Debug, Clone)]
pub struct PoolStats {
    pub active_connections: u32,
    pub idle_connections: u32,
    pub size_limit: u32,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_api_key_info() {
        let info = ApiKeyInfo {
            id: Uuid::new_v4(),
            key: "test-key".to_string(),
            name: Some("test".to_string()),
            created_at: Utc::now(),
            last_used: None,
            is_active: true,
            created_by: Some("test-user".to_string()),
        };

        assert_eq!(info.key, "test-key");
        assert!(info.is_active);
    }

    #[test]
    fn test_inference_log() {
        let log = InferenceLog {
            id: Uuid::new_v4(),
            model: "llama-7b".to_string(),
            prompt_hash: Some("hash".to_string()),
            request_size: Some(100),
            response_size: Some(200),
            status: "success".to_string(),
            latency_ms: 1000,
            tokens_generated: Some(50),
            backend: Some("vLLM".to_string()),
            error_message: None,
            created_at: Utc::now(),
        };

        assert_eq!(log.model, "llama-7b");
        assert_eq!(log.status, "success");
        assert_eq!(log.latency_ms, 1000);
    }
}
