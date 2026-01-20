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
}
