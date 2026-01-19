//! Local directory layout helpers for `.dvs/`.
//!
//! The `.dvs/` directory contains:
//! - `config.toml` - local configuration (optional)
//! - `cache/objects/{algo}/{prefix}/{hash}` - object cache
//! - `state/` - materialization state, locks, etc.
//! - `locks/` - lock files for concurrent operations

use fs_err as fs;
use std::path::{Path, PathBuf};
use crate::DvsError;
use crate::types::{Oid, HashAlgo, Manifest};

/// The local DVS directory name.
pub const DVS_DIR: &str = ".dvs";

/// Layout helper for the `.dvs/` directory structure.
#[derive(Debug, Clone)]
pub struct Layout {
    /// Repository root directory.
    repo_root: PathBuf,
}

impl Layout {
    /// Create a new layout for a repository.
    pub fn new(repo_root: PathBuf) -> Self {
        Self { repo_root }
    }

    /// Get the repository root.
    pub fn repo_root(&self) -> &Path {
        &self.repo_root
    }

    /// Get the `.dvs/` directory path.
    pub fn dvs_dir(&self) -> PathBuf {
        self.repo_root.join(DVS_DIR)
    }

    /// Get the config file path (`.dvs/config.toml`).
    pub fn config_path(&self) -> PathBuf {
        self.dvs_dir().join("config.toml")
    }

    /// Get the cache directory (`.dvs/cache/`).
    pub fn cache_dir(&self) -> PathBuf {
        self.dvs_dir().join("cache")
    }

    /// Get the objects cache directory (`.dvs/cache/objects/`).
    pub fn objects_dir(&self) -> PathBuf {
        self.cache_dir().join("objects")
    }

    /// Get the state directory (`.dvs/state/`).
    pub fn state_dir(&self) -> PathBuf {
        self.dvs_dir().join("state")
    }

    /// Get the locks directory (`.dvs/locks/`).
    pub fn locks_dir(&self) -> PathBuf {
        self.dvs_dir().join("locks")
    }

    /// Get the refs directory (`.dvs/refs/`).
    pub fn refs_dir(&self) -> PathBuf {
        self.dvs_dir().join("refs")
    }

    /// Get the logs directory (`.dvs/logs/`).
    pub fn logs_dir(&self) -> PathBuf {
        self.dvs_dir().join("logs")
    }

    /// Get the snapshots directory (`.dvs/state/snapshots/`).
    pub fn snapshots_dir(&self) -> PathBuf {
        self.state_dir().join("snapshots")
    }

    /// Get the HEAD ref path (`.dvs/refs/HEAD`).
    pub fn head_ref_path(&self) -> PathBuf {
        self.refs_dir().join("HEAD")
    }

    /// Get the HEAD reflog path (`.dvs/logs/refs/HEAD`).
    pub fn head_log_path(&self) -> PathBuf {
        self.logs_dir().join("refs").join("HEAD")
    }

    /// Get a snapshot file path by its ID.
    pub fn snapshot_path(&self, id: &str) -> PathBuf {
        self.snapshots_dir().join(format!("{}.json", id))
    }

    /// Get the manifest file path (`dvs.lock` in repo root).
    pub fn manifest_path(&self) -> PathBuf {
        self.repo_root.join(Manifest::filename())
    }

    /// Get the cached object path for an OID.
    pub fn cached_object_path(&self, oid: &Oid) -> PathBuf {
        self.objects_dir().join(oid.storage_subpath())
    }

    /// Get the materialized file state path.
    ///
    /// Stores info about which files have been materialized.
    pub fn materialized_state_path(&self) -> PathBuf {
        self.state_dir().join("materialized.json")
    }

    /// Get a lock file path for an operation.
    pub fn lock_path(&self, name: &str) -> PathBuf {
        self.locks_dir().join(format!("{}.lock", name))
    }

