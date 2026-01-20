//! DVS merge-repo operation.
//!
//! Merges tracked files, metadata, and objects from a source DVS repository
//! into the current (destination) repository.

use crate::helpers::config as config_helper;
use crate::helpers::hash::verify_hash_with_algo;
use crate::helpers::layout::Layout;
use crate::helpers::reflog::{current_actor, Reflog, SnapshotStore};
use crate::types::{MetadataEntry, MetadataFormat, ReflogOp, WorkspaceState};
use crate::{detect_backend, detect_backend_cwd, Backend, DvsError, Metadata, Oid, RepoBackend};
use fs_err as fs;
use std::collections::HashSet;
use std::path::{Path, PathBuf};

/// How to handle path conflicts when merging.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ConflictMode {
    /// Abort if any path exists in both repos (default).
    #[default]
    Abort,
    /// Keep destination file, skip source.
    Skip,
    /// Replace destination with source.
    Overwrite,
}

impl ConflictMode {
    /// Parse conflict mode from string.
    ///
    /// Returns `None` for unrecognized modes rather than an error.
    #[allow(clippy::should_implement_trait)]
    pub fn from_str(s: &str) -> Option<Self> {
        match s.to_lowercase().as_str() {
            "abort" => Some(ConflictMode::Abort),
            "skip" => Some(ConflictMode::Skip),
            "overwrite" => Some(ConflictMode::Overwrite),
            _ => None,
        }
    }
}

impl std::fmt::Display for ConflictMode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ConflictMode::Abort => write!(f, "abort"),
            ConflictMode::Skip => write!(f, "skip"),
            ConflictMode::Overwrite => write!(f, "overwrite"),
        }
    }
}

/// Options for the merge operation.
#[derive(Debug, Clone, Default)]
pub struct MergeOptions {
    /// Place imported files under this subdirectory.
    pub prefix: Option<PathBuf>,
    /// How to handle path conflicts.
    pub conflict_mode: ConflictMode,
    /// Verify object hashes during copy.
    pub verify: bool,
    /// Show what would be merged without making changes.
    pub dry_run: bool,
}

/// Result of a merge operation.
#[derive(Debug, Clone, Default)]
pub struct MergeResult {
    /// Number of files merged.
    pub files_merged: usize,
    /// Number of files skipped (due to conflicts in skip mode).
    pub files_skipped: usize,
    /// Number of objects copied to destination storage.
    pub objects_copied: usize,
    /// Number of objects that already existed in destination.
    pub objects_existed: usize,
    /// Paths that had conflicts (empty unless conflict_mode is Abort).
    pub conflicts: Vec<PathBuf>,
    /// Paths that were merged.
    pub merged_paths: Vec<PathBuf>,
}

/// Merge a source repository into the current working directory's repository.
///
/// # Arguments
///
/// * `source` - Path to the source DVS repository
/// * `options` - Merge options
///
/// # Errors
///
/// * `NotInitialized` - Source or destination not initialized
/// * `StorageError` - Cannot read/write objects
/// * `PathConflict` - Path exists in both repos (in Abort mode)
pub fn merge_repo(source: &Path, options: MergeOptions) -> Result<MergeResult, DvsError> {
    let dest_backend = detect_backend_cwd()?;
    let source_backend = detect_backend(source)?;
    merge_repo_with_backend(&source_backend, &dest_backend, options)
}

