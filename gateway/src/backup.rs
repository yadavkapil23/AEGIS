use serde::{Deserialize, Serialize};
use sha2::{Sha256, Digest};
use std::path::Path;
use std::time::Instant;
use tokio::fs;
use tokio::process::Command;
use tracing::{info, warn, error};

// ── Configuration ─────────────────────────────────────────────

#[derive(Clone, Debug)]
pub struct BackupConfig {
    pub backup_dir: String,
    pub retention_days: u32,
    pub db_url: String,
    pub prometheus_url: String,
}

impl Default for BackupConfig {
    fn default() -> Self {
        Self {
            backup_dir: "/var/backups/aegis".into(),
            retention_days: 30,
            db_url: std::env::var("DATABASE_URL")
                .unwrap_or_else(|_| "postgres://postgres:postgres@localhost:5432/aegis".into()),
            prometheus_url: std::env::var("PROMETHEUS_URL")
                .unwrap_or_else(|_| "http://localhost:9090".into()),
        }
    }
}

// ── Backup metadata ───────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BackupMetadata {
    pub name: String,
    pub status: String,
    pub path: Option<String>,
    pub size_bytes: Option<u64>,
    pub duration_ms: Option<u64>,
    pub checksum: Option<String>,
    pub created_at: String,
}

// ── Manager ───────────────────────────────────────────────────

pub struct BackupManager {
    config: BackupConfig,
}

impl BackupManager {
    pub fn new(config: BackupConfig) -> Self {
        info!(
            backup_dir = %config.backup_dir,
            retention_days = config.retention_days,
            "Backup manager initialized"
        );
        Self { config }
    }

    // ── Database backup via pg_dump ──────────────────────────

    pub async fn backup_database(&self) -> Result<BackupMetadata, String> {
        let start = Instant::now();
        let timestamp = chrono::Utc::now().format("%Y%m%d_%H%M%S");
        let filename = format!("aegis_db_{}.sql", timestamp);
        let filepath = self.backup_path(&filename);

        // Ensure backup directory exists
        fs::create_dir_all(&self.config.backup_dir)
            .await
            .map_err(|e| format!("Failed to create backup dir: {}", e))?;

        info!(path = %filepath, "Starting database backup");

        let output = Command::new("pg_dump")
            .arg(&self.config.db_url)
            .arg("--format=plain")
            .arg("--no-owner")
            .arg("--no-privileges")
            .output()
            .await
            .map_err(|e| format!("Failed to execute pg_dump: {}", e))?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(format!("pg_dump failed: {}", stderr));
        }

        // Write dump to file
        fs::write(&filepath, &output.stdout)
            .await
            .map_err(|e| format!("Failed to write backup file: {}", e))?;

        // Compress with gzip
        let gz_path = format!("{}.gz", filepath);
        let gz_output = Command::new("gzip")
            .arg("-f")
            .arg(&filepath)
            .output()
            .await
            .map_err(|e| format!("Failed to compress backup: {}", e))?;

        if !gz_output.status.success() {
            warn!("gzip compression failed, keeping uncompressed file");
        }

        let final_path = if Path::new(&gz_path).exists() {
            gz_path
        } else {
            filepath
        };

        // Compute checksum
        let file_bytes = fs::read(&final_path)
            .await
            .map_err(|e| format!("Failed to read backup for checksum: {}", e))?;
        let size = file_bytes.len() as u64;
        let mut hasher = Sha256::new();
        hasher.update(&file_bytes);
        let checksum = format!("{:x}", hasher.finalize());

        let duration_ms = start.elapsed().as_millis() as u64;

        info!(
            path = %final_path,
            size_bytes = size,
            duration_ms = duration_ms,
            "Database backup completed"
        );

