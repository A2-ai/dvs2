//! Audit log types.
//!
//! Security-focused audit logging for DVS operations and integrity events.

use jiff::Timestamp;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

/// A single audit log entry.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuditEntry {
    /// Timestamp of the event (UTC).
    pub ts: Timestamp,
    /// Who triggered the event (username or process).
    pub actor: String,
    /// Type of event.
    pub event: AuditEvent,
    /// Severity level.
    pub severity: AuditSeverity,
    /// File path involved (if applicable).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub path: Option<PathBuf>,
    /// Additional details about the event.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub details: Option<String>,
    /// Hash of the previous entry (for chain integrity).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub prev_hash: Option<String>,
    /// Sequence number within the log.
    pub seq: u64,
}

impl AuditEntry {
    /// Create a new audit entry.
    pub fn new(actor: String, event: AuditEvent, severity: AuditSeverity) -> Self {
        Self {
            ts: Timestamp::now(),
            actor,
            event,
            severity,
            path: None,
            details: None,
            prev_hash: None,
            seq: 0,
        }
    }

    /// Add a file path to the entry.
    pub fn with_path(mut self, path: PathBuf) -> Self {
        self.path = Some(path);
        self
    }

    /// Add details to the entry.
    pub fn with_details(mut self, details: String) -> Self {
        self.details = Some(details);
        self
    }

    /// Set the previous hash for chain integrity.
    pub fn with_prev_hash(mut self, prev_hash: String) -> Self {
        self.prev_hash = Some(prev_hash);
        self
    }

    /// Set the sequence number.
    pub fn with_seq(mut self, seq: u64) -> Self {
        self.seq = seq;
        self
    }

    /// Compute the hash of this entry for chaining.
    pub fn compute_hash(&self) -> String {
        // Use a simple hash of the serialized entry
        let json = serde_json::to_string(self).unwrap_or_default();
        #[cfg(feature = "blake3")]
        {
            blake3::hash(json.as_bytes()).to_hex().to_string()
        }
        #[cfg(not(feature = "blake3"))]
        {
            // Fallback to a simple hash
            use std::collections::hash_map::DefaultHasher;
            use std::hash::{Hash, Hasher};
            let mut hasher = DefaultHasher::new();
            json.hash(&mut hasher);
            format!("{:016x}", hasher.finish())
        }
    }
}

/// Types of audit events.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AuditEvent {
    /// Storage file is corrupted (hash mismatch).
    StorageCorrupted,
    /// Storage file is missing.
    StorageMissing,
    /// Integrity verification completed.
    IntegrityVerified,
    /// Hash mismatch detected.
    HashMismatch,
    /// Operation completed successfully.
    OperationCompleted,
    /// Operation failed.
    OperationFailed,
    /// File added to DVS.
    FileAdded,
    /// File retrieved from storage.
    FileRetrieved,
    /// File pushed to remote.
    FilePushed,
    /// File pulled from remote.
    FilePulled,
    /// Configuration changed.
    ConfigChanged,
    /// Repository initialized.
    RepoInitialized,
    /// Rollback performed.
    RollbackPerformed,
}

impl AuditEvent {
    /// Get the default severity for this event type.
    pub fn default_severity(&self) -> AuditSeverity {
        match self {
            AuditEvent::StorageCorrupted => AuditSeverity::Critical,
            AuditEvent::StorageMissing => AuditSeverity::Error,
            AuditEvent::HashMismatch => AuditSeverity::Warning,
            AuditEvent::OperationFailed => AuditSeverity::Error,
            AuditEvent::IntegrityVerified
            | AuditEvent::OperationCompleted
            | AuditEvent::FileAdded
            | AuditEvent::FileRetrieved
            | AuditEvent::FilePushed
            | AuditEvent::FilePulled
            | AuditEvent::ConfigChanged
            | AuditEvent::RepoInitialized
            | AuditEvent::RollbackPerformed => AuditSeverity::Info,
        }
    }
}

/// Severity levels for audit events.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, PartialOrd, Ord)]
#[serde(rename_all = "snake_case")]
pub enum AuditSeverity {
    /// Informational event.
    Info,
    /// Warning event (potential issue).
    Warning,
    /// Error event (operation failed).
    Error,
    /// Critical event (integrity issue).
    Critical,
}

impl std::fmt::Display for AuditSeverity {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            AuditSeverity::Info => write!(f, "INFO"),
            AuditSeverity::Warning => write!(f, "WARN"),
            AuditSeverity::Error => write!(f, "ERROR"),
            AuditSeverity::Critical => write!(f, "CRITICAL"),
        }
    }
}

/// Options for querying audit logs.
#[derive(Debug, Clone, Default)]
pub struct AuditQueryOptions {
    /// Filter by minimum severity.
    pub min_severity: Option<AuditSeverity>,
    /// Filter by event types.
    pub events: Option<Vec<AuditEvent>>,
    /// Filter by path prefix.
    pub path_prefix: Option<PathBuf>,
    /// Maximum number of entries to return.
    pub limit: Option<usize>,
    /// Start after this sequence number.
    pub after_seq: Option<u64>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_audit_entry_creation() {
        let entry = AuditEntry::new(
            "testuser".to_string(),
            AuditEvent::FileAdded,
            AuditSeverity::Info,
        )
        .with_path(PathBuf::from("data.csv"))
        .with_details("Added new data file".to_string());

        assert_eq!(entry.actor, "testuser");
        assert_eq!(entry.event, AuditEvent::FileAdded);
        assert_eq!(entry.severity, AuditSeverity::Info);
        assert_eq!(entry.path, Some(PathBuf::from("data.csv")));
        assert!(entry.details.is_some());
    }

    #[test]
    fn test_audit_entry_hash() {
        let entry = AuditEntry::new(
            "testuser".to_string(),
            AuditEvent::FileAdded,
            AuditSeverity::Info,
        );

        let hash1 = entry.compute_hash();
        let hash2 = entry.compute_hash();
        assert_eq!(hash1, hash2, "Hash should be deterministic");
    }

    #[test]
    fn test_event_default_severity() {
        assert_eq!(
            AuditEvent::StorageCorrupted.default_severity(),
            AuditSeverity::Critical
        );
        assert_eq!(
            AuditEvent::FileAdded.default_severity(),
            AuditSeverity::Info
        );
        assert_eq!(
            AuditEvent::HashMismatch.default_severity(),
            AuditSeverity::Warning
        );
    }

    #[test]
    fn test_severity_ordering() {
        assert!(AuditSeverity::Info < AuditSeverity::Warning);
        assert!(AuditSeverity::Warning < AuditSeverity::Error);
        assert!(AuditSeverity::Error < AuditSeverity::Critical);
    }

    #[test]
    fn test_audit_entry_serialization() {
        let entry = AuditEntry::new(
            "testuser".to_string(),
            AuditEvent::FileAdded,
            AuditSeverity::Info,
        )
        .with_path(PathBuf::from("data.csv"));

        let json = serde_json::to_string(&entry).unwrap();
        let parsed: AuditEntry = serde_json::from_str(&json).unwrap();

        assert_eq!(parsed.actor, entry.actor);
        assert_eq!(parsed.event, entry.event);
        assert_eq!(parsed.path, entry.path);
    }
}
