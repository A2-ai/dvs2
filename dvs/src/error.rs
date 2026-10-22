//! Stable, machine-readable codes for backend failures, shared by every command.

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
        }
    }
}

#[derive(Debug, Clone)]
pub struct BackendError {
    pub code: ErrorCode,
    pub message: String,
}

impl BackendError {
    pub fn new(code: ErrorCode, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
        }
    }
}

impl std::fmt::Display for BackendError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for BackendError {}

pub fn backend_error_code(error: &anyhow::Error) -> Option<ErrorCode> {
    error.downcast_ref::<BackendError>().map(|e| e.code)
}
