//! DVS pull operation - download objects from remote storage.
//!
//! Note: Remote storage is not yet implemented. These functions return errors
//! indicating that remote pull is not available.

use crate::{DvsError, Oid};
use std::path::PathBuf;

/// Result of a pull operation for a single object.
#[derive(Debug, Clone)]
pub struct PullResult {
    /// Object ID.
    pub oid: Oid,
    /// Whether the object was downloaded (false = already cached).
    pub downloaded: bool,
    /// Error message if failed.
    pub error: Option<String>,
}

impl PullResult {
    /// Create a successful pull result.
    pub fn success(oid: Oid, downloaded: bool) -> Self {
        Self {
            oid,
            downloaded,
            error: None,
        }
    }

    /// Create an error pull result.
    pub fn error(oid: Oid, message: String) -> Self {
        Self {
            oid,
            downloaded: false,
            error: Some(message),
        }
    }

    /// Check if this result is an error.
    pub fn is_error(&self) -> bool {
        self.error.is_some()
    }
}

/// Summary of a pull operation.
#[derive(Debug, Clone, Default)]
pub struct PullSummary {
    /// Number of objects downloaded.
    pub downloaded: usize,
    /// Number of objects already cached.
    pub cached: usize,
    /// Number of objects that failed.
    pub failed: usize,
    /// Individual results.
    pub results: Vec<PullResult>,
}

/// Pull objects from remote storage.
///
/// Note: Remote storage is not yet implemented.
pub fn pull(_remote_url: Option<&str>) -> Result<PullSummary, DvsError> {
    Err(DvsError::config_error(
        "Remote storage is not yet implemented. Pull is not available.",
    ))
}

/// Pull specific files by path.
///
/// Note: Remote storage is not yet implemented.
pub fn pull_files(_files: &[PathBuf], _remote_url: Option<&str>) -> Result<PullSummary, DvsError> {
    Err(DvsError::config_error(
        "Remote storage is not yet implemented. Pull is not available.",
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_pull_result_success() {
        let oid = Oid::blake3("a".repeat(64));
        let result = PullResult::success(oid.clone(), true);
        assert!(result.downloaded);
        assert!(!result.is_error());
    }

    #[test]
    fn test_pull_result_error() {
        let oid = Oid::blake3("a".repeat(64));
        let result = PullResult::error(oid.clone(), "test error".to_string());
        assert!(!result.downloaded);
        assert!(result.is_error());
        assert_eq!(result.error.unwrap(), "test error");
    }

    #[test]
    fn test_pull_summary_default() {
        let summary = PullSummary::default();
        assert_eq!(summary.downloaded, 0);
        assert_eq!(summary.cached, 0);
        assert_eq!(summary.failed, 0);
        assert!(summary.results.is_empty());
    }

    #[test]
    fn test_pull_not_implemented() {
        let result = pull(None);
        assert!(result.is_err());
        assert!(result
            .unwrap_err()
            .to_string()
            .contains("not yet implemented"));
    }
}
