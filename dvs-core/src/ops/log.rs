//! DVS log operation.
//!
//! View reflog entries showing the history of DVS state changes.

use crate::helpers::layout::Layout;
use crate::helpers::reflog::Reflog;
use crate::types::ReflogEntry;
use crate::{detect_backend_cwd, Backend, DvsError, RepoBackend};

/// Log entry for display.
#[derive(Debug, Clone)]
pub struct LogEntry {
    /// 0-based index (0 = most recent).
    pub index: usize,
    /// The underlying reflog entry.
    pub entry: ReflogEntry,
}

/// View reflog entries.
///
/// Returns entries in reverse chronological order (newest first).
///
/// # Arguments
///
/// * `limit` - Maximum number of entries to return (None = all)
///
/// # Errors
///
/// * `NotInitialized` - DVS not initialized
pub fn log(limit: Option<usize>) -> Result<Vec<LogEntry>, DvsError> {
    let backend = detect_backend_cwd()?;
    log_with_backend(&backend, limit)
}

/// View reflog entries with a specific backend.
pub fn log_with_backend(
    backend: &Backend,
    limit: Option<usize>,
) -> Result<Vec<LogEntry>, DvsError> {
    let layout = Layout::new(backend.root().to_path_buf());
    let reflog = Reflog::new(&layout);

    let entries = reflog.read_recent()?;

    let entries: Vec<LogEntry> = entries
        .into_iter()
        .enumerate()
        .take(limit.unwrap_or(usize::MAX))
        .map(|(index, entry)| LogEntry { index, entry })
        .collect();

    Ok(entries)
}

/// Get a specific reflog entry by index.
///
/// Index 0 is the most recent entry.
pub fn log_entry(index: usize) -> Result<Option<LogEntry>, DvsError> {
    let backend = detect_backend_cwd()?;
    log_entry_with_backend(&backend, index)
}

/// Get a specific reflog entry with a specific backend.
pub fn log_entry_with_backend(
    backend: &Backend,
    index: usize,
) -> Result<Option<LogEntry>, DvsError> {
    let layout = Layout::new(backend.root().to_path_buf());
    let reflog = Reflog::new(&layout);

    Ok(reflog
        .get_by_index(index)?
        .map(|entry| LogEntry { index, entry }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::helpers::reflog::current_actor;
    use crate::types::ReflogOp;
    use fs_err as fs;
    use std::path::PathBuf;

    fn setup_test_repo() -> (tempfile::TempDir, Backend) {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path();

        // Create .git directory
        fs::create_dir_all(root.join(".git")).unwrap();

        // Create .dvs directory
        let layout = Layout::new(root.to_path_buf());
        layout.init().unwrap();

        // Create config file
        let config = crate::Config::new(root.join("storage"), None, None);
        config
            .save(&root.join(crate::Config::config_filename()))
            .unwrap();

        let backend = crate::detect_backend(root).unwrap();
        (temp, backend)
    }

    #[test]
    fn test_log_empty() {
        let (_temp, backend) = setup_test_repo();
        let entries = log_with_backend(&backend, None).unwrap();
        assert!(entries.is_empty());
    }

    #[test]
    fn test_log_with_entries() {
        let (_temp, backend) = setup_test_repo();
        let layout = Layout::new(backend.root().to_path_buf());
        let reflog = Reflog::new(&layout);

        // Add some entries
        reflog
            .record(
                current_actor(),
                ReflogOp::Init,
                Some("initial".to_string()),
                None,
                "state1".to_string(),
                vec![],
            )
            .unwrap();

        reflog
            .record(
                current_actor(),
                ReflogOp::Add,
                Some("added file".to_string()),
                Some("state1".to_string()),
                "state2".to_string(),
                vec![PathBuf::from("data.csv")],
            )
            .unwrap();

        let entries = log_with_backend(&backend, None).unwrap();
        assert_eq!(entries.len(), 2);
        // Most recent first
        assert_eq!(entries[0].index, 0);
        assert_eq!(entries[0].entry.op, ReflogOp::Add);
        assert_eq!(entries[1].index, 1);
        assert_eq!(entries[1].entry.op, ReflogOp::Init);
    }

    #[test]
    fn test_log_with_limit() {
        let (_temp, backend) = setup_test_repo();
        let layout = Layout::new(backend.root().to_path_buf());
        let reflog = Reflog::new(&layout);

        // Add several entries
        for i in 0..5 {
            reflog
                .record(
                    current_actor(),
                    ReflogOp::Add,
                    None,
                    if i == 0 {
                        None
                    } else {
                        Some(format!("state{}", i - 1))
                    },
                    format!("state{}", i),
                    vec![],
                )
                .unwrap();
        }

        let entries = log_with_backend(&backend, Some(3)).unwrap();
        assert_eq!(entries.len(), 3);
    }

    #[test]
    fn test_log_entry() {
        let (_temp, backend) = setup_test_repo();
        let layout = Layout::new(backend.root().to_path_buf());
        let reflog = Reflog::new(&layout);

        reflog
            .record(
                current_actor(),
                ReflogOp::Init,
                None,
                None,
                "state1".to_string(),
                vec![],
            )
            .unwrap();

        let entry = log_entry_with_backend(&backend, 0).unwrap();
        assert!(entry.is_some());
        assert_eq!(entry.unwrap().entry.op, ReflogOp::Init);

        let none = log_entry_with_backend(&backend, 10).unwrap();
        assert!(none.is_none());
    }
}
