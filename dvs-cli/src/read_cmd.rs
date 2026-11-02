//! Batch, backend-neutral queries for consumers that invoke the CLI.
use anyhow::Result;
use dvs::{Backend, DvsPaths, ErrorCode, PathFilter};
use serde_json::{Value, json};
use std::path::PathBuf;

use crate::output;

fn invalid(message: impl Into<String>) -> anyhow::Error {
    output::coded(ErrorCode::InvalidArguments, message)
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

/// Resolve query paths lexically; the files need not exist in this checkout.
/// Each entry is the audit entry with `timestamp` as RFC 3339 UTC.
/// Reject escaping paths rather than silently broadening an invalid query to all history.
pub fn history(
    backend: &Backend,
    paths: &DvsPaths,
    user_paths: Vec<PathBuf>,
) -> Result<Vec<Value>> {
    let relative = resolve_paths(paths, user_paths)?;
    backend.check_readable()?;
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
}
