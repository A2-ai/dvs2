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
};

// Re-export operations at crate root
pub use ops::{init, add, get, status};
