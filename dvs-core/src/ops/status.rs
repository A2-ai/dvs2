//! DVS status operation.

use crate::helpers::{config as config_helper, hash};
use crate::{
    detect_backend_cwd, Backend, Config, DvsError, FileStatus, Metadata, RepoBackend, StatusResult,
};
use glob::glob;
use std::path::{Path, PathBuf};
#[cfg(feature = "walkdir")]
use walkdir::WalkDir;

/// Check status of tracked files.
///
/// Compares local file hashes with stored metadata.
///
/// # Arguments
///
/// * `files` - File paths or glob patterns to check (empty = all tracked files)
///
/// # Returns
///
/// A vector of results, one per file (including errors).
///
/// # Errors
///
/// * `NotInitialized` - DVS not initialized
pub fn status(files: &[PathBuf]) -> Result<Vec<StatusResult>, DvsError> {
    let backend = detect_backend_cwd()?;
    status_with_backend(&backend, files)
}

/// Check status with a specific backend.
///
/// Use this when you already have a backend reference.
pub fn status_with_backend(
    backend: &Backend,
    files: &[PathBuf],
) -> Result<Vec<StatusResult>, DvsError> {
    let repo_root = backend.root();

    // Load configuration
    let config = config_helper::load_config(repo_root)?;

    // Determine which files to check
    let target_files = if files.is_empty() {
        // Check all tracked files
        find_all_tracked_files(backend)?
    } else {
        // Expand provided patterns
        expand_patterns(backend, files)?
    };

    // Process each file
    let mut results = Vec::with_capacity(target_files.len());
    for path in target_files {
        let result = status_single_file(backend, &path, &config);
        results.push(result);
    }

    Ok(results)
}

/// Find all tracked files in the repository.
fn find_all_tracked_files(backend: &Backend) -> Result<Vec<PathBuf>, DvsError> {
    let repo_root = backend.root();

    #[cfg(feature = "walkdir")]
    {
        find_tracked_files_walkdir(repo_root)
    }

    #[cfg(not(feature = "walkdir"))]
    {
        find_tracked_files_recursive(repo_root)
    }
}

/// Find tracked files using walkdir crate.
#[cfg(feature = "walkdir")]
fn find_tracked_files_walkdir(repo_root: &Path) -> Result<Vec<PathBuf>, DvsError> {
    let mut files = Vec::new();

    // Walk the repository and find all .dvs files
    for entry in WalkDir::new(repo_root)
        .follow_links(false)
        .into_iter()
        .filter_map(|e| e.ok())
    {
        let path = entry.path();

        // Skip hidden directories (like .git)
        if path
            .components()
            .any(|c| c.as_os_str().to_string_lossy().starts_with('.'))
        {
            continue;
        }

        // Check if this is a .dvs or .dvs.toml metadata file
        let filename = path.file_name().map(|f| f.to_string_lossy()).unwrap_or_default();
        if filename.ends_with(".dvs") || filename.ends_with(".dvs.toml") {
            if let Some(data_path) = Metadata::data_path(path) {
                // Avoid duplicates if both formats exist
                if !files.contains(&data_path) {
                    files.push(data_path);
                }
            }
        }
    }

    Ok(files)
}

/// Find tracked files using recursive fs::read_dir (fallback when walkdir disabled).
#[cfg(not(feature = "walkdir"))]
fn find_tracked_files_recursive(repo_root: &Path) -> Result<Vec<PathBuf>, DvsError> {
    use fs_err as fs;

    fn recurse(dir: &Path, files: &mut Vec<PathBuf>) -> Result<(), DvsError> {
        let entries = match fs::read_dir(dir) {
            Ok(e) => e,
            Err(_) => return Ok(()), // Skip unreadable directories
        };

        for entry in entries.flatten() {
            let path = entry.path();
            let name = path.file_name().unwrap_or_default().to_string_lossy();

            // Skip hidden directories (like .git, .dvs)
            if name.starts_with('.') {
                continue;
            }

            if path.is_dir() {
                recurse(&path, files)?;
            } else {
                // Check if this is a .dvs or .dvs.toml metadata file
                let filename = path.file_name().map(|f| f.to_string_lossy()).unwrap_or_default();
                if filename.ends_with(".dvs") || filename.ends_with(".dvs.toml") {
                    if let Some(data_path) = Metadata::data_path(&path) {
                        // Avoid duplicates if both formats exist
                        if !files.contains(&data_path) {
                            files.push(data_path);
                        }
                    }
                }
            }
        }

        Ok(())
    }

    let mut files = Vec::new();
    recurse(repo_root, &mut files)?;
    Ok(files)
}

