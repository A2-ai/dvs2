//! Core type definitions for DVS.

mod config;
mod metadata;
mod file_info;
mod outcome;
mod error;

pub use config::Config;
pub use metadata::Metadata;
pub use file_info::FileInfo;
pub use outcome::{Outcome, FileStatus, AddResult, GetResult, StatusResult};
pub use error::DvsError;
