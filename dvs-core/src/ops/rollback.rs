//! DVS rollback operation.
//!
//! Restore workspace state to a previous snapshot.

use fs_err as fs;
use std::path::PathBuf;

use crate::helpers::layout::Layout;
use crate::helpers::reflog::{current_actor, Reflog, SnapshotStore};
use crate::types::ReflogOp;
use crate::{detect_backend_cwd, Backend, DvsError, Metadata, RepoBackend};

/// Target for rollback - either a state ID or reflog index.
#[derive(Debug, Clone)]
pub enum RollbackTarget {
    /// State ID (hex string).
    StateId(String),
    /// Reflog index (0 = most recent).
    Index(usize),
}

impl RollbackTarget {
    /// Parse a target string.
    ///
    /// Numeric strings are treated as indices, others as state IDs.
    pub fn parse(s: &str) -> Self {
        if let Ok(index) = s.parse::<usize>() {
            RollbackTarget::Index(index)
        } else {
            RollbackTarget::StateId(s.to_string())
        }
    }
}

/// Result of a rollback operation.
#[derive(Debug, Clone)]
pub struct RollbackResult {
    /// Whether the rollback succeeded.
    pub success: bool,
    /// State ID we rolled back from.
    pub from_state: Option<String>,
    /// State ID we rolled back to.
    pub to_state: String,
    /// Files restored.
    pub restored_files: Vec<PathBuf>,
    /// Files removed (no longer tracked in target state).
    pub removed_files: Vec<PathBuf>,
    /// Error message if failed.
    pub error: Option<String>,
}

impl RollbackResult {
    fn success(
        from_state: Option<String>,
        to_state: String,
        restored_files: Vec<PathBuf>,
        removed_files: Vec<PathBuf>,
    ) -> Self {
        Self {
            success: true,
            from_state,
            to_state,
            restored_files,
            removed_files,
            error: None,
        }
    }
}

/// Rollback to a previous state.
///
/// # Arguments
///
/// * `target` - Target state (ID or reflog index)
/// * `force` - Skip dirty working tree check
/// * `materialize` - Whether to materialize data files (default: true)
///
/// # Errors
///
/// * `NotInitialized` - DVS not initialized
/// * `NotFound` - Target state not found
pub fn rollback(
    target: RollbackTarget,
    force: bool,
    materialize: bool,
) -> Result<RollbackResult, DvsError> {
    let backend = detect_backend_cwd()?;
    rollback_with_backend(&backend, target, force, materialize)
}

