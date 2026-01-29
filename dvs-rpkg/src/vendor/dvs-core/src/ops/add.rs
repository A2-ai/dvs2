//! DVS add operation.

use crate::helpers::layout::Layout;
use crate::helpers::reflog::{current_actor, Reflog, SnapshotStore};
use crate::helpers::{config as config_helper, copy, file, hash};
use crate::types::{
    Manifest, ManifestEntry, MetadataEntry, MetadataFormat, Oid, ReflogOp, WorkspaceState,
};
use crate::{
    detect_backend_cwd, AddResult, Backend, Config, DvsError, Metadata, Outcome, RepoBackend,
};
use fs_err as fs;
use glob::glob;
use std::path::{Path, PathBuf};

/// Add files to DVS tracking.
///
/// Computes hashes, creates metadata files, and copies files to storage.
///
/// # Arguments
///
/// * `files` - File paths or glob patterns to add
/// * `message` - Optional message describing this version
///
/// # Returns
///
/// A vector of results, one per file (including errors).
///
/// # Errors
///
/// * `NotInitialized` - DVS not initialized
/// * `BatchError` - Multiple explicit paths don't exist
pub fn add(files: &[PathBuf], message: Option<&str>) -> Result<Vec<AddResult>, DvsError> {
    let backend = detect_backend_cwd()?;
    add_with_backend(&backend, files, message, None)
}

/// Add files with a specific metadata format override.
///
/// Use this when you want to override the config's default metadata format.
pub fn add_with_format(
    files: &[PathBuf],
    message: Option<&str>,
    metadata_format: Option<MetadataFormat>,
) -> Result<Vec<AddResult>, DvsError> {
    let backend = detect_backend_cwd()?;
    add_with_backend(&backend, files, message, metadata_format)
}

/// Add files with a specific backend.
///
/// Use this when you already have a backend reference.
///
/// # Arguments
///
/// * `backend` - The repository backend
/// * `files` - Files to add (paths or glob patterns)
/// * `message` - Optional message describing this version
/// * `metadata_format` - Optional format override (uses config default if None)
pub fn add_with_backend(
    backend: &Backend,
    files: &[PathBuf],
    message: Option<&str>,
    metadata_format: Option<MetadataFormat>,
) -> Result<Vec<AddResult>, DvsError> {
    let repo_root = backend.root();

    // Load configuration
    let config = config_helper::load_config(repo_root)?;

    // Setup reflog infrastructure
    let layout = Layout::new(repo_root.to_path_buf());
    let snapshot_store = SnapshotStore::new(&layout);
    let reflog = Reflog::new(&layout);

    // Load or create manifest (dvs.lock)
    let manifest_path = layout.manifest_path();
    let mut manifest = if manifest_path.exists() {
        Manifest::load(&manifest_path)?
    } else {
        Manifest::new()
    };

    // Capture state before add
    let old_state = capture_workspace_state(backend)?;
    let old_state_id = if !old_state.is_empty() {
        Some(snapshot_store.save(&old_state)?)
    } else {
        None
    };

    // Expand glob patterns
    let expanded_files = expand_globs(backend, files)?;

    if expanded_files.is_empty() {
        return Err(DvsError::no_files_matched(
            files
                .iter()
                .map(|p| p.display().to_string())
                .collect::<Vec<_>>()
                .join(", "),
        ));
    }

    // Process each file
    let mut results = Vec::with_capacity(expanded_files.len());
    let mut changed_paths = Vec::new();
    let mut manifest_updated = false;

    for path in expanded_files {
        let result = add_single_file(backend, &path, message, &config, metadata_format);

        // Track paths that were actually added/updated
        if result.outcome == Outcome::Copied || result.outcome == Outcome::Present {
            // Update manifest entry for successfully tracked files
            let oid = Oid::new(config.hash_algorithm(), result.blake3_checksum.clone());
            let entry = ManifestEntry::new(result.relative_path.clone(), oid, result.size);
            manifest.upsert(entry);
            manifest_updated = true;

            if result.outcome == Outcome::Copied {
                changed_paths.push(result.relative_path.clone());
            }
        }
        results.push(result);
    }

    // Save manifest if updated
    if manifest_updated {
        manifest.save(&manifest_path)?;
    }

    // Record state change in reflog if anything changed
    if !changed_paths.is_empty() {
        let new_state = capture_workspace_state(backend)?;
        let new_state_id = snapshot_store.save(&new_state)?;

        // Only record if state actually changed
        if old_state_id.as_ref() != Some(&new_state_id) {
            reflog.record(
                current_actor(),
                ReflogOp::Add,
                message.map(|s| s.to_string()),
                old_state_id,
                new_state_id,
                changed_paths,
            )?;
        }
    }

    Ok(results)
}