    /// Initialize the `.dvs/` directory structure.
    pub fn init(&self) -> Result<(), DvsError> {
        // Create main directories
        fs::create_dir_all(self.dvs_dir())?;
        fs::create_dir_all(self.objects_dir())?;
        fs::create_dir_all(self.state_dir())?;
        fs::create_dir_all(self.locks_dir())?;
        fs::create_dir_all(self.refs_dir())?;
        fs::create_dir_all(self.snapshots_dir())?;
        // logs/refs/HEAD parent directory
        if let Some(parent) = self.head_log_path().parent() {
            fs::create_dir_all(parent)?;
        }

        Ok(())
    }

    /// Check if the `.dvs/` directory exists.
    pub fn exists(&self) -> bool {
        self.dvs_dir().exists()
    }

    /// Check if an object is cached locally.
    pub fn is_cached(&self, oid: &Oid) -> bool {
        self.cached_object_path(oid).exists()
    }

    /// Get all cached OIDs (for GC purposes).
    pub fn cached_oids(&self) -> Result<Vec<Oid>, DvsError> {
        let mut oids = Vec::new();
        let objects_dir = self.objects_dir();

        if !objects_dir.exists() {
            return Ok(oids);
        }

        // Walk {algo}/{prefix}/{suffix}
        for algo_entry in fs::read_dir(&objects_dir)? {
            let algo_entry = algo_entry?;
            let algo_name = algo_entry.file_name();
            let algo_str = algo_name.to_string_lossy();

            let algo = match HashAlgo::from_prefix(&algo_str) {
                Some(a) => a,
                None => continue, // Unknown algorithm, skip
            };

            let algo_path = algo_entry.path();
            if !algo_path.is_dir() {
                continue;
            }

            for prefix_entry in fs::read_dir(&algo_path)? {
                let prefix_entry = prefix_entry?;
                let prefix_path = prefix_entry.path();
                if !prefix_path.is_dir() {
                    continue;
                }

                let prefix = prefix_entry.file_name().to_string_lossy().to_string();

                for suffix_entry in fs::read_dir(&prefix_path)? {
                    let suffix_entry = suffix_entry?;
                    let suffix = suffix_entry.file_name().to_string_lossy().to_string();

                    // Reconstruct the full hex
                    let hex = format!("{}{}", prefix, suffix);

                    // Validate hex length
                    if hex.len() == algo.hex_len() && hex.chars().all(|c| c.is_ascii_hexdigit()) {
                        oids.push(Oid::new(algo, hex));
                    }
                }
            }
        }

        Ok(oids)
    }
}

/// Materialization state tracking.
#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
pub struct MaterializedState {
    /// Map from path to OID of last materialized version.
    pub files: std::collections::HashMap<PathBuf, String>,

    /// Timestamp of last materialization.
    #[serde(default)]
    pub last_materialized: Option<String>,
}

impl MaterializedState {
    /// Load state from file.
    pub fn load(path: &Path) -> Result<Self, DvsError> {
        if !path.exists() {
            return Ok(Self::default());
        }
        let contents = fs::read_to_string(path)?;
        let state: MaterializedState = serde_json::from_str(&contents)?;
        Ok(state)
    }

    /// Save state to file.
    pub fn save(&self, path: &Path) -> Result<(), DvsError> {
        let json = serde_json::to_string_pretty(self)?;
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::write(path, json)?;
        Ok(())
    }

    /// Check if a file needs materialization.
    pub fn needs_materialize(&self, path: &Path, oid: &str) -> bool {
        match self.files.get(path) {
            Some(existing_oid) => existing_oid != oid,
            None => true,
        }
    }