/// Rollback with a specific backend.
pub fn rollback_with_backend(
    backend: &Backend,
    target: RollbackTarget,
    force: bool,
    materialize: bool,
) -> Result<RollbackResult, DvsError> {
    let repo_root = backend.root();
    let layout = Layout::new(repo_root.to_path_buf());
    let snapshot_store = SnapshotStore::new(&layout);
    let reflog = Reflog::new(&layout);

    // Resolve target to state ID
    let target_state_id = match &target {
        RollbackTarget::StateId(id) => {
            if !snapshot_store.exists(id) {
                return Err(DvsError::not_found(format!("State not found: {}", id)));
            }
            id.clone()
        }
        RollbackTarget::Index(index) => {
            let entry = reflog.get_by_index(*index)?;
            match entry {
                Some(e) => {
                    // Parse state ID from "state:xxx" format
                    crate::types::ReflogEntry::parse_state_id(&e.new)
                        .ok_or_else(|| {
                            DvsError::not_found(format!("Invalid state ID format: {}", e.new))
                        })?
                        .to_string()
                }
                None => {
                    return Err(DvsError::not_found(format!(
                        "Reflog entry not found at index {}",
                        index
                    )));
                }
            }
        }
    };

    // Get current state
    let current_state_id = reflog.read_head()?;

    // Check if already at target state
    if current_state_id.as_ref() == Some(&target_state_id) {
        return Ok(RollbackResult::success(
            current_state_id,
            target_state_id,
            Vec::new(),
            Vec::new(),
        ));
    }

    // Check for dirty working tree if not forcing
    if !force {
        // For now, we skip this check - could compare current files to current state
        // A proper implementation would check if any tracked files have changed
    }

    // Load target state
    let target_state = snapshot_store.load(&target_state_id)?;

    // Load current state for comparison
    let current_state = if let Some(ref id) = current_state_id {
        if snapshot_store.exists(id) {
            Some(snapshot_store.load(id)?)
        } else {
            None
        }
    } else {
        None
    };

    // Restore metadata files
    let mut restored_files = Vec::new();
    let mut removed_files = Vec::new();

    // Create set of target paths for comparison
    let target_paths: std::collections::HashSet<_> = target_state
        .metadata
        .iter()
        .map(|e| e.path.clone())
        .collect();

    // Restore each metadata entry from target state
    for entry in &target_state.metadata {
        let data_path = repo_root.join(&entry.path);
        let meta_path = Metadata::metadata_path(&data_path);

        // Create parent directory if needed
        if let Some(parent) = meta_path.parent() {
            fs::create_dir_all(parent)?;
        }

        // Save metadata
        entry.meta.save(&meta_path)?;
        restored_files.push(entry.path.clone());
    }

    // Remove metadata files that exist in current state but not in target
    if let Some(ref current) = current_state {
        for entry in &current.metadata {
            if !target_paths.contains(&entry.path) {
                let data_path = repo_root.join(&entry.path);
                let meta_path = Metadata::metadata_path(&data_path);
                if meta_path.exists() {
                    fs::remove_file(&meta_path)?;
                    removed_files.push(entry.path.clone());
                }
            }
        }
    }

    // Materialize data files if requested
    if materialize && !target_state.metadata.is_empty() {
        // TODO: Call materialize operation to restore actual data files
        // For now, we only restore metadata
    }

    // Record rollback in reflog
    reflog.record(
        current_actor(),
        ReflogOp::Rollback,
        Some(format!(
            "Rolled back to {}",
            &target_state_id[..8.min(target_state_id.len())]
        )),
        current_state_id.clone(),
        target_state_id.clone(),
        restored_files.clone(),
    )?;

    Ok(RollbackResult::success(
        current_state_id,
        target_state_id,
        restored_files,
        removed_files,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{MetadataEntry, WorkspaceState};

    fn setup_test_repo() -> (tempfile::TempDir, Backend) {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path();

        // Create .git directory
        fs::create_dir_all(root.join(".git")).unwrap();

        // Create storage directory
        fs::create_dir_all(root.join("storage")).unwrap();

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
    fn test_rollback_target_parse() {
        assert!(matches!(
            RollbackTarget::parse("0"),
            RollbackTarget::Index(0)
        ));
        assert!(matches!(
            RollbackTarget::parse("5"),
            RollbackTarget::Index(5)
        ));
        assert!(matches!(
            RollbackTarget::parse("abc123"),
            RollbackTarget::StateId(_)
        ));
    }

    #[test]
    fn test_rollback_not_found() {
        let (_temp, backend) = setup_test_repo();

        let result = rollback_with_backend(
            &backend,
            RollbackTarget::StateId("nonexistent".to_string()),
            true,
            false,
        );

        assert!(result.is_err());
    }

    #[test]
    fn test_rollback_to_state() {
        let (_temp, backend) = setup_test_repo();
        let repo_root = backend.root();
        let layout = Layout::new(repo_root.to_path_buf());
        let snapshot_store = SnapshotStore::new(&layout);
        let reflog = Reflog::new(&layout);

        // Create an initial empty state
        let state1 = WorkspaceState::empty();
        let state1_id = snapshot_store.save(&state1).unwrap();

        // Record it
        reflog
            .record(
                current_actor(),
                ReflogOp::Init,
                None,
                None,
                state1_id.clone(),
                vec![],
            )
            .unwrap();

        // Create a state with a file
        let meta = Metadata::new(
            "a".repeat(64),
            100,
            Some("test".to_string()),
            "user".to_string(),
        );

        // Actually create the metadata file on disk
        let data_path = repo_root.join("data.csv");
        let meta_path = Metadata::metadata_path(&data_path);
        meta.save(&meta_path).unwrap();

        let state2 = WorkspaceState::new(
            None,
            vec![MetadataEntry::new(PathBuf::from("data.csv"), meta)],
        );
        let state2_id = snapshot_store.save(&state2).unwrap();

        // Record it
        reflog
            .record(
                current_actor(),
                ReflogOp::Add,
                None,
                Some(state1_id.clone()),
                state2_id.clone(),
                vec![PathBuf::from("data.csv")],
            )
            .unwrap();

        // Verify metadata file exists before rollback
        assert!(meta_path.exists());

        // Rollback to state1 (the empty state)
        let result = rollback_with_backend(
            &backend,
            RollbackTarget::StateId(state1_id.clone()),
            true,
            false,
        )
        .unwrap();

        assert!(result.success);
        assert_eq!(result.to_state, state1_id);
        // The file should have been removed
        assert!(result.removed_files.contains(&PathBuf::from("data.csv")));
        // Verify metadata file was deleted
        assert!(!meta_path.exists());
    }

    #[test]
    fn test_rollback_by_index() {
        let (_temp, backend) = setup_test_repo();
        let layout = Layout::new(backend.root().to_path_buf());
        let snapshot_store = SnapshotStore::new(&layout);
        let reflog = Reflog::new(&layout);

        // Create states
        let state1 = WorkspaceState::empty();
        let state1_id = snapshot_store.save(&state1).unwrap();

        reflog
            .record(
                current_actor(),
                ReflogOp::Init,
                None,
                None,
                state1_id.clone(),
                vec![],
            )
            .unwrap();

        // Rollback to index 0 (most recent = current state)
        let result =
            rollback_with_backend(&backend, RollbackTarget::Index(0), true, false).unwrap();

        assert!(result.success);
        // Should be a no-op since we're already at this state
        assert!(result.restored_files.is_empty());
    }
}
