//! Workspace snapshot utilities for conformance testing.

use dvs_core::{Config, Manifest, Metadata};
use fs_err as fs;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use crate::repo::{TestRepo, TestRepoError};

/// A snapshot of DVS workspace state for comparison.
///
/// Captures:
/// - Tracked files and their metadata
/// - Storage objects (presence only)
/// - Config state
/// - Manifest state (if present)
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct WorkspaceSnapshot {
    /// Tracked files: relative path -> metadata snapshot.
    pub tracked_files: BTreeMap<PathBuf, FileSnapshot>,

    /// Objects present in storage (relative paths).
    pub storage_objects: BTreeSet<PathBuf>,

    /// Config state (if initialized).
    pub config: Option<ConfigSnapshot>,

    /// Manifest state (if present).
    pub manifest: Option<ManifestSnapshot>,

    /// .gitignore contains *.dvs pattern.
    pub gitignore_has_dvs: bool,
}

/// Snapshot of a tracked file's metadata.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct FileSnapshot {
    /// The file's checksum.
    pub checksum: String,

    /// The hash algorithm used.
    pub hash_algo: String,

    /// File size in bytes.
    pub file_bytes: u64,

    /// Whether the data file exists locally.
    pub data_exists: bool,

    /// Whether the storage object exists.
    pub storage_exists: ObjectPresence,
}

/// Whether a storage object is present.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum ObjectPresence {
    /// Object exists in storage.
    Present,
    /// Object is missing from storage.
    Missing,
    /// Could not determine (e.g., remote storage).
    Unknown,
}

/// Snapshot of DVS config.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ConfigSnapshot {
    /// Storage directory (relative to repo root).
    pub storage_dir: PathBuf,

    /// Hash algorithm setting.
    pub hash_algo: Option<String>,

    /// Permissions setting.
    pub permissions: Option<u32>,

    /// Group setting.
    pub group: Option<String>,
}

/// Snapshot of manifest state.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ManifestSnapshot {
    /// Number of entries.
    pub entry_count: usize,

    /// Base URL (if set).
    pub base_url: Option<String>,

    /// Entry paths.
    pub paths: BTreeSet<PathBuf>,
}

impl WorkspaceSnapshot {
    /// Capture a snapshot of the workspace state.
    pub fn capture(repo: &TestRepo) -> Result<Self, SnapshotError> {
        let mut tracked_files = BTreeMap::new();

        // Find all .dvs metadata files
        let meta_files = repo.list_metadata_files()?;

        for meta_path in meta_files {
            let full_meta_path = repo.root().join(&meta_path);

            // Load metadata
            let metadata = Metadata::load(&full_meta_path)?;

            // Determine data file path (strip .dvs extension)
            let data_path = meta_path.with_extension("");

            // Check if data file exists
            let data_exists = repo.file_exists(&data_path.to_string_lossy());

            // Check if storage object exists
            let storage_exists = check_storage_object(repo.storage_dir(), metadata.checksum());

            let snapshot = FileSnapshot {
                checksum: metadata.checksum().to_string(),
                hash_algo: format!("{:?}", metadata.hash_algo),
                file_bytes: metadata.size,
                data_exists,
                storage_exists,
            };

            tracked_files.insert(data_path, snapshot);
        }

        // List storage objects
        let storage_objects = repo.list_storage_objects()?.into_iter().collect();

        // Load config if present
        let config = if repo.config_path().exists() {
            let cfg = Config::load(&repo.config_path())?;
            Some(ConfigSnapshot {
                storage_dir: cfg.storage_dir.clone(),
                hash_algo: cfg.hash_algo.map(|a| format!("{:?}", a)),
                permissions: cfg.permissions,
                group: cfg.group.clone(),
            })
        } else {
            None
        };

        // Load manifest if present
        let manifest = if repo.manifest_path().exists() {
            let mf = Manifest::load(&repo.manifest_path())?;
            Some(ManifestSnapshot {
                entry_count: mf.entries.len(),
                base_url: mf.base_url.clone(),
                paths: mf.entries.iter().map(|e| e.path.clone()).collect(),
            })
        } else {
            None
        };

        // Check .gitignore for *.dvs
        let gitignore_has_dvs = check_gitignore_has_dvs(repo.root());

        Ok(Self {
            tracked_files,
            storage_objects,
            config,
            manifest,
            gitignore_has_dvs,
        })
    }

