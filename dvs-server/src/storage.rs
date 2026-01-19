//! Storage backend abstraction.

use std::path::{Path, PathBuf};
use crate::ServerError;

/// Storage backend trait for file storage operations.
pub trait StorageBackend: Send + Sync {
    /// Check if a file with the given hash exists.
    fn exists(&self, hash: &str) -> Result<bool, ServerError>;

    /// Get the path to a file by hash.
    fn get_path(&self, hash: &str) -> Result<PathBuf, ServerError>;

    /// Store a file and return its hash.
    fn store(&self, data: &[u8]) -> Result<String, ServerError>;

    /// Delete a file by hash.
    fn delete(&self, hash: &str) -> Result<(), ServerError>;

    /// Get storage statistics.
    fn stats(&self) -> Result<StorageStats, ServerError>;
}

/// Storage statistics.
#[derive(Debug, Clone, serde::Serialize)]
pub struct StorageStats {
    /// Total bytes used.
    pub bytes_used: u64,
    /// Number of files stored.
    pub file_count: u64,
    /// Total capacity (if known).
    pub capacity: Option<u64>,
}

/// Local filesystem storage backend.
pub struct LocalStorage {
    _root: PathBuf,
}

impl LocalStorage {
    /// Create a new local storage backend.
    pub fn new(_root: PathBuf) -> Result<Self, ServerError> {
        todo!("Create local storage backend")
    }

    /// Get storage path for a hash.
    fn hash_to_path(&self, _hash: &str) -> PathBuf {
        todo!("Convert hash to storage path")
    }
}

impl StorageBackend for LocalStorage {
    fn exists(&self, _hash: &str) -> Result<bool, ServerError> {
        todo!("Check if file exists")
    }

    fn get_path(&self, _hash: &str) -> Result<PathBuf, ServerError> {
        todo!("Get path for hash")
    }

    fn store(&self, _data: &[u8]) -> Result<String, ServerError> {
        todo!("Store file data")
    }

    fn delete(&self, _hash: &str) -> Result<(), ServerError> {
        todo!("Delete file")
    }

    fn stats(&self) -> Result<StorageStats, ServerError> {
        todo!("Get storage stats")
    }
}

/// Validate a blake3 hash string.
pub fn validate_hash(_hash: &str) -> bool {
    todo!("Validate hash format")
}

/// Calculate blake3 hash of data.
pub fn hash_data(_data: &[u8]) -> String {
    todo!("Hash data")
}
