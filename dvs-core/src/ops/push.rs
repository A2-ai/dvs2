//! DVS push operation - upload objects to remote storage.

use crate::helpers::{
    layout::Layout,
    store::{HttpStore, LocalStore, ObjectStore},
};
use crate::{detect_backend_cwd, Backend, DvsError, LocalConfig, Manifest, Oid, RepoBackend};
use std::path::PathBuf;

/// Result of a push operation for a single object.
#[derive(Debug, Clone)]
pub struct PushResult {
    /// Object ID.
    pub oid: Oid,
    /// Whether the object was uploaded (false = already present).
    pub uploaded: bool,
    /// Error message if failed.
    pub error: Option<String>,
}

impl PushResult {
    /// Create a successful push result.
    pub fn success(oid: Oid, uploaded: bool) -> Self {
        Self {
            oid,
            uploaded,
            error: None,
        }
    }

    /// Create an error push result.
    pub fn error(oid: Oid, message: String) -> Self {
        Self {
            oid,
            uploaded: false,
            error: Some(message),
        }
    }

    /// Check if this result is an error.
    pub fn is_error(&self) -> bool {
        self.error.is_some()
    }
}

/// Summary of a push operation.
#[derive(Debug, Clone, Default)]
pub struct PushSummary {
    /// Number of objects uploaded.
    pub uploaded: usize,
    /// Number of objects already present.
    pub present: usize,
    /// Number of objects that failed.
    pub failed: usize,
    /// Individual results.
    pub results: Vec<PushResult>,
}

/// Push objects to remote storage.
///
/// Uploads any manifest objects that are missing from the remote store.
pub fn push(remote_url: Option<&str>) -> Result<PushSummary, DvsError> {
    let backend = detect_backend_cwd()?;
    push_with_backend(&backend, remote_url)
}

/// Push with a specific backend.
pub fn push_with_backend(
    backend: &Backend,
    remote_url: Option<&str>,
) -> Result<PushSummary, DvsError> {
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

    // Get unique OIDs
    let oids = manifest.unique_oids();

    let mut summary = PushSummary::default();

    for oid in oids {
        let result = push_single_object(oid, &local_store, &remote_store, &layout);
        match &result {
            PushResult {
                uploaded: true,
                error: None,
                ..
            } => summary.uploaded += 1,
            PushResult {
                uploaded: false,
                error: None,
                ..
            } => summary.present += 1,
            PushResult { error: Some(_), .. } => summary.failed += 1,
        }
        summary.results.push(result);
    }

    Ok(summary)
}

/// Push a single object to remote.
fn push_single_object(
    oid: &Oid,
    _local_store: &LocalStore,
    remote_store: &HttpStore,
    layout: &Layout,
) -> PushResult {
    // Check if already present on remote
    match remote_store.has(oid) {
        Ok(true) => return PushResult::success(oid.clone(), false),
        Ok(false) => {}
        Err(e) => return PushResult::error(oid.clone(), format!("Failed to check remote: {}", e)),
    }

    // Check if we have it locally
    let local_path = layout.cached_object_path(oid);
    if !local_path.exists() {
        return PushResult::error(
            oid.clone(),
            format!("Object not found in local cache: {}", oid),
        );
    }

    // Upload to remote
    match remote_store.put(oid, &local_path) {
        Ok(()) => PushResult::success(oid.clone(), true),
        Err(e) => PushResult::error(oid.clone(), format!("Failed to upload: {}", e)),
    }
}

/// Push specific files by path.
pub fn push_files(files: &[PathBuf], remote_url: Option<&str>) -> Result<PushSummary, DvsError> {
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

    let mut summary = PushSummary::default();
    let repo_root = backend.root();

    // Find OIDs for requested files
    for file in files {
        // Convert absolute paths to repo-relative for manifest lookup
        let relative_path = if file.is_absolute() {
            match pathdiff::diff_paths(file, repo_root) {
                Some(rel) => rel,
                None => {
                    summary.failed += 1;
                    summary.results.push(PushResult::error(
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
                summary.results.push(PushResult::error(
                    Oid::blake3("0".repeat(64)), // placeholder
                    format!("File not in manifest: {}", relative_path.display()),
                ));
                continue;
            }
        };

        let result = push_single_object(&entry.oid, &local_store, &remote_store, &layout);
        match &result {
            PushResult {
                uploaded: true,
                error: None,
                ..
            } => summary.uploaded += 1,
            PushResult {
                uploaded: false,
                error: None,
                ..
            } => summary.present += 1,
            PushResult { error: Some(_), .. } => summary.failed += 1,
        }
        summary.results.push(result);
    }

    Ok(summary)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_push_result_success() {
        let oid = Oid::blake3("a".repeat(64));
        let result = PushResult::success(oid.clone(), true);
        assert!(result.uploaded);
        assert!(!result.is_error());
    }

    #[test]
    fn test_push_result_error() {
        let oid = Oid::blake3("a".repeat(64));
        let result = PushResult::error(oid.clone(), "test error".to_string());
        assert!(!result.uploaded);
        assert!(result.is_error());
        assert_eq!(result.error.unwrap(), "test error");
    }

    #[test]
    fn test_push_summary_default() {
        let summary = PushSummary::default();
        assert_eq!(summary.uploaded, 0);
        assert_eq!(summary.present, 0);
        assert_eq!(summary.failed, 0);
        assert!(summary.results.is_empty());
    }
}
