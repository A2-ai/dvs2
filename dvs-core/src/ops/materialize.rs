//! DVS materialize operation - copy cached objects to working tree.

use crate::helpers::layout::{Layout, MaterializedState};
use crate::{detect_backend_cwd, Backend, DvsError, Manifest, Oid, RepoBackend};
use fs_err as fs;
use std::path::PathBuf;

/// Result of a materialize operation for a single file.
#[derive(Debug, Clone)]
pub struct MaterializeResult {
    /// File path.
    pub path: PathBuf,
    /// Object ID.
    pub oid: Oid,
    /// Whether the file was materialized (false = already up to date).
    pub materialized: bool,
    /// Error message if failed.
    pub error: Option<String>,
}

impl MaterializeResult {
    /// Create a successful materialize result.
    pub fn success(path: PathBuf, oid: Oid, materialized: bool) -> Self {
        Self {
            path,
            oid,
            materialized,
            error: None,
        }
    }

    /// Create an error materialize result.
    pub fn error(path: PathBuf, oid: Oid, message: String) -> Self {
        Self {
            path,
            oid,
            materialized: false,
            error: Some(message),
        }
    }

    /// Check if this result is an error.
    pub fn is_error(&self) -> bool {
        self.error.is_some()
    }
}

/// Summary of a materialize operation.
#[derive(Debug, Clone, Default)]
pub struct MaterializeSummary {
    /// Number of files materialized.
    pub materialized: usize,
    /// Number of files already up to date.
    pub up_to_date: usize,
    /// Number of files that failed.
    pub failed: usize,
    /// Individual results.
    pub results: Vec<MaterializeResult>,
}

/// Materialize all files in the manifest.
///
/// Copies cached objects to their working tree locations.
pub fn materialize() -> Result<MaterializeSummary, DvsError> {
    let backend = detect_backend_cwd()?;
    materialize_with_backend(&backend)
}

/// Materialize with a specific backend.
pub fn materialize_with_backend(backend: &Backend) -> Result<MaterializeSummary, DvsError> {
    let layout = Layout::new(backend.root().to_path_buf());

    // Load manifest
    let manifest_path = layout.manifest_path();
    if !manifest_path.exists() {
        return Err(DvsError::not_initialized());
    }
    let manifest = Manifest::load(&manifest_path)?;

    // Load materialized state
    let state_path = layout.materialized_state_path();
    let mut state = MaterializedState::load(&state_path)?;

    let mut summary = MaterializeSummary::default();

    for entry in &manifest.entries {
        let result =
            materialize_single_file(&entry.path, &entry.oid, &layout, &mut state, backend.root());
        match &result {
            MaterializeResult {
                materialized: true,
                error: None,
                ..
            } => summary.materialized += 1,
            MaterializeResult {
                materialized: false,
                error: None,
                ..
            } => summary.up_to_date += 1,
            MaterializeResult { error: Some(_), .. } => summary.failed += 1,
        }
        summary.results.push(result);
    }

    // Save updated state
    state.save(&state_path)?;

    Ok(summary)
}

/// Materialize a single file.
fn materialize_single_file(
    path: &PathBuf,
    oid: &Oid,
    layout: &Layout,
    state: &mut MaterializedState,
    repo_root: &std::path::Path,
) -> MaterializeResult {
    let oid_str = oid.to_string();

    // Check if already materialized with same OID
    if !state.needs_materialize(path, &oid_str) {
        return MaterializeResult::success(path.clone(), oid.clone(), false);
    }

    // Check if object is cached
    let cached_path = layout.cached_object_path(oid);
    if !cached_path.exists() {
        return MaterializeResult::error(
            path.clone(),
            oid.clone(),
            format!("Object not cached: {} (run pull first)", oid),
        );
    }

    // Determine destination path
    let dest_path = repo_root.join(path);

    // Create parent directory
    if let Some(parent) = dest_path.parent() {
        if let Err(e) = fs::create_dir_all(parent) {
            return MaterializeResult::error(
                path.clone(),
                oid.clone(),
                format!("Failed to create directory: {}", e),
            );
        }
    }

    // Copy cached object to destination
    if let Err(e) = fs::copy(&cached_path, &dest_path) {
        return MaterializeResult::error(
            path.clone(),
            oid.clone(),
            format!("Failed to copy file: {}", e),
        );
    }

    // Update state
    state.mark_materialized(path.clone(), oid_str);

    MaterializeResult::success(path.clone(), oid.clone(), true)
}