        Ok(BackupMetadata {
            name: filename,
            status: "success".into(),
            path: Some(final_path),
            size_bytes: Some(size),
            duration_ms: Some(duration_ms),
            checksum: Some(checksum),
            created_at: chrono::Utc::now().to_rfc3339(),
        })
    }

    // ── Prometheus snapshot via HTTP API ──────────────────────

    pub async fn backup_prometheus(&self) -> Result<BackupMetadata, String> {
        let start = Instant::now();
        let timestamp = chrono::Utc::now().format("%Y%m%d_%H%M%S");
        let filename = format!("prometheus_{}.json", timestamp);
        let filepath = self.backup_path(&filename);

        fs::create_dir_all(&self.config.backup_dir)
            .await
            .map_err(|e| format!("Failed to create backup dir: {}", e))?;

        info!(path = %filepath, "Starting Prometheus snapshot");

        let client = reqwest::Client::new();
        let url = format!("{}/api/v1/label/__name__/values", self.config.prometheus_url);

        let response = client
            .get(&url)
            .timeout(std::time::Duration::from_secs(30))
            .send()
            .await
            .map_err(|e| format!("Failed to query Prometheus: {}", e))?;

        if !response.status().is_success() {
            return Err(format!("Prometheus returned status {}", response.status()));
        }

        let body = response
            .bytes()
            .await
            .map_err(|e| format!("Failed to read Prometheus response: {}", e))?;

        let size = body.len() as u64;
        fs::write(&filepath, &body)
            .await
            .map_err(|e| format!("Failed to write Prometheus snapshot: {}", e))?;

        let mut hasher = Sha256::new();
        hasher.update(&body);
        let checksum = format!("{:x}", hasher.finalize());

        let duration_ms = start.elapsed().as_millis() as u64;

        info!(
            path = %filepath,
            size_bytes = size,
            duration_ms = duration_ms,
            "Prometheus snapshot completed"
        );

        Ok(BackupMetadata {
            name: filename,
            status: "success".into(),
            path: Some(filepath),
            size_bytes: Some(size),
            duration_ms: Some(duration_ms),
            checksum: Some(checksum),
            created_at: chrono::Utc::now().to_rfc3339(),
        })
    }

    // ── List existing backups ─────────────────────────────────

    pub async fn list_backups(&self) -> Vec<BackupMetadata> {
        let dir = Path::new(&self.config.backup_dir);
        let mut backups = Vec::new();

        if !dir.exists() {
            return backups;
        }

        let mut entries = match fs::read_dir(dir).await {
            Ok(e) => e,
            Err(_) => return backups,
        };

        while let Ok(Some(entry)) = entries.next_entry().await {
            let path = entry.path();
            if path.extension().is_some_and(|e| e == "gz" || e == "sql" || e == "json") {
                let metadata = match fs::metadata(&path).await {
                    Ok(m) => m,
                    Err(_) => continue,
                };

                let name = path.file_name()
                    .map(|n| n.to_string_lossy().to_string())
                    .unwrap_or_default();

                backups.push(BackupMetadata {
                    name,
                    status: "on-disk".into(),
                    path: Some(path.to_string_lossy().to_string()),
                    size_bytes: Some(metadata.len()),
                    duration_ms: None,
                    checksum: None,
                    created_at: metadata.modified()
                        .map(|t| {
                            let dt: chrono::DateTime<chrono::Utc> = t.into();
                            dt.to_rfc3339()
                        })
                        .unwrap_or_default(),
                });
            }
        }

        backups.sort_by(|a, b| b.created_at.cmp(&a.created_at));
        backups
    }

    // ── Cleanup old backups ───────────────────────────────────

    pub async fn cleanup_old_backups(&self) -> Result<u32, String> {
        let dir = Path::new(&self.config.backup_dir);
        if !dir.exists() {
            return Ok(0);
        }

        let cutoff = chrono::Utc::now() - chrono::Duration::days(self.config.retention_days as i64);
        let mut deleted = 0u32;

        let mut entries = match fs::read_dir(dir).await {
            Ok(e) => e,
            Err(e) => return Err(format!("Failed to read backup dir: {}", e)),
        };

        while let Ok(Some(entry)) = entries.next_entry().await {
            let path = entry.path();
            let metadata = match fs::metadata(&path).await {
                Ok(m) => m,
                Err(_) => continue,
            };

            if let Ok(modified) = metadata.modified() {
                let modified_dt: chrono::DateTime<chrono::Utc> = modified.into();
                if modified_dt < cutoff {
                    if let Err(e) = fs::remove_file(&path).await {
                        error!(path = %path.display(), error = %e, "Failed to delete old backup");
                    } else {
                        info!(path = %path.display(), "Deleted old backup");
                        deleted += 1;
                    }
                }
            }
        }

        info!(deleted = deleted, retention_days = self.config.retention_days, "Cleanup completed");
        Ok(deleted)
    }

    // ── Helpers ───────────────────────────────────────────────

    fn backup_path(&self, filename: &str) -> String {
        Path::new(&self.config.backup_dir)
            .join(filename)
            .to_string_lossy()
            .to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_config() {
        let cfg = BackupConfig::default();
        assert_eq!(cfg.retention_days, 30);
        assert!(!cfg.backup_dir.is_empty());
    }

    #[test]
    fn backup_path_construction() {
        let mgr = BackupManager::new(BackupConfig {
            backup_dir: "/tmp/backups".into(),
            ..Default::default()
        });
        assert_eq!(mgr.backup_path("test.sql"), "/tmp/backups/test.sql");
    }
}
