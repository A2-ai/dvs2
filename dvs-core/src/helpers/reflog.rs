//! Reflog helpers for snapshot storage and ref management.
//!
//! Provides persistence for workspace state snapshots and reflog entries:
//! - `SnapshotStore`: Save and load workspace state snapshots
//! - `Reflog`: Manage HEAD ref and append-only log of state changes

use std::io::{BufRead, BufReader, Write};
use fs_err as fs;

use crate::helpers::layout::Layout;
use crate::types::{WorkspaceState, ReflogEntry, ReflogOp};
use crate::DvsError;

/// Snapshot store for workspace state persistence.
///
/// Snapshots are stored as JSON files in `.dvs/state/snapshots/<id>.json`.
pub struct SnapshotStore<'a> {
    layout: &'a Layout,
}

impl<'a> SnapshotStore<'a> {
    /// Create a new snapshot store.
    pub fn new(layout: &'a Layout) -> Self {
        Self { layout }
    }

    /// Save a workspace state and return its ID.
    ///
    /// The snapshot is only written if it doesn't already exist.
    pub fn save(&self, state: &WorkspaceState) -> Result<String, DvsError> {
        let id = state.compute_id()?;
        let path = self.layout.snapshot_path(&id);

        // Only write if it doesn't exist (content-addressed)
        if !path.exists() {
            fs::create_dir_all(self.layout.snapshots_dir())?;
            let json = serde_json::to_string_pretty(state)?;
            fs::write(&path, json)?;
        }

        Ok(id)
    }

    /// Load a workspace state by ID.
    pub fn load(&self, id: &str) -> Result<WorkspaceState, DvsError> {
        let path = self.layout.snapshot_path(id);
        if !path.exists() {
            return Err(DvsError::not_found(format!("Snapshot not found: {}", id)));
        }

        let contents = fs::read_to_string(&path)?;
        let state: WorkspaceState = serde_json::from_str(&contents)?;
        Ok(state)
    }

    /// Check if a snapshot exists.
    pub fn exists(&self, id: &str) -> bool {
        self.layout.snapshot_path(id).exists()
    }

    /// List all snapshot IDs.
    pub fn list(&self) -> Result<Vec<String>, DvsError> {
        let dir = self.layout.snapshots_dir();
        if !dir.exists() {
            return Ok(Vec::new());
        }

        let mut ids = Vec::new();
        for entry in fs::read_dir(&dir)? {
            let entry = entry?;
            let name = entry.file_name();
            let name_str = name.to_string_lossy();
            if let Some(id) = name_str.strip_suffix(".json") {
                ids.push(id.to_string());
            }
        }
        Ok(ids)
    }
}

/// Reflog for tracking state changes.
///
/// Manages the HEAD ref (`.dvs/refs/HEAD`) and the reflog
/// (`.dvs/logs/refs/HEAD`).
pub struct Reflog<'a> {
    layout: &'a Layout,
}

impl<'a> Reflog<'a> {
    /// Create a new reflog.
    pub fn new(layout: &'a Layout) -> Self {
        Self { layout }
    }

    /// Read the current HEAD state ID.
    ///
    /// Returns None if HEAD doesn't exist (no state recorded yet).
    pub fn read_head(&self) -> Result<Option<String>, DvsError> {
        let path = self.layout.head_ref_path();
        if !path.exists() {
            return Ok(None);
        }

        let contents = fs::read_to_string(&path)?;
        let id = contents.trim().to_string();
        if id.is_empty() {
            Ok(None)
        } else {
            Ok(Some(id))
        }
    }

    /// Update the HEAD ref to a new state ID.
    pub fn update_head(&self, id: &str) -> Result<(), DvsError> {
        let path = self.layout.head_ref_path();
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::write(&path, format!("{}\n", id))?;
        Ok(())
    }

    /// Append an entry to the reflog.
    pub fn append(&self, entry: &ReflogEntry) -> Result<(), DvsError> {
        let path = self.layout.head_log_path();
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }

        let line = entry.to_jsonl()?;

        // Open in append mode
        let mut file = fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&path)?;

        writeln!(file, "{}", line)?;
        Ok(())
    }

    /// Read all reflog entries, oldest first.
    pub fn read_all(&self) -> Result<Vec<ReflogEntry>, DvsError> {
        let path = self.layout.head_log_path();
        if !path.exists() {
            return Ok(Vec::new());
        }

        let file = fs::File::open(&path)?;
        let reader = BufReader::new(file);
        let mut entries = Vec::new();

        for line in reader.lines() {
            let line = line?;
            if line.trim().is_empty() {
                continue;
            }
            let entry = ReflogEntry::from_jsonl(&line)?;
            entries.push(entry);
        }

        Ok(entries)
    }

    /// Read reflog entries in reverse order (newest first).
    pub fn read_recent(&self) -> Result<Vec<ReflogEntry>, DvsError> {
        let mut entries = self.read_all()?;
        entries.reverse();
        Ok(entries)
    }

    /// Get the most recent N entries.
    pub fn recent(&self, n: usize) -> Result<Vec<ReflogEntry>, DvsError> {
        let entries = self.read_recent()?;
        Ok(entries.into_iter().take(n).collect())
    }

    /// Record a state change.
    ///
    /// This is a high-level method that:
    /// 1. Updates HEAD to the new state ID
    /// 2. Appends a reflog entry
    pub fn record(
        &self,
        actor: String,
        op: ReflogOp,
        message: Option<String>,
        old_state: Option<String>,
        new_state: String,
        paths: Vec<std::path::PathBuf>,
    ) -> Result<(), DvsError> {
        // Update HEAD
        self.update_head(&new_state)?;

        // Create and append reflog entry
        let entry = ReflogEntry::new(
            actor,
            op,
            message,
            old_state.map(|s| ReflogEntry::format_state_id(&s)),
            ReflogEntry::format_state_id(&new_state),
            paths,
        );
        self.append(&entry)?;

        Ok(())
    }

    /// Get entry by index (0 = most recent).
    pub fn get_by_index(&self, index: usize) -> Result<Option<ReflogEntry>, DvsError> {
        let entries = self.read_recent()?;
        Ok(entries.into_iter().nth(index))
    }

    /// Count total entries.
    pub fn len(&self) -> Result<usize, DvsError> {
        Ok(self.read_all()?.len())
    }

    /// Check if reflog is empty.
    pub fn is_empty(&self) -> Result<bool, DvsError> {
        Ok(self.len()? == 0)
    }
}

