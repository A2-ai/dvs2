//! DVS pull operation - download objects from remote storage.

use crate::helpers::{
    layout::Layout,
    store::{HttpStore, LocalStore, ObjectStore},
};
use crate::{detect_backend_cwd, Backend, DvsError, LocalConfig, Manifest, Oid, RepoBackend};
use std::path::PathBuf;

/// Result of a pull operation for a single object.
#[derive(Debug, Clone)]
pub struct PullResult {
    /// Object ID.
    pub oid: Oid,
    /// Whether the object was downloaded (false = already cached).
    pub downloaded: bool,
    /// Error message if failed.
    pub error: Option<String>,
}

impl PullResult {
    /// Create a successful pull result.
    pub fn success(oid: Oid, downloaded: bool) -> Self {
        Self {
            oid,
            downloaded,
            error: None,
        }
    }

    /// Create an error pull result.
    pub fn error(oid: Oid, message: String) -> Self {
        Self {
            oid,
            downloaded: false,
            error: Some(message),
        }
    }

    /// Check if this result is an error.
    pub fn is_error(&self) -> bool {
        self.error.is_some()
    }
}

/// Summary of a pull operation.
#[derive(Debug, Clone, Default)]
pub struct PullSummary {
    /// Number of objects downloaded.
    pub downloaded: usize,
    /// Number of objects already cached.
    pub cached: usize,
    /// Number of objects that failed.
    pub failed: usize,
    /// Individual results.
    pub results: Vec<PullResult>,
}

/// Pull objects from remote storage.
///
/// Downloads any manifest objects that are missing from the local cache.
pub fn pull(remote_url: Option<&str>) -> Result<PullSummary, DvsError> {
    let backend = detect_backend_cwd()?;
    pull_with_backend(&backend, remote_url)
}

/// Pull with a specific backend.
pub fn pull_with_backend(
    backend: &Backend,
    remote_url: Option<&str>,
) -> Result<PullSummary, DvsError> {
    let layout = Layout::new(backend.root().to_path_buf());

    // Load manifest
    let manifest_path = layout.manifest_path();
    if !manifest_path.exists() {
        return Err(DvsError::not_initialized());
    }
    let manifest = Manifest::load(&manifest_path)?;

    // Load local config for default base_url
    let local_config = LocalConfig::load(&layout.config_path())?;

    // Determine remote URL (priority: explicit > local config > manifest)
    let url = remote_url
        .map(|s| s.to_string())
        .or_else(|| local_config.base_url().map(|s| s.to_string()))
        .or(manifest.base_url.clone())
        .ok_or_else(|| {
            DvsError::config_error(
                "No remote URL specified. Use --remote or set base_url in .dvs/config.toml",
            )
        })?;

    // Get auth token from local config
    let auth_token = local_config.auth_token().map(|s| s.to_string());

    // Create stores
    let local_store = LocalStore::new(layout.objects_dir());
    let remote_store = HttpStore::with_auth(url, auth_token);

    // Initialize local cache directory
    layout.init()?;

    // Get unique OIDs
    let oids = manifest.unique_oids();

    let mut summary = PullSummary::default();

    for oid in oids {
        let result = pull_single_object(oid, &local_store, &remote_store, &layout);
        match &result {
            PullResult {
                downloaded: true,
                error: None,
                ..
            } => summary.downloaded += 1,
            PullResult {
                downloaded: false,
                error: None,
                ..
            } => summary.cached += 1,
            PullResult { error: Some(_), .. } => summary.failed += 1,
        }
        summary.results.push(result);
    }

    Ok(summary)
}

/// Pull a single object from remote.
fn pull_single_object(
    oid: &Oid,
    _local_store: &LocalStore,
    remote_store: &HttpStore,
    layout: &Layout,
) -> PullResult {
    // Check if already cached locally
    if layout.is_cached(oid) {
        return PullResult::success(oid.clone(), false);
    }

    // Download from remote
    let local_path = layout.cached_object_path(oid);
    match remote_store.get(oid, &local_path) {
        Ok(()) => PullResult::success(oid.clone(), true),
        Err(e) => PullResult::error(oid.clone(), format!("Failed to download: {}", e)),
    }
}

/// Pull specific files by path.
pub fn pull_files(files: &[PathBuf], remote_url: Option<&str>) -> Result<PullSummary, DvsError> {
    let backend = detect_backend_cwd()?;
    let layout = Layout::new(backend.root().to_path_buf());

    // Load manifest
    let manifest_path = layout.manifest_path();
    if !manifest_path.exists() {
        return Err(DvsError::not_initialized());
    }
    let manifest = Manifest::load(&manifest_path)?;

    // Load local config for default base_url
    let local_config = LocalConfig::load(&layout.config_path())?;

    // Determine remote URL (priority: explicit > local config > manifest)
    let url = remote_url
        .map(|s| s.to_string())
        .or_else(|| local_config.base_url().map(|s| s.to_string()))
        .or(manifest.base_url.clone())
        .ok_or_else(|| {
            DvsError::config_error(
                "No remote URL specified. Use --remote or set base_url in .dvs/config.toml",
            )
        })?;

    // Get auth token from local config
    let auth_token = local_config.auth_token().map(|s| s.to_string());

    // Create stores
    let local_store = LocalStore::new(layout.objects_dir());
    let remote_store = HttpStore::with_auth(url, auth_token);

    // Initialize local cache directory
    layout.init()?;

    let mut summary = PullSummary::default();
    let repo_root = backend.root();

    // Find OIDs for requested files
    for file in files {
        // Convert absolute paths to repo-relative for manifest lookup
        let relative_path = if file.is_absolute() {
            match pathdiff::diff_paths(file, repo_root) {
                Some(rel) => rel,
                None => {
                    summary.failed += 1;
                    summary.results.push(PullResult::error(
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
                summary.results.push(PullResult::error(
                    Oid::blake3("0".repeat(64)), // placeholder
                    format!("File not in manifest: {}", relative_path.display()),
                ));
                continue;
            }
        };

        let result = pull_single_object(&entry.oid, &local_store, &remote_store, &layout);
        match &result {
            PullResult {
                downloaded: true,
                error: None,
                ..
            } => summary.downloaded += 1,
            PullResult {
                downloaded: false,
                error: None,
                ..
            } => summary.cached += 1,
            PullResult { error: Some(_), .. } => summary.failed += 1,
        }
        summary.results.push(result);
    }

    Ok(summary)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_pull_result_success() {
        let oid = Oid::blake3("a".repeat(64));
        let result = PullResult::success(oid.clone(), true);
        assert!(result.downloaded);
        assert!(!result.is_error());
    }

    #[test]
    fn test_pull_result_error() {
        let oid = Oid::blake3("a".repeat(64));
        let result = PullResult::error(oid.clone(), "test error".to_string());
        assert!(!result.downloaded);
        assert!(result.is_error());
        assert_eq!(result.error.unwrap(), "test error");
    }

    #[test]
    fn test_pull_summary_default() {
        let summary = PullSummary::default();
        assert_eq!(summary.downloaded, 0);
        assert_eq!(summary.cached, 0);
        assert_eq!(summary.failed, 0);
        assert!(summary.results.is_empty());
    }
}
