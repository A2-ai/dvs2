use std::collections::{BTreeSet, HashMap};
use std::path::{Path, PathBuf};

use anyhow::Result;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::Hashes;
use crate::api::v1::{BatchKind, CompleteBatch, GitContext};
use crate::audit::AuditEntry;
use crate::config::Compression;
use crate::paths::ProjectPath;

pub mod local;
pub mod server;

use local::LocalBackend;
use server::ServerBackend;

/// A batch of transfers: one `add` or one `get`. The server issues the id; the local
/// backend makes one up for its log.
#[derive(Debug, Clone)]
pub struct Batch {
    pub id: Uuid,
    pub kind: BatchKind,
    /// The `add` message, for the local log
    pub message: Option<String>,
    /// What the local backend compresses new content with
    pub compression: Compression,
}

/// One file in a manifest, with every hash the local log records.
#[derive(Debug, Clone)]
pub struct ManifestItem {
    pub path: ProjectPath,
    pub hashes: Hashes,
    pub size: u64,
}

/// Content the project already has: its size and stored size.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PresentSize {
    pub size: u64,
    pub stored_size: u64,
}

/// What a manifest call learned.
#[derive(Debug, Clone)]
pub struct ManifestReply {
    /// Hashes whose bytes must be sent
    pub missing: BTreeSet<String>,
    /// Hashes the project has, by hash
    pub present: HashMap<String, PresentSize>,
    /// What the project compresses with
    pub compression: Compression,
}

/// What storing one content did.
#[derive(Debug, Clone, Copy)]
pub struct Stored {
    /// Unknown when the upload's response was lost but the server has the content
    pub stored_size: Option<u64>,
    pub compression: Compression,
}

/// How much one manifest call may carry.
#[derive(Debug, Clone, Copy)]
pub struct ManifestLimits {
    pub max_entries: usize,
    pub max_body_bytes: usize,
    /// The largest file the server takes
    pub max_blob_size: Option<u64>,
    /// The longest path, in bytes
    pub max_path_bytes: Option<usize>,
}

/// Fetch the content of `hashes` into `target`, decompressed.
pub struct RetrieveRequest<'a> {
    pub hashes: &'a Hashes,
    pub target: &'a Path,
    /// What the `.dvs` file says it was stored with; the server's answer decides
    pub compression: Compression,
    /// The path being fetched, for the audit
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

    /// Check that the current user can read the project, so a dry run fails clearly
    /// instead of reporting everything as missing.
    pub fn check_access(&self) -> Result<()> {
        match self {
            // The filesystem permissions are the access control for local storage.
            Backend::Local(_) => Ok(()),
            Backend::Server(s) => s.check_access(),
        }
    }

    pub fn manifest_limits(&self) -> Result<ManifestLimits> {
        match self {
            Backend::Local(_) => Ok(ManifestLimits {
                max_entries: usize::MAX,
                max_body_bytes: usize::MAX,
                max_blob_size: None,
                max_path_bytes: None,
            }),
            Backend::Server(s) => s.manifest_limits(),
        }
    }

    /// Start an `add` (upload) or a `get` (download). The server checks the role, and
    /// the lock for uploads, before any bytes move.
    pub fn open_batch(
        &self,
        kind: BatchKind,
        message: Option<&str>,
        git: &GitContext,
        compression: Compression,
    ) -> Result<Batch> {
        match self {
            Backend::Local(_) => Ok(Batch {
                id: Uuid::new_v4(),
                kind,
                message: message.map(str::to_owned),
                compression,
            }),
            Backend::Server(s) => s.open_batch(kind, message, git),
        }
    }

    /// Record a placement for every item and learn which contents need their bytes.
    pub fn manifest(&self, batch: &Batch, items: &[ManifestItem]) -> Result<ManifestReply> {
        match self {
            Backend::Local(b) => b.manifest(batch, items),
            Backend::Server(s) => s.manifest(batch, items),
        }
    }

    /// A path whose content the project now has, and whose `.dvs` is written. The
    /// local backend logs it; the server recorded it with the manifest.
    pub fn record(&self, batch: &Batch, item: &ManifestItem) -> Result<()> {
        match self {
            Backend::Local(b) => b.record(batch, item),
            Backend::Server(_) => Ok(()),
        }
    }

    /// Send the bytes of one content listed in the batch's manifest.
    pub fn put(
        &self,
        batch: &Batch,
        hashes: &Hashes,
        source: &Path,
        on_bytes: Option<&(dyn Fn(u64) + Send + Sync)>,
    ) -> Result<Stored> {
        match self {
            Backend::Local(b) => b.put(batch, hashes, source, on_bytes),
            Backend::Server(s) => s.put(batch, hashes, source, on_bytes),
        }
    }

    /// Close the batch with the client's counts.
    pub fn complete(&self, batch: &Batch, counts: &CompleteBatch) -> Result<()> {
        match self {
            Backend::Local(_) => Ok(()),
            Backend::Server(s) => s.complete(batch, counts),
        }
    }

    /// Fetch content into `req.target`. False when the project has no such content.
    pub fn fetch(&self, batch: &Batch, req: RetrieveRequest<'_>) -> Result<bool> {
        match self {
            Backend::Local(b) => b.retrieve(req),
            Backend::Server(s) => s.fetch(batch, req),
        }
    }

    /// Whether the project has the content. Records nothing; for dry runs.
    pub fn exists(&self, hash: &Hashes) -> Result<bool> {
        match self {
            Backend::Local(b) => b.exists(hash),
            Backend::Server(s) => s.exists(hash),
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
