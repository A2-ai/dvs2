//! Whether named BLAKE3 content is stored, without downloading it.

use anyhow::Result;
use serde::Serialize;

use crate::backends::Backend;
use crate::error::{Error, ErrorCode};
use crate::hashes::Hashes;
use crate::operation::{OperationRow, RowState};

/// Whether the storage holds the content of a hash.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum StoredState {
    Present,
    Absent,
    /// The storage could not say. The row has a `code` and a `message`.
    Unknown,
}

impl StoredState {
    pub fn as_str(&self) -> &'static str {
        match self {
            StoredState::Present => "present",
            StoredState::Absent => "absent",
            StoredState::Unknown => "unknown",
        }
    }
}

/// One hash of an availability query, a row of `dvs availability --json`.
#[derive(Debug, Clone, Serialize)]
pub struct AvailabilityRow {
    /// The BLAKE3 hash, in lowercase
    pub hash: String,
    pub state: StoredState,
    /// Why the storage could not say, with `message`
    pub code: Option<ErrorCode>,
    pub message: Option<String>,
}

impl OperationRow for AvailabilityRow {
    fn state(&self) -> RowState {
        if self.code.is_some() {
            RowState::Failed
        } else {
            RowState::Succeeded
        }
    }
}

fn coded(error: anyhow::Error) -> Error {
    Error::new(crate::error_code(&error), format!("{error:#}"))
}

/// Looks up each BLAKE3 hash in the storage, in the order given, duplicates
/// included. Every hash must be 64 hex digits, checked before any I/O. A
/// storage that cannot be read (no login, no access, no answer, no storage
/// folder) is not an error: every row is `unknown`, with the reason.
pub fn availability(backend: &Backend, hashes: &[String]) -> Result<Vec<AvailabilityRow>> {
    if hashes.is_empty() {
        return Err(Error::new(ErrorCode::InvalidArguments, "no hashes to look up").into());
    }
    for hash in hashes {
        if hash.len() != 64 || !hash.bytes().all(|c| c.is_ascii_hexdigit()) {
            return Err(Error::new(
                ErrorCode::InvalidArguments,
                format!("invalid BLAKE3 hash: {hash}"),
            )
            .into());
        }
    }
    let readable = backend.check_readable().map_err(coded);
    Ok(hashes
        .iter()
        .map(|hash| {
            let hash = hash.to_ascii_lowercase();
            let hashes = Hashes {
                blake3: hash.clone(),
                md5: None,
            };
            let stored = readable
                .clone()
                .and_then(|()| backend.exists(&hashes).map_err(coded));
            match stored {
                Ok(present) => AvailabilityRow {
                    hash,
                    state: if present {
                        StoredState::Present
                    } else {
                        StoredState::Absent
                    },
                    code: None,
                    message: None,
                },
                Err(error) => AvailabilityRow {
                    hash,
                    state: StoredState::Unknown,
                    code: Some(error.code),
                    message: Some(error.message),
                },
            }
        })
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hashes_are_checked_before_the_storage() {
        // A storage folder that does not exist: a lookup would be an unknown row
        let backend = Backend::Local(
            crate::backends::local::LocalBackend::new(
                std::env::temp_dir().join("dvs-no-such-storage-for-availability"),
                None,
            )
            .unwrap(),
        );
        for hashes in [vec![], vec!["abc".to_string()]] {
            let error = availability(&backend, &hashes).unwrap_err();
            assert_eq!(crate::error_code(&error), ErrorCode::InvalidArguments);
        }
        let rows = availability(&backend, &["A".repeat(64)]).unwrap();
        assert_eq!(rows[0].hash, "a".repeat(64));
        assert_eq!(rows[0].state, StoredState::Unknown);
        assert_eq!(rows[0].code, Some(ErrorCode::IoError));
    }
}
