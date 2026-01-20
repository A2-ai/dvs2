//! Glob pattern and path parsing utilities.

use crate::DvsError;
use std::path::PathBuf;

/// Expand glob patterns into file paths.
///
/// Returns absolute paths for all matching files.
pub fn expand_globs(_patterns: &[PathBuf]) -> Result<Vec<PathBuf>, DvsError> {
    todo!("Expand glob patterns")
}

/// Check if a path matches any glob pattern.
pub fn matches_glob(_path: &std::path::Path, _pattern: &str) -> bool {
    todo!("Check if path matches glob pattern")
}

/// Normalize a path relative to the repository root.
pub fn normalize_path(
    _path: &std::path::Path,
    _repo_root: &std::path::Path,
) -> Result<PathBuf, DvsError> {
    todo!("Normalize path relative to repo root")
}

/// Parse a file size string (e.g., "10MB") into bytes.
pub fn parse_size(_size_str: &str) -> Result<u64, DvsError> {
    todo!("Parse size string into bytes")
}
