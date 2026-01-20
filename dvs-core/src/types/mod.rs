//! Core type definitions for DVS.

mod config;
mod metadata;
mod file_info;
mod outcome;
mod error;
mod oid;
mod manifest;
mod reflog;

pub use config::{Config, GeneratedBy};
pub use metadata::Metadata;
pub use file_info::FileInfo;
pub use outcome::{Outcome, FileStatus, AddResult, GetResult, StatusResult};
pub use error::DvsError;
pub use oid::{Oid, HashAlgo};
pub use manifest::{Manifest, ManifestEntry, Compression};
pub use reflog::{WorkspaceState, MetadataEntry, ReflogEntry, ReflogOp};
