//! Core type definitions for DVS.

mod config;
mod error;
mod file_info;
mod local_config;
mod manifest;
mod metadata;
mod oid;
mod outcome;
mod reflog;

pub use config::{Config, GeneratedBy};
pub use error::{DvsError, ErrorKind};
pub use file_info::FileInfo;
pub use local_config::{AuthConfig, CacheConfig, LocalConfig};
pub use manifest::{Compression, Manifest, ManifestEntry};
pub use metadata::{Metadata, MetadataFormat};
pub use oid::{HashAlgo, Oid};
pub use outcome::{AddResult, FileStatus, GetResult, Outcome, StatusResult};
pub use reflog::{MetadataEntry, ReflogEntry, ReflogOp, WorkspaceState};
