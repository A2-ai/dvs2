//! DVS Core Library
//!
//! Pure Rust implementation of the Data Versioning System.
//! This crate provides the core business logic without any I/O bindings.
//!
//! # Architecture
//!
//! - `types`: Core data types (Config, Metadata, FileInfo, etc.)
//! - `ops`: High-level operations (init, add, get, status)
//! - `helpers`: Low-level utilities (hashing, file ops, config parsing)

pub mod types;
pub mod ops;
pub mod helpers;

// Re-export commonly used types at crate root
pub use types::{
    Config,
    Metadata,
    FileInfo,
    AddResult,
    GetResult,
    StatusResult,
    Outcome,
    FileStatus,
    DvsError,
    Oid,
    HashAlgo,
    Manifest,
    ManifestEntry,
    Compression,
};

// Re-export operations at crate root
pub use ops::{init, init_with_backend, add, add_with_backend, get, get_with_backend, status, status_with_backend};
pub use ops::{push, push_with_backend, push_files, PushResult, PushSummary};
pub use ops::{pull, pull_with_backend, pull_files, PullResult, PullSummary};
pub use ops::{materialize, materialize_with_backend, materialize_files, MaterializeResult, MaterializeSummary};
pub use ops::{log, log_with_backend, log_entry, log_entry_with_backend, LogEntry};
pub use ops::{rollback, rollback_with_backend, RollbackTarget, RollbackResult};

// Re-export backend types
pub use helpers::backend::{Backend, RepoBackend, GitBackend, DvsBackend, detect_backend, detect_backend_cwd};

// Re-export store types for testing
pub use helpers::store::{LocalStore, ObjectStore};
