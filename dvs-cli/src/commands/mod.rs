//! DVS command implementations.
//!
//! Each subcommand is implemented in its own module and delegates
//! to dvs-core for actual DVS logic.

pub mod init;
pub mod add;
pub mod get;
pub mod status;
pub mod push;
pub mod pull;
pub mod materialize;

use std::io;
use thiserror::Error;

/// CLI-specific error type.
#[derive(Debug, Error)]
pub enum CliError {
    /// DVS core error.
    #[error("{0}")]
    Dvs(#[from] dvs_core::DvsError),

    /// I/O error.
    #[error("I/O error: {0}")]
    Io(#[from] io::Error),

    /// Invalid argument.
    #[error("Invalid argument: {0}")]
    InvalidArg(String),

    /// Path error.
    #[error("Path error: {0}")]
    Path(String),
}

pub type Result<T> = std::result::Result<T, CliError>;
