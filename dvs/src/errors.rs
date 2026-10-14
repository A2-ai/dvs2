//! Errors a caller may branch on, with stable codes like the server's: `DvsError` for
//! what the client finds itself (no repository, untracked paths, …), and helpers that
//! read a code and a plain message from any error, the server's included.

use std::fmt;

use serde::Serialize;

use crate::remote::ApiError;

/// What went wrong on the client side, as a stable code.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ErrorCode {
    /// No `dvs.toml` here or in a parent folder
    NotInRepository,
    /// The project keeps its files in local storage, but this needs a dvs server
    NotAServerProject,
    /// `dvs.toml`, the metadata folder or the storage is already there
    AlreadyInitialized,
    /// Paths given to add that cannot be added (missing, a folder, outside the project,
    /// dvs.toml or the metadata folder); nothing was done
    InvalidPaths,
    /// Paths given to get (or to look up) that dvs does not track; nothing was done
    NotTracked,
    /// The paths and glob given match no file to add
    NothingToAdd,
    /// The paths and glob given match no tracked file
    NothingToGet,
    /// Another dvs process held the credentials store's lock for too long
    CredentialsBusy,
}

impl ErrorCode {
    pub const ALL: [ErrorCode; 8] = [
        ErrorCode::NotInRepository,
        ErrorCode::NotAServerProject,
        ErrorCode::AlreadyInitialized,
        ErrorCode::InvalidPaths,
        ErrorCode::NotTracked,
        ErrorCode::NothingToAdd,
        ErrorCode::NothingToGet,
        ErrorCode::CredentialsBusy,
    ];

    pub fn as_str(self) -> &'static str {
        match self {
            ErrorCode::NotInRepository => "not_in_repository",
            ErrorCode::NotAServerProject => "not_a_server_project",
            ErrorCode::AlreadyInitialized => "already_initialized",
            ErrorCode::InvalidPaths => "invalid_paths",
            ErrorCode::NotTracked => "not_tracked",
            ErrorCode::NothingToAdd => "nothing_to_add",
            ErrorCode::NothingToGet => "nothing_to_get",
            ErrorCode::CredentialsBusy => "credentials_busy",
        }
    }
}

impl fmt::Display for ErrorCode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// An error the client found, with its code. Displays as the message alone.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DvsError {
    pub code: ErrorCode,
    pub message: String,
}

impl DvsError {
    pub fn new(code: ErrorCode, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
        }
    }

    pub fn code(&self) -> &'static str {
        self.code.as_str()
    }
}

impl fmt::Display for DvsError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for DvsError {}

/// The stable code behind an error, if it has one: the client's (`not_tracked`, …) or
/// the server's (`project_locked`, …). Contexts added on the way are looked through.
pub fn error_code(e: &anyhow::Error) -> Option<&str> {
    e.chain().find_map(|c| {
        c.downcast_ref::<DvsError>()
            .map(DvsError::code)
            .or_else(|| c.downcast_ref::<ApiError>().map(|a| a.code.as_str()))
    })
}

/// An error's message with its contexts, as `{:#}` shows it, but without the
/// ` (code)` a server error shows: for callers that keep the code separately.
pub fn error_message(e: &anyhow::Error) -> String {
    e.chain()
        .map(|c| match c.downcast_ref::<ApiError>() {
            Some(api) => api.message.clone(),
            None => c.to_string(),
        })
        .collect::<Vec<_>>()
        .join(": ")
}

#[cfg(test)]
mod tests {
    use super::*;
    use anyhow::Context;

    #[test]
    fn codes_and_messages_are_read_through_contexts() {
        let local: anyhow::Error =
            DvsError::new(ErrorCode::NotTracked, "a.csv is not tracked").into();
        let local = Err::<(), _>(local).context("getting").unwrap_err();
        assert_eq!(error_code(&local), Some("not_tracked"));
        assert_eq!(error_message(&local), "getting: a.csv is not tracked");

        let server: anyhow::Error = ApiError::new(410, "yanked", "it was yanked").into();
        let server = Err::<(), _>(server)
            .context("Failed to retrieve a.csv")
            .unwrap_err();
        assert_eq!(error_code(&server), Some("yanked"));
        assert_eq!(
            error_message(&server),
            "Failed to retrieve a.csv: it was yanked"
        );
        assert_eq!(
            format!("{server:#}"),
            "Failed to retrieve a.csv: it was yanked (yanked)"
        );

        assert_eq!(error_code(&anyhow::anyhow!("plain")), None);
        let names: Vec<String> = ErrorCode::ALL
            .iter()
            .map(|c| {
                serde_json::to_value(c)
                    .unwrap()
                    .as_str()
                    .unwrap()
                    .to_string()
            })
            .collect();
        assert!(
            ErrorCode::ALL
                .iter()
                .zip(&names)
                .all(|(c, n)| c.as_str() == n)
        );
    }
}