/// Get the current username for reflog entries.
pub fn current_actor() -> String {
    whoami::username().unwrap_or_else(|_| "unknown".to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn setup_temp_layout() -> (tempfile::TempDir, Layout) {
        let temp = tempfile::tempdir().unwrap();
        let layout = Layout::new(temp.path().to_path_buf());
        layout.init().unwrap();
        (temp, layout)
    }

    #[test]
    fn test_snapshot_store_save_load() {
        let (_temp, layout) = setup_temp_layout();
        let store = SnapshotStore::new(&layout);

        let state = WorkspaceState::empty();
        let id = store.save(&state).unwrap();

        // ID should be deterministic
        let id2 = store.save(&state).unwrap();
        assert_eq!(id, id2);

        // Should be able to load
        let loaded = store.load(&id).unwrap();
        assert!(loaded.is_empty());
    }

    #[test]
    fn test_snapshot_store_exists() {
        let (_temp, layout) = setup_temp_layout();
        let store = SnapshotStore::new(&layout);

        assert!(!store.exists("nonexistent"));

        let state = WorkspaceState::empty();
        let id = store.save(&state).unwrap();

        assert!(store.exists(&id));
    }

    #[test]
    fn test_snapshot_store_list() {
        let (_temp, layout) = setup_temp_layout();
        let store = SnapshotStore::new(&layout);

        assert!(store.list().unwrap().is_empty());

        let state = WorkspaceState::empty();
        store.save(&state).unwrap();

        assert_eq!(store.list().unwrap().len(), 1);
    }

    #[test]
    fn test_reflog_read_write_head() {
        let (_temp, layout) = setup_temp_layout();
        let reflog = Reflog::new(&layout);

        // Initially empty
        assert!(reflog.read_head().unwrap().is_none());

        // Write HEAD
        reflog.update_head("abc123").unwrap();
        assert_eq!(reflog.read_head().unwrap(), Some("abc123".to_string()));

        // Update HEAD
        reflog.update_head("def456").unwrap();
        assert_eq!(reflog.read_head().unwrap(), Some("def456".to_string()));
    }

    #[test]
    fn test_reflog_append_read() {
        let (_temp, layout) = setup_temp_layout();
        let reflog = Reflog::new(&layout);

        // Initially empty
        assert!(reflog.is_empty().unwrap());

        // Append entries
        let entry1 = ReflogEntry::new(
            "alice".to_string(),
            ReflogOp::Init,
            None,
            None,
            "state:aaa".to_string(),
            vec![],
        );
        reflog.append(&entry1).unwrap();

        let entry2 = ReflogEntry::new(
            "alice".to_string(),
            ReflogOp::Add,
            Some("added file".to_string()),
            Some("state:aaa".to_string()),
            "state:bbb".to_string(),
            vec![PathBuf::from("data.csv")],
        );
        reflog.append(&entry2).unwrap();

        // Read all
        let entries = reflog.read_all().unwrap();
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].op, ReflogOp::Init);
        assert_eq!(entries[1].op, ReflogOp::Add);

        // Read recent (reversed)
        let recent = reflog.read_recent().unwrap();
        assert_eq!(recent[0].op, ReflogOp::Add);
        assert_eq!(recent[1].op, ReflogOp::Init);
    }

    #[test]
    fn test_reflog_record() {
        let (_temp, layout) = setup_temp_layout();
        let reflog = Reflog::new(&layout);

        reflog
            .record(
                "bob".to_string(),
                ReflogOp::Init,
                Some("initial commit".to_string()),
                None,
                "state123".to_string(),
                vec![],
            )
            .unwrap();

        // HEAD should be updated
        assert_eq!(reflog.read_head().unwrap(), Some("state123".to_string()));

        // Reflog should have entry
        let entries = reflog.read_all().unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].actor, "bob");
        assert_eq!(entries[0].new, "state:state123");
    }

    #[test]
    fn test_reflog_get_by_index() {
        let (_temp, layout) = setup_temp_layout();
        let reflog = Reflog::new(&layout);

        reflog.record("a".to_string(), ReflogOp::Init, None, None, "s1".to_string(), vec![]).unwrap();
        reflog.record("a".to_string(), ReflogOp::Add, None, Some("s1".to_string()), "s2".to_string(), vec![]).unwrap();

        // Index 0 = most recent
        let entry = reflog.get_by_index(0).unwrap().unwrap();
        assert_eq!(entry.new, "state:s2");

        // Index 1 = older
        let entry = reflog.get_by_index(1).unwrap().unwrap();
        assert_eq!(entry.new, "state:s1");

        // Index out of bounds
        assert!(reflog.get_by_index(5).unwrap().is_none());
    }
}
