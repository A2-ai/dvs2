//! DVS add operation.

use std::path::{Path, PathBuf};
use fs_err as fs;
use glob::glob;
use crate::{AddResult, Config, Metadata, Outcome, DvsError, Backend, RepoBackend, detect_backend_cwd};
use crate::helpers::{config as config_helper, copy, file, hash};

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
    add_with_backend(&backend, files, message)
}

/// Add files with a specific backend.
///
/// Use this when you already have a backend reference.
pub fn add_with_backend(
    backend: &Backend,
    files: &[PathBuf],
    message: Option<&str>,
) -> Result<Vec<AddResult>, DvsError> {
    let repo_root = backend.root();

    // Load configuration
    let config = config_helper::load_config(repo_root)?;

    // Expand glob patterns
    let expanded_files = expand_globs(backend, files)?;

    if expanded_files.is_empty() {
        return Err(DvsError::NoFilesMatched {
            pattern: files.iter().map(|p| p.display().to_string()).collect::<Vec<_>>().join(", "),
        });
    }

    // Process each file
    let mut results = Vec::with_capacity(expanded_files.len());
    for path in expanded_files {
        let result = add_single_file(backend, &path, message, &config);
        results.push(result);
    }

    Ok(results)
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
                    return Err(DvsError::InvalidGlob {
                        pattern: pattern_str.to_string(),
                    });
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

    // Compute hash
    let checksum = match hash::get_file_hash(path) {
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
    let storage_path = hash::storage_path_for_hash(&config.storage_dir, &checksum);
    let metadata_path = Metadata::metadata_path(path);

    // Check if already present with same hash
    if metadata_path.exists() {
        if let Ok(existing_meta) = Metadata::load(&metadata_path) {
            if existing_meta.blake3_checksum == checksum {
                // Same file already tracked
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

    // Create metadata
    let username = file::get_current_username().unwrap_or_else(|_| "unknown".to_string());
    let metadata = Metadata::new(
        checksum.clone(),
        size,
        message.map(|s| s.to_string()),
        username,
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
fn rollback_add(
    metadata_path: &Path,
    storage_path: &Path,
) -> Result<(), DvsError> {
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
        let temp_dir = std::env::temp_dir().join(format!("dvs-test-add-{}-{}-{}", std::process::id(), test_name, unique_id));
        let _ = fs::remove_dir_all(&temp_dir);
        fs::create_dir_all(&temp_dir).unwrap();

        // Create a fake .git directory to make it a git repo
        fs::create_dir_all(temp_dir.join(".git")).unwrap();

        // Create storage directory
        let storage_dir = temp_dir.join("storage");
        fs::create_dir_all(&storage_dir).unwrap();

        // Create dvs.yaml
        let config = Config::new(storage_dir.clone(), None, None);
        config.save(&temp_dir.join("dvs.yaml")).unwrap();

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
        let result = add_single_file(&backend, &test_file, Some("test message"), &config);

        assert_eq!(result.outcome, Outcome::Copied);
        assert!(!result.blake3_checksum.is_empty());
        assert_eq!(result.size, 18); // "col1,col2\n1,2\n3,4\n"

        // Verify metadata file exists
        let meta_path = Metadata::metadata_path(&test_file);
        assert!(meta_path.exists());

        // Verify storage file exists
        let storage_path = hash::storage_path_for_hash(&storage_dir, &result.blake3_checksum);
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
        let result1 = add_single_file(&backend, &test_file, None, &config);
        assert_eq!(result1.outcome, Outcome::Copied);

        // Add the same file again (unchanged)
        let result2 = add_single_file(&backend, &test_file, None, &config);
        assert_eq!(result2.outcome, Outcome::Present);

        let _ = fs::remove_dir_all(&temp_dir);
    }

    #[test]
    fn test_add_file_not_found() {
        let (temp_dir, storage_dir) = setup_test_repo("add_file_not_found");

        let backend = crate::detect_backend(&temp_dir).unwrap();
        let config = Config::new(storage_dir, None, None);

        let nonexistent = temp_dir.join("nonexistent.txt");
        let result = add_single_file(&backend, &nonexistent, None, &config);

        assert_eq!(result.outcome, Outcome::Error);
        assert!(result.error_message.unwrap().contains("not found"));

        let _ = fs::remove_dir_all(&temp_dir);
    }
}
