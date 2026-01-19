//! DVS add operation.

use std::path::PathBuf;
use crate::{AddResult, DvsError};

/// Add files to DVS tracking.
///
/// Computes hashes, creates metadata files, and copies files to storage.
///
/// # Arguments
///
/// * `files` - File paths or glob patterns to add
/// * `message` - Optional message describing this version
///
/// # Returns
///
/// A vector of results, one per file (including errors).
///
/// # Errors
///
/// * `NotInitialized` - DVS not initialized
/// * `BatchError` - Multiple explicit paths don't exist
pub fn add(files: &[PathBuf], message: Option<&str>) -> Result<Vec<AddResult>, DvsError> {
    todo!("Implement add operation")
}

/// Expand glob patterns and filter files.
fn expand_globs(patterns: &[PathBuf]) -> Result<Vec<PathBuf>, DvsError> {
    todo!("Expand glob patterns")
}

/// Process a single file for adding.
fn add_single_file(
    path: &std::path::Path,
    message: Option<&str>,
    config: &crate::Config,
) -> AddResult {
    todo!("Add single file")
}

/// Compute the storage path for a file hash.
fn storage_path_for_hash(storage_dir: &std::path::Path, hash: &str) -> PathBuf {
    todo!("Compute storage path from hash")
}

/// Rollback metadata and storage on error.
fn rollback_add(
    _metadata_path: &std::path::Path,
    _storage_path: &std::path::Path,
) -> Result<(), DvsError> {
    todo!("Rollback failed add")
}