    /// Check if this snapshot represents an initialized workspace.
    pub fn is_initialized(&self) -> bool {
        self.config.is_some()
    }

    /// Get the number of tracked files.
    pub fn tracked_count(&self) -> usize {
        self.tracked_files.len()
    }

    /// Get the number of storage objects.
    pub fn storage_count(&self) -> usize {
        self.storage_objects.len()
    }

    /// Check if a specific file is tracked.
    pub fn is_tracked(&self, path: &Path) -> bool {
        self.tracked_files.contains_key(path)
    }

    /// Get the snapshot for a tracked file.
    pub fn get_file(&self, path: &Path) -> Option<&FileSnapshot> {
        self.tracked_files.get(path)
    }
}

/// Check if a storage object exists.
fn check_storage_object(storage_dir: &Path, checksum: &str) -> ObjectPresence {
    // Storage uses prefix/suffix format: first 2 chars / rest
    if checksum.len() < 3 {
        return ObjectPresence::Unknown;
    }

    let prefix = &checksum[..2];
    let suffix = &checksum[2..];
    let object_path = storage_dir.join(prefix).join(suffix);

    if object_path.exists() {
        ObjectPresence::Present
    } else {
        ObjectPresence::Missing
    }
}

/// Check if .gitignore contains *.dvs pattern.
fn check_gitignore_has_dvs(repo_root: &Path) -> bool {
    let gitignore_path = repo_root.join(".gitignore");
    if !gitignore_path.exists() {
        return false;
    }

    match fs::read_to_string(&gitignore_path) {
        Ok(contents) => contents.lines().any(|line| {
            let trimmed = line.trim();
            trimmed == "*.dvs" || trimmed == "*.dvs\n"
        }),
        Err(_) => false,
    }
}

/// Error type for snapshot operations.
#[derive(Debug)]
pub enum SnapshotError {
    /// Test repo error.
    Repo(TestRepoError),
    /// DVS error.
    Dvs(dvs_core::DvsError),
}

impl std::fmt::Display for SnapshotError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SnapshotError::Repo(e) => write!(f, "repo error: {}", e),
            SnapshotError::Dvs(e) => write!(f, "dvs error: {}", e),
        }
    }
}

impl std::error::Error for SnapshotError {}

impl From<TestRepoError> for SnapshotError {
    fn from(e: TestRepoError) -> Self {
        SnapshotError::Repo(e)
    }
}

impl From<dvs_core::DvsError> for SnapshotError {
    fn from(e: dvs_core::DvsError) -> Self {
        SnapshotError::Dvs(e)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_empty_snapshot() {
        let repo = TestRepo::new().unwrap();
        let snapshot = WorkspaceSnapshot::capture(&repo).unwrap();

        assert!(!snapshot.is_initialized());
        assert_eq!(snapshot.tracked_count(), 0);
        assert_eq!(snapshot.storage_count(), 0);
        assert!(!snapshot.gitignore_has_dvs);
    }

    #[test]
    fn test_object_presence_check() {
        let repo = TestRepo::new().unwrap();

        // Create a fake storage object
        let checksum = "abcdef1234567890";
        let prefix = &checksum[..2];
        let suffix = &checksum[2..];
        let obj_dir = repo.storage_dir().join(prefix);
        fs::create_dir_all(&obj_dir).unwrap();
        fs::write(obj_dir.join(suffix), b"test").unwrap();

        let presence = check_storage_object(repo.storage_dir(), checksum);
        assert_eq!(presence, ObjectPresence::Present);

        let missing = check_storage_object(repo.storage_dir(), "missing123");
        assert_eq!(missing, ObjectPresence::Missing);
    }
}
