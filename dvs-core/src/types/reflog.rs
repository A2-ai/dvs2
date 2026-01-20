//! Reflog types for workspace state tracking.
//!
//! Provides types for capturing and restoring workspace state:
//! - `WorkspaceState`: A snapshot of DVS-tracked state
//! - `ReflogEntry`: A log entry recording state changes

use super::{Manifest, Metadata};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

/// A metadata entry with its associated path.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MetadataEntry {
    /// Repo-relative path to the data file.
    pub path: PathBuf,
    /// The metadata for this file.
    pub meta: Metadata,
}

impl MetadataEntry {
    /// Create a new metadata entry.
    pub fn new(path: PathBuf, meta: Metadata) -> Self {
        Self { path, meta }
    }
}

/// A snapshot of workspace state.
///
/// Captures the DVS-tracked state at a point in time, including
/// the manifest and all metadata files.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkspaceState {
    /// Schema version.
    pub version: u32,

    /// The manifest contents (dvs.lock), if present.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub manifest: Option<Manifest>,

    /// Metadata entries, sorted by path for deterministic hashing.
    pub metadata: Vec<MetadataEntry>,
}

impl WorkspaceState {
    /// Schema version for workspace state.
    pub const VERSION: u32 = 1;

    /// Create a new workspace state.
    pub fn new(manifest: Option<Manifest>, mut metadata: Vec<MetadataEntry>) -> Self {
        // Sort metadata by path for deterministic hashing
        metadata.sort_by(|a, b| a.path.cmp(&b.path));

        Self {
            version: Self::VERSION,
            manifest,
            metadata,
        }
    }

    /// Create an empty workspace state.
    pub fn empty() -> Self {
        Self {
            version: Self::VERSION,
            manifest: None,
            metadata: Vec::new(),
        }
    }

    /// Check if this state is empty.
    pub fn is_empty(&self) -> bool {
        self.manifest.as_ref().map_or(true, |m| m.is_empty()) && self.metadata.is_empty()
    }

    /// Serialize to canonical JSON for hashing.
    ///
    /// Uses a deterministic serialization to ensure the same state
    /// always produces the same hash.
    pub fn to_canonical_json(&self) -> Result<String, crate::DvsError> {
        // Use serde_json with sorted keys for determinism
        let json = serde_json::to_string(self)?;
        Ok(json)
    }

    /// Compute the state ID (blake3 hash of canonical JSON).
    pub fn compute_id(&self) -> Result<String, crate::DvsError> {
        let json = self.to_canonical_json()?;
        crate::helpers::hash::hash_bytes(json.as_bytes(), crate::HashAlgo::Blake3)
    }
}

/// Operation type for reflog entries.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ReflogOp {
    /// Files were added or updated.
    Add,
    /// Files were removed.
    Remove,
    /// State was rolled back.
    Rollback,
    /// Initial state.
    Init,
}

impl std::fmt::Display for ReflogOp {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ReflogOp::Add => write!(f, "add"),
            ReflogOp::Remove => write!(f, "remove"),
            ReflogOp::Rollback => write!(f, "rollback"),
            ReflogOp::Init => write!(f, "init"),
        }
    }
}

/// A single entry in the reflog.
///
/// Records a state transition in JSONL format.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReflogEntry {
    /// Timestamp of the operation.
    pub ts: DateTime<Utc>,

    /// User who performed the operation.
    pub actor: String,

    /// Type of operation.
    pub op: ReflogOp,

    /// Optional message describing the change.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,

    /// Previous state ID (state:<id> format), None for first entry.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub old: Option<String>,

    /// New state ID (state:<id> format).
    pub new: String,

    /// Paths affected by this operation.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub paths: Vec<PathBuf>,
}

impl ReflogEntry {
    /// Create a new reflog entry.
    pub fn new(
        actor: String,
        op: ReflogOp,
        message: Option<String>,
        old: Option<String>,
        new: String,
        paths: Vec<PathBuf>,
    ) -> Self {
        Self {
            ts: Utc::now(),
            actor,
            op,
            message,
            old,
            new,
            paths,
        }
    }

