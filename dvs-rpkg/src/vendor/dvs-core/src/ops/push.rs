//! DVS push operation - upload objects to remote storage.
//!
//! Note: Remote storage is not yet implemented. These functions return errors
//! indicating that remote push is not available.

use crate::{DvsError, Oid};
use std::path::PathBuf;

/// Result of a push operation for a single object.
#[derive(Debug, Clone)]
pub struct PushResult {
    /// Object ID.
    pub oid: Oid,
    /// Whether the object was uploaded (false = already present).
    pub uploaded: bool,
    /// Error message if failed.
    pub error: Option<String>,
}

impl PushResult {
    /// Create a successful push result.
    pub fn success(oid: Oid, uploaded: bool) -> Self {
        Self {
            oid,
            uploaded,
            error: None,
        }
    }

    /// Create an error push result.
    pub fn error(oid: Oid, message: String) -> Self {
        Self {
            oid,
            uploaded: false,
            error: Some(message),
        }
    }

    /// Check if this result is an error.
    pub fn is_error(&self) -> bool {
        self.error.is_some()
    }
}

/// Summary of a push operation.
#[derive(Debug, Clone, Default)]
pub struct PushSummary {
    /// Number of objects uploaded.
    pub uploaded: usize,
    /// Number of objects already present.
    pub present: usize,
    /// Number of objects that failed.
    pub failed: usize,
    /// Individual results.
    pub results: Vec<PushResult>,
}

/// Push objects to remote storage.
///
/// Note: Remote storage is not yet implemented.
pub fn push(_remote_url: Option<&str>) -> Result<PushSummary, DvsError> {
    Err(DvsError::config_error(
        "Remote storage is not yet implemented. Push is not available.",
    ))
}

/// Push specific files by path.
///
/// Note: Remote storage is not yet implemented.
pub fn push_files(_files: &[PathBuf], _remote_url: Option<&str>) -> Result<PushSummary, DvsError> {
    Err(DvsError::config_error(
        "Remote storage is not yet implemented. Push is not available.",
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_push_result_success() {
        let oid = Oid::blake3("a".repeat(64));
        let result = PushResult::success(oid.clone(), true);
        assert!(result.uploaded);
        assert!(!result.is_error());
    }

    #[test]
    fn test_push_result_error() {
        let oid = Oid::blake3("a".repeat(64));
        let result = PushResult::error(oid.clone(), "test error".to_string());
        assert!(!result.uploaded);
        assert!(result.is_error());
        assert_eq!(result.error.unwrap(), "test error");
    }

    #[test]
    fn test_push_summary_default() {
        let summary = PushSummary::default();
        assert_eq!(summary.uploaded, 0);
        assert_eq!(summary.present, 0);
        assert_eq!(summary.failed, 0);
        assert!(summary.results.is_empty());
    }

    #[test]
    fn test_push_not_implemented() {
        let result = push(None);
        assert!(result.is_err());
        assert!(result
            .unwrap_err()
            .to_string()
            .contains("not yet implemented"));
    }
}
