//! Audit log persistence.
//!
//! Stores audit entries in a JSONL file with chain hashing for tamper detection.

use crate::types::audit::{AuditEntry, AuditEvent, AuditQueryOptions, AuditSeverity};
use crate::DvsError;
use fs_err as fs;
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};

/// Default audit log filename.
pub const AUDIT_LOG_FILENAME: &str = "audit.jsonl";

/// Audit log manager.
///
/// Handles reading and writing audit entries to a JSONL file.
/// Entries are chained with hashes to detect tampering.
pub struct AuditLog {
    /// Path to the audit log file.
    path: PathBuf,
    /// Current sequence number.
    seq: u64,
    /// Hash of the last entry (for chaining).
    last_hash: Option<String>,
}

impl AuditLog {
    /// Open or create an audit log at the given path.
    pub fn open(path: PathBuf) -> Result<Self, DvsError> {
        let mut log = Self {
            path,
            seq: 0,
            last_hash: None,
        };

        // Load existing state
        if log.path.exists() {
            log.load_state()?;
        }

        Ok(log)
    }

    /// Open the audit log in the .dvs/logs directory.
    pub fn open_for_repo(dvs_dir: &Path) -> Result<Self, DvsError> {
        let logs_dir = dvs_dir.join("logs");
        fs::create_dir_all(&logs_dir)?;
        let path = logs_dir.join(AUDIT_LOG_FILENAME);
        Self::open(path)
    }

    /// Get the path to the audit log file.
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Get the current sequence number.
    pub fn seq(&self) -> u64 {
        self.seq
    }

    /// Load the current state from the log file.
    fn load_state(&mut self) -> Result<(), DvsError> {
        let file = fs::File::open(&self.path)?;
        let reader = BufReader::new(file);

        for line in reader.lines() {
            let line = line?;
            if line.trim().is_empty() {
                continue;
            }

            match serde_json::from_str::<AuditEntry>(&line) {
                Ok(entry) => {
                    self.seq = entry.seq;
                    self.last_hash = Some(entry.compute_hash());
                }
                Err(_) => {
                    // Skip malformed entries
                    continue;
                }
            }
        }

        self.seq += 1; // Next entry will have next sequence number
        Ok(())
    }

    /// Append an entry to the audit log.
    pub fn append(&mut self, mut entry: AuditEntry) -> Result<(), DvsError> {
        // Set chain fields
        entry.seq = self.seq;
        if let Some(ref hash) = self.last_hash {
            entry.prev_hash = Some(hash.clone());
        }

        // Serialize to JSON
        let json = serde_json::to_string(&entry).map_err(|e| {
            DvsError::config_error(format!("Failed to serialize audit entry: {}", e))
        })?;

        // Append to file
        let mut file = fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.path)?;
        writeln!(file, "{}", json)?;

        // Update state
        self.last_hash = Some(entry.compute_hash());
        self.seq += 1;

        Ok(())
    }

    /// Log an event with default severity.
    pub fn log_event(
        &mut self,
        actor: String,
        event: AuditEvent,
        path: Option<PathBuf>,
        details: Option<String>,
    ) -> Result<(), DvsError> {
        let mut entry = AuditEntry::new(actor, event, event.default_severity());
        if let Some(p) = path {
            entry = entry.with_path(p);
        }
        if let Some(d) = details {
            entry = entry.with_details(d);
        }
        self.append(entry)
    }

    /// Log a security event (always uses the given severity).
    pub fn log_security_event(
        &mut self,
        actor: String,
        event: AuditEvent,
        severity: AuditSeverity,
        path: Option<PathBuf>,
        details: Option<String>,
    ) -> Result<(), DvsError> {
        let mut entry = AuditEntry::new(actor, event, severity);
        if let Some(p) = path {
            entry = entry.with_path(p);
        }
        if let Some(d) = details {
            entry = entry.with_details(d);
        }
        self.append(entry)
    }

    /// Query entries from the audit log.
    pub fn query(&self, options: &AuditQueryOptions) -> Result<Vec<AuditEntry>, DvsError> {
        if !self.path.exists() {
            return Ok(Vec::new());
        }

        let file = fs::File::open(&self.path)?;
        let reader = BufReader::new(file);

        let mut entries = Vec::new();

        for line in reader.lines() {
            let line = line?;
            if line.trim().is_empty() {
                continue;
            }

            let entry: AuditEntry = match serde_json::from_str(&line) {
                Ok(e) => e,
                Err(_) => continue,
            };

            // Apply filters
            if let Some(after_seq) = options.after_seq {
                if entry.seq <= after_seq {
                    continue;
                }
            }

            if let Some(ref min_severity) = options.min_severity {
                if entry.severity < *min_severity {
                    continue;
                }
            }

            if let Some(ref events) = options.events {
                if !events.contains(&entry.event) {
                    continue;
                }
            }

            if let Some(ref prefix) = options.path_prefix {
                if let Some(ref path) = entry.path {
                    if !path.starts_with(prefix) {
                        continue;
                    }
                } else {
                    continue;
                }
            }

            entries.push(entry);

            if let Some(limit) = options.limit {
                if entries.len() >= limit {
                    break;
                }
            }
        }

        Ok(entries)
    }

    /// Get all entries (no filtering).
    pub fn all_entries(&self) -> Result<Vec<AuditEntry>, DvsError> {
        self.query(&AuditQueryOptions::default())
    }

    /// Verify the integrity of the audit log chain.
    ///
    /// Returns Ok(true) if the chain is intact, Ok(false) if tampering is detected.
    pub fn verify_chain(&self) -> Result<bool, DvsError> {
        if !self.path.exists() {
            return Ok(true); // Empty log is valid
        }

        let file = fs::File::open(&self.path)?;
        let reader = BufReader::new(file);

        let mut prev_hash: Option<String> = None;
        let mut prev_seq: Option<u64> = None;

        for line in reader.lines() {
            let line = line?;
            if line.trim().is_empty() {
                continue;
            }

            let entry: AuditEntry = match serde_json::from_str(&line) {
                Ok(e) => e,
                Err(_) => return Ok(false), // Malformed entry
            };

            // Check sequence continuity
            if let Some(prev) = prev_seq {
                if entry.seq != prev + 1 {
                    return Ok(false); // Sequence gap
                }
            }

            // Check hash chain
            if entry.prev_hash != prev_hash {
                return Ok(false); // Hash chain broken
            }

            prev_hash = Some(entry.compute_hash());
            prev_seq = Some(entry.seq);
        }

        Ok(true)
    }
}