    /// Format state ID with prefix.
    pub fn format_state_id(id: &str) -> String {
        format!("state:{}", id)
    }

    /// Parse state ID, removing the prefix.
    pub fn parse_state_id(s: &str) -> Option<&str> {
        s.strip_prefix("state:")
    }

    /// Serialize to a single JSONL line.
    pub fn to_jsonl(&self) -> Result<String, crate::DvsError> {
        let json = serde_json::to_string(self)?;
        Ok(json)
    }

    /// Parse from a JSONL line.
    pub fn from_jsonl(line: &str) -> Result<Self, crate::DvsError> {
        let entry: ReflogEntry = serde_json::from_str(line)?;
        Ok(entry)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_metadata() -> Metadata {
        Metadata::new(
            "a".repeat(64),
            1024,
            Some("test".to_string()),
            "testuser".to_string(),
        )
    }

    #[test]
    fn test_metadata_entry() {
        let entry = MetadataEntry::new(PathBuf::from("data/file.csv"), test_metadata());
        assert_eq!(entry.path, PathBuf::from("data/file.csv"));
    }

    #[test]
    fn test_workspace_state_new() {
        let entries = vec![
            MetadataEntry::new(PathBuf::from("b.csv"), test_metadata()),
            MetadataEntry::new(PathBuf::from("a.csv"), test_metadata()),
        ];
        let state = WorkspaceState::new(None, entries);

        // Should be sorted by path
        assert_eq!(state.metadata[0].path, PathBuf::from("a.csv"));
        assert_eq!(state.metadata[1].path, PathBuf::from("b.csv"));
    }

    #[test]
    fn test_workspace_state_empty() {
        let state = WorkspaceState::empty();
        assert!(state.is_empty());
        assert!(state.manifest.is_none());
        assert!(state.metadata.is_empty());
    }

    #[test]
    fn test_workspace_state_compute_id() {
        let state = WorkspaceState::empty();
        let id = state.compute_id().unwrap();

        // ID should be a valid blake3 hex string
        assert_eq!(id.len(), 64);
        assert!(id.chars().all(|c| c.is_ascii_hexdigit()));

        // Same state should produce same ID
        let state2 = WorkspaceState::empty();
        assert_eq!(state.compute_id().unwrap(), state2.compute_id().unwrap());
    }

    #[test]
    fn test_reflog_op_display() {
        assert_eq!(ReflogOp::Add.to_string(), "add");
        assert_eq!(ReflogOp::Remove.to_string(), "remove");
        assert_eq!(ReflogOp::Rollback.to_string(), "rollback");
        assert_eq!(ReflogOp::Init.to_string(), "init");
    }

    #[test]
    fn test_reflog_entry_new() {
        let entry = ReflogEntry::new(
            "alice".to_string(),
            ReflogOp::Add,
            Some("added training data".to_string()),
            Some("state:abc123".to_string()),
            "state:def456".to_string(),
            vec![PathBuf::from("data/train.csv")],
        );

        assert_eq!(entry.actor, "alice");
        assert_eq!(entry.op, ReflogOp::Add);
        assert_eq!(entry.message, Some("added training data".to_string()));
        assert_eq!(entry.paths.len(), 1);
    }

    #[test]
    fn test_reflog_entry_state_id() {
        assert_eq!(ReflogEntry::format_state_id("abc123"), "state:abc123");
        assert_eq!(ReflogEntry::parse_state_id("state:abc123"), Some("abc123"));
        assert_eq!(ReflogEntry::parse_state_id("abc123"), None);
    }

    #[test]
    fn test_reflog_entry_jsonl_roundtrip() {
        let entry = ReflogEntry::new(
            "bob".to_string(),
            ReflogOp::Rollback,
            None,
            Some("state:old".to_string()),
            "state:new".to_string(),
            vec![],
        );

        let jsonl = entry.to_jsonl().unwrap();
        let parsed = ReflogEntry::from_jsonl(&jsonl).unwrap();

        assert_eq!(parsed.actor, entry.actor);
        assert_eq!(parsed.op, entry.op);
        assert_eq!(parsed.old, entry.old);
        assert_eq!(parsed.new, entry.new);
    }
}
