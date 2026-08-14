use std::path::{Path, PathBuf};

use crate::Hashes;
use crate::audit::AuditEntry;
use crate::config::Compression;
use anyhow::Result;
use uuid::Uuid;

pub mod local;
pub mod server;

/// Common fields needed for storing a file across backends
pub struct StoreRequest<'a> {
    pub hashes: &'a Hashes,
    pub source: &'a Path,
    pub compression: Compression,
    pub path: &'a Path,
    pub operation_id: Uuid,
    pub size: u64,
    pub message: Option<&'a str>,
    pub on_bytes: Option<&'a (dyn Fn(u64) + Send + Sync)>,
}

impl<'a> StoreRequest<'a> {
    pub fn new_local(hashes: &'a Hashes, source: &'a Path, compression: Compression) -> Self {
        Self {
            hashes,
            source,
            compression,
            path: Path::new(""),
            operation_id: Uuid::nil(),
            size: 0,
            message: None,
            on_bytes: None,
        }
    }
}

/// Common fields needed for retrieving a file across backends
pub struct RetrieveRequest<'a> {
    pub hashes: &'a Hashes,
    pub target: &'a Path,
    pub compression: Compression,
    pub path: &'a Path,
    pub on_bytes: Option<&'a (dyn Fn(u64) + Send + Sync)>,
}

impl<'a> RetrieveRequest<'a> {
    pub fn new_local(hashes: &'a Hashes, target: &'a Path, compression: Compression) -> Self {
        Self {
            hashes,
            target,
            compression,
            path: Path::new(""),
            on_bytes: None,
        }
    }
}

pub trait Backend: Send + Sync {
    /// Initialize the backend storage
    /// Idempotent: returns `true` if the backend was already initialized.
    fn init(&self, compression: Compression) -> Result<bool>;

    /// Check that the current user can access the backend.
    /// Called once before batch operations so auth/permission problems fail
    /// fast with a single error instead of once per file.
    fn check_access(&self) -> Result<()> {
        Ok(())
    }

    /// Store file to backend by hash, optionally compressing.
    /// Returns the stored (compressed) size in bytes.
    fn store(&self, req: StoreRequest<'_>) -> Result<u64>;

    /// Retrieve content by hash to target path, optionally decompressing.
    /// Returns true if the file was copied to the target path.
    fn retrieve(&self, req: RetrieveRequest<'_>) -> Result<bool>;

    /// Check if the file exists in the backend
    fn exists(&self, hash: &Hashes) -> Result<bool>;

    /// Remove content by hash (for rollback). Best-effort, may silently fail.
    fn remove(&self, hash: &Hashes) -> Result<()>;

    /// Read the whole audit file, filtered by the given file paths.
    /// If `files` is empty, return the full audit log
    fn get_audit_entries(&self, files: &[PathBuf]) -> Result<Vec<AuditEntry>>;
}
