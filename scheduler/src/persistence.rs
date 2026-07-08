// Persistence layer: Write-ahead logging for durability
// Enables recovery from disk on startup

use crate::replicated_log::LogEntry;
use std::fs::{File, OpenOptions};
use std::io::{Write, BufReader, BufWriter};
use std::path::PathBuf;
use parking_lot::Mutex;
use anyhow::Result;
use tracing::{debug, info, warn};
use serde_json;

/// Write-ahead log configuration
#[derive(Clone, Debug)]
pub struct WalConfig {
    pub log_dir: PathBuf,
    pub max_log_size_mb: usize,
    pub fsync_interval: usize, // fsync after N writes
}

impl Default for WalConfig {
    fn default() -> Self {
        Self {
            log_dir: PathBuf::from("./consensus_wal"),
            max_log_size_mb: 100,
            fsync_interval: 100,
        }
    }
}

/// Write-ahead log entry (stored format)
#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub struct WalEntry {
    pub lsn: u64,
    pub term: u64,
    pub operation_type: String,
    pub data: String, // JSON serialized
}

/// Write-ahead log manager
pub struct WriteAheadLog {
    config: WalConfig,
    current_file: Mutex<Option<BufWriter<File>>>,
    write_count: Mutex<usize>,
    entries_written: Mutex<u64>,
}

impl WriteAheadLog {
    /// Create new WAL
    pub fn new(config: WalConfig) -> Result<Self> {
        // Create log directory if it doesn't exist
        std::fs::create_dir_all(&config.log_dir)?;

        info!("Initialized WAL at {:?}", config.log_dir);

        Ok(Self {
            config,
            current_file: Mutex::new(None),
            write_count: Mutex::new(0),
            entries_written: Mutex::new(0),
        })
    }

    /// Append entry to WAL
    pub fn append(&self, entry: &LogEntry) -> Result<()> {
        // Serialize entry
        let json = serde_json::to_string(entry)?;
        let wal_entry = WalEntry {
            lsn: entry.lsn,
            term: entry.term,
            operation_type: format!("{:?}", entry.operation),
            data: json,
        };

        // Write to file
        let wal_json = serde_json::to_string(&wal_entry)?;
        let line = format!("{}\n", wal_json);

        self.write_line(&line)?;

        debug!("WAL append: lsn={}, term={}", entry.lsn, entry.term);

        Ok(())
    }

    /// Write line to WAL file
    fn write_line(&self, line: &str) -> Result<()> {
        // Open or create log file if needed
        let mut file_guard = self.current_file.lock();
        if file_guard.is_none() {
            let path = self.config.log_dir.join("wal.log");
            let file = OpenOptions::new()
                .create(true)
                .append(true)
                .open(&path)?;
            *file_guard = Some(BufWriter::new(file));
        }

        if let Some(file) = file_guard.as_mut() {
            file.write_all(line.as_bytes())?;

            // Periodic fsync
            let mut count = self.write_count.lock();
            *count += 1;
            if *count >= self.config.fsync_interval {
                file.flush()?;
                *count = 0;
            }
        }

        *self.entries_written.lock() += 1;
        Ok(())
    }

    /// Flush WAL to disk
    pub fn flush(&self) -> Result<()> {
        let mut file_guard = self.current_file.lock();
        if let Some(file) = file_guard.as_mut() {
            file.flush()?;
        }
        Ok(())
    }

    /// Recover entries from WAL
    pub fn recover(&self) -> Result<Vec<LogEntry>> {
        let path = self.config.log_dir.join("wal.log");

        if !path.exists() {
            info!("WAL file does not exist, starting fresh");
            return Ok(vec![]);
        }

        let file = File::open(&path)?;
        let reader = BufReader::new(file);
        let mut entries = vec![];

        for line in std::io::BufRead::lines(reader) {
            let line = line?;
            if line.trim().is_empty() {
                continue;
            }

            match serde_json::from_str::<WalEntry>(&line) {
                Ok(wal_entry) => {
                    match serde_json::from_str::<LogEntry>(&wal_entry.data) {
                        Ok(entry) => {
                            entries.push(entry);
                        }
                        Err(e) => {
                            warn!("Failed to deserialize log entry: {}", e);
                        }
                    }
                }
                Err(e) => {
                    warn!("Failed to deserialize WAL entry: {}", e);
                }
            }
        }

        info!("Recovered {} entries from WAL", entries.len());
        Ok(entries)
    }

    /// Get total entries written
    pub fn entries_written(&self) -> u64 {
        *self.entries_written.lock()
    }

    /// Clear WAL (typically after snapshot)
    pub fn clear(&self) -> Result<()> {
        let path = self.config.log_dir.join("wal.log");
        if path.exists() {
            std::fs::remove_file(&path)?;
            debug!("Cleared WAL");
        }
        Ok(())
    }
}

/// Snapshot manager for log compaction
pub struct SnapshotManager {
    config: WalConfig,
    last_snapshot_lsn: Mutex<u64>,
}

impl SnapshotManager {
    /// Create new snapshot manager
    pub fn new(config: WalConfig) -> Self {
        Self {
            config,
            last_snapshot_lsn: Mutex::new(0),
        }
    }

    /// Create snapshot of state at given LSN
    pub fn create_snapshot(&self, lsn: u64, state_data: &str) -> Result<()> {
        let path = self.config.log_dir.join(format!("snapshot_{}.json", lsn));
        let mut file = File::create(&path)?;
        file.write_all(state_data.as_bytes())?;

        *self.last_snapshot_lsn.lock() = lsn;
        info!("Created snapshot at LSN {}", lsn);

        Ok(())
    }

