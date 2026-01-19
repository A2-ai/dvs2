//! Configuration loading and validation utilities.

use std::path::Path;
use crate::{Config, DvsError};

/// Find the repository root (directory containing .git).
pub fn find_repo_root() -> Result<std::path::PathBuf, DvsError> {
    todo!("Find repository root by searching for .git")
}

/// Load configuration from dvs.yaml.
pub fn load_config(_repo_root: &Path) -> Result<Config, DvsError> {
    todo!("Load configuration from dvs.yaml")
}

/// Save configuration to dvs.yaml.
pub fn save_config(_config: &Config, _repo_root: &Path) -> Result<(), DvsError> {
    todo!("Save configuration to dvs.yaml")
}

/// Validate storage directory exists and is accessible.
pub fn validate_storage_dir(_storage_dir: &Path) -> Result<(), DvsError> {
    todo!("Validate storage directory")
}

/// Get the path to dvs.yaml in the repository.
pub fn config_path(repo_root: &Path) -> std::path::PathBuf {
    repo_root.join("dvs.yaml")
}
