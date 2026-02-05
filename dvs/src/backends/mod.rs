use std::path::{Path, PathBuf};

use crate::Hashes;
use crate::audit::AuditEntry;
use anyhow::Result;

pub mod local;

pub trait Backend: Send + Sync {
    /// Initialize the backend storage (create directories, set permissions, etc.)
    fn init(&self) -> Result<()>;

    /// Store file to backend by hash.
    fn store(&self, hash: &Hashes, source: &Path) -> Result<()>;

    /// Store raw bytes to backend by hash (for rollback).
    fn store_bytes(&self, hash: &Hashes, content: &[u8]) -> Result<()>;

    /// Retrieve content by hash to target path. Returns true if the file was copied to the target
    /// path.
    fn retrieve(&self, hash: &Hashes, target: &Path) -> Result<bool>;

    /// Check if the file exists in the backend
    fn exists(&self, hash: &Hashes) -> Result<bool>;

    /// Remove content by hash (for rollback). Best-effort, may silently fail.
    fn remove(&self, hash: &Hashes) -> Result<()>;

    /// Read content by hash. Returns None if not found.
    fn read(&self, hash: &Hashes) -> Result<Option<Vec<u8>>>;

    /// Log an audit entry to the backend's audit log.
    fn log_audit(&self, entry: &AuditEntry) -> Result<()>;

    /// Read the whole audit file, filtered by the given file paths.
    /// If `files` is empty, return the full audit log
    fn read_audit_file(&self, files: &[PathBuf]) -> Result<Vec<AuditEntry>>;
}