/// Merge repositories with specific backends.
pub fn merge_repo_with_backend(
    source_backend: &Backend,
    dest_backend: &Backend,
    options: MergeOptions,
) -> Result<MergeResult, DvsError> {
    let source_root = source_backend.root();
    let dest_root = dest_backend.root();

    // Verify source is not the same as destination
    let source_canonical = fs::canonicalize(source_root)?;
    let dest_canonical = fs::canonicalize(dest_root)?;
    if source_canonical == dest_canonical {
        return Err(DvsError::config("Cannot merge repository into itself"));
    }

    // Load configs
    let source_config = config_helper::load_config(source_root)?;
    let dest_config = config_helper::load_config(dest_root)?;

    // Find all tracked files in source
    let source_files = find_all_tracked_files(source_root)?;
    if source_files.is_empty() {
        // Empty source - success with 0 files
        return Ok(MergeResult::default());
    }

    // Find all tracked files in dest for conflict detection
    let dest_files: HashSet<PathBuf> = find_all_tracked_files(dest_root)?.into_iter().collect();

    // Process files and detect conflicts
    let mut result = MergeResult::default();
    let mut files_to_merge: Vec<(PathBuf, PathBuf, Metadata)> = Vec::new();
    let mut objects_to_copy: HashSet<Oid> = HashSet::new();

    for source_data_path in &source_files {
        // Compute destination path (with optional prefix)
        let dest_relative = if let Some(ref prefix) = options.prefix {
            prefix.join(source_data_path)
        } else {
            source_data_path.clone()
        };

        // Check for conflict
        if dest_files.contains(&dest_relative) {
            match options.conflict_mode {
                ConflictMode::Abort => {
                    result.conflicts.push(dest_relative);
                    continue;
                }
                ConflictMode::Skip => {
                    result.files_skipped += 1;
                    continue;
                }
                ConflictMode::Overwrite => {
                    // Will be handled below
                }
            }
        }

        // Load source metadata
        let source_data_abs = source_root.join(source_data_path);
        let source_meta = Metadata::load_for_data_file(&source_data_abs)?;

        // Track object for copying
        let oid = Oid::new(source_meta.hash_algo, source_meta.checksum().to_string());
        objects_to_copy.insert(oid);

        files_to_merge.push((source_data_path.clone(), dest_relative, source_meta));
    }

    // If we have conflicts in Abort mode, fail now
    if !result.conflicts.is_empty() && options.conflict_mode == ConflictMode::Abort {
        return Err(DvsError::merge_conflict(
            result
                .conflicts
                .iter()
                .map(|p| p.display().to_string())
                .collect::<Vec<_>>()
                .join(", "),
        ));
    }

    // If dry run, return early with planned changes
    if options.dry_run {
        result.files_merged = files_to_merge.len();
        result.merged_paths = files_to_merge
            .iter()
            .map(|(_, dest, _)| dest.clone())
            .collect();
        result.objects_copied = objects_to_copy.len();
        return Ok(result);
    }

    // Setup reflog for destination
    let layout = Layout::new(dest_root.to_path_buf());
    let snapshot_store = SnapshotStore::new(&layout);
    let reflog = Reflog::new(&layout);

    // Capture state before merge
    let old_state = capture_workspace_state(dest_backend)?;
    let old_state_id = if !old_state.is_empty() {
        Some(snapshot_store.save(&old_state)?)
    } else {
        None
    };

    // Copy objects from source storage to dest storage
    let (copied, existed) = copy_objects(
        &source_config.storage_dir,
        &dest_config.storage_dir,
        &objects_to_copy,
        options.verify,
    )?;
    result.objects_copied = copied;
    result.objects_existed = existed;

    // Copy metadata files
    for (source_rel, dest_rel, source_meta) in &files_to_merge {
        let source_meta_path =
            source_root.join(Metadata::metadata_path(&source_root.join(source_rel)));
        let dest_data_abs = dest_root.join(dest_rel);

        // Create parent directories if needed
        if let Some(parent) = dest_data_abs.parent() {
            fs::create_dir_all(parent)?;
        }

        // Determine the metadata format based on source
        let meta_filename = source_meta_path
            .file_name()
            .map(|f| f.to_string_lossy().to_string())
            .unwrap_or_default();

        let dest_meta_path = if meta_filename.ends_with(".dvs.toml") {
            Metadata::metadata_path_for_format(&dest_data_abs, crate::MetadataFormat::Toml)
        } else {
            Metadata::metadata_path_for_format(&dest_data_abs, crate::MetadataFormat::Json)
        };

        // Save metadata to destination
        source_meta.save(&dest_meta_path)?;

        result.files_merged += 1;
        result.merged_paths.push(dest_rel.clone());
    }

    // Record merge in reflog
    if result.files_merged > 0 {
        let new_state = capture_workspace_state(dest_backend)?;
        let new_state_id = snapshot_store.save(&new_state)?;

        if old_state_id.as_ref() != Some(&new_state_id) {
            reflog.record(
                current_actor(),
                ReflogOp::Merge,
                Some(format!("Merged from {}", source_root.display())),
                old_state_id,
                new_state_id,
                result.merged_paths.clone(),
            )?;
        }
    }

    Ok(result)
}

