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
    AddResult, Compression, Config, DvsError, ErrorKind, FileInfo, FileStatus, GeneratedBy,
    GetResult, HashAlgo, Manifest, ManifestEntry, Metadata, MetadataFormat, Oid, Outcome,
    StatusResult,
};

// Re-export reflog types for detailed log analysis
pub use types::{MetadataEntry, ReflogEntry, ReflogOp, WorkspaceState};

// Re-export local config types
pub use types::{AuthConfig, CacheConfig, LocalConfig};

// Re-export operations at crate root (requires serde feature)
#[cfg(feature = "serde")]
pub use ops::{
    add, add_with_backend, add_with_format, get, get_with_backend, init, init_with_backend, status,
    status_with_backend,
};
#[cfg(feature = "serde")]
pub use ops::{log, log_entry, log_entry_with_backend, log_with_backend, LogEntry};
#[cfg(feature = "serde")]
pub use ops::{
    materialize, materialize_files, materialize_with_backend, MaterializeResult, MaterializeSummary,
};
#[cfg(feature = "serde")]
pub use ops::{merge_repo, merge_repo_with_backend, ConflictMode, MergeOptions, MergeResult};
#[cfg(feature = "serde")]
pub use ops::{pull, pull_files, pull_with_backend, PullResult, PullSummary};
#[cfg(feature = "serde")]
pub use ops::{push, push_files, push_with_backend, PushResult, PushSummary};
#[cfg(feature = "serde")]
pub use ops::{rollback, rollback_with_backend, RollbackResult, RollbackTarget};

// Re-export backend types
pub use helpers::backend::{
    detect_backend, detect_backend_cwd, Backend, DvsBackend, GitBackend, RepoBackend,
};

// Re-export store types for testing
pub use helpers::store::{LocalStore, ObjectStore};

// Re-export hash utilities for advanced use
pub use helpers::hash::{
    create_hasher, default_algorithm, get_file_hash, get_file_hash_with_algo, hash_bytes,
    verify_hash, verify_hash_with_algo, Hasher,
};

// Re-export layout for .dvs/ directory access
pub use helpers::layout::Layout;

// Re-export version information
pub use helpers::version::{commit_sha, version, version_string, DvsVersion, VERSION_STRING};
