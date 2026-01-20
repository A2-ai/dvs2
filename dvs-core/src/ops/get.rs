//! DVS get operation.

use crate::helpers::{config as config_helper, copy, hash};
use crate::{
    detect_backend_cwd, Backend, Config, DvsError, GetResult, Metadata, Outcome, RepoBackend,
};
use glob::glob;
use std::path::{Path, PathBuf};

/// Retrieve files from DVS storage.
///
/// Reads metadata files, checks local file hashes, and copies from storage if needed.
///
/// # Arguments
///
/// * `files` - File paths or glob patterns to retrieve
///
/// # Returns
///
/// A vector of results, one per file (including errors).
///
/// # Errors
///
/// * `NotInitialized` - DVS not initialized
/// * `BatchError` - Multiple explicit paths don't have metadata
pub fn get(files: &[PathBuf]) -> Result<Vec<GetResult>, DvsError> {
    let backend = detect_backend_cwd()?;
    get_with_backend(&backend, files)
}

/// Retrieve files with a specific backend.
///
/// Use this when you already have a backend reference.
pub fn get_with_backend(backend: &Backend, files: &[PathBuf]) -> Result<Vec<GetResult>, DvsError> {
    let repo_root = backend.root();

    // Load configuration
    let config = config_helper::load_config(repo_root)?;

    // Expand glob patterns to tracked files
    let expanded_files = expand_globs_tracked(backend, files)?;

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
    for path in expanded_files {
        let result = get_single_file(backend, &path, &config);
        results.push(result);
    }

    Ok(results)
}

/// Expand glob patterns to tracked files only.
fn expand_globs_tracked(backend: &Backend, patterns: &[PathBuf]) -> Result<Vec<PathBuf>, DvsError> {
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

            // We need to find .dvs and .dvs.toml files and convert back to data paths
            // Search for both JSON and TOML metadata files
            for ext in &[".dvs", ".dvs.toml"] {
                let meta_pattern = format!("{}{}", full_pattern.display(), ext);
                match glob(&meta_pattern) {
                    Ok(paths) => {
                        for entry in paths.flatten() {
                            if let Some(data_path) = Metadata::data_path(&entry) {
                                // Avoid duplicates if both formats exist
                                if !files.contains(&data_path) {
                                    files.push(data_path);
                                }
                            }
                        }
                    }
                    Err(_) => {
                        return Err(DvsError::invalid_glob(pattern_str.to_string()));
                    }
                }
            }
        } else {
            // Regular file path
            let full_path = if pattern.is_relative() {
                repo_root.join(pattern)
            } else {
                pattern.clone()
            };

            // Check if metadata exists for this file
            let meta_path = Metadata::metadata_path(&full_path);
            if meta_path.exists() {
                files.push(full_path);
            } else {
                // Include anyway - we'll report error in get_single_file
                files.push(full_path);
            }
        }
    }

    Ok(files)
}

/// Process a single file for retrieval.
fn get_single_file(backend: &Backend, path: &Path, config: &Config) -> GetResult {
    let repo_root = backend.root();

    // Compute relative path
    let relative_path = match pathdiff::diff_paths(path, repo_root) {
        Some(p) => p,
        None => {
            return GetResult::error(
                path.display().to_string(),
                "file_outside_repo".to_string(),
                format!("File is outside repository: {}", path.display()),
            );
        }
    };

    // Load metadata (tries TOML first, then JSON)
    let metadata = match Metadata::load_for_data_file(path) {
        Ok(m) => m,
        Err(_) => {
            return GetResult::error(
                path.display().to_string(),
                "metadata_not_found".to_string(),
                format!("Metadata not found for: {}", path.display()),
            );
        }
    };

    // Check if local file already matches
    if file_matches_metadata(path, &metadata).unwrap_or(false) {
        return GetResult::success(
            relative_path,
            path.to_path_buf(),
            Outcome::Present,
            metadata.size,
            metadata.blake3_checksum,
        );
    }

    // Get storage path
    let storage_path = hash::storage_path_for_hash(&config.storage_dir, &metadata.blake3_checksum);

    // Check if file exists in storage
    if !storage_path.exists() {
        return GetResult::error(
            path.display().to_string(),
            "storage_error".to_string(),
            format!("File not found in storage: {}", storage_path.display()),
        );
    }

    // Copy from storage
    if let Err(e) = copy::copy_from_storage(&storage_path, path) {
        return GetResult::error(
            path.display().to_string(),
            "copy_error".to_string(),
            e.to_string(),
        );
    }

    // Verify the copy using the algorithm stored in metadata
    match hash::verify_hash_with_algo(path, &metadata.blake3_checksum, metadata.hash_algo) {
        Ok(true) => GetResult::success(
            relative_path,
            path.to_path_buf(),
            Outcome::Copied,
            metadata.size,
            metadata.blake3_checksum,
        ),
        Ok(false) => {
            // Hash mismatch after copy - this shouldn't happen
            GetResult::error(
                path.display().to_string(),
                "hash_mismatch".to_string(),
                "Hash mismatch after copy from storage".to_string(),
            )
        }
        Err(e) => GetResult::error(
            path.display().to_string(),
            "hash_error".to_string(),
            e.to_string(),
        ),
    }
}