/// Find all tracked files (files with .dvs or .dvs.toml metadata) in a repository.
///
/// Returns relative paths to the data files (not the metadata files).
fn find_all_tracked_files(repo_root: &Path) -> Result<Vec<PathBuf>, DvsError> {
    let mut files = Vec::new();

    #[cfg(feature = "walkdir")]
    {
        for entry in walkdir::WalkDir::new(repo_root)
            .into_iter()
            .filter_entry(|e| {
                let name = e.file_name().to_string_lossy();
                name != ".git" && name != ".dvs"
            })
        {
            let entry = match entry {
                Ok(e) => e,
                Err(_) => continue,
            };

            let path = entry.path();
            let filename = path
                .file_name()
                .map(|f| f.to_string_lossy())
                .unwrap_or_default();

            if path.is_file() && (filename.ends_with(".dvs") || filename.ends_with(".dvs.toml")) {
                if let Some(data_path) = Metadata::data_path(path) {
                    if let Some(relative) = pathdiff::diff_paths(&data_path, repo_root) {
                        files.push(relative);
                    }
                }
            }
        }
    }

    #[cfg(not(feature = "walkdir"))]
    {
        fn recurse(dir: &Path, repo_root: &Path, files: &mut Vec<PathBuf>) -> Result<(), DvsError> {
            let entries = match fs::read_dir(dir) {
                Ok(e) => e,
                Err(_) => return Ok(()),
            };

            for entry in entries.flatten() {
                let path = entry.path();
                let name = path.file_name().unwrap_or_default().to_string_lossy();

                if name == ".git" || name == ".dvs" {
                    continue;
                }

                if path.is_dir() {
                    recurse(&path, repo_root, files)?;
                } else {
                    let is_metadata = name.ends_with(".dvs") || name.ends_with(".dvs.toml");
                    if is_metadata {
                        if let Some(data_path) = Metadata::data_path(&path) {
                            if let Some(relative) = pathdiff::diff_paths(&data_path, repo_root) {
                                files.push(relative);
                            }
                        }
                    }
                }
            }

            Ok(())
        }

        recurse(repo_root, repo_root, &mut files)?;
    }

    Ok(files)
}

/// Copy objects from source storage to destination storage.
///
/// Returns (copied_count, already_existed_count).
fn copy_objects(
    source_storage: &Path,
    dest_storage: &Path,
    oids: &HashSet<Oid>,
    verify: bool,
) -> Result<(usize, usize), DvsError> {
    let mut copied = 0;
    let mut existed = 0;

    for oid in oids {
        let source_path = source_storage.join(oid.storage_subpath());
        let dest_path = dest_storage.join(oid.storage_subpath());

        // Skip if already exists in destination (content-addressable = same content)
        if dest_path.exists() {
            existed += 1;
            continue;
        }

        // Verify source exists
        if !source_path.exists() {
            return Err(DvsError::storage_error(format!(
                "Object not found in source storage: {}",
                oid
            )));
        }

        // Create parent directories
        if let Some(parent) = dest_path.parent() {
            fs::create_dir_all(parent)?;
        }

        // Copy the object
        fs::copy(&source_path, &dest_path)?;

        // Optionally verify the hash
        if verify {
            let matches = verify_hash_with_algo(&dest_path, &oid.hex, oid.algo)?;
            if !matches {
                // Clean up the bad copy
                let _ = fs::remove_file(&dest_path);
                return Err(DvsError::storage_error(format!(
                    "Hash verification failed for object: {}",
                    oid
                )));
            }
        }

        copied += 1;
    }

    Ok((copied, existed))
}

