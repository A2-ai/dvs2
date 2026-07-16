use std::path::{Path, PathBuf};

use crate::Hashes;
use crate::audit::AuditEntry;
use crate::config::Compression;
use anyhow::Result;

pub mod local;

pub trait Backend: Send + Sync {
    /// Check whether the backend has already been initialized.
    fn is_initialized(&self) -> Result<bool>;

    /// Initialize the backend storage (create directories, set permissions, etc.)
    fn init(&self) -> Result<()>;

    /// Store file to backend by hash, optionally compressing.
    /// Returns the stored (compressed) size in bytes.
    fn store(
        &self,
        hash: &Hashes,
        source: &Path,
        compression: Compression,
        on_bytes: Option<&(dyn Fn(u64) + Send + Sync)>,
    ) -> Result<u64>;

    /// Retrieve content by hash to target path, optionally decompressing.
    /// Returns true if the file was copied to the target path.
    fn retrieve(
        &self,
        hash: &Hashes,
        target: &Path,
        compression: Compression,
        on_bytes: Option<&(dyn Fn(u64) + Send + Sync)>,
    ) -> Result<bool>;

    /// Check if the file exists in the backend
    fn exists(&self, hash: &Hashes) -> Result<bool>;

    /// Remove content by hash. Removing a missing blob is a no-op.
    /// Must not be used for rollback: a blob may be referenced by a
    /// concurrent add.
    fn remove(&self, hash: &Hashes) -> Result<()>;

    /// Log an audit entry to the backend's audit log.
    fn log_audit(&self, entry: &AuditEntry) -> Result<()>;

    /// Read the whole audit file, filtered by the given file paths.
    /// If `files` is empty, return the full audit log
    fn read_audit_file(&self, files: &[PathBuf]) -> Result<Vec<AuditEntry>>;

    /// For backends backed by a local filesystem directory, the storage path.
    /// `None` for backends where "inside the repository" is not meaningful
    /// (e.g. a future remote backend). Used by `init` to reject storage that
    /// lives inside the repo root.
    fn local_path(&self) -> Option<&Path> {
        None
    }
}