    /// Load snapshot
    pub fn load_snapshot(&self) -> Result<Option<(u64, String)>> {
        let last_lsn = *self.last_snapshot_lsn.lock();

        if last_lsn == 0 {
            return Ok(None);
        }

        let path = self.config.log_dir.join(format!("snapshot_{}.json", last_lsn));
        if !path.exists() {
            return Ok(None);
        }

        let data = std::fs::read_to_string(&path)?;
        Ok(Some((last_lsn, data)))
    }

    /// Get last snapshot LSN
    pub fn last_snapshot_lsn(&self) -> u64 {
        *self.last_snapshot_lsn.lock()
    }

    /// Clean up old snapshots
    pub fn cleanup_old_snapshots(&self, keep_count: usize) -> Result<()> {
        let mut entries: Vec<_> = std::fs::read_dir(&self.config.log_dir)?
            .filter_map(|e| {
                e.ok().and_then(|entry| {
                    let name = entry.file_name();
                    let name_str = name.to_string_lossy();
                    if name_str.starts_with("snapshot_") && name_str.ends_with(".json") {
                        Some((entry.path(), entry.metadata().ok()?.modified().ok()?))
                    } else {
                        None
                    }
                })
            })
            .collect();

        if entries.len() <= keep_count {
            return Ok(());
        }

        // Sort by modification time (newest first)
        entries.sort_by_key(|(_, time)| std::cmp::Reverse(*time));

        // Remove old snapshots
        for (path, _) in entries.iter().skip(keep_count) {
            std::fs::remove_file(path)?;
            debug!("Removed old snapshot: {:?}", path);
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn create_test_wal() -> (WriteAheadLog, TempDir) {
        let temp_dir = TempDir::new().unwrap();
        let config = WalConfig {
            log_dir: temp_dir.path().to_path_buf(),
            max_log_size_mb: 100,
            fsync_interval: 10,
        };
        let wal = WriteAheadLog::new(config).unwrap();
        (wal, temp_dir)
    }

    #[test]
    fn test_wal_creation() {
        let (wal, _temp) = create_test_wal();
        assert_eq!(wal.entries_written(), 0);
    }

    #[test]
    fn test_wal_append() {
        let (wal, _temp) = create_test_wal();

        let entry = LogEntry::new(
            1,
            1,
            crate::replicated_log::LogOperation::Allocate {
                request_id: "req-1".to_string(),
                num_blocks: 100,
            },
        );

        assert!(wal.append(&entry).is_ok());
        assert_eq!(wal.entries_written(), 1);
    }

    #[test]
    fn test_wal_recovery() {
        let temp_dir = TempDir::new().unwrap();
        let config = WalConfig {
            log_dir: temp_dir.path().to_path_buf(),
            max_log_size_mb: 100,
            fsync_interval: 10,
        };

        // Write entries
        {
            let wal = WriteAheadLog::new(config.clone()).unwrap();

            for i in 1..=3 {
                let entry = LogEntry::new(
                    i,
                    1,
                    crate::replicated_log::LogOperation::Allocate {
                        request_id: format!("req-{}", i),
                        num_blocks: 10 * i as usize,
                    },
                );
                wal.append(&entry).unwrap();
            }

            wal.flush().unwrap();
        }

        // Recover entries
        {
            let wal = WriteAheadLog::new(config).unwrap();
            let entries = wal.recover().unwrap();

            assert_eq!(entries.len(), 3);
            assert_eq!(entries[0].lsn, 1);
            assert_eq!(entries[1].lsn, 2);
            assert_eq!(entries[2].lsn, 3);
        }
    }

    #[test]
    fn test_snapshot_manager() {
        let temp_dir = TempDir::new().unwrap();
        let config = WalConfig {
            log_dir: temp_dir.path().to_path_buf(),
            max_log_size_mb: 100,
            fsync_interval: 10,
        };

        let manager = SnapshotManager::new(config);

        let state_data = r#"{"allocations": 5, "peers": 2}"#;
        assert!(manager.create_snapshot(100, state_data).is_ok());

        let snapshot = manager.load_snapshot().unwrap();
        assert!(snapshot.is_some());
        let (lsn, data) = snapshot.unwrap();
        assert_eq!(lsn, 100);
        assert_eq!(data, state_data);
    }

    #[test]
    fn test_snapshot_cleanup() {
        let temp_dir = TempDir::new().unwrap();
        let config = WalConfig {
            log_dir: temp_dir.path().to_path_buf(),
            max_log_size_mb: 100,
            fsync_interval: 10,
        };

        let manager = SnapshotManager::new(config);

        // Create multiple snapshots
        for i in 1..=5 {
            let data = format!(r#"{{"snapshot": {}}}"#, i);
            manager.create_snapshot(i * 100, &data).ok();
            std::thread::sleep(std::time::Duration::from_millis(10));
        }

        // Cleanup, keep 2 most recent
        manager.cleanup_old_snapshots(2).ok();

        // Verify
        let entries: Vec<_> = std::fs::read_dir(temp_dir.path())
            .unwrap()
            .filter_map(|e| {
                e.ok().map(|entry| {
                    entry.file_name().to_string_lossy().to_string()
                })
            })
            .filter(|name| name.starts_with("snapshot_"))
            .collect();

        assert_eq!(entries.len(), 2);
    }
}
