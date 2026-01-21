//! DVS error types.
//!
//! This module provides error handling using `exn` for context-aware errors
//! while preserving stable `error_type()` strings for R interop.

use std::fmt;
use std::path::PathBuf;

/// Error kind enum for DVS operations.
///
/// This defines the stable error types that map to `error_type()` strings.
/// Each variant corresponds to a specific error condition.
#[derive(Debug, Clone)]
pub enum ErrorKind {
    /// Not in a git repository.
    NotInGitRepo,
    /// DVS not initialized (config file not found).
    NotInitialized,
    /// File not found.
    FileNotFound { path: PathBuf },
    /// Metadata file not found.
    MetadataNotFound { path: PathBuf },
    /// File exists outside the git repository.
    FileOutsideRepo { path: PathBuf },
    /// Storage directory error.
    StorageError { message: String },
    /// Hash mismatch.
    HashMismatch {
        path: PathBuf,
        expected: String,
        actual: String,
    },
    /// Permission denied.
    PermissionDenied { message: String },
    /// Group membership error.
    GroupNotSet { group: String },
    /// Configuration error.
    ConfigError { message: String },
    /// Configuration already exists with different settings.
    ConfigMismatch,
    /// Git operation error.
    GitError { message: String },
    /// Glob pattern error.
    InvalidGlob { pattern: String },
    /// No files matched.
    NoFilesMatched { pattern: String },
    /// Batch error (multiple files failed validation).
    BatchError { message: String },
    /// YAML parsing error.
    YamlError { message: String },
    /// JSON parsing error.
    JsonError { message: String },
    /// TOML parsing error.
    TomlError { message: String },
    /// I/O error.
    IoError { message: String },
    /// Generic not found error.
    NotFound { message: String },
    /// Merge conflict (paths exist in both source and destination).
    MergeConflict { paths: String },
}

impl ErrorKind {
    /// Get the error type as a string (for R interop).
    ///
    /// These strings are stable and must not change to maintain compatibility
    /// with the R interface.
    pub fn error_type(&self) -> &'static str {
        match self {
            ErrorKind::NotInGitRepo => "not_in_git_repo",
            ErrorKind::NotInitialized => "not_initialized",
            ErrorKind::FileNotFound { .. } => "file_not_found",
            ErrorKind::MetadataNotFound { .. } => "metadata_not_found",
            ErrorKind::FileOutsideRepo { .. } => "file_outside_repo",
            ErrorKind::StorageError { .. } => "storage_error",
            ErrorKind::HashMismatch { .. } => "hash_mismatch",
            ErrorKind::PermissionDenied { .. } => "permission_denied",
            ErrorKind::GroupNotSet { .. } => "group_not_set",
            ErrorKind::ConfigError { .. } => "config_error",
            ErrorKind::ConfigMismatch => "config_mismatch",
            ErrorKind::GitError { .. } => "git_error",
            ErrorKind::InvalidGlob { .. } => "invalid_glob",
            ErrorKind::NoFilesMatched { .. } => "no_files_matched",
            ErrorKind::BatchError { .. } => "batch_error",
            ErrorKind::YamlError { .. } => "yaml_error",
            ErrorKind::JsonError { .. } => "json_error",
            ErrorKind::TomlError { .. } => "toml_error",
            ErrorKind::IoError { .. } => "io_error",
            ErrorKind::NotFound { .. } => "not_found",
            ErrorKind::MergeConflict { .. } => "merge_conflict",
        }
    }
}

