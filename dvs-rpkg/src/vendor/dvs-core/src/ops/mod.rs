//! High-level DVS operations.
//!
//! Operations require the `serde` feature as they need to load/save metadata
//! and manifests to disk.

#[cfg(feature = "serde")]
mod add;
#[cfg(feature = "serde")]
mod get;
#[cfg(feature = "serde")]
mod init;
#[cfg(feature = "serde")]
mod log;
#[cfg(feature = "serde")]
mod materialize;
#[cfg(feature = "serde")]
mod merge_repo;
#[cfg(feature = "serde")]
mod pull;
#[cfg(feature = "serde")]
mod push;
#[cfg(feature = "serde")]
mod rollback;
#[cfg(feature = "serde")]
mod status;
#[cfg(feature = "serde")]
mod verify;

#[cfg(feature = "serde")]
pub use add::{add, add_with_backend, add_with_format};
#[cfg(feature = "serde")]
pub use get::{get, get_with_backend};
#[cfg(feature = "serde")]
pub use init::{init, init_with_backend};
#[cfg(feature = "serde")]
pub use log::{log, log_entry, log_entry_with_backend, log_with_backend, LogEntry};
#[cfg(feature = "serde")]
pub use materialize::{
    materialize, materialize_files, materialize_with_backend, MaterializeResult, MaterializeSummary,
};
#[cfg(feature = "serde")]
pub use merge_repo::{
    merge_repo, merge_repo_with_backend, ConflictMode, MergeOptions, MergeResult,
};
#[cfg(feature = "serde")]
pub use pull::{pull, pull_files, PullResult, PullSummary};
#[cfg(feature = "serde")]
pub use push::{push, push_files, PushResult, PushSummary};
#[cfg(feature = "serde")]
pub use rollback::{rollback, rollback_with_backend, RollbackResult, RollbackTarget};
#[cfg(feature = "serde")]
pub use status::{status, status_with_backend, status_with_options, StatusOptions};
#[cfg(feature = "serde")]
pub use verify::{verify, verify_with_backend, VerifyResult, VerifySummary};
