//! DVS get operation.

use std::path::PathBuf;
use crate::{GetResult, DvsError, Backend, detect_backend_cwd};

/// Retrieve files from DVS storage.
///
/// Reads metadata files, checks local file hashes, and copies from storage if needed.
///
/// # Arguments
///
/// * `files` - File paths or glob patterns to retrieve
///
/// # Returns
///
/// A vector of results, one per file (including errors).
///
/// # Errors
///
/// * `NotInitialized` - DVS not initialized
/// * `BatchError` - Multiple explicit paths don't have metadata
pub fn get(files: &[PathBuf]) -> Result<Vec<GetResult>, DvsError> {
    let backend = detect_backend_cwd()?;
    get_with_backend(&backend, files)
}

/// Retrieve files with a specific backend.
///
/// Use this when you already have a backend reference.
pub fn get_with_backend(
    _backend: &Backend,
    _files: &[PathBuf],
) -> Result<Vec<GetResult>, DvsError> {
    todo!("Implement get operation with backend")
}

/// Expand glob patterns to tracked files only.
fn expand_globs_tracked(_backend: &Backend, _patterns: &[PathBuf]) -> Result<Vec<PathBuf>, DvsError> {
    todo!("Expand glob patterns to tracked files")
}

/// Process a single file for retrieval.
fn get_single_file(_backend: &Backend, _path: &std::path::Path, _config: &crate::Config) -> GetResult {
    todo!("Get single file")
}

/// Check if local file matches metadata hash.
fn file_matches_metadata(
    _local_path: &std::path::Path,
    _metadata: &crate::Metadata,
) -> Result<bool, DvsError> {
    todo!("Check if file matches metadata")
}

/// Copy file from storage to local path.
fn copy_from_storage(
    _storage_path: &std::path::Path,
    _local_path: &std::path::Path,
) -> Result<(), DvsError> {
    todo!("Copy from storage")
}