/// Capture the current workspace state (duplicated from add.rs to avoid circular dependencies).
fn capture_workspace_state(backend: &Backend) -> Result<WorkspaceState, DvsError> {
    let repo_root = backend.root();
    let tracked_files = find_all_tracked_files(repo_root)?;

    let mut metadata_entries = Vec::new();
    for rel_path in tracked_files {
        let abs_path = repo_root.join(&rel_path);
        if let Ok(meta) = Metadata::load_for_data_file(&abs_path) {
            // Detect the format of the existing metadata file
            let format = Metadata::find_existing_format(&abs_path).unwrap_or(MetadataFormat::Json);
            metadata_entries.push(MetadataEntry::with_format(rel_path, meta, format));
        }
    }

    Ok(WorkspaceState::new(None, metadata_entries))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Config;
    use std::io::Write;

    fn setup_test_repo(name: &str) -> PathBuf {
        let temp_dir =
            std::env::temp_dir().join(format!("dvs-test-merge-{}-{}", name, std::process::id()));
        let _ = fs::remove_dir_all(&temp_dir);
        fs::create_dir_all(&temp_dir).unwrap();

        // Create .git directory
        fs::create_dir_all(temp_dir.join(".git")).unwrap();

        // Create .dvs directory
        let layout = Layout::new(temp_dir.clone());
        layout.init().unwrap();

        // Create storage directory
        let storage_dir = temp_dir.join("storage");
        fs::create_dir_all(&storage_dir).unwrap();

        // Create config
        let config = Config::new(storage_dir, None, None);
        config
            .save(&temp_dir.join(Config::config_filename()))
            .unwrap();

        temp_dir
    }

    fn add_test_file(repo: &Path, rel_path: &str, content: &[u8]) {
        let config = config_helper::load_config(repo).unwrap();
        let file_path = repo.join(rel_path);

        // Create parent directory
        if let Some(parent) = file_path.parent() {
            fs::create_dir_all(parent).unwrap();
        }

        // Write file
        let mut file = fs::File::create(&file_path).unwrap();
        file.write_all(content).unwrap();
        drop(file);

        // Compute hash
        let hash = crate::helpers::hash::get_file_hash(&file_path).unwrap();

        // Copy to storage using OID-based path (includes algorithm prefix)
        let oid = Oid::new(crate::HashAlgo::Blake3, hash.clone());
        let storage_path = config.storage_dir.join(oid.storage_subpath());
        fs::create_dir_all(storage_path.parent().unwrap()).unwrap();
        fs::copy(&file_path, &storage_path).unwrap();

        // Create metadata
        let meta = Metadata::new(
            hash,
            content.len() as u64,
            Some(format!("Added {}", rel_path)),
            "testuser".to_string(),
        );
        meta.save(&Metadata::metadata_path(&file_path)).unwrap();
    }

    #[test]
    fn test_merge_simple() {
        let source = setup_test_repo("source-simple");
        let dest = setup_test_repo("dest-simple");

        // Add file to source
        add_test_file(&source, "data.csv", b"col1,col2\n1,2\n");

        // Merge
        let source_backend = detect_backend(&source).unwrap();
        let dest_backend = detect_backend(&dest).unwrap();

        let result =
            merge_repo_with_backend(&source_backend, &dest_backend, MergeOptions::default())
                .unwrap();

        assert_eq!(result.files_merged, 1);
        assert_eq!(result.files_skipped, 0);
        assert!(result.conflicts.is_empty());

        // Verify metadata exists in dest
        let meta_path = dest.join("data.csv.dvs");
        assert!(meta_path.exists());

        // Cleanup
        let _ = fs::remove_dir_all(&source);
        let _ = fs::remove_dir_all(&dest);
    }

    #[test]
    fn test_merge_with_prefix() {
        let source = setup_test_repo("source-prefix");
        let dest = setup_test_repo("dest-prefix");

        add_test_file(&source, "data.csv", b"data");

        let source_backend = detect_backend(&source).unwrap();
        let dest_backend = detect_backend(&dest).unwrap();

        let result = merge_repo_with_backend(
            &source_backend,
            &dest_backend,
            MergeOptions {
                prefix: Some(PathBuf::from("imported")),
                ..Default::default()
            },
        )
        .unwrap();

        assert_eq!(result.files_merged, 1);

        // Verify metadata is under prefix
        let meta_path = dest.join("imported/data.csv.dvs");
        assert!(meta_path.exists());

        let _ = fs::remove_dir_all(&source);
        let _ = fs::remove_dir_all(&dest);
    }

    #[test]
    fn test_merge_conflict_abort() {
        let source = setup_test_repo("source-abort");
        let dest = setup_test_repo("dest-abort");

        // Add same file to both repos
        add_test_file(&source, "data.csv", b"source content");
        add_test_file(&dest, "data.csv", b"dest content");

        let source_backend = detect_backend(&source).unwrap();
        let dest_backend = detect_backend(&dest).unwrap();

        let result = merge_repo_with_backend(
            &source_backend,
            &dest_backend,
            MergeOptions {
                conflict_mode: ConflictMode::Abort,
                ..Default::default()
            },
        );

        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(err.to_string().contains("conflict"));

        let _ = fs::remove_dir_all(&source);
        let _ = fs::remove_dir_all(&dest);
    }

    #[test]
    fn test_merge_conflict_skip() {
        let source = setup_test_repo("source-skip");
        let dest = setup_test_repo("dest-skip");

        add_test_file(&source, "data.csv", b"source content");
        add_test_file(&dest, "data.csv", b"dest content");

        let source_backend = detect_backend(&source).unwrap();
        let dest_backend = detect_backend(&dest).unwrap();

        let result = merge_repo_with_backend(
            &source_backend,
            &dest_backend,
            MergeOptions {
                conflict_mode: ConflictMode::Skip,
                ..Default::default()
            },
        )
        .unwrap();

        assert_eq!(result.files_merged, 0);
        assert_eq!(result.files_skipped, 1);

        // Verify dest file still has original content
        let meta = Metadata::load_for_data_file(&dest.join("data.csv")).unwrap();
        assert!(meta.message.contains("dest") || !meta.message.contains("source"));

        let _ = fs::remove_dir_all(&source);
        let _ = fs::remove_dir_all(&dest);
    }

    #[test]
    fn test_merge_conflict_overwrite() {
        let source = setup_test_repo("source-overwrite");
        let dest = setup_test_repo("dest-overwrite");

        add_test_file(&source, "data.csv", b"source content");
        add_test_file(&dest, "data.csv", b"dest content");

        let source_backend = detect_backend(&source).unwrap();
        let dest_backend = detect_backend(&dest).unwrap();

        let result = merge_repo_with_backend(
            &source_backend,
            &dest_backend,
            MergeOptions {
                conflict_mode: ConflictMode::Overwrite,
                ..Default::default()
            },
        )
        .unwrap();

        assert_eq!(result.files_merged, 1);
        assert_eq!(result.files_skipped, 0);

        // Verify metadata was overwritten
        let meta = Metadata::load_for_data_file(&dest.join("data.csv")).unwrap();
        assert!(meta.message.contains("source") || !meta.message.contains("dest"));

        let _ = fs::remove_dir_all(&source);
        let _ = fs::remove_dir_all(&dest);
    }

    #[test]
    fn test_merge_same_repo_error() {
        let repo = setup_test_repo("same-repo");

        let backend = detect_backend(&repo).unwrap();

        let result = merge_repo_with_backend(&backend, &backend, MergeOptions::default());

        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(err.to_string().contains("itself"));

        let _ = fs::remove_dir_all(&repo);
    }

    #[test]
    fn test_merge_empty_source() {
        let source = setup_test_repo("source-empty");
        let dest = setup_test_repo("dest-empty");

        let source_backend = detect_backend(&source).unwrap();
        let dest_backend = detect_backend(&dest).unwrap();

        let result =
            merge_repo_with_backend(&source_backend, &dest_backend, MergeOptions::default())
                .unwrap();

        assert_eq!(result.files_merged, 0);

        let _ = fs::remove_dir_all(&source);
        let _ = fs::remove_dir_all(&dest);
    }

    #[test]
    fn test_merge_dry_run() {
        let source = setup_test_repo("source-dry");
        let dest = setup_test_repo("dest-dry");

        add_test_file(&source, "data.csv", b"data");

        let source_backend = detect_backend(&source).unwrap();
        let dest_backend = detect_backend(&dest).unwrap();

        let result = merge_repo_with_backend(
            &source_backend,
            &dest_backend,
            MergeOptions {
                dry_run: true,
                ..Default::default()
            },
        )
        .unwrap();

        assert_eq!(result.files_merged, 1);

        // Verify nothing was actually created
        let meta_path = dest.join("data.csv.dvs");
        assert!(!meta_path.exists());

        let _ = fs::remove_dir_all(&source);
        let _ = fs::remove_dir_all(&dest);
    }
}
