//! File hash caching utilities.
//!
//! Caches file hashes based on mtime+size to avoid re-hashing unchanged files.

use std::path::Path;
use crate::DvsError;

/// Cache entry for a file hash.
#[derive(Debug, Clone)]
pub struct CacheEntry {
    /// File path (relative to repo root).
    pub path: std::path::PathBuf,
    /// Blake3 hash of the file.
    pub hash: String,
    /// File modification time (Unix timestamp).
    pub mtime: i64,
    /// File size in bytes.
    pub size: u64,
}

/// Load the hash cache from disk.
pub fn load_cache(_repo_root: &Path) -> Result<Vec<CacheEntry>, DvsError> {
    todo!("Load hash cache from .dvs/cache")
}

/// Save the hash cache to disk.
pub fn save_cache(_repo_root: &Path, _entries: &[CacheEntry]) -> Result<(), DvsError> {
    todo!("Save hash cache to .dvs/cache")
}

/// Get cached hash if file hasn't changed (mtime+size match).
pub fn get_cached_hash(_path: &Path, _cache: &[CacheEntry]) -> Option<String> {
    todo!("Get cached hash if file unchanged")
}

/// Update or add a cache entry.
pub fn update_cache_entry(_cache: &mut Vec<CacheEntry>, _entry: CacheEntry) {
    todo!("Update cache entry")
}

/// Get cache file path.
pub fn cache_path(repo_root: &Path) -> std::path::PathBuf {
    repo_root.join(".dvs").join("cache")
}
