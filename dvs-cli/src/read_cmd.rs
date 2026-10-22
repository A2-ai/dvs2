//! Batch, backend-neutral queries for consumers that invoke the CLI.
use anyhow::Result;
use dvs::audit::{Action, AuditEntry};
use dvs::{Backend, DvsPaths, Hashes, PathFilter};
use serde_json::{Value, json};
use std::collections::HashSet;
use std::path::PathBuf;

#[derive(Debug)]
pub struct QueryInputError(pub String);
impl std::fmt::Display for QueryInputError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}
impl std::error::Error for QueryInputError {}
fn invalid(message: impl Into<String>) -> anyhow::Error {
    QueryInputError(message.into()).into()
}

pub(crate) fn reason(error: &anyhow::Error) -> &'static str {
    match dvs::backend_error_code(error) {
        Some(code) => code.as_str(),
        None => "backend_read_failed",
    }
}

#[derive(Clone)]
struct ReadError {
    code: &'static str,
    message: String,
}

impl ReadError {
    fn new(error: &anyhow::Error) -> Self {
        Self {
            code: reason(error),
            message: format!("{error:#}"),
        }
    }
}

fn check_backend(backend: &Backend) -> Result<()> {
    match backend {
        Backend::Server(s) => {
            s.access_readonly()?;
        }
        Backend::Local(b) => {
            std::fs::read_dir(&b.path)?;
        }
    }
    Ok(())
}
fn resolve_paths(paths: &DvsPaths, user_paths: Vec<PathBuf>) -> Result<Vec<PathBuf>> {
    user_paths
        .into_iter()
        .map(|path| {
            // A folder's history is the history of every file in it, which can be
            // thousands of entries: name the files
            if paths.cwd().join(&path).is_dir() {
                return Err(invalid(format!(
                    "history takes files, not folders: {}",
                    path.display()
                )));
            }
            let filter = PathFilter::from_user_paths(vec![path.clone()], false, paths);
            if filter.paths().len() != 1 || filter.paths()[0].as_os_str().is_empty() {
                return Err(invalid(format!(
                    "history requires file paths inside the project: {}",
                    path.display()
                )));
            }
            let relative = filter.paths()[0].clone();
            dvs::ProjectPath::from_path(&relative).map_err(|e| invalid(e.to_string()))?;
            Ok(relative)
        })
        .collect()
}

fn read_history(backend: &Backend, paths: &[PathBuf]) -> Result<Vec<AuditEntry>> {
    match backend {
        Backend::Server(server) => server.history_readonly(paths),
        Backend::Local(local) => local.get_audit_entries(paths),
    }
}

fn hashes_by_path(entries: Vec<AuditEntry>) -> HashSet<(PathBuf, String)> {
    entries
        .into_iter()
        .filter_map(|entry| match entry.action {
            Action::Add { file, .. } => Some((file.path, file.hashes.blake3.to_ascii_lowercase())),
            Action::Init { .. } => None,
        })
        .collect()
}

/// Validate hashes before I/O and preserve input order and duplicates.
/// `state` describes blob storage; `in_history` is added when a file is supplied.
pub fn availability(
    backend: &Backend,
    paths: &DvsPaths,
    hashes: &[String],
    user_path: Option<PathBuf>,
) -> Result<(Vec<Value>, usize)> {
    if hashes.is_empty() {
        return Err(invalid("provide at least one BLAKE3 hash"));
    }
    for hash in hashes {
        if hash.len() != 64 || !hash.bytes().all(|c| c.is_ascii_hexdigit()) {
            return Err(invalid(format!("invalid BLAKE3 hash: {hash}")));
        }
    }
    let relative_path = user_path
        .map(|path| resolve_paths(paths, vec![path]))
        .transpose()?
        .and_then(|mut paths| paths.pop());
    let preflight = check_backend(backend).map_err(|error| ReadError::new(&error));
    let history = relative_path.as_ref().map(|path| {
        preflight
            .clone()
            .and_then(|()| {
                read_history(backend, std::slice::from_ref(path))
                    .map_err(|error| ReadError::new(&error))
            })
            .map(|entries| {
                hashes_by_path(entries)
                    .into_iter()
                    .map(|(_, hash)| hash)
                    .collect::<HashSet<_>>()
            })
    });

    let mut rows = Vec::with_capacity(hashes.len());
    let mut failed = 0;
    for hash in hashes {
        let normalized = hash.to_ascii_lowercase();
        let value = Hashes {
            blake3: normalized.clone(),
            md5: None,
        };
        let storage = preflight.clone().and_then(|()| match backend {
            Backend::Server(server) => server
                .exists_readonly(&value)
                .map_err(|error| ReadError::new(&error)),
            Backend::Local(local) => local
                .exists_checked(&value)
                .map_err(|error| ReadError::new(&error)),
        });
        let mut row_failed = false;
        let (state, code, message) = match storage {
            Ok(present) => (
                if present { "present" } else { "absent" },
                Value::Null,
                Value::Null,
            ),
            Err(error) => {
                row_failed = true;
                ("unknown", json!(error.code), json!(error.message))
            }
        };
        // The storage check and the history lookup fail separately, so one
        // row can carry both errors
        let mut row = json!({
            "hash": normalized,
            "state": state,
            "in_history": null,
            "code": code,
            "message": message,
            "hint": null,
            "history_code": null,
            "history_message": null,
        });
        if let Some(history) = &history {
            match history {
                Ok(hashes) => row["in_history"] = json!(hashes.contains(&normalized)),
                Err(error) => {
                    row_failed = true;
                    row["history_code"] = json!(error.code);
                    row["history_message"] = json!(error.message.clone());
                }
            }
        }
        if row_failed {
            failed += 1;
        }
        rows.push(row);
    }
    Ok((rows, failed))
}
/// Resolve query paths lexically; the files need not exist in this checkout.
/// Each entry is the audit entry with `timestamp` as RFC 3339 UTC.
/// Reject escaping paths rather than silently broadening an invalid query to all history.
pub fn history(
    backend: &Backend,
    paths: &DvsPaths,
    user_paths: Vec<PathBuf>,
) -> Result<Vec<Value>> {
    let relative = resolve_paths(paths, user_paths)?;
    check_backend(backend)?;
    read_history(backend, &relative)?
        .into_iter()
        .map(|e| {
            let timestamp = std::time::SystemTime::from(e.timestamp);
            let mut value = serde_json::to_value(e)?;
            // The audit log on disk keeps unix seconds
            value["timestamp"] = json!(dvs::operation::rfc3339_ms(timestamp));
            Ok(value)
        })
        .collect()
}
