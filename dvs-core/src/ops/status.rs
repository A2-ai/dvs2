//! DVS status operation.

use std::path::PathBuf;
use crate::{StatusResult, DvsError};

/// Check status of tracked files.
///
/// Compares local file hashes with stored metadata.
///
/// # Arguments
///
/// * `files` - File paths or glob patterns to check (empty = all tracked files)
///
/// # Returns
///
/// A vector of results, one per file (including errors).
///
/// # Errors
///
/// * `NotInitialized` - DVS not initialized
pub fn status(files: &[PathBuf]) -> Result<Vec<StatusResult>, DvsError> {
    todo!("Implement status operation")
}

/// Find all tracked files in the repository.
fn find_all_tracked_files() -> Result<Vec<PathBuf>, DvsError> {
    todo!("Find all tracked files")
}

/// Check status of a single file.
fn status_single_file(path: &std::path::Path, config: &crate::Config) -> StatusResult {
    todo!("Check status of single file")
}

/// Determine the status of a file by comparing hashes.
fn determine_status(
    _local_path: &std::path::Path,
    _metadata: &crate::Metadata,
) -> Result<crate::FileStatus, DvsError> {
    todo!("Determine file status")
}
