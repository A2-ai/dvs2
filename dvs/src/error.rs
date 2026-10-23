//! Stable, machine-readable codes for failures, shared by every command.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ErrorCode {
    AuthRequired,
    AuthExpired,
    AccessDenied,
    ServerUnavailable,
    ServerTooOld,
    ProjectNotFound,
    /// An unreadable `.dvs` record, including git conflict markers
    BadMetadata,
    CredentialsUnreadable,
    NotInProject,
    InvalidArguments,
    InvalidConfiguration,
    NoFilesSelected,
    IoError,
    /// The fallback when nothing more specific is known
    OperationFailed,
    /// A row that failed without a more specific code
    FileFailed,
    RevokeFailed,
    /// A row a cancel stopped before it started: a row state, not a failure
    Cancelled,
}

impl ErrorCode {
    pub fn as_str(&self) -> &'static str {
        match self {
            ErrorCode::AuthRequired => "auth_required",
            ErrorCode::AuthExpired => "auth_expired",
            ErrorCode::AccessDenied => "access_denied",
            ErrorCode::ServerUnavailable => "server_unavailable",
            ErrorCode::ServerTooOld => "server_too_old",
            ErrorCode::ProjectNotFound => "project_not_found",
            ErrorCode::BadMetadata => "bad_metadata",
            ErrorCode::CredentialsUnreadable => "credentials_unreadable",
            ErrorCode::NotInProject => "not_in_project",
            ErrorCode::InvalidArguments => "invalid_arguments",
            ErrorCode::InvalidConfiguration => "invalid_configuration",
            ErrorCode::NoFilesSelected => "no_files_selected",
            ErrorCode::IoError => "io_error",
            ErrorCode::OperationFailed => "operation_failed",
            ErrorCode::FileFailed => "file_failed",
            ErrorCode::RevokeFailed => "revoke_failed",
            ErrorCode::Cancelled => "cancelled",
        }
    }
}

/// A failure with its code: backend, records and credentials.
#[derive(Debug, Clone, Serialize)]
pub struct Error {
    pub code: ErrorCode,
    pub message: String,
}

impl Error {
    pub fn new(code: ErrorCode, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
        }
    }
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for Error {}

pub fn backend_error_code(error: &anyhow::Error) -> Option<ErrorCode> {
    error.downcast_ref::<Error>().map(|e| e.code)
}
