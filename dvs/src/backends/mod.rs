use std::path::Path;

use anyhow::Result;

pub mod local;

pub trait Backend: Send + Sync {
    /// Initialize the backend storage (create directories, set permissions, etc.)
    fn init(&self) -> Result<()>;

    /// Store file to backend by hash.
    fn store(&self, hash: &str, source: &Path) -> Result<()>;

    /// Store raw bytes to backend by hash (for rollback).
    fn store_bytes(&self, hash: &str, content: &[u8]) -> Result<()>;

    /// Retrieve content by hash to target path. Returns true if the file was copied to the target
    /// path.
    fn retrieve(&self, hash: &str, target: &Path) -> Result<bool>;

    /// Check if the file exists in the backend
    fn exists(&self, hash: &str) -> Result<bool>;

    /// Remove content by hash (for rollback). Best-effort, may silently fail.
    fn remove(&self, hash: &str) -> Result<()>;

    /// Read content by hash. Returns None if not found.
    fn read(&self, hash: &str) -> Result<Option<Vec<u8>>>;
}
