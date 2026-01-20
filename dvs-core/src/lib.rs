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

pub mod helpers;
pub mod ops;
pub mod types;

// Re-export commonly used types at crate root
pub use types::{
    AddResult, Compression, Config, DvsError, FileInfo, FileStatus, GeneratedBy, GetResult,
    HashAlgo, Manifest, ManifestEntry, Metadata, MetadataFormat, Oid, Outcome, StatusResult,
};

// Re-export operations at crate root
pub use ops::{
    add, add_with_backend, get, get_with_backend, init, init_with_backend, status,
    status_with_backend,
};
pub use ops::{log, log_entry, log_entry_with_backend, log_with_backend, LogEntry};
pub use ops::{
    materialize, materialize_files, materialize_with_backend, MaterializeResult, MaterializeSummary,
};
pub use ops::{pull, pull_files, pull_with_backend, PullResult, PullSummary};
pub use ops::{push, push_files, push_with_backend, PushResult, PushSummary};
pub use ops::{rollback, rollback_with_backend, RollbackResult, RollbackTarget};

// Re-export backend types
pub use helpers::backend::{
    detect_backend, detect_backend_cwd, Backend, DvsBackend, GitBackend, RepoBackend,
};

// Re-export store types for testing
pub use helpers::store::{LocalStore, ObjectStore};

// Re-export version information
pub use helpers::version::{commit_sha, version, version_string, DvsVersion, VERSION_STRING};