/// Capture the current workspace state.
///
/// Collects all metadata files and manifest, returning a WorkspaceState snapshot.
fn capture_workspace_state(backend: &Backend) -> Result<WorkspaceState, DvsError> {
    let repo_root = backend.root();

    #[cfg(feature = "walkdir")]
    let metadata_entries = capture_metadata_walkdir(repo_root)?;

    #[cfg(not(feature = "walkdir"))]
    let metadata_entries = capture_metadata_recursive(repo_root)?;

    // Load manifest (dvs.lock) if present
    let layout = Layout::new(repo_root.to_path_buf());
    let manifest = if layout.manifest_path().exists() {
        Manifest::load(&layout.manifest_path()).ok()
    } else {
        None
    };

    Ok(WorkspaceState::new(manifest, metadata_entries))
}

/// Capture metadata using walkdir crate.
#[cfg(feature = "walkdir")]
fn capture_metadata_walkdir(repo_root: &Path) -> Result<Vec<MetadataEntry>, DvsError> {
    let mut metadata_entries = Vec::new();

    for entry in walkdir::WalkDir::new(repo_root)
        .into_iter()
        .filter_entry(|e| {
            // Skip .git and .dvs directories
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
        // Check for both .dvs (JSON) and .dvs.toml (TOML) files
        let format = if filename.ends_with(".dvs.toml") {
            Some(MetadataFormat::Toml)
        } else if filename.ends_with(".dvs") {
            Some(MetadataFormat::Json)
        } else {
            None
        };
        if let Some(format) = format {
            if path.is_file() {
                // Load metadata
                if let Ok(meta) = Metadata::load(path) {
                    // Get the relative path to the data file
                    if let Some(data_path) = Metadata::data_path(path) {
                        if let Some(relative) = pathdiff::diff_paths(&data_path, repo_root) {
                            metadata_entries
                                .push(MetadataEntry::with_format(relative, meta, format));
                        }
                    }
                }
            }
        }
    }

    Ok(metadata_entries)
}

/// Capture metadata using recursive fs::read_dir (fallback when walkdir disabled).
#[cfg(not(feature = "walkdir"))]
fn capture_metadata_recursive(repo_root: &Path) -> Result<Vec<MetadataEntry>, DvsError> {
    fn recurse(
        dir: &Path,
        repo_root: &Path,
        entries: &mut Vec<MetadataEntry>,
    ) -> Result<(), DvsError> {
        let dir_entries = match fs::read_dir(dir) {
            Ok(e) => e,
            Err(_) => return Ok(()), // Skip unreadable directories
        };

        for entry in dir_entries.flatten() {
            let path = entry.path();
            let name = path.file_name().unwrap_or_default().to_string_lossy();

            // Skip .git and .dvs directories
            if name == ".git" || name == ".dvs" {
                continue;
            }

            if path.is_dir() {
                recurse(&path, repo_root, entries)?;
            } else {
                let filename = path
                    .file_name()
                    .map(|f| f.to_string_lossy())
                    .unwrap_or_default();
                // Check for both .dvs (JSON) and .dvs.toml (TOML) files
                let format = if filename.ends_with(".dvs.toml") {
                    Some(MetadataFormat::Toml)
                } else if filename.ends_with(".dvs") {
                    Some(MetadataFormat::Json)
                } else {
                    None
                };
                if let Some(format) = format {
                    // Load metadata
                    if let Ok(meta) = Metadata::load(&path) {
                        // Get the relative path to the data file
                        if let Some(data_path) = Metadata::data_path(&path) {
                            if let Some(relative) = pathdiff::diff_paths(&data_path, repo_root) {
                                entries.push(MetadataEntry::with_format(relative, meta, format));
                            }
                        }
                    }
                }
            }
        }

        Ok(())
    }

    let mut metadata_entries = Vec::new();
    recurse(repo_root, repo_root, &mut metadata_entries)?;
    Ok(metadata_entries)
}

/// Expand glob patterns and filter files.
fn expand_globs(backend: &Backend, patterns: &[PathBuf]) -> Result<Vec<PathBuf>, DvsError> {
    let mut files = Vec::new();
    let repo_root = backend.root();

    for pattern in patterns {
        let pattern_str = pattern.to_string_lossy();

        // Check if it's a glob pattern
        if pattern_str.contains('*') || pattern_str.contains('?') || pattern_str.contains('[') {
            // Expand relative to repo root
            let full_pattern = if pattern.is_relative() {
                repo_root.join(pattern)
            } else {
                pattern.clone()
            };

            match glob(&full_pattern.to_string_lossy()) {
                Ok(paths) => {
                    for entry in paths.flatten() {
                        let is_ignored = backend.is_ignored(&entry).unwrap_or(false);
                        if entry.is_file() && !is_ignored {
                            files.push(entry);
                        }
                    }
                }
                Err(_) => {
                    return Err(DvsError::invalid_glob(pattern_str.to_string())
                        .context("check glob syntax: * matches any chars, ** matches directories"));
                }
            }
        } else {
            // Regular file path
            let full_path = if pattern.is_relative() {
                repo_root.join(pattern)
            } else {
                pattern.clone()
            };

            if full_path.exists() && full_path.is_file() {
                files.push(full_path);
            } else if full_path.exists() && full_path.is_dir() {
                // Include directories so add_single_file can report proper error
                files.push(full_path);
            } else if !full_path.exists() {
                // File doesn't exist - we'll handle this error in add_single_file
                files.push(full_path);
            }
        }
    }

    Ok(files)
}

/// Process a single file for adding.
fn add_single_file(
    backend: &Backend,
    path: &Path,
    message: Option<&str>,
    config: &Config,
    format_override: Option<MetadataFormat>,
) -> AddResult {
    let repo_root = backend.root();

    // Compute relative path
    let relative_path = match pathdiff::diff_paths(path, repo_root) {
        Some(p) => p,
        None => {
            return AddResult::error(
                path.display().to_string(),
                "file_outside_repo".to_string(),
                format!("File is outside repository: {}", path.display()),
            );
        }
    };

    // Check if file exists
    if !path.exists() {
        return AddResult::error(
            path.display().to_string(),
            "file_not_found".to_string(),
            format!("File not found: {}", path.display()),
        );
    }

    // Check if path is a directory (not a file)
    if path.is_dir() {
        return AddResult::error(
            path.display().to_string(),
            "is_directory".to_string(),
            format!(
                "Cannot add directory: {} (use glob pattern to add files)",
                path.display()
            ),
        );
    }

    // Resolve symlinks and validate path security
    // canonicalize() resolves all symlinks, "..", and "." components.
    // This also detects symlink loops (returns an error if a loop is found).
    let canonical_path = match path.canonicalize() {
        Ok(p) => p,
        Err(e) => {
            // Check if it's a broken symlink
            if path.is_symlink() {
                return AddResult::error(
                    path.display().to_string(),
                    "broken_symlink".to_string(),
                    format!("Broken or invalid symlink: {} ({})", path.display(), e),
                );
            } else {
                return AddResult::error(
                    path.display().to_string(),
                    "path_error".to_string(),
                    format!("Cannot resolve path: {} - {}", path.display(), e),
                );
            }
        }
    };

    // Security check: ensure resolved path stays within repo root
    // This prevents symlink attacks that point outside the repository
    let canonical_repo_root = match repo_root.canonicalize() {
        Ok(p) => p,
        Err(e) => {
            return AddResult::error(
                path.display().to_string(),
                "repo_error".to_string(),
                format!("Cannot resolve repo root: {} - {}", repo_root.display(), e),
            );
        }
    };

    if !canonical_path.starts_with(&canonical_repo_root) {
        return AddResult::error(
            path.display().to_string(),
            "path_traversal".to_string(),
            format!(
                "Security: resolved path {} is outside repository root {}",
                canonical_path.display(),
                canonical_repo_root.display()
            ),
        );
    }

    // Note: For symlinks pointing to valid targets within the repo,
    // we operate on the resolved (canonical) path for hashing and storage,
    // which ensures consistent content-addressable behavior.

    // Get file size
    let size = match file::get_file_size(path) {
        Ok(s) => s,
        Err(e) => {
            return AddResult::error(
                path.display().to_string(),
                "io_error".to_string(),
                e.to_string(),
            );
        }
    };

    // Compute hash using configured algorithm
    let hash_algo = config.hash_algorithm();
    let checksum = match hash::get_file_hash_with_algo(path, hash_algo) {
        Ok(h) => h,
        Err(e) => {
            return AddResult::error(
                path.display().to_string(),
                "hash_error".to_string(),
                e.to_string(),
            );
        }
    };

    // Check if file already exists in storage with same hash
    let storage_path = hash::storage_path_for_hash(&config.storage_dir, hash_algo, &checksum);
    // Use format override if provided, otherwise use config default
    let metadata_format = format_override.unwrap_or_else(|| config.metadata_format());
    let metadata_path = Metadata::metadata_path_for_format(path, metadata_format);

    // Check if already present with same hash (check both JSON and TOML formats)
    if let Some(existing_format) = Metadata::find_existing_format(path) {
        let existing_path = Metadata::metadata_path_for_format(path, existing_format);
        if let Ok(existing_meta) = Metadata::load(&existing_path) {
            if existing_meta.checksum() == checksum && existing_meta.hash_algo == hash_algo {
                // Same file already tracked with same algorithm
                return AddResult::success(
                    relative_path,
                    path.to_path_buf(),
                    Outcome::Present,
                    size,
                    checksum,
                );
            }
        }
    }

    // Copy to storage if not already there
    if !storage_path.exists() {
        if let Err(e) = copy::copy_to_storage(
            path,
            &storage_path,
            config.permissions,
            config.group.as_deref(),
        ) {
            return AddResult::error(
                path.display().to_string(),
                "storage_error".to_string(),
                e.to_string(),
            );
        }
    }

    // Create metadata with configured hash algorithm
    let username = file::get_current_username().unwrap_or_else(|_| "unknown".to_string());
    let metadata = Metadata::with_algo(
        checksum.clone(),
        size,
        message.map(|s| s.to_string()),
        username,
        hash_algo,
    );

    // Save metadata
    if let Err(e) = metadata.save(&metadata_path) {
        // Attempt rollback
        let _ = rollback_add(&metadata_path, &storage_path);
        return AddResult::error(
            path.display().to_string(),
            "metadata_error".to_string(),
            e.to_string(),
        );
    }

    AddResult::success(
        relative_path,
        path.to_path_buf(),
        Outcome::Copied,
        size,
        checksum,
    )
}

/// Rollback metadata and storage on error.
fn rollback_add(metadata_path: &Path, storage_path: &Path) -> Result<(), DvsError> {
    // Remove metadata file if it was created
    if metadata_path.exists() {
        fs::remove_file(metadata_path)?;
    }

    // Note: We don't remove from storage because other files may reference the same hash
    // Storage cleanup should be done separately via a gc operation
    let _ = storage_path; // Acknowledge we're not using this

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use fs_err as fs;
    use std::io::Write;
    use std::sync::atomic::{AtomicU64, Ordering};

    static TEST_COUNTER: AtomicU64 = AtomicU64::new(0);

    fn setup_test_repo(test_name: &str) -> (PathBuf, PathBuf) {
        let unique_id = TEST_COUNTER.fetch_add(1, Ordering::SeqCst);
        let temp_dir = std::env::temp_dir().join(format!(
            "dvs-test-add-{}-{}-{}",
            std::process::id(),
            test_name,
            unique_id
        ));
        let _ = fs::remove_dir_all(&temp_dir);
        fs::create_dir_all(&temp_dir).unwrap();

        // Create a fake .git directory to make it a git repo
        fs::create_dir_all(temp_dir.join(".git")).unwrap();

        // Create storage directory
        let storage_dir = temp_dir.join("storage");
        fs::create_dir_all(&storage_dir).unwrap();

        // Create config file
        let config = Config::new(storage_dir.clone(), None, None);
        config
            .save(&temp_dir.join(Config::config_filename()))
            .unwrap();

        (temp_dir, storage_dir)
    }

    #[test]
    fn test_expand_globs_literal() {
        let (temp_dir, _storage) = setup_test_repo("expand_globs_literal");

        // Create a test file
        let test_file = temp_dir.join("test.txt");
        fs::write(&test_file, "content").unwrap();

        // Create backend
        let backend = crate::detect_backend(&temp_dir).unwrap();

        // Expand
        let patterns = vec![PathBuf::from("test.txt")];
        let files = expand_globs(&backend, &patterns).unwrap();

        assert_eq!(files.len(), 1);
        assert!(files[0].ends_with("test.txt"));

        let _ = fs::remove_dir_all(&temp_dir);
    }

    #[test]
    fn test_expand_globs_pattern() {
        let (temp_dir, _storage) = setup_test_repo("expand_globs_pattern");

        // Create test files
        fs::write(temp_dir.join("file1.txt"), "1").unwrap();
        fs::write(temp_dir.join("file2.txt"), "2").unwrap();
        fs::write(temp_dir.join("other.csv"), "3").unwrap();

        // Create backend
        let backend = crate::detect_backend(&temp_dir).unwrap();

        // Expand
        let patterns = vec![PathBuf::from("*.txt")];
        let files = expand_globs(&backend, &patterns).unwrap();

        assert_eq!(files.len(), 2);

        let _ = fs::remove_dir_all(&temp_dir);
    }

    #[test]
    fn test_add_single_file_success() {
        let (temp_dir, storage_dir) = setup_test_repo("add_single_file_success");

        // Create a test file
        let test_file = temp_dir.join("data.csv");
        let mut file = fs::File::create(&test_file).unwrap();
        file.write_all(b"col1,col2\n1,2\n3,4\n").unwrap();
        drop(file);

        // Create backend and config
        let backend = crate::detect_backend(&temp_dir).unwrap();
        let config = Config::new(storage_dir.clone(), None, None);

        // Add the file
        let result = add_single_file(&backend, &test_file, Some("test message"), &config, None);

        assert_eq!(result.outcome, Outcome::Copied);
        assert!(!result.blake3_checksum.is_empty());
        assert_eq!(result.size, 18); // "col1,col2\n1,2\n3,4\n"

        // Verify metadata file exists
        let meta_path = Metadata::metadata_path(&test_file);
        assert!(meta_path.exists());

        // Verify storage file exists (uses default algorithm from config)
        let storage_path = hash::storage_path_for_hash(
            &storage_dir,
            config.hash_algorithm(),
            &result.blake3_checksum,
        );
        assert!(storage_path.exists());

        let _ = fs::remove_dir_all(&temp_dir);
    }

    #[test]
    fn test_add_single_file_already_present() {
        let (temp_dir, storage_dir) = setup_test_repo("add_single_file_already_present");

        // Create a test file
        let test_file = temp_dir.join("data.csv");
        fs::write(&test_file, "content").unwrap();

        // Create backend and config
        let backend = crate::detect_backend(&temp_dir).unwrap();
        let config = Config::new(storage_dir.clone(), None, None);

        // Add the file first time
        let result1 = add_single_file(&backend, &test_file, None, &config, None);
        assert_eq!(result1.outcome, Outcome::Copied);

        // Add the same file again (unchanged)
        let result2 = add_single_file(&backend, &test_file, None, &config, None);
        assert_eq!(result2.outcome, Outcome::Present);

        let _ = fs::remove_dir_all(&temp_dir);
    }

    #[test]
    fn test_add_file_not_found() {
        let (temp_dir, storage_dir) = setup_test_repo("add_file_not_found");

        let backend = crate::detect_backend(&temp_dir).unwrap();
        let config = Config::new(storage_dir, None, None);

        let nonexistent = temp_dir.join("nonexistent.txt");
        let result = add_single_file(&backend, &nonexistent, None, &config, None);

        assert_eq!(result.outcome, Outcome::Error);
        assert!(result.error_message.unwrap().contains("not found"));

        let _ = fs::remove_dir_all(&temp_dir);
    }

    #[test]
    fn test_add_updates_manifest() {
        let (temp_dir, _storage_dir) = setup_test_repo("add_updates_manifest");

        // Create test files
        fs::write(temp_dir.join("file1.txt"), "content one").unwrap();
        fs::write(temp_dir.join("file2.txt"), "content two").unwrap();

        // Create .dvs directory for layout
        let layout = Layout::new(temp_dir.clone());
        layout.init().unwrap();

        // Add files using the full add_with_backend function
        let backend = crate::detect_backend(&temp_dir).unwrap();
        let files = vec![PathBuf::from("file1.txt"), PathBuf::from("file2.txt")];
        let results = add_with_backend(&backend, &files, Some("test add"), None).unwrap();

        // Verify both files were added
        assert_eq!(results.len(), 2);
        assert_eq!(results[0].outcome, Outcome::Copied);
        assert_eq!(results[1].outcome, Outcome::Copied);

        // Verify manifest was created and contains entries
        let manifest_path = layout.manifest_path();
        assert!(manifest_path.exists(), "dvs.lock should be created");

        let manifest = Manifest::load(&manifest_path).unwrap();
        assert_eq!(manifest.len(), 2, "Manifest should have 2 entries");

        // Verify entries have correct paths
        assert!(manifest.get(std::path::Path::new("file1.txt")).is_some());
        assert!(manifest.get(std::path::Path::new("file2.txt")).is_some());

        // Verify entries have correct OIDs
        let entry1 = manifest.get(std::path::Path::new("file1.txt")).unwrap();
        assert_eq!(entry1.bytes, 11); // "content one".len()
        assert!(!entry1.oid.hex.is_empty());

        let _ = fs::remove_dir_all(&temp_dir);
    }

    #[test]
    fn test_add_updates_existing_manifest() {
        let (temp_dir, _storage_dir) = setup_test_repo("add_updates_existing_manifest");

        // Create test file
        fs::write(temp_dir.join("file1.txt"), "original content").unwrap();

        // Create .dvs directory for layout
        let layout = Layout::new(temp_dir.clone());
        layout.init().unwrap();

        let backend = crate::detect_backend(&temp_dir).unwrap();

        // First add
        let results = add_with_backend(
            &backend,
            &[PathBuf::from("file1.txt")],
            Some("first add"),
            None,
        )
        .unwrap();
        assert_eq!(results[0].outcome, Outcome::Copied);

        let manifest = Manifest::load(&layout.manifest_path()).unwrap();
        let original_oid = manifest
            .get(std::path::Path::new("file1.txt"))
            .unwrap()
            .oid
            .clone();

        // Modify file and add again
        fs::write(temp_dir.join("file1.txt"), "modified content").unwrap();
        let results = add_with_backend(
            &backend,
            &[PathBuf::from("file1.txt")],
            Some("second add"),
            None,
        )
        .unwrap();
        assert_eq!(results[0].outcome, Outcome::Copied);

        // Verify manifest was updated with new OID
        let manifest = Manifest::load(&layout.manifest_path()).unwrap();
        let new_entry = manifest.get(std::path::Path::new("file1.txt")).unwrap();
        assert_ne!(new_entry.oid, original_oid, "OID should have changed");
        assert_eq!(manifest.len(), 1, "Should still have only 1 entry");

        let _ = fs::remove_dir_all(&temp_dir);
    }

    /// Helper to create a test repo with a specific hash algorithm.
    #[allow(dead_code)]
    fn setup_test_repo_with_algo(
        test_name: &str,
        hash_algo: crate::HashAlgo,
    ) -> (PathBuf, PathBuf) {
        let unique_id = TEST_COUNTER.fetch_add(1, Ordering::SeqCst);
        let temp_dir = std::env::temp_dir().join(format!(
            "dvs-test-add-{}-{}-{}",
            std::process::id(),
            test_name,
            unique_id
        ));
        let _ = fs::remove_dir_all(&temp_dir);
        fs::create_dir_all(&temp_dir).unwrap();

        // Create a fake .git directory
        fs::create_dir_all(temp_dir.join(".git")).unwrap();

        // Create storage directory
        let storage_dir = temp_dir.join("storage");
        fs::create_dir_all(&storage_dir).unwrap();

        // Create config file with specific hash algorithm
        let config = Config::with_hash_algo(storage_dir.clone(), None, None, hash_algo);
        config
            .save(&temp_dir.join(Config::config_filename()))
            .unwrap();

        (temp_dir, storage_dir)
    }

    #[test]
    #[cfg(feature = "sha256")]
    fn test_add_with_sha256_algorithm() {
        use crate::HashAlgo;

        let (temp_dir, storage_dir) = setup_test_repo_with_algo("add_sha256", HashAlgo::Sha256);

        // Create a test file
        let test_file = temp_dir.join("data.csv");
        let content = b"sha256 test content";
        fs::write(&test_file, content).unwrap();

        // Create .dvs directory
        let layout = Layout::new(temp_dir.clone());
        layout.init().unwrap();

        // Add the file
        let backend = crate::detect_backend(&temp_dir).unwrap();
        let results = add_with_backend(&backend, &[PathBuf::from("data.csv")], None, None).unwrap();

        assert_eq!(results.len(), 1);
        assert_eq!(results[0].outcome, Outcome::Copied);
        assert!(!results[0].blake3_checksum.is_empty());

        // Verify metadata has sha256 algorithm
        let metadata = Metadata::load_for_data_file(&test_file).unwrap();
        assert_eq!(metadata.hash_algo, HashAlgo::Sha256);

        // Verify storage path uses sha256 prefix
        let storage_path = crate::helpers::hash::storage_path_for_hash(
            &storage_dir,
            HashAlgo::Sha256,
            &results[0].blake3_checksum,
        );
        assert!(storage_path.exists(), "Storage file should exist");
        assert!(
            storage_path.to_string_lossy().contains("/sha256/"),
            "Storage path should contain sha256 directory"
        );

        let _ = fs::remove_dir_all(&temp_dir);
    }

    #[test]
    #[cfg(feature = "xxh3")]
    fn test_add_with_xxh3_algorithm() {
        use crate::HashAlgo;

        let (temp_dir, storage_dir) = setup_test_repo_with_algo("add_xxh3", HashAlgo::Xxh3);

        // Create a test file
        let test_file = temp_dir.join("data.csv");
        let content = b"xxh3 test content";
        fs::write(&test_file, content).unwrap();

        // Create .dvs directory
        let layout = Layout::new(temp_dir.clone());
        layout.init().unwrap();

        // Add the file
        let backend = crate::detect_backend(&temp_dir).unwrap();
        let results = add_with_backend(&backend, &[PathBuf::from("data.csv")], None, None).unwrap();

        assert_eq!(results.len(), 1);
        assert_eq!(results[0].outcome, Outcome::Copied);
        assert!(!results[0].blake3_checksum.is_empty());
        // XXH3 produces 16-char hex hashes (64-bit)
        assert_eq!(
            results[0].blake3_checksum.len(),
            16,
            "XXH3 should produce 16-char hex string"
        );

        // Verify metadata has xxh3 algorithm
        let metadata = Metadata::load_for_data_file(&test_file).unwrap();
        assert_eq!(metadata.hash_algo, HashAlgo::Xxh3);

        // Verify storage path uses xxh3 prefix
        let storage_path = crate::helpers::hash::storage_path_for_hash(
            &storage_dir,
            HashAlgo::Xxh3,
            &results[0].blake3_checksum,
        );
        assert!(storage_path.exists(), "Storage file should exist");
        assert!(
            storage_path.to_string_lossy().contains("/xxh3/"),
            "Storage path should contain xxh3 directory"
        );

        let _ = fs::remove_dir_all(&temp_dir);
    }

    #[test]
    #[cfg(feature = "sha256")]
    fn test_add_sha256_already_present() {
        use crate::HashAlgo;

        let (temp_dir, _storage_dir) =
            setup_test_repo_with_algo("add_sha256_present", HashAlgo::Sha256);

        // Create a test file
        let test_file = temp_dir.join("data.csv");
        fs::write(&test_file, b"sha256 content").unwrap();

        // Create .dvs directory
        let layout = Layout::new(temp_dir.clone());
        layout.init().unwrap();

        let backend = crate::detect_backend(&temp_dir).unwrap();

        // Add the file first time
        let results = add_with_backend(&backend, &[PathBuf::from("data.csv")], None, None).unwrap();
        assert_eq!(results[0].outcome, Outcome::Copied);

        // Add the same file again (unchanged)
        let results = add_with_backend(&backend, &[PathBuf::from("data.csv")], None, None).unwrap();
        assert_eq!(
            results[0].outcome,
            Outcome::Present,
            "Should detect file is already present with sha256"
        );

        let _ = fs::remove_dir_all(&temp_dir);
    }
}
