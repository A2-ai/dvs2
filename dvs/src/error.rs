//! Stable, machine-readable codes for backend and operation failures.
//!
//! One vocabulary shared by every command, at the top level and per row: a
//! 401, a 403 or a missing token must read the same whether it surfaced in
//! `add`, `get`, `status`, `availability` or `info`. Codes are the API;
//! messages are for people and may change.
//!
//! Server and auth code return [`BackendError`]; everything else asks
//! [`backend_error_code`] what an error means instead of downcasting itself.

use serde::{Deserialize, Serialize};

/// A failure category shared by every command. Serialized in snake_case,
/// e.g. `"auth_expired"`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ErrorCode {
    /// No token / not logged in
    AuthRequired,
    /// The server rejected the token (401)
    AuthExpired,
    /// The server refused this user (403)
    AccessDenied,
    /// Transport error, timeout or 5xx
    ServerUnavailable,
    /// An endpoint that should exist answered 404/405
    ServerTooOld,
    /// The project does not exist on the server (404)
    ProjectNotFound,
    /// An unreadable `.dvs` record, including git conflict markers. The one
    /// code for it in every command (K25; `invalid_record` is gone)
    BadMetadata,
    /// The credentials store exists but cannot be read or parsed
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

/// A backend or credentials failure that already knows its code. Returned
/// inside `anyhow::Error`; read it back with [`backend_error_code`].
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

/// The one mapping from an error to a shared code. `None` means the error is
/// not a backend/credentials/record failure, and the caller falls back
/// to its own codes (`io_error`, `operation_failed`, get's `GetCode`s, ...).
pub fn backend_error_code(error: &anyhow::Error) -> Option<ErrorCode> {
    error.downcast_ref::<BackendError>().map(|e| e.code)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn codes_serialize_as_their_snake_case_names() {
        for code in [
            ErrorCode::AuthRequired,
            ErrorCode::AuthExpired,
            ErrorCode::AccessDenied,
            ErrorCode::ServerUnavailable,
            ErrorCode::ServerTooOld,
            ErrorCode::ProjectNotFound,
            ErrorCode::BadMetadata,
            ErrorCode::CredentialsUnreadable,
        ] {
            assert_eq!(serde_json::to_value(code).unwrap(), code.as_str());
        }
    }

    #[test]
    fn the_mapping_reads_backend_errors_through_context() {
        let typed: anyhow::Error = BackendError::new(ErrorCode::AuthExpired, "expired").into();
        assert_eq!(backend_error_code(&typed), Some(ErrorCode::AuthExpired));
        let wrapped = anyhow::Error::from(BackendError::new(ErrorCode::AccessDenied, "no"))
            .context("while adding");
        assert_eq!(backend_error_code(&wrapped), Some(ErrorCode::AccessDenied));
        assert_eq!(backend_error_code(&anyhow::anyhow!("other")), None);
    }
}