/// Get the current user/actor for audit entries.
pub fn current_actor() -> String {
    // Try to get username from environment
    std::env::var("USER")
        .or_else(|_| std::env::var("USERNAME"))
        .unwrap_or_else(|_| "unknown".to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn setup_test_log() -> (TempDir, AuditLog) {
        let temp = TempDir::new().unwrap();
        let log_path = temp.path().join("audit.jsonl");
        let log = AuditLog::open(log_path).unwrap();
        (temp, log)
    }

    #[test]
    fn test_audit_log_append() {
        let (_temp, mut log) = setup_test_log();

        log.log_event(
            "testuser".to_string(),
            AuditEvent::FileAdded,
            Some(PathBuf::from("data.csv")),
            None,
        )
        .unwrap();

        let entries = log.all_entries().unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].actor, "testuser");
        assert_eq!(entries[0].event, AuditEvent::FileAdded);
    }

    #[test]
    fn test_audit_log_chain() {
        let (_temp, mut log) = setup_test_log();

        log.log_event("user".to_string(), AuditEvent::FileAdded, None, None)
            .unwrap();
        log.log_event("user".to_string(), AuditEvent::FileRetrieved, None, None)
            .unwrap();
        log.log_event(
            "user".to_string(),
            AuditEvent::OperationCompleted,
            None,
            None,
        )
        .unwrap();

        let entries = log.all_entries().unwrap();
        assert_eq!(entries.len(), 3);

        // Verify sequence numbers
        assert_eq!(entries[0].seq, 0);
        assert_eq!(entries[1].seq, 1);
        assert_eq!(entries[2].seq, 2);

        // Verify chain
        assert!(entries[0].prev_hash.is_none());
        assert!(entries[1].prev_hash.is_some());
        assert!(entries[2].prev_hash.is_some());

        // Verify chain integrity
        assert!(log.verify_chain().unwrap());
    }

    #[test]
    fn test_audit_log_query_severity() {
        let (_temp, mut log) = setup_test_log();

        log.log_event("user".to_string(), AuditEvent::FileAdded, None, None)
            .unwrap();
        log.log_security_event(
            "user".to_string(),
            AuditEvent::StorageCorrupted,
            AuditSeverity::Critical,
            None,
            None,
        )
        .unwrap();

        let options = AuditQueryOptions {
            min_severity: Some(AuditSeverity::Critical),
            ..Default::default()
        };
        let entries = log.query(&options).unwrap();

        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].event, AuditEvent::StorageCorrupted);
    }

    #[test]
    fn test_audit_log_persistence() {
        let temp = TempDir::new().unwrap();
        let log_path = temp.path().join("audit.jsonl");

        // Create and write
        {
            let mut log = AuditLog::open(log_path.clone()).unwrap();
            log.log_event("user1".to_string(), AuditEvent::FileAdded, None, None)
                .unwrap();
            log.log_event("user2".to_string(), AuditEvent::FileRetrieved, None, None)
                .unwrap();
        }

        // Reopen and read
        {
            let log = AuditLog::open(log_path).unwrap();
            let entries = log.all_entries().unwrap();
            assert_eq!(entries.len(), 2);
            assert_eq!(entries[0].actor, "user1");
            assert_eq!(entries[1].actor, "user2");
            assert!(log.verify_chain().unwrap());
        }
    }

    #[test]
    fn test_audit_log_reopen_continues_chain() {
        let temp = TempDir::new().unwrap();
        let log_path = temp.path().join("audit.jsonl");

        // Write first entry
        {
            let mut log = AuditLog::open(log_path.clone()).unwrap();
            log.log_event("user".to_string(), AuditEvent::FileAdded, None, None)
                .unwrap();
            assert_eq!(log.seq(), 1);
        }

        // Reopen and write second entry
        {
            let mut log = AuditLog::open(log_path.clone()).unwrap();
            assert_eq!(log.seq(), 1); // Should continue from seq 1
            log.log_event("user".to_string(), AuditEvent::FileRetrieved, None, None)
                .unwrap();
            assert_eq!(log.seq(), 2);
        }

        // Verify chain is intact
        {
            let log = AuditLog::open(log_path).unwrap();
            assert!(log.verify_chain().unwrap());
            let entries = log.all_entries().unwrap();
            assert_eq!(entries.len(), 2);
            assert_eq!(entries[0].seq, 0);
            assert_eq!(entries[1].seq, 1);
        }
    }
}