impl fmt::Display for ErrorKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ErrorKind::NotInGitRepo => write!(f, "not in a git repository"),
            ErrorKind::NotInitialized => write!(f, "config file not found - run dvs init first"),
            ErrorKind::FileNotFound { path } => write!(f, "file not found: {}", path.display()),
            ErrorKind::MetadataNotFound { path } => {
                write!(f, "metadata not found: {}", path.display())
            }
            ErrorKind::FileOutsideRepo { path } => {
                write!(f, "file is outside git repository: {}", path.display())
            }
            ErrorKind::StorageError { message } => write!(f, "storage error: {}", message),
            ErrorKind::HashMismatch {
                path,
                expected,
                actual,
            } => write!(
                f,
                "hash mismatch for {}: expected {}, got {}",
                path.display(),
                expected,
                actual
            ),
            ErrorKind::PermissionDenied { message } => write!(f, "permission denied: {}", message),
            ErrorKind::GroupNotSet { group } => {
                write!(f, "linux primary group not set: {}", group)
            }
            ErrorKind::ConfigError { message } => write!(f, "config error: {}", message),
            ErrorKind::ConfigMismatch => {
                write!(
                    f,
                    "config already exists with different settings - edit config file manually"
                )
            }
            ErrorKind::GitError { message } => write!(f, "git error: {}", message),
            ErrorKind::InvalidGlob { pattern } => write!(f, "invalid glob pattern: {}", pattern),
            ErrorKind::NoFilesMatched { pattern } => write!(f, "no files matched: {}", pattern),
            ErrorKind::BatchError { message } => write!(f, "batch validation failed: {}", message),
            ErrorKind::YamlError { message } => write!(f, "yaml error: {}", message),
            ErrorKind::JsonError { message } => write!(f, "json error: {}", message),
            ErrorKind::TomlError { message } => write!(f, "toml error: {}", message),
            ErrorKind::IoError { message } => write!(f, "io error: {}", message),
            ErrorKind::NotFound { message } => write!(f, "{}", message),
            ErrorKind::MergeConflict { paths } => {
                write!(f, "merge conflict - paths exist in both repos: {}", paths)
            }
        }
    }
}

impl std::error::Error for ErrorKind {}

/// Main error type for DVS operations.
///
/// This wraps `exn::Exn<ErrorKind>` to provide context-aware error handling
/// while maintaining the stable `error_type()` interface for R interop.
#[derive(Debug)]
pub struct DvsError(exn::Exn<ErrorKind>);

impl DvsError {
    /// Create a new error from an error kind.
    pub fn new(kind: ErrorKind) -> Self {
        Self(exn::Exn::new(kind))
    }

    /// Get the error kind.
    pub fn kind(&self) -> &ErrorKind {
        self.0.as_error()
    }

    /// Get the error type as a string (for R interop).
    ///
    /// This delegates to `ErrorKind::error_type()` to maintain stable strings.
    pub fn error_type(&self) -> &'static str {
        self.kind().error_type()
    }

    // Convenience constructors for common error types

    /// Create a "not in git repo" error.
    pub fn not_in_git_repo() -> Self {
        Self::new(ErrorKind::NotInGitRepo)
    }

    /// Create a "not initialized" error.
    pub fn not_initialized() -> Self {
        Self::new(ErrorKind::NotInitialized)
    }

    /// Create a "file not found" error.
    pub fn file_not_found(path: impl Into<PathBuf>) -> Self {
        Self::new(ErrorKind::FileNotFound { path: path.into() })
    }

    /// Create a "metadata not found" error.
    pub fn metadata_not_found(path: impl Into<PathBuf>) -> Self {
        Self::new(ErrorKind::MetadataNotFound { path: path.into() })
    }

    /// Create a "file outside repo" error.
    pub fn file_outside_repo(path: impl Into<PathBuf>) -> Self {
        Self::new(ErrorKind::FileOutsideRepo { path: path.into() })
    }

    /// Create a "storage error".
    pub fn storage_error(message: impl Into<String>) -> Self {
        Self::new(ErrorKind::StorageError {
            message: message.into(),
        })
    }

    /// Create a "hash mismatch" error.
    pub fn hash_mismatch(
        path: impl Into<PathBuf>,
        expected: impl Into<String>,
        actual: impl Into<String>,
    ) -> Self {
        Self::new(ErrorKind::HashMismatch {
            path: path.into(),
            expected: expected.into(),
            actual: actual.into(),
        })
    }

    /// Create a "permission denied" error.
    pub fn permission_denied(message: impl Into<String>) -> Self {
        Self::new(ErrorKind::PermissionDenied {
            message: message.into(),
        })
    }

    /// Create a "group not set" error.
    pub fn group_not_set(group: impl Into<String>) -> Self {
        Self::new(ErrorKind::GroupNotSet {
            group: group.into(),
        })
    }

    /// Create a "config error".
    pub fn config_error(message: impl Into<String>) -> Self {
        Self::new(ErrorKind::ConfigError {
            message: message.into(),
        })
    }

    /// Create a "config mismatch" error.
    pub fn config_mismatch() -> Self {
        Self::new(ErrorKind::ConfigMismatch)
    }

    /// Create a "git error".
    pub fn git_error(message: impl Into<String>) -> Self {
        Self::new(ErrorKind::GitError {
            message: message.into(),
        })
    }

    /// Create an "invalid glob" error.
    pub fn invalid_glob(pattern: impl Into<String>) -> Self {
        Self::new(ErrorKind::InvalidGlob {
            pattern: pattern.into(),
        })
    }

    /// Create a "no files matched" error.
    pub fn no_files_matched(pattern: impl Into<String>) -> Self {
        Self::new(ErrorKind::NoFilesMatched {
            pattern: pattern.into(),
        })
    }

    /// Create a "batch error".
    pub fn batch_error(message: impl Into<String>) -> Self {
        Self::new(ErrorKind::BatchError {
            message: message.into(),
        })
    }

    /// Create a "not found" error.
    pub fn not_found(message: impl Into<String>) -> Self {
        Self::new(ErrorKind::NotFound {
            message: message.into(),
        })
    }

    /// Create a "merge conflict" error.
    pub fn merge_conflict(paths: impl Into<String>) -> Self {
        Self::new(ErrorKind::MergeConflict {
            paths: paths.into(),
        })
    }

    /// Alias for config_error (used for general configuration issues).
    pub fn config(message: impl Into<String>) -> Self {
        Self::config_error(message)
    }
}

