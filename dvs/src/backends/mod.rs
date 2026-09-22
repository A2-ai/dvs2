use std::path::{Path, PathBuf};

use anyhow::Result;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::Hashes;
use crate::audit::AuditEntry;
use crate::config::Compression;
use crate::paths::ProjectPath;

pub mod local;
pub mod server;

use local::LocalBackend;
use server::ServerBackend;

/// Common fields needed for storing a file across backends
pub struct StoreRequest<'a> {
    pub hashes: &'a Hashes,
    pub source: &'a Path,
    /// The compression the caller would like. It could be overriden eg in the case of a dvs
    /// server backend if the project was init with a different alg
    pub compression: Compression,
    pub path: ProjectPath,
    pub operation_id: Uuid,
    pub message: Option<&'a str>,
    pub on_bytes: Option<&'a (dyn Fn(u64) + Send + Sync)>,
}

impl<'a> StoreRequest<'a> {
    pub fn new_local(
        hashes: &'a Hashes,
        source: &'a Path,
        path: ProjectPath,
        compression: Compression,
    ) -> Self {
        Self {
            hashes,
            source,
            compression,
            path,
            operation_id: Uuid::nil(),
            message: None,
            on_bytes: None,
        }
    }
}

/// What the backend actually did with the file.
pub struct StoreResult {
    /// Size of the stored blob, after compression.
    pub stored_size: u64,
    /// The compression that was actually applied.
    pub compression: Compression,
}

/// Common fields needed for retrieving a file across backends
pub struct RetrieveRequest<'a> {
    pub hashes: &'a Hashes,
    pub target: &'a Path,
    pub compression: Compression,
    pub path: ProjectPath,
    pub on_bytes: Option<&'a (dyn Fn(u64) + Send + Sync)>,
}

impl<'a> RetrieveRequest<'a> {
    pub fn new_local(
        hashes: &'a Hashes,
        target: &'a Path,
        path: ProjectPath,
        compression: Compression,
    ) -> Self {
        Self {
            hashes,
            target,
            compression,
            path,
            on_bytes: None,
        }
    }
}

#[derive(Debug, Serialize, Deserialize, Clone, PartialEq)]
#[serde(untagged)]
pub enum Backend {
    Local(LocalBackend),
    Server(ServerBackend),
}

impl Backend {
    /// Initialize the backend storage
    /// Idempotent: returns `true` if the backend was already initialized.
    pub fn init(&self, compression: Compression) -> Result<bool> {
        match self {
            Backend::Local(b) => b.init(compression),
            Backend::Server(s) => s.init(compression),
        }
    }

    /// Check that the current user can access the backend.
    /// Called once before batch operations so auth/permission problems fail
    /// fast with a single error instead of once per file.
    pub fn check_access(&self) -> Result<()> {
        match self {
            // The filesystem permissions are the access control for local storage.
            Backend::Local(_) => Ok(()),
            Backend::Server(s) => s.check_access(),
        }
    }

    /// Store file to backend under `req.hashes`, compressing it.
    /// The source is passed uncompressed, the hash we send is only for (optional) verification by
    /// a server
    pub fn store(&self, req: StoreRequest<'_>) -> Result<StoreResult> {
        match self {
            Backend::Local(b) => b.store(req),
            Backend::Server(s) => s.store(req),
        }
    }

    /// Retrieve content by hash to target path, optionally decompressing.
    /// Returns true if the file was copied to the target path.
    pub fn retrieve(&self, req: RetrieveRequest<'_>) -> Result<bool> {
        match self {
            Backend::Local(b) => b.retrieve(req),
            Backend::Server(s) => s.retrieve(req),
        }
    }

    /// Check if the file exists in the backend
    pub fn exists(&self, hash: &Hashes) -> Result<bool> {
        match self {
            Backend::Local(b) => b.exists(hash),
            Backend::Server(s) => s.exists(hash),
        }
    }

    /// Remove content by hash (for rollback). Best-effort, may silently fail.
    pub fn remove(&self, hash: &Hashes) -> Result<()> {
        match self {
            Backend::Local(b) => b.remove(hash),
            Backend::Server(s) => s.remove(hash),
        }
    }

    /// Read the whole audit file, filtered by the given file paths.
    /// If `files` is empty, return the full audit log
    pub fn get_audit_entries(&self, files: &[PathBuf]) -> Result<Vec<AuditEntry>> {
        match self {
            Backend::Local(b) => b.get_audit_entries(files),
            Backend::Server(s) => s.get_audit_entries(files),
        }
    }
}
