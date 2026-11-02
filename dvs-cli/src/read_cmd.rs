//! Batch, backend-neutral queries for consumers that invoke the CLI.
use anyhow::Result;
use dvs::{Backend, DvsPaths, ErrorCode, Hashes, PathFilter};
use serde_json::{Value, json};
use std::path::PathBuf;

use crate::output;

fn invalid(message: impl Into<String>) -> anyhow::Error {
    output::coded(ErrorCode::InvalidArguments, message)
}

fn check_backend(backend: &Backend) -> Result<()> {
    match backend {
        Backend::Server(s) => s.check_access()?,
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
                    "takes files, not folders: {}",
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

fn coded_message(error: &anyhow::Error) -> (ErrorCode, String) {
    (output::error_code(error), format!("{error:#}"))
}

/// Why `hash` is not a full BLAKE3 hash, if it is not one.
fn invalid_hash(hash: &str) -> Option<String> {
    if hash.is_empty() || !hash.bytes().all(|c| c.is_ascii_hexdigit()) {
        Some(format!("invalid BLAKE3 hash: {hash} is not hexadecimal"))
    } else if hash.len() != 64 {
        // Most likely the start that the history table shows
        Some(format!(
            "invalid BLAKE3 hash: {hash} has {} characters, a BLAKE3 hash has 64. The history \
             table shows only the start of a hash. dvs history --json shows all of it.",
            hash.len()
        ))
    } else {
        None
    }
}

/// Validate hashes before I/O and preserve input order and duplicates.
/// `state` describes blob storage.
pub fn availability(backend: &Backend, hashes: &[String]) -> Result<(Vec<Value>, usize)> {
    if let Some(message) = hashes.iter().find_map(|hash| invalid_hash(hash)) {
        return Err(invalid(message));
    }
    let preflight = check_backend(backend).map_err(|error| coded_message(&error));

    let mut rows = Vec::with_capacity(hashes.len());
    let mut failed = 0;
    for hash in hashes {
        let normalized = hash.to_ascii_lowercase();
        let value = Hashes {
            blake3: normalized.clone(),
            md5: None,
        };
        let storage = preflight.clone().and_then(|()| {
            backend
                .exists(&value)
                .map_err(|error| coded_message(&error))
        });
        let (state, code, message) = match storage {
            Ok(present) => (if present { "present" } else { "absent" }, None, None),
            Err((code, message)) => {
                failed += 1;
                ("unknown", Some(code), Some(message))
            }
        };
        rows.push(json!({
            "hash": normalized,
            "state": state,
            "code": code,
            "message": message,
            "hint": null,
        }));
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
    backend
        .get_audit_entries(&relative)?
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn history_refuses_folders() {
        let root = tempfile::tempdir().unwrap();
        std::fs::create_dir(root.path().join("data")).unwrap();
        let paths =
            DvsPaths::new(root.path().to_path_buf(), root.path().to_path_buf(), ".dvs").unwrap();
        let error = resolve_paths(&paths, vec![PathBuf::from("data")]).unwrap_err();
        assert_eq!(output::error_code(&error), ErrorCode::InvalidArguments);
        assert_eq!(output::exit_code(&error), output::EXIT_USAGE);
        assert_eq!(error.to_string(), "takes files, not folders: data");
    }

    #[test]
    fn a_short_hash_says_where_the_full_one_is() {
        let full = "d1cd1ec45291d06cdde016568971990c7e4da895f2e5a8a705d4feeb79578a69";
        assert_eq!(invalid_hash(full), None);
        assert_eq!(invalid_hash(&full.to_uppercase()), None);
        let short = invalid_hash(&full[..12]).unwrap();
        assert!(
            short.starts_with("invalid BLAKE3 hash: d1cd1ec45291 has 12 characters"),
            "{short}"
        );
        assert!(
            short.contains("dvs history --json shows all of it"),
            "{short}"
        );
        assert!(
            invalid_hash(&format!("{full}0"))
                .unwrap()
                .contains("has 65 characters")
        );
        for text in ["xyz", "", &format!("{}g", &full[..63])] {
            assert_eq!(
                invalid_hash(text),
                Some(format!("invalid BLAKE3 hash: {text} is not hexadecimal"))
            );
        }
        // Refused before storage is looked at: a missing store would give rows
        let root = tempfile::tempdir().unwrap();
        let store =
            dvs::backends::local::LocalBackend::new(root.path().join("missing"), None).unwrap();
        let error = availability(
            &Backend::Local(store),
            &[full.to_string(), full[..12].to_string()],
        )
        .unwrap_err();
        assert_eq!(output::error_code(&error), ErrorCode::InvalidArguments);
        assert_eq!(output::exit_code(&error), output::EXIT_USAGE);
    }
}
