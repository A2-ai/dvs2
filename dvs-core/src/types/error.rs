//! DVS error types.

use std::path::PathBuf;
use thiserror::Error;

/// Main error type for DVS operations.
#[derive(Debug, Error)]
pub enum DvsError {
    /// Not in a git repository.
    #[error("not in a git repository")]
    NotInGitRepo,

    /// DVS not initialized (dvs.yaml not found).
    #[error("dvs.yaml not found - run dvs_init first")]
    NotInitialized,

    /// File not found.
    #[error("file not found: {path}")]
    FileNotFound { path: PathBuf },

    /// Metadata file not found.
    #[error("metadata not found: {path}")]
    MetadataNotFound { path: PathBuf },

    /// File exists outside the git repository.
    #[error("file is outside git repository: {path}")]
    FileOutsideRepo { path: PathBuf },

    /// Storage directory error.
    #[error("storage error: {message}")]
    StorageError { message: String },

    /// Hash mismatch.
    #[error("hash mismatch for {path}: expected {expected}, got {actual}")]
    HashMismatch {
        path: PathBuf,
        expected: String,
        actual: String,
    },

    /// Permission denied.
    #[error("permission denied: {message}")]
    PermissionDenied { message: String },

    /// Group membership error.
    #[error("linux primary group not set: {group}")]
    GroupNotSet { group: String },

    /// Configuration error.
    #[error("config error: {message}")]
    ConfigError { message: String },

    /// Configuration already exists with different settings.
    #[error("config already exists with different settings - edit dvs.yaml manually")]
    ConfigMismatch,

    /// Git operation error.
    #[error("git error: {message}")]
    GitError { message: String },

    /// Glob pattern error.
    #[error("invalid glob pattern: {pattern}")]
    InvalidGlob { pattern: String },

    /// No files matched.
    #[error("no files matched: {pattern}")]
    NoFilesMatched { pattern: String },

    /// Batch error (multiple files failed validation).
    #[error("batch validation failed: {message}")]
    BatchError { message: String },

    /// YAML parsing error.
    #[error("yaml error: {0}")]
    Yaml(#[from] serde_yaml::Error),

    /// JSON parsing error.
    #[error("json error: {0}")]
    Json(#[from] serde_json::Error),

    /// I/O error.
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),

    /// Generic not found error.
    #[error("{0}")]
    NotFound(String),
}

impl DvsError {
    /// Get the error type as a string (for R interop).
    pub fn error_type(&self) -> &'static str {
        match self {
            DvsError::NotInGitRepo => "not_in_git_repo",
            DvsError::NotInitialized => "not_initialized",
            DvsError::FileNotFound { .. } => "file_not_found",
            DvsError::MetadataNotFound { .. } => "metadata_not_found",
            DvsError::FileOutsideRepo { .. } => "file_outside_repo",
            DvsError::StorageError { .. } => "storage_error",
            DvsError::HashMismatch { .. } => "hash_mismatch",
            DvsError::PermissionDenied { .. } => "permission_denied",
            DvsError::GroupNotSet { .. } => "group_not_set",
            DvsError::ConfigError { .. } => "config_error",
            DvsError::ConfigMismatch => "config_mismatch",
            DvsError::GitError { .. } => "git_error",
            DvsError::InvalidGlob { .. } => "invalid_glob",
            DvsError::NoFilesMatched { .. } => "no_files_matched",
            DvsError::BatchError { .. } => "batch_error",
            DvsError::Yaml(_) => "yaml_error",
            DvsError::Json(_) => "json_error",
            DvsError::Io(_) => "io_error",
            DvsError::NotFound(_) => "not_found",
        }
    }
}
