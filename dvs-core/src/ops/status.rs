//! DVS status operation.

use std::path::PathBuf;
use crate::{StatusResult, DvsError, Backend, detect_backend_cwd};

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
    let backend = detect_backend_cwd()?;
    status_with_backend(&backend, files)
}

/// Check status with a specific backend.
///
/// Use this when you already have a backend reference.
pub fn status_with_backend(
    _backend: &Backend,
    _files: &[PathBuf],
) -> Result<Vec<StatusResult>, DvsError> {
    todo!("Implement status operation with backend")
}

/// Find all tracked files in the repository.
fn find_all_tracked_files(_backend: &Backend) -> Result<Vec<PathBuf>, DvsError> {
    todo!("Find all tracked files")
}

/// Check status of a single file.
fn status_single_file(_backend: &Backend, _path: &std::path::Path, _config: &crate::Config) -> StatusResult {
    todo!("Check status of single file")
}

/// Determine the status of a file by comparing hashes.
fn determine_status(
    _local_path: &std::path::Path,
    _metadata: &crate::Metadata,
) -> Result<crate::FileStatus, DvsError> {
    todo!("Determine file status")
}
