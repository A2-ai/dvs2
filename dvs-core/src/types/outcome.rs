//! Operation outcome types.

use std::path::PathBuf;
use chrono::{DateTime, Utc};

/// Outcome of an add or get operation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Outcome {
    /// File was copied to/from storage.
    Copied,
    /// File was already present (no action needed).
    Present,
    /// An error occurred.
    Error,
}

/// Status of a tracked file.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FileStatus {
    /// Local file exists and matches stored version.
    Current,
    /// Metadata exists but local file is missing.
    Absent,
    /// Local file exists but differs from stored version.
    Unsynced,
    /// Could not determine status.
    Error,
}

/// Result of an `add` operation for a single file.
#[derive(Debug, Clone)]
pub struct AddResult {
    /// Relative path from the working directory.
    pub relative_path: PathBuf,

    /// Absolute path on the filesystem.
    pub absolute_path: PathBuf,

    /// Outcome of the operation.
    pub outcome: Outcome,

    /// File size in bytes.
    pub size: u64,

    /// Blake3 hash of the file contents.
    pub blake3_checksum: String,

    /// Error type (if outcome is Error).
    pub error: Option<String>,

    /// Error message (if outcome is Error).
    pub error_message: Option<String>,

    /// Original input that caused the error.
    pub input: Option<String>,
}

/// Result of a `get` operation for a single file.
#[derive(Debug, Clone)]
pub struct GetResult {
    /// Relative path from the working directory.
    pub relative_path: PathBuf,

    /// Absolute path on the filesystem.
    pub absolute_path: PathBuf,

    /// Outcome of the operation.
    pub outcome: Outcome,

    /// File size in bytes.
    pub size: u64,

    /// Blake3 hash of the file contents.
    pub blake3_checksum: String,

    /// Error type (if outcome is Error).
    pub error: Option<String>,

    /// Error message (if outcome is Error).
    pub error_message: Option<String>,

    /// Original input that caused the error.
    pub input: Option<String>,
}

/// Result of a `status` operation for a single file.
#[derive(Debug, Clone)]
pub struct StatusResult {
    /// Relative path from the working directory.
    pub relative_path: PathBuf,

    /// Absolute path on the filesystem.
    pub absolute_path: PathBuf,

    /// Status of the file.
    pub status: FileStatus,

    /// File size in bytes.
    pub size: u64,

    /// Blake3 hash of the file contents.
    pub blake3_checksum: String,

    /// When the file was last added.
    pub add_time: Option<DateTime<Utc>>,

    /// Who last added the file.
    pub saved_by: Option<String>,

    /// Last add message.
    pub message: Option<String>,

    /// Error type (if status is Error).
    pub error: Option<String>,

    /// Error message (if status is Error).
    pub error_message: Option<String>,

    /// Original input that caused the error.
    pub input: Option<String>,
}

impl AddResult {
    /// Create a successful add result.
    pub fn success(
        relative_path: PathBuf,
        absolute_path: PathBuf,
        outcome: Outcome,
        size: u64,
        blake3_checksum: String,
    ) -> Self {
        Self {
            relative_path,
            absolute_path,
            outcome,
            size,
            blake3_checksum,
            error: None,
            error_message: None,
            input: None,
        }
    }

    /// Create an error add result.
    pub fn error(input: String, error: String, error_message: String) -> Self {
        Self {
            relative_path: PathBuf::new(),
            absolute_path: PathBuf::new(),
            outcome: Outcome::Error,
            size: 0,
            blake3_checksum: String::new(),
            error: Some(error),
            error_message: Some(error_message),
            input: Some(input),
        }
    }
}

impl GetResult {
    /// Create a successful get result.
    pub fn success(
        relative_path: PathBuf,
        absolute_path: PathBuf,
        outcome: Outcome,
        size: u64,
        blake3_checksum: String,
    ) -> Self {
        Self {
            relative_path,
            absolute_path,
            outcome,
            size,
            blake3_checksum,
            error: None,
            error_message: None,
            input: None,
        }
    }

    /// Create an error get result.
    pub fn error(input: String, error: String, error_message: String) -> Self {
        Self {
            relative_path: PathBuf::new(),
            absolute_path: PathBuf::new(),
            outcome: Outcome::Error,
            size: 0,
            blake3_checksum: String::new(),
            error: Some(error),
            error_message: Some(error_message),
            input: Some(input),
        }
    }
}

impl StatusResult {
    /// Create a successful status result.
    #[allow(clippy::too_many_arguments)]
    pub fn success(
        relative_path: PathBuf,
        absolute_path: PathBuf,
        status: FileStatus,
        size: u64,
        blake3_checksum: String,
        add_time: Option<DateTime<Utc>>,
        saved_by: Option<String>,
        message: Option<String>,
    ) -> Self {
        Self {
            relative_path,
            absolute_path,
            status,
            size,
            blake3_checksum,
            add_time,
            saved_by,
            message,
            error: None,
            error_message: None,
            input: None,
        }
    }

    /// Create an error status result.
    pub fn error(input: String, error: String, error_message: String) -> Self {
        Self {
            relative_path: PathBuf::new(),
            absolute_path: PathBuf::new(),
            status: FileStatus::Error,
            size: 0,
            blake3_checksum: String::new(),
            add_time: None,
            saved_by: None,
            message: None,
            error: Some(error),
            error_message: Some(error_message),
            input: Some(input),
        }
    }
}
