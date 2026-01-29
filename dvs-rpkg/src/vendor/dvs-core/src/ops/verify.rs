//! DVS verify operation.
//!
//! Verifies integrity of tracked files by checking:
//! - Local file exists and matches metadata hash
//! - Storage file exists and matches metadata hash
//! - Metadata file is valid

use crate::helpers::{config as config_helper, hash};
use crate::{detect_backend_cwd, Backend, Config, DvsError, Metadata, RepoBackend};
use glob::glob;
use serde::Serialize;
use std::path::{Path, PathBuf};
#[cfg(feature = "walkdir")]
use walkdir::WalkDir;

/// Result of verifying a single file.
#[derive(Debug, Clone, Serialize)]
pub struct VerifyResult {
    /// Relative path from the working directory.
    pub path: PathBuf,
    /// Whether the local file exists and matches metadata.
    pub local_ok: bool,
    /// Whether the storage file exists and matches metadata.
    pub storage_ok: bool,
    /// Whether the metadata file is valid.
    pub metadata_ok: bool,
    /// Overall verification passed.
    pub ok: bool,
    /// Detailed error/warning message.
    pub details: Option<String>,
}

impl VerifyResult {
    /// Create a successful result (all checks passed).
    pub fn ok(path: PathBuf) -> Self {
        Self {
            path,
            local_ok: true,
            storage_ok: true,
            metadata_ok: true,
            ok: true,
            details: None,
        }
    }

    /// Create a result with a specific issue.
    pub fn issue(
        path: PathBuf,
        local_ok: bool,
        storage_ok: bool,
        metadata_ok: bool,
        details: String,
    ) -> Self {
        Self {
            path,
            local_ok,
            storage_ok,
            metadata_ok,
            ok: local_ok && storage_ok && metadata_ok,
            details: Some(details),
        }
    }

    /// Create an error result.
    pub fn error(path: PathBuf, details: String) -> Self {
        Self {
            path,
            local_ok: false,
            storage_ok: false,
            metadata_ok: false,
            ok: false,
            details: Some(details),
        }
    }
}

/// Summary of verification results.
#[derive(Debug, Clone, Serialize)]
pub struct VerifySummary {
    /// Total files checked.
    pub total: usize,
    /// Files that passed all checks.
    pub passed: usize,
    /// Files with local issues (missing or wrong hash).
    pub local_issues: usize,
    /// Files with storage issues (missing or corrupted).
    pub storage_issues: usize,
    /// Files with metadata issues.
    pub metadata_issues: usize,
    /// Files with errors during verification.
    pub errors: usize,
}

impl VerifySummary {
    /// Create a new summary from results.
    pub fn from_results(results: &[VerifyResult]) -> Self {
        let total = results.len();
        let passed = results.iter().filter(|r| r.ok).count();
        let local_issues = results.iter().filter(|r| !r.local_ok).count();
        let storage_issues = results.iter().filter(|r| !r.storage_ok).count();
        let metadata_issues = results.iter().filter(|r| !r.metadata_ok).count();
        let errors = results
            .iter()
            .filter(|r| !r.local_ok && !r.storage_ok && !r.metadata_ok)
            .count();

        Self {
            total,
            passed,
            local_issues,
            storage_issues,
            metadata_issues,
            errors,
        }
    }

    /// Check if all files passed verification.
    pub fn all_ok(&self) -> bool {
        self.passed == self.total
    }
}

/// Verify integrity of tracked files.
///
/// Checks local files, storage files, and metadata for integrity.
///
/// # Arguments
///
/// * `files` - File paths or glob patterns to verify (empty = all tracked files)
///
/// # Returns
///
/// A vector of results, one per file.
pub fn verify(files: &[PathBuf]) -> Result<Vec<VerifyResult>, DvsError> {
    let backend = detect_backend_cwd()?;
    verify_with_backend(&backend, files)
}

