//! DVS initialization operation.

use std::fs;
use std::path::Path;
use crate::{Config, DvsError, Backend, RepoBackend, detect_backend_cwd};
use crate::helpers::{config as config_helper, copy};

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
    backend: &Backend,
    storage_dir: &Path,
    permissions: Option<u32>,
    group: Option<&str>,
) -> Result<Config, DvsError> {
    let repo_root = backend.root();

    // Validate group membership if specified
    if let Some(grp) = group {
        validate_group(grp)?;
    }

    // Create or validate storage directory
    setup_storage_directory(storage_dir, permissions)?;

    // Create the configuration
    let config = Config::new(
        storage_dir.to_path_buf(),
        permissions,
        group.map(|s| s.to_string()),
    );

    // Check for existing configuration
    let config_path = config_helper::config_path(repo_root);
    if config_path.exists() {
        let existing = Config::load(&config_path)?;

        // Check for mismatched settings
        if existing.storage_dir != config.storage_dir
            || existing.permissions != config.permissions
            || existing.group != config.group
        {
            return Err(DvsError::ConfigMismatch);
        }

        // Same config already exists, return it
        return Ok(existing);
    }

    // Save the new configuration
    config.save(&config_path)?;

    // Add dvs.yaml to .gitignore if we're in a git repo and it exists
    if let Backend::Git(_) = backend {
        let gitignore_path = repo_root.join(".gitignore");
        add_to_gitignore(&gitignore_path, "*.dvs")?;
    }

    Ok(config)
}

/// Validate and create the storage directory.
fn setup_storage_directory(path: &Path, permissions: Option<u32>) -> Result<(), DvsError> {
    if path.exists() {
        // Validate existing directory
        if !path.is_dir() {
            return Err(DvsError::StorageError {
                message: format!("Storage path exists but is not a directory: {}", path.display()),
            });
        }
        config_helper::validate_storage_dir(path)?;
    } else {
        // Create the directory
        config_helper::create_storage_dir(path)?;

        // Set permissions if specified (Unix only)
        #[cfg(unix)]
        if let Some(perms) = permissions {
            copy::set_permissions(path, perms)?;
        }
    }

    Ok(())
}

/// Validate group membership.
fn validate_group(group: &str) -> Result<(), DvsError> {
    if !copy::group_exists(group) {
        return Err(DvsError::GroupNotSet {
            group: group.to_string(),
        });
    }
    Ok(())
}

/// Add a pattern to .gitignore if not already present.
fn add_to_gitignore(gitignore_path: &Path, pattern: &str) -> Result<(), DvsError> {
    let contents = if gitignore_path.exists() {
        fs::read_to_string(gitignore_path)?
    } else {
        String::new()
    };

    // Check if pattern already exists
    for line in contents.lines() {
        if line.trim() == pattern {
            return Ok(()); // Already present
        }
    }

    // Append the pattern
    let new_contents = if contents.is_empty() || contents.ends_with('\n') {
        format!("{}{}\n", contents, pattern)
    } else {
        format!("{}\n{}\n", contents, pattern)
    };

    fs::write(gitignore_path, new_contents)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_setup_storage_directory_new() {
        let temp_dir = std::env::temp_dir().join("dvs-test-init-storage");
        let _ = fs::remove_dir_all(&temp_dir);

        let storage = temp_dir.join("storage");
        setup_storage_directory(&storage, None).unwrap();
        assert!(storage.exists());
        assert!(storage.is_dir());

        let _ = fs::remove_dir_all(&temp_dir);
    }

    #[test]
    fn test_setup_storage_directory_existing() {
        let temp_dir = std::env::temp_dir().join("dvs-test-init-storage-existing");
        let _ = fs::create_dir_all(&temp_dir);

        // Should succeed for existing directory
        setup_storage_directory(&temp_dir, None).unwrap();

        let _ = fs::remove_dir_all(&temp_dir);
    }

    #[test]
    fn test_add_to_gitignore_new() {
        let temp_dir = std::env::temp_dir().join("dvs-test-gitignore-new");
        let _ = fs::create_dir_all(&temp_dir);

        let gitignore = temp_dir.join(".gitignore");
        add_to_gitignore(&gitignore, "*.dvs").unwrap();

        let contents = fs::read_to_string(&gitignore).unwrap();
        assert!(contents.contains("*.dvs"));

        let _ = fs::remove_dir_all(&temp_dir);
    }

    #[test]
    fn test_add_to_gitignore_existing() {
        let temp_dir = std::env::temp_dir().join("dvs-test-gitignore-existing");
        let _ = fs::create_dir_all(&temp_dir);

        let gitignore = temp_dir.join(".gitignore");
        fs::write(&gitignore, "node_modules/\n").unwrap();

        add_to_gitignore(&gitignore, "*.dvs").unwrap();

        let contents = fs::read_to_string(&gitignore).unwrap();
        assert!(contents.contains("node_modules/"));
        assert!(contents.contains("*.dvs"));

        let _ = fs::remove_dir_all(&temp_dir);
    }

    #[test]
    fn test_add_to_gitignore_already_present() {
        let temp_dir = std::env::temp_dir().join("dvs-test-gitignore-present");
        let _ = fs::create_dir_all(&temp_dir);

        let gitignore = temp_dir.join(".gitignore");
        fs::write(&gitignore, "*.dvs\n").unwrap();

        add_to_gitignore(&gitignore, "*.dvs").unwrap();

        // Should not duplicate
        let contents = fs::read_to_string(&gitignore).unwrap();
        let count = contents.matches("*.dvs").count();
        assert_eq!(count, 1);

        let _ = fs::remove_dir_all(&temp_dir);
    }
}