/// Expand file patterns to concrete paths.
fn expand_patterns(backend: &Backend, patterns: &[PathBuf]) -> Result<Vec<PathBuf>, DvsError> {
    let mut files = Vec::new();
    let repo_root = backend.root();

    for pattern in patterns {
        let pattern_str = pattern.to_string_lossy();

        if pattern_str.contains('*') || pattern_str.contains('?') || pattern_str.contains('[') {
            // Expand glob
            let full_pattern = if pattern.is_relative() {
                repo_root.join(pattern)
            } else {
                pattern.clone()
            };

            // Look for .dvs and .dvs.toml files
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
            // Regular path
            let full_path = if pattern.is_relative() {
                repo_root.join(pattern)
            } else {
                pattern.clone()
            };
            files.push(full_path);
        }
    }

    Ok(files)
}

/// Check status of a single file.
fn status_single_file(backend: &Backend, path: &Path, config: &Config) -> StatusResult {
    let repo_root = backend.root();

    // Compute relative path
    let relative_path = match pathdiff::diff_paths(path, repo_root) {
        Some(p) => p,
        None => {
            return StatusResult::error(
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
            return StatusResult::error(
                path.display().to_string(),
                "metadata_not_found".to_string(),
                format!("Metadata not found for: {}", path.display()),
            );
        }
    };

    // Determine status
    let status = match determine_status(path, &metadata) {
        Ok(s) => s,
        Err(e) => {
            return StatusResult::error(
                path.display().to_string(),
                "status_error".to_string(),
                e.to_string(),
            );
        }
    };

    // Verify file exists in storage (using algorithm from metadata)
    let storage_path = hash::storage_path_for_hash(
        &config.storage_dir,
        metadata.hash_algo,
        &metadata.blake3_checksum,
    );
    if !storage_path.exists() && status != FileStatus::Unsynced {
        // Storage file missing - this is an error condition
        return StatusResult::error(
            path.display().to_string(),
            "storage_missing".to_string(),
            format!("File missing from storage: {}", storage_path.display()),
        );
    }

    StatusResult::success(
        relative_path,
        path.to_path_buf(),
        status,
        metadata.size,
        metadata.blake3_checksum.clone(),
        Some(metadata.add_time),
        Some(metadata.saved_by.clone()),
        if metadata.message.is_empty() {
            None
        } else {
            Some(metadata.message.clone())
        },
    )
}

/// Determine the status of a file by comparing hashes.
fn determine_status(local_path: &Path, metadata: &Metadata) -> Result<FileStatus, DvsError> {
    // Check if local file exists
    if !local_path.exists() {
        return Ok(FileStatus::Absent);
    }

    // Compare hashes using the algorithm stored in metadata
    match hash::verify_hash_with_algo(local_path, &metadata.blake3_checksum, metadata.hash_algo) {
        Ok(true) => Ok(FileStatus::Current),
        Ok(false) => Ok(FileStatus::Unsynced),
        Err(e) => Err(e),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::helpers::copy;
    use fs_err as fs;
    use std::sync::atomic::{AtomicU64, Ordering};

    static TEST_COUNTER: AtomicU64 = AtomicU64::new(0);

    fn setup_test_repo(test_name: &str) -> (PathBuf, PathBuf) {
        let unique_id = TEST_COUNTER.fetch_add(1, Ordering::SeqCst);
        let temp_dir = std::env::temp_dir().join(format!(
            "dvs-test-status-{}-{}-{}",
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
    fn test_status_current() {
        let (temp_dir, storage_dir) = setup_test_repo("status_current");

        // Create a tracked file
        let test_file = temp_dir.join("data.csv");
        let content = b"test content";
        fs::write(&test_file, content).unwrap();

        let algo = hash::default_algorithm();
        let checksum = hash::get_file_hash(&test_file).unwrap();

        // Store in storage (with algo prefix)
        let storage_path = hash::storage_path_for_hash(&storage_dir, algo, &checksum);
        copy::copy_to_storage(&test_file, &storage_path, None, None).unwrap();

        // Create metadata
        let metadata = Metadata::new(checksum, content.len() as u64, None, "tester".to_string());
        metadata.save(&Metadata::metadata_path(&test_file)).unwrap();

        // Check status
        let backend = crate::detect_backend(&temp_dir).unwrap();
        let config = Config::new(storage_dir, None, None);
        let result = status_single_file(&backend, &test_file, &config);

        assert_eq!(result.status, FileStatus::Current);

        let _ = fs::remove_dir_all(&temp_dir);
    }

    #[test]
    fn test_status_absent() {
        let (temp_dir, storage_dir) = setup_test_repo("status_absent");

        // Create metadata for a file that doesn't exist locally
        let test_file = temp_dir.join("missing.csv");
        let checksum = "deadbeefdeadbeefdeadbeefdeadbeefdeadbeefdeadbeefdeadbeefdeadbeef";

        // Store something in storage (with algo prefix)
        let algo = hash::default_algorithm();
        let storage_path = hash::storage_path_for_hash(&storage_dir, algo, checksum);
        fs::create_dir_all(storage_path.parent().unwrap()).unwrap();
        fs::write(&storage_path, b"content").unwrap();

        // Create metadata pointing to it
        let metadata = Metadata::new(checksum.to_string(), 7, None, "tester".to_string());
        metadata.save(&Metadata::metadata_path(&test_file)).unwrap();

        // Check status
        let backend = crate::detect_backend(&temp_dir).unwrap();
        let config = Config::new(storage_dir, None, None);
        let result = status_single_file(&backend, &test_file, &config);

        assert_eq!(result.status, FileStatus::Absent);

        let _ = fs::remove_dir_all(&temp_dir);
    }

    #[test]
    fn test_status_unsynced() {
        let (temp_dir, storage_dir) = setup_test_repo("status_unsynced");

        // Create a file with one content
        let test_file = temp_dir.join("data.csv");
        fs::write(&test_file, b"original content").unwrap();
        let algo = hash::default_algorithm();
        let original_checksum = hash::get_file_hash(&test_file).unwrap();

        // Store in storage (with algo prefix)
        let storage_path = hash::storage_path_for_hash(&storage_dir, algo, &original_checksum);
        copy::copy_to_storage(&test_file, &storage_path, None, None).unwrap();

        // Create metadata
        let metadata = Metadata::new(original_checksum, 16, None, "tester".to_string());
        metadata.save(&Metadata::metadata_path(&test_file)).unwrap();

        // Modify the local file
        fs::write(&test_file, b"modified content!").unwrap();

        // Check status
        let backend = crate::detect_backend(&temp_dir).unwrap();
        let config = Config::new(storage_dir, None, None);
        let result = status_single_file(&backend, &test_file, &config);

        assert_eq!(result.status, FileStatus::Unsynced);

        let _ = fs::remove_dir_all(&temp_dir);
    }

    #[test]
    fn test_find_all_tracked_files() {
        let (temp_dir, storage_dir) = setup_test_repo("find_all_tracked_files");

        // Create some tracked files
        let algo = hash::default_algorithm();
        for name in ["file1.csv", "file2.csv", "file3.txt"] {
            let path = temp_dir.join(name);
            fs::write(&path, "content").unwrap();

            let checksum = hash::get_file_hash(&path).unwrap();
            let storage_path = hash::storage_path_for_hash(&storage_dir, algo, &checksum);
            copy::copy_to_storage(&path, &storage_path, None, None).unwrap();

            let metadata = Metadata::new(checksum, 7, None, "tester".to_string());
            metadata.save(&Metadata::metadata_path(&path)).unwrap();
        }

        // Find tracked files
        let backend = crate::detect_backend(&temp_dir).unwrap();
        let files = find_all_tracked_files(&backend).unwrap();

        assert_eq!(files.len(), 3);

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
            "dvs-test-status-{}-{}-{}",
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
    fn test_status_current_with_sha256() {
        use crate::HashAlgo;

        let (temp_dir, storage_dir) =
            setup_test_repo_with_algo("status_sha256_current", HashAlgo::Sha256);

        // Create a tracked file
        let test_file = temp_dir.join("data.csv");
        let content = b"sha256 status test content";
        fs::write(&test_file, content).unwrap();

        let checksum = hash::get_file_hash_with_algo(&test_file, HashAlgo::Sha256).unwrap();

        // Store in storage with sha256 prefix
        let storage_path = hash::storage_path_for_hash(&storage_dir, HashAlgo::Sha256, &checksum);
        copy::copy_to_storage(&test_file, &storage_path, None, None).unwrap();

        // Create metadata with sha256
        let metadata = Metadata::with_algo(
            checksum,
            content.len() as u64,
            None,
            "tester".to_string(),
            HashAlgo::Sha256,
        );
        metadata.save(&Metadata::metadata_path(&test_file)).unwrap();

        // Check status
        let backend = crate::detect_backend(&temp_dir).unwrap();
        let config = Config::with_hash_algo(storage_dir, None, None, HashAlgo::Sha256);
        let result = status_single_file(&backend, &test_file, &config);

        assert_eq!(
            result.status,
            FileStatus::Current,
            "File with sha256 should be current"
        );

        let _ = fs::remove_dir_all(&temp_dir);
    }

    #[test]
    #[cfg(feature = "xxh3")]
    fn test_status_current_with_xxh3() {
        use crate::HashAlgo;

        let (temp_dir, storage_dir) =
            setup_test_repo_with_algo("status_xxh3_current", HashAlgo::Xxh3);

        // Create a tracked file
        let test_file = temp_dir.join("data.csv");
        let content = b"xxh3 status test content";
        fs::write(&test_file, content).unwrap();

        let checksum = hash::get_file_hash_with_algo(&test_file, HashAlgo::Xxh3).unwrap();

        // Store in storage with xxh3 prefix
        let storage_path = hash::storage_path_for_hash(&storage_dir, HashAlgo::Xxh3, &checksum);
        copy::copy_to_storage(&test_file, &storage_path, None, None).unwrap();

        // Create metadata with xxh3
        let metadata = Metadata::with_algo(
            checksum,
            content.len() as u64,
            None,
            "tester".to_string(),
            HashAlgo::Xxh3,
        );
        metadata.save(&Metadata::metadata_path(&test_file)).unwrap();

        // Check status
        let backend = crate::detect_backend(&temp_dir).unwrap();
        let config = Config::with_hash_algo(storage_dir, None, None, HashAlgo::Xxh3);
        let result = status_single_file(&backend, &test_file, &config);

        assert_eq!(
            result.status,
            FileStatus::Current,
            "File with xxh3 should be current"
        );

        let _ = fs::remove_dir_all(&temp_dir);
    }

    #[test]
    #[cfg(feature = "sha256")]
    fn test_status_unsynced_with_sha256() {
        use crate::HashAlgo;

        let (temp_dir, storage_dir) =
            setup_test_repo_with_algo("status_sha256_unsynced", HashAlgo::Sha256);

        // Create a file
        let test_file = temp_dir.join("data.csv");
        fs::write(&test_file, b"original sha256 content").unwrap();
        let original_checksum =
            hash::get_file_hash_with_algo(&test_file, HashAlgo::Sha256).unwrap();

        // Store in storage
        let storage_path =
            hash::storage_path_for_hash(&storage_dir, HashAlgo::Sha256, &original_checksum);
        copy::copy_to_storage(&test_file, &storage_path, None, None).unwrap();

        // Create metadata
        let metadata = Metadata::with_algo(
            original_checksum,
            23,
            None,
            "tester".to_string(),
            HashAlgo::Sha256,
        );
        metadata.save(&Metadata::metadata_path(&test_file)).unwrap();

        // Modify the local file
        fs::write(&test_file, b"modified sha256 content!!").unwrap();

        // Check status
        let backend = crate::detect_backend(&temp_dir).unwrap();
        let config = Config::with_hash_algo(storage_dir, None, None, HashAlgo::Sha256);
        let result = status_single_file(&backend, &test_file, &config);

        assert_eq!(
            result.status,
            FileStatus::Unsynced,
            "Modified file with sha256 should be unsynced"
        );

        let _ = fs::remove_dir_all(&temp_dir);
    }

    #[test]
    #[cfg(feature = "sha256")]
    fn test_status_absent_with_sha256() {
        use crate::HashAlgo;

        let (temp_dir, storage_dir) =
            setup_test_repo_with_algo("status_sha256_absent", HashAlgo::Sha256);

        // Create metadata for a file that doesn't exist locally
        let test_file = temp_dir.join("missing.csv");
        let checksum = "abcd1234abcd1234abcd1234abcd1234abcd1234abcd1234abcd1234abcd1234";

        // Store something in storage
        let storage_path =
            hash::storage_path_for_hash(&storage_dir, HashAlgo::Sha256, checksum);
        fs::create_dir_all(storage_path.parent().unwrap()).unwrap();
        fs::write(&storage_path, b"content").unwrap();

        // Create metadata with sha256
        let metadata = Metadata::with_algo(
            checksum.to_string(),
            7,
            None,
            "tester".to_string(),
            HashAlgo::Sha256,
        );
        metadata.save(&Metadata::metadata_path(&test_file)).unwrap();

        // Check status
        let backend = crate::detect_backend(&temp_dir).unwrap();
        let config = Config::with_hash_algo(storage_dir, None, None, HashAlgo::Sha256);
        let result = status_single_file(&backend, &test_file, &config);

        assert_eq!(
            result.status,
            FileStatus::Absent,
            "Missing file with sha256 should be absent"
        );

        let _ = fs::remove_dir_all(&temp_dir);
    }

    #[test]
    fn test_status_current_with_toml_metadata() {
        use crate::MetadataFormat;

        let (temp_dir, storage_dir) = setup_test_repo("status_toml_current");

        // Create a tracked file
        let test_file = temp_dir.join("data.csv");
        let content = b"toml status test content";
        fs::write(&test_file, content).unwrap();

        let algo = hash::default_algorithm();
        let checksum = hash::get_file_hash(&test_file).unwrap();

        // Store in storage
        let storage_path = hash::storage_path_for_hash(&storage_dir, algo, &checksum);
        copy::copy_to_storage(&test_file, &storage_path, None, None).unwrap();

        // Create metadata in TOML format
        let metadata = Metadata::new(checksum, content.len() as u64, None, "tester".to_string());
        let toml_meta_path = Metadata::metadata_path_for_format(&test_file, MetadataFormat::Toml);
        metadata.save(&toml_meta_path).unwrap();

        // Verify TOML file exists
        assert!(toml_meta_path.exists(), "TOML metadata should exist");

        // Check status
        let backend = crate::detect_backend(&temp_dir).unwrap();
        let config = Config::new(storage_dir, None, None);
        let result = status_single_file(&backend, &test_file, &config);

        assert_eq!(
            result.status,
            FileStatus::Current,
            "File with TOML metadata should be current"
        );

        let _ = fs::remove_dir_all(&temp_dir);
    }

    #[test]
    fn test_status_unsynced_with_toml_metadata() {
        use crate::MetadataFormat;

        let (temp_dir, storage_dir) = setup_test_repo("status_toml_unsynced");

        // Create a tracked file
        let test_file = temp_dir.join("data.csv");
        fs::write(&test_file, b"original toml content").unwrap();
        let algo = hash::default_algorithm();
        let original_checksum = hash::get_file_hash(&test_file).unwrap();

        // Store in storage
        let storage_path = hash::storage_path_for_hash(&storage_dir, algo, &original_checksum);
        copy::copy_to_storage(&test_file, &storage_path, None, None).unwrap();

        // Create metadata in TOML format
        let metadata = Metadata::new(original_checksum, 21, None, "tester".to_string());
        let toml_meta_path = Metadata::metadata_path_for_format(&test_file, MetadataFormat::Toml);
        metadata.save(&toml_meta_path).unwrap();

        // Modify the local file
        fs::write(&test_file, b"modified toml content!").unwrap();

        // Check status
        let backend = crate::detect_backend(&temp_dir).unwrap();
        let config = Config::new(storage_dir, None, None);
        let result = status_single_file(&backend, &test_file, &config);

        assert_eq!(
            result.status,
            FileStatus::Unsynced,
            "Modified file with TOML metadata should be unsynced"
        );

        let _ = fs::remove_dir_all(&temp_dir);
    }

    #[test]
    fn test_find_tracked_files_with_toml() {
        use crate::MetadataFormat;

        let (temp_dir, storage_dir) = setup_test_repo("find_tracked_toml");

        let algo = hash::default_algorithm();

        // Create files with mixed metadata formats
        for (name, format) in [
            ("file1.csv", MetadataFormat::Json),
            ("file2.csv", MetadataFormat::Toml),
            ("file3.txt", MetadataFormat::Toml),
        ] {
            let path = temp_dir.join(name);
            fs::write(&path, "content").unwrap();

            let checksum = hash::get_file_hash(&path).unwrap();
            let storage_path = hash::storage_path_for_hash(&storage_dir, algo, &checksum);
            copy::copy_to_storage(&path, &storage_path, None, None).unwrap();

            let metadata = Metadata::new(checksum, 7, None, "tester".to_string());
            let meta_path = Metadata::metadata_path_for_format(&path, format);
            metadata.save(&meta_path).unwrap();
        }

        // Find tracked files
        let backend = crate::detect_backend(&temp_dir).unwrap();
        let files = find_all_tracked_files(&backend).unwrap();

        assert_eq!(
            files.len(),
            3,
            "Should find all 3 files regardless of metadata format"
        );

        let _ = fs::remove_dir_all(&temp_dir);
    }
}