/// Check if local file matches metadata hash.
fn file_matches_metadata(local_path: &Path, metadata: &Metadata) -> Result<bool, DvsError> {
    if !local_path.exists() {
        return Ok(false);
    }

    // Use the algorithm stored in metadata for verification
    hash::verify_hash_with_algo(local_path, &metadata.blake3_checksum, metadata.hash_algo)
}

#[cfg(test)]
mod tests {
    use super::*;
    use fs_err as fs;
    use std::sync::atomic::{AtomicU64, Ordering};

    static TEST_COUNTER: AtomicU64 = AtomicU64::new(0);

    fn setup_test_repo(test_name: &str) -> (PathBuf, PathBuf) {
        let unique_id = TEST_COUNTER.fetch_add(1, Ordering::SeqCst);
        let temp_dir = std::env::temp_dir().join(format!(
            "dvs-test-get-{}-{}-{}",
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

        // Create config file
        let config = Config::new(storage_dir.clone(), None, None);
        config
            .save(&temp_dir.join(Config::config_filename()))
            .unwrap();

        (temp_dir, storage_dir)
    }

    #[test]
    fn test_get_single_file_success() {
        let (temp_dir, storage_dir) = setup_test_repo("get_single_file_success");

        // Create a test file and its metadata
        let test_file = temp_dir.join("data.csv");
        let content = b"col1,col2\n1,2\n";
        fs::write(&test_file, content).unwrap();

        // Compute hash
        let checksum = hash::get_file_hash(&test_file).unwrap();

        // Store in storage
        let storage_path = hash::storage_path_for_hash(&storage_dir, &checksum);
        copy::copy_to_storage(&test_file, &storage_path, None, None).unwrap();

        // Create metadata
        let metadata = Metadata::new(
            checksum.clone(),
            content.len() as u64,
            Some("test".to_string()),
            "tester".to_string(),
        );
        metadata.save(&Metadata::metadata_path(&test_file)).unwrap();

        // Remove the local file
        fs::remove_file(&test_file).unwrap();
        assert!(!test_file.exists());

        // Get it back
        let backend = crate::detect_backend(&temp_dir).unwrap();
        let config = Config::new(storage_dir, None, None);
        let result = get_single_file(&backend, &test_file, &config);

        assert_eq!(result.outcome, Outcome::Copied);
        assert!(test_file.exists());

        // Verify content
        let restored = fs::read(&test_file).unwrap();
        assert_eq!(restored, content);

        let _ = fs::remove_dir_all(&temp_dir);
    }

    #[test]
    fn test_get_single_file_already_present() {
        let (temp_dir, storage_dir) = setup_test_repo("get_single_file_already_present");

        // Create a test file and its metadata
        let test_file = temp_dir.join("data.csv");
        let content = b"test content";
        fs::write(&test_file, content).unwrap();

        // Compute hash
        let checksum = hash::get_file_hash(&test_file).unwrap();

        // Store in storage
        let storage_path = hash::storage_path_for_hash(&storage_dir, &checksum);
        copy::copy_to_storage(&test_file, &storage_path, None, None).unwrap();

        // Create metadata
        let metadata = Metadata::new(checksum, content.len() as u64, None, "tester".to_string());
        metadata.save(&Metadata::metadata_path(&test_file)).unwrap();

        // Get when file already exists with correct content
        let backend = crate::detect_backend(&temp_dir).unwrap();
        let config = Config::new(storage_dir, None, None);
        let result = get_single_file(&backend, &test_file, &config);

        assert_eq!(result.outcome, Outcome::Present);

        let _ = fs::remove_dir_all(&temp_dir);
    }

    #[test]
    fn test_get_single_file_no_metadata() {
        let (temp_dir, storage_dir) = setup_test_repo("get_single_file_no_metadata");

        let test_file = temp_dir.join("no_meta.txt");

        let backend = crate::detect_backend(&temp_dir).unwrap();
        let config = Config::new(storage_dir, None, None);
        let result = get_single_file(&backend, &test_file, &config);

        assert_eq!(result.outcome, Outcome::Error);
        assert!(result
            .error
            .as_ref()
            .unwrap()
            .contains("metadata_not_found"));

        let _ = fs::remove_dir_all(&temp_dir);
    }
}