    /// Mark a file as materialized.
    pub fn mark_materialized(&mut self, path: PathBuf, oid: String) {
        self.files.insert(path, oid);
        self.last_materialized = Some(chrono::Utc::now().to_rfc3339());
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::HashAlgo;

    fn test_oid() -> Oid {
        Oid::new(HashAlgo::Blake3, "a".repeat(64))
    }

    #[test]
    fn test_layout_paths() {
        let layout = Layout::new(PathBuf::from("/repo"));

        assert_eq!(layout.dvs_dir(), PathBuf::from("/repo/.dvs"));
        assert_eq!(layout.config_path(), PathBuf::from("/repo/.dvs/config.toml"));
        assert_eq!(layout.cache_dir(), PathBuf::from("/repo/.dvs/cache"));
        assert_eq!(layout.objects_dir(), PathBuf::from("/repo/.dvs/cache/objects"));
        assert_eq!(layout.state_dir(), PathBuf::from("/repo/.dvs/state"));
        assert_eq!(layout.locks_dir(), PathBuf::from("/repo/.dvs/locks"));
        assert_eq!(layout.manifest_path(), PathBuf::from("/repo/dvs.lock"));
        // Reflog paths
        assert_eq!(layout.refs_dir(), PathBuf::from("/repo/.dvs/refs"));
        assert_eq!(layout.logs_dir(), PathBuf::from("/repo/.dvs/logs"));
        assert_eq!(layout.snapshots_dir(), PathBuf::from("/repo/.dvs/state/snapshots"));
        assert_eq!(layout.head_ref_path(), PathBuf::from("/repo/.dvs/refs/HEAD"));
        assert_eq!(layout.head_log_path(), PathBuf::from("/repo/.dvs/logs/refs/HEAD"));
        assert_eq!(layout.snapshot_path("abc123"), PathBuf::from("/repo/.dvs/state/snapshots/abc123.json"));
    }

    #[test]
    fn test_layout_cached_object_path() {
        let layout = Layout::new(PathBuf::from("/repo"));
        let oid = test_oid();
        let path = layout.cached_object_path(&oid);

        assert!(path.to_string_lossy().contains("blake3"));
        assert!(path.to_string_lossy().contains("/aa/"));
    }

    #[test]
    fn test_layout_init() {
        let temp_dir = std::env::temp_dir().join("dvs-test-layout-init");
        let _ = fs::remove_dir_all(&temp_dir);

        let layout = Layout::new(temp_dir.clone());
        assert!(!layout.exists());

        layout.init().unwrap();
        assert!(layout.exists());
        assert!(layout.objects_dir().exists());
        assert!(layout.state_dir().exists());
        assert!(layout.locks_dir().exists());
        // Reflog directories
        assert!(layout.refs_dir().exists());
        assert!(layout.snapshots_dir().exists());
        assert!(layout.head_log_path().parent().unwrap().exists());

        let _ = fs::remove_dir_all(&temp_dir);
    }

    #[test]
    fn test_layout_is_cached() {
        let temp_dir = std::env::temp_dir().join("dvs-test-layout-cached");
        let _ = fs::remove_dir_all(&temp_dir);

        let layout = Layout::new(temp_dir.clone());
        layout.init().unwrap();

        let oid = test_oid();
        assert!(!layout.is_cached(&oid));

        // Create cached object
        let cached_path = layout.cached_object_path(&oid);
        fs::create_dir_all(cached_path.parent().unwrap()).unwrap();
        fs::write(&cached_path, b"content").unwrap();

        assert!(layout.is_cached(&oid));

        let _ = fs::remove_dir_all(&temp_dir);
    }

    #[test]
    fn test_materialized_state() {
        let mut state = MaterializedState::default();

        let path = PathBuf::from("data/file.csv");
        let oid = "abc123";

        assert!(state.needs_materialize(&path, oid));

        state.mark_materialized(path.clone(), oid.to_string());
        assert!(!state.needs_materialize(&path, oid));
        assert!(state.needs_materialize(&path, "different"));
    }

    #[test]
    fn test_materialized_state_roundtrip() {
        let temp_dir = std::env::temp_dir().join("dvs-test-mat-state");
        let _ = fs::create_dir_all(&temp_dir);

        let state_path = temp_dir.join("state.json");

        let mut state = MaterializedState::default();
        state.mark_materialized(PathBuf::from("a.txt"), "oid1".to_string());
        state.save(&state_path).unwrap();

        let loaded = MaterializedState::load(&state_path).unwrap();
        assert_eq!(loaded.files.len(), 1);
        assert!(!loaded.needs_materialize(Path::new("a.txt"), "oid1"));

        let _ = fs::remove_dir_all(&temp_dir);
    }
}
