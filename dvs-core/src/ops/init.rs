//! DVS initialization operation.

use std::path::Path;
use crate::{Config, DvsError, Backend, detect_backend_cwd};

/// Initialize DVS for a project.
///
/// Creates `dvs.yaml` configuration file and validates/creates the storage directory.
///
/// # Arguments
///
/// * `storage_dir` - Path to the external storage directory
/// * `permissions` - Optional file permissions (octal, e.g., 0o664)
/// * `group` - Optional Linux group for stored files
///
/// # Returns
///
/// The created configuration on success.
///
/// # Errors
///
/// * `NotInitialized` - Not in a git repository or DVS workspace
/// * `ConfigMismatch` - Config exists with different settings
/// * `PermissionDenied` - Cannot create storage directory
pub fn init(
    storage_dir: &Path,
    permissions: Option<u32>,
    group: Option<&str>,
) -> Result<Config, DvsError> {
    // Detect backend (prefer git, fallback to dvs workspace)
    let backend = detect_backend_cwd()?;
    init_with_backend(&backend, storage_dir, permissions, group)
}

/// Initialize DVS with a specific backend.
///
/// Use this when you already have a backend reference.
pub fn init_with_backend(
    _backend: &Backend,
    _storage_dir: &Path,
    _permissions: Option<u32>,
    _group: Option<&str>,
) -> Result<Config, DvsError> {
    todo!("Implement init operation with backend")
}

/// Validate and create the storage directory.
fn setup_storage_directory(_path: &Path, _permissions: Option<u32>) -> Result<(), DvsError> {
    todo!("Setup storage directory")
}

/// Validate group membership.
fn validate_group(_group: &str) -> Result<(), DvsError> {
    todo!("Validate group membership")
}