impl fmt::Display for DvsError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl std::error::Error for DvsError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        // ErrorKind is the root cause, no further source
        None
    }
}

// Conversion from common error types

impl From<std::io::Error> for DvsError {
    fn from(e: std::io::Error) -> Self {
        Self::new(ErrorKind::IoError {
            message: e.to_string(),
        })
    }
}

#[cfg(feature = "yaml-config")]
impl From<serde_yaml::Error> for DvsError {
    fn from(e: serde_yaml::Error) -> Self {
        Self::new(ErrorKind::YamlError {
            message: e.to_string(),
        })
    }
}

#[cfg(feature = "serde")]
impl From<serde_json::Error> for DvsError {
    fn from(e: serde_json::Error) -> Self {
        Self::new(ErrorKind::JsonError {
            message: e.to_string(),
        })
    }
}

#[cfg(feature = "toml-config")]
impl From<toml::de::Error> for DvsError {
    fn from(e: toml::de::Error) -> Self {
        Self::new(ErrorKind::TomlError {
            message: e.to_string(),
        })
    }
}

#[cfg(feature = "toml-config")]
impl From<toml::ser::Error> for DvsError {
    fn from(e: toml::ser::Error) -> Self {
        Self::new(ErrorKind::TomlError {
            message: e.to_string(),
        })
    }
}

#[cfg(feature = "git2-backend")]
impl From<git2::Error> for DvsError {
    fn from(e: git2::Error) -> Self {
        Self::new(ErrorKind::GitError {
            message: e.to_string(),
        })
    }
}

// Backward compatibility: allow matching on old-style variants
// These are kept for gradual migration

/// Backward compatibility alias.
#[allow(non_upper_case_globals)]
impl DvsError {
    /// Check if this is a NotInGitRepo error.
    pub fn is_not_in_git_repo(&self) -> bool {
        matches!(self.kind(), ErrorKind::NotInGitRepo)
    }

    /// Check if this is a NotInitialized error.
    pub fn is_not_initialized(&self) -> bool {
        matches!(self.kind(), ErrorKind::NotInitialized)
    }

    /// Check if this is a FileNotFound error.
    pub fn is_file_not_found(&self) -> bool {
        matches!(self.kind(), ErrorKind::FileNotFound { .. })
    }

    /// Check if this is a ConfigError.
    pub fn is_config_error(&self) -> bool {
        matches!(self.kind(), ErrorKind::ConfigError { .. })
    }

    /// Check if this is an IoError.
    pub fn is_io_error(&self) -> bool {
        matches!(self.kind(), ErrorKind::IoError { .. })
    }
}