/// Verify with a specific backend.
pub fn verify_with_backend(
    backend: &Backend,
    files: &[PathBuf],
) -> Result<Vec<VerifyResult>, DvsError> {
    let repo_root = backend.root();

    // Load configuration
    let config = config_helper::load_config(repo_root)?;

    // Determine which files to verify
    let target_files = if files.is_empty() {
        find_all_tracked_files(backend)?
    } else {
        expand_patterns(backend, files)?
    };

    // Process each file
    let mut results = Vec::with_capacity(target_files.len());
    for path in target_files {
        let result = verify_single_file(backend, &path, &config);
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

    for entry in WalkDir::new(repo_root)
        .follow_links(false)
        .into_iter()
        .filter_map(|e| e.ok())
    {
        let path = entry.path();

        // Skip hidden directories
        if path
            .components()
            .any(|c| c.as_os_str().to_string_lossy().starts_with('.'))
        {
            continue;
        }

        // Check if this is a .dvs or .dvs.toml metadata file
        let filename = path
            .file_name()
            .map(|f| f.to_string_lossy())
            .unwrap_or_default();
        if filename.ends_with(".dvs") || filename.ends_with(".dvs.toml") {
            if let Some(data_path) = Metadata::data_path(path) {
                if !files.contains(&data_path) {
                    files.push(data_path);
                }
            }
        }
    }

    Ok(files)
}

/// Find tracked files without walkdir.
#[cfg(not(feature = "walkdir"))]
fn find_tracked_files_recursive(repo_root: &Path) -> Result<Vec<PathBuf>, DvsError> {
    use fs_err as fs;

    fn recurse(dir: &Path, files: &mut Vec<PathBuf>) -> Result<(), DvsError> {
        let entries = match fs::read_dir(dir) {
            Ok(e) => e,
            Err(_) => return Ok(()),
        };

        for entry in entries.flatten() {
            let path = entry.path();
            let name = path.file_name().unwrap_or_default().to_string_lossy();

            if name.starts_with('.') {
                continue;
            }

            if path.is_dir() {
                recurse(&path, files)?;
            } else {
                let filename = path
                    .file_name()
                    .map(|f| f.to_string_lossy())
                    .unwrap_or_default();
                if filename.ends_with(".dvs") || filename.ends_with(".dvs.toml") {
                    if let Some(data_path) = Metadata::data_path(&path) {
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

/// Expand glob patterns to tracked files.
fn expand_patterns(backend: &Backend, patterns: &[PathBuf]) -> Result<Vec<PathBuf>, DvsError> {
    let mut files = Vec::new();
    let repo_root = backend.root();

    for pattern in patterns {
        let pattern_str = pattern.to_string_lossy();

        if pattern_str.contains('*') || pattern_str.contains('?') || pattern_str.contains('[') {
            let full_pattern = if pattern.is_relative() {
                repo_root.join(pattern)
            } else {
                pattern.clone()
            };

            for ext in &[".dvs", ".dvs.toml"] {
                let meta_pattern = format!("{}{}", full_pattern.display(), ext);
                match glob(&meta_pattern) {
                    Ok(paths) => {
                        for entry in paths.flatten() {
                            if let Some(data_path) = Metadata::data_path(&entry) {
                                if !files.contains(&data_path) {
                                    files.push(data_path);
                                }
                            }
                        }
                    }
                    Err(_) => {
                        return Err(DvsError::invalid_glob(pattern_str.to_string())
                            .context("check glob syntax: * matches any chars, ** matches directories"));
                    }
                }
            }
        } else {
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

/// Verify a single file.
fn verify_single_file(backend: &Backend, path: &Path, config: &Config) -> VerifyResult {
    let repo_root = backend.root();

    // Compute relative path
    let relative_path = match pathdiff::diff_paths(path, repo_root) {
        Some(p) => p,
        None => {
            return VerifyResult::error(
                path.to_path_buf(),
                format!("File is outside repository: {}", path.display()),
            );
        }
    };

    // Load metadata
    let metadata = match Metadata::load_for_data_file(path) {
        Ok(m) => m,
        Err(e) => {
            return VerifyResult::issue(
                relative_path,
                false,
                false,
                false,
                format!("Metadata not found or invalid: {}", e),
            );
        }
    };

    // Check local file
    let local_ok = if path.exists() {
        match hash::verify_hash_with_algo(path, &metadata.blake3_checksum, metadata.hash_algo) {
            Ok(true) => true,
            Ok(false) => false,
            Err(_) => false,
        }
    } else {
        false
    };

    // Check storage file
    let storage_path = hash::storage_path_for_hash(
        &config.storage_dir,
        metadata.hash_algo,
        &metadata.blake3_checksum,
    );

    let storage_ok = if storage_path.exists() {
        match hash::verify_hash_with_algo(
            &storage_path,
            &metadata.blake3_checksum,
            metadata.hash_algo,
        ) {
            Ok(true) => true,
            Ok(false) => false,
            Err(_) => false,
        }
    } else {
        false
    };

    // Build details message
    let details = if local_ok && storage_ok {
        None
    } else {
        let mut issues = Vec::new();
        if !local_ok {
            if !path.exists() {
                issues.push("local file missing");
            } else {
                issues.push("local file hash mismatch");
            }
        }
        if !storage_ok {
            if !storage_path.exists() {
                issues.push("storage file missing");
            } else {
                issues.push("storage file corrupted");
            }
        }
        Some(issues.join(", "))
    };

    if local_ok && storage_ok {
        VerifyResult::ok(relative_path)
    } else {
        VerifyResult::issue(
            relative_path,
            local_ok,
            storage_ok,
            true, // metadata was valid if we got here
            details.unwrap_or_default(),
        )
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
            "dvs-test-verify-{}-{}-{}",
            std::process::id(),
            test_name,
            unique_id
        ));
        let _ = fs::remove_dir_all(&temp_dir);
        fs::create_dir_all(&temp_dir).unwrap();

        fs::create_dir_all(temp_dir.join(".git")).unwrap();

        let storage_dir = temp_dir.join("storage");
        fs::create_dir_all(&storage_dir).unwrap();

        let config = Config::new(storage_dir.clone(), None, None);
        config
            .save(&temp_dir.join(Config::config_filename()))
            .unwrap();

        (temp_dir, storage_dir)
    }

    #[test]
    fn test_verify_ok() {
        let (temp_dir, storage_dir) = setup_test_repo("verify_ok");

        let test_file = temp_dir.join("data.csv");
        let content = b"test content";
        fs::write(&test_file, content).unwrap();

        let algo = hash::default_algorithm();
        let checksum = hash::get_file_hash(&test_file).unwrap();

        let storage_path = hash::storage_path_for_hash(&storage_dir, algo, &checksum);
        copy::copy_to_storage(&test_file, &storage_path, None, None).unwrap();

        let metadata = Metadata::new(checksum, content.len() as u64, None, "tester".to_string());
        metadata.save(&Metadata::metadata_path(&test_file)).unwrap();

        let backend = crate::detect_backend(&temp_dir).unwrap();
        let config = Config::new(storage_dir, None, None);
        let result = verify_single_file(&backend, &test_file, &config);

        assert!(result.ok, "Verification should pass");
        assert!(result.local_ok);
        assert!(result.storage_ok);
        assert!(result.metadata_ok);

        let _ = fs::remove_dir_all(&temp_dir);
    }

    #[test]
    fn test_verify_local_missing() {
        let (temp_dir, storage_dir) = setup_test_repo("verify_local_missing");

        let test_file = temp_dir.join("data.csv");
        let checksum = "deadbeefdeadbeefdeadbeefdeadbeefdeadbeefdeadbeefdeadbeefdeadbeef";

        let algo = hash::default_algorithm();
        let storage_path = hash::storage_path_for_hash(&storage_dir, algo, checksum);
        fs::create_dir_all(storage_path.parent().unwrap()).unwrap();
        fs::write(&storage_path, b"content").unwrap();

        let metadata = Metadata::new(checksum.to_string(), 7, None, "tester".to_string());
        metadata.save(&Metadata::metadata_path(&test_file)).unwrap();

        let backend = crate::detect_backend(&temp_dir).unwrap();
        let config = Config::new(storage_dir, None, None);
        let result = verify_single_file(&backend, &test_file, &config);

        assert!(
            !result.ok,
            "Verification should fail for missing local file"
        );
        assert!(!result.local_ok);
        assert!(result.details.unwrap().contains("local file missing"));

        let _ = fs::remove_dir_all(&temp_dir);
    }

    #[test]
    fn test_verify_storage_corrupted() {
        let (temp_dir, storage_dir) = setup_test_repo("verify_storage_corrupted");

        let test_file = temp_dir.join("data.csv");
        let content = b"test content";
        fs::write(&test_file, content).unwrap();

        let algo = hash::default_algorithm();
        let checksum = hash::get_file_hash(&test_file).unwrap();

        // Write corrupted data to storage
        let storage_path = hash::storage_path_for_hash(&storage_dir, algo, &checksum);
        fs::create_dir_all(storage_path.parent().unwrap()).unwrap();
        fs::write(&storage_path, b"CORRUPTED").unwrap();

        let metadata = Metadata::new(checksum, content.len() as u64, None, "tester".to_string());
        metadata.save(&Metadata::metadata_path(&test_file)).unwrap();

        let backend = crate::detect_backend(&temp_dir).unwrap();
        let config = Config::new(storage_dir, None, None);
        let result = verify_single_file(&backend, &test_file, &config);

        assert!(!result.ok, "Verification should fail for corrupted storage");
        assert!(result.local_ok);
        assert!(!result.storage_ok);
        assert!(result.details.unwrap().contains("storage file corrupted"));

        let _ = fs::remove_dir_all(&temp_dir);
    }

    #[test]
    fn test_verify_summary() {
        let results = vec![
            VerifyResult::ok(PathBuf::from("ok1.csv")),
            VerifyResult::ok(PathBuf::from("ok2.csv")),
            VerifyResult::issue(
                PathBuf::from("bad.csv"),
                false,
                true,
                true,
                "local file missing".to_string(),
            ),
        ];

        let summary = VerifySummary::from_results(&results);

        assert_eq!(summary.total, 3);
        assert_eq!(summary.passed, 2);
        assert_eq!(summary.local_issues, 1);
        assert_eq!(summary.storage_issues, 0);
        assert!(!summary.all_ok());
    }
}