/// Materialize specific files by path.
pub fn materialize_files(files: &[PathBuf]) -> Result<MaterializeSummary, DvsError> {
    let backend = detect_backend_cwd()?;
    let layout = Layout::new(backend.root().to_path_buf());

    // Load manifest
    let manifest_path = layout.manifest_path();
    if !manifest_path.exists() {
        return Err(DvsError::not_initialized());
    }
    let manifest = Manifest::load(&manifest_path)?;

    // Load materialized state
    let state_path = layout.materialized_state_path();
    let mut state = MaterializedState::load(&state_path)?;

    let mut summary = MaterializeSummary::default();
    let repo_root = backend.root();

    for file in files {
        // Convert absolute paths to repo-relative for manifest lookup
        let relative_path = if file.is_absolute() {
            match pathdiff::diff_paths(file, repo_root) {
                Some(rel) => rel,
                None => {
                    summary.failed += 1;
                    summary.results.push(MaterializeResult::error(
                        file.clone(),
                        Oid::blake3("0".repeat(64)),
                        format!("File is outside repository: {}", file.display()),
                    ));
                    continue;
                }
            }
        } else {
            file.clone()
        };

        let entry = match manifest.get(&relative_path) {
            Some(e) => e,
            None => {
                summary.failed += 1;
                summary.results.push(MaterializeResult::error(
                    relative_path.clone(),
                    Oid::blake3("0".repeat(64)), // placeholder
                    format!("File not in manifest: {}", relative_path.display()),
                ));
                continue;
            }
        };

        let result =
            materialize_single_file(&entry.path, &entry.oid, &layout, &mut state, backend.root());
        match &result {
            MaterializeResult {
                materialized: true,
                error: None,
                ..
            } => summary.materialized += 1,
            MaterializeResult {
                materialized: false,
                error: None,
                ..
            } => summary.up_to_date += 1,
            MaterializeResult { error: Some(_), .. } => summary.failed += 1,
        }
        summary.results.push(result);
    }

    // Save updated state
    state.save(&state_path)?;

    Ok(summary)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Config, ManifestEntry};
    use std::sync::atomic::{AtomicU64, Ordering};

    static TEST_COUNTER: AtomicU64 = AtomicU64::new(0);

    fn setup_test_repo(test_name: &str) -> (PathBuf, PathBuf) {
        let unique_id = TEST_COUNTER.fetch_add(1, Ordering::SeqCst);
        let temp_dir = std::env::temp_dir().join(format!(
            "dvs-test-materialize-{}-{}-{}",
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

        // Initialize .dvs directory
        let layout = Layout::new(temp_dir.clone());
        layout.init().unwrap();

        (temp_dir, storage_dir)
    }

    #[test]
    fn test_materialize_result_success() {
        let path = PathBuf::from("data/file.csv");
        let oid = Oid::blake3("a".repeat(64));
        let result = MaterializeResult::success(path.clone(), oid.clone(), true);
        assert!(result.materialized);
        assert!(!result.is_error());
        assert_eq!(result.path, path);
    }

    #[test]
    fn test_materialize_result_error() {
        let path = PathBuf::from("data/file.csv");
        let oid = Oid::blake3("a".repeat(64));
        let result = MaterializeResult::error(path.clone(), oid.clone(), "test error".to_string());
        assert!(!result.materialized);
        assert!(result.is_error());
        assert_eq!(result.error.unwrap(), "test error");
    }

    #[test]
    fn test_materialize_summary_default() {
        let summary = MaterializeSummary::default();
        assert_eq!(summary.materialized, 0);
        assert_eq!(summary.up_to_date, 0);
        assert_eq!(summary.failed, 0);
        assert!(summary.results.is_empty());
    }

    #[test]
    fn test_materialize_from_cache() {
        let (temp_dir, _storage_dir) = setup_test_repo("materialize_from_cache");
        let layout = Layout::new(temp_dir.clone());

        // Create test content and compute its hash
        let content = b"test file content for materialize";
        let hash =
            crate::helpers::hash::hash_bytes(content, crate::helpers::hash::default_algorithm())
                .unwrap();
        let oid = Oid::new(crate::helpers::hash::default_algorithm(), hash);

        // Cache the object
        let cache_path = layout.cached_object_path(&oid);
        fs::create_dir_all(cache_path.parent().unwrap()).unwrap();
        fs::write(&cache_path, content).unwrap();
        assert!(cache_path.exists(), "Cache should contain object");

        // Create a manifest with an entry
        let mut manifest = Manifest::new();
        manifest.upsert(ManifestEntry::new(
            PathBuf::from("data.csv"),
            oid.clone(),
            content.len() as u64,
        ));

        // Save the manifest
        manifest.save(&layout.manifest_path()).unwrap();

        // Materialize
        let backend = crate::detect_backend(&temp_dir).unwrap();
        let summary = materialize_with_backend(&backend).unwrap();

        // Verify
        assert_eq!(summary.materialized, 1);
        assert_eq!(summary.failed, 0);
        assert_eq!(summary.up_to_date, 0);

        // Verify file was created
        let dest_path = temp_dir.join("data.csv");
        assert!(dest_path.exists(), "File should be materialized");
        assert_eq!(
            fs::read(&dest_path).unwrap(),
            content,
            "Content should match"
        );

        let _ = fs::remove_dir_all(&temp_dir);
    }

    #[test]
    fn test_materialize_already_up_to_date() {
        let (temp_dir, _storage_dir) = setup_test_repo("materialize_up_to_date");
        let layout = Layout::new(temp_dir.clone());

        // Create test content and compute its hash
        let content = b"cached content";
        let hash =
            crate::helpers::hash::hash_bytes(content, crate::helpers::hash::default_algorithm())
                .unwrap();
        let oid = Oid::new(crate::helpers::hash::default_algorithm(), hash);

        // Cache the object
        let cache_path = layout.cached_object_path(&oid);
        fs::create_dir_all(cache_path.parent().unwrap()).unwrap();
        fs::write(&cache_path, content).unwrap();

        // Create manifest
        let mut manifest = Manifest::new();
        manifest.upsert(ManifestEntry::new(
            PathBuf::from("file.txt"),
            oid.clone(),
            content.len() as u64,
        ));
        manifest.save(&layout.manifest_path()).unwrap();

        // First materialize
        let backend = crate::detect_backend(&temp_dir).unwrap();
        let summary1 = materialize_with_backend(&backend).unwrap();
        assert_eq!(summary1.materialized, 1);
        assert_eq!(summary1.up_to_date, 0);

        // Second materialize - should be up to date
        let summary2 = materialize_with_backend(&backend).unwrap();
        assert_eq!(summary2.materialized, 0);
        assert_eq!(summary2.up_to_date, 1);

        let _ = fs::remove_dir_all(&temp_dir);
    }

    #[test]
    fn test_materialize_missing_cache() {
        let (temp_dir, _storage_dir) = setup_test_repo("materialize_missing_cache");
        let layout = Layout::new(temp_dir.clone());

        // Create manifest with an entry that doesn't have a cached object
        // Use valid hex characters (not cached)
        let oid = Oid::blake3("deadbeef".repeat(8));
        let mut manifest = Manifest::new();
        manifest.upsert(ManifestEntry::new(
            PathBuf::from("missing.txt"),
            oid.clone(),
            100,
        ));
        manifest.save(&layout.manifest_path()).unwrap();

        // Materialize - should fail because object not cached
        let backend = crate::detect_backend(&temp_dir).unwrap();
        let summary = materialize_with_backend(&backend).unwrap();

        assert_eq!(summary.failed, 1);
        assert_eq!(summary.materialized, 0);
        assert!(
            summary.results[0]
                .error
                .as_ref()
                .unwrap()
                .contains("not cached"),
            "Error should mention cache"
        );

        let _ = fs::remove_dir_all(&temp_dir);
    }

    #[test]
    fn test_materialize_multiple_files() {
        let (temp_dir, _storage_dir) = setup_test_repo("materialize_multiple");
        let layout = Layout::new(temp_dir.clone());

        let mut manifest = Manifest::new();

        // Create multiple cached objects
        for i in 0..3 {
            let content = format!("content for file {}", i);
            let hash = crate::helpers::hash::hash_bytes(
                content.as_bytes(),
                crate::helpers::hash::default_algorithm(),
            )
            .unwrap();
            let oid = Oid::new(crate::helpers::hash::default_algorithm(), hash);

            // Cache the object
            let cache_path = layout.cached_object_path(&oid);
            fs::create_dir_all(cache_path.parent().unwrap()).unwrap();
            fs::write(&cache_path, content.as_bytes()).unwrap();

            // Add to manifest
            manifest.upsert(ManifestEntry::new(
                PathBuf::from(format!("dir/file{}.txt", i)),
                oid,
                content.len() as u64,
            ));
        }

        manifest.save(&layout.manifest_path()).unwrap();

        // Materialize
        let backend = crate::detect_backend(&temp_dir).unwrap();
        let summary = materialize_with_backend(&backend).unwrap();

        assert_eq!(summary.materialized, 3);
        assert_eq!(summary.failed, 0);

        // Verify all files exist
        for i in 0..3 {
            let path = temp_dir.join(format!("dir/file{}.txt", i));
            assert!(path.exists(), "File {} should be materialized", i);
        }

        let _ = fs::remove_dir_all(&temp_dir);
    }
}
