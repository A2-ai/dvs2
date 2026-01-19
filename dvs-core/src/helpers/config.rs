//! Configuration loading and validation utilities.

use std::fs;
use std::path::{Path, PathBuf};
use crate::{Config, DvsError};

/// Find the repository root (directory containing .git or dvs.yaml).
///
/// Searches from the current directory upward.
pub fn find_repo_root() -> Result<PathBuf, DvsError> {
    let cwd = std::env::current_dir()?;
    find_repo_root_from(&cwd)
}

/// Find the repository root starting from a specific directory.
pub fn find_repo_root_from(start: &Path) -> Result<PathBuf, DvsError> {
    let mut current = start.to_path_buf();

    loop {
        // Check for .git directory (Git repository)
        if current.join(".git").exists() {
            return Ok(current);
        }

        // Check for dvs.yaml (DVS workspace without Git)
        if current.join(Config::config_filename()).exists() {
            return Ok(current);
        }

        // Check for .dvs directory (DVS workspace marker)
        if current.join(".dvs").exists() {
            return Ok(current);
        }

        // Move up to parent directory
        match current.parent() {
            Some(parent) => current = parent.to_path_buf(),
            None => {
                return Err(DvsError::NotInGitRepo);
            }
        }
    }
}

/// Load configuration from dvs.yaml.
pub fn load_config(repo_root: &Path) -> Result<Config, DvsError> {
    let config_path = config_path(repo_root);

    if !config_path.exists() {
        return Err(DvsError::NotInitialized);
    }

    Config::load(&config_path)
}

/// Save configuration to dvs.yaml.
pub fn save_config(config: &Config, repo_root: &Path) -> Result<(), DvsError> {
    let config_path = config_path(repo_root);
    config.save(&config_path)
}

/// Validate storage directory exists and is accessible.
pub fn validate_storage_dir(storage_dir: &Path) -> Result<(), DvsError> {
    if !storage_dir.exists() {
        return Err(DvsError::StorageError {
            message: format!("Storage directory does not exist: {}", storage_dir.display()),
        });
    }

    if !storage_dir.is_dir() {
        return Err(DvsError::StorageError {
            message: format!("Storage path is not a directory: {}", storage_dir.display()),
        });
    }

    // Check if we can write to the directory by attempting to create a temp file
    let test_file = storage_dir.join(".dvs_write_test");
    match fs::write(&test_file, b"test") {
        Ok(_) => {
            let _ = fs::remove_file(&test_file);
            Ok(())
        }
        Err(e) => Err(DvsError::StorageError {
            message: format!(
                "Cannot write to storage directory {}: {}",
                storage_dir.display(),
                e
            ),
        }),
    }
}

/// Create storage directory if it doesn't exist.
pub fn create_storage_dir(storage_dir: &Path) -> Result<(), DvsError> {
    if storage_dir.exists() {
        if !storage_dir.is_dir() {
            return Err(DvsError::StorageError {
                message: format!("Storage path exists but is not a directory: {}", storage_dir.display()),
            });
        }
        return Ok(());
    }

    fs::create_dir_all(storage_dir).map_err(|e| DvsError::StorageError {
        message: format!("Failed to create storage directory {}: {}", storage_dir.display(), e),
    })
}

/// Get the path to dvs.yaml in the repository.
pub fn config_path(repo_root: &Path) -> PathBuf {
    repo_root.join(Config::config_filename())
}

/// Check if DVS is initialized in the repository.
pub fn is_initialized(repo_root: &Path) -> bool {
    config_path(repo_root).exists()
}

/// Expand a path to be absolute, resolving ~ and relative paths.
pub fn expand_path(path: &Path, base: &Path) -> PathBuf {
    let path_str = path.to_string_lossy();

    // Handle ~ expansion
    if path_str.starts_with("~/") || path_str == "~" {
        if let Some(home) = home_dir() {
            if path_str == "~" {
                return home;
            }
            return home.join(&path_str[2..]);
        }
    }

    // Handle relative paths
    if path.is_relative() {
        base.join(path)
    } else {
        path.to_path_buf()
    }
}

/// Get the user's home directory.
fn home_dir() -> Option<PathBuf> {
    std::env::var_os("HOME")
        .or_else(|| std::env::var_os("USERPROFILE"))
        .map(PathBuf::from)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_config_path() {
        let repo_root = PathBuf::from("/project");
        let path = config_path(&repo_root);
        assert_eq!(path, PathBuf::from("/project/dvs.yaml"));
    }

    #[test]
    fn test_is_initialized() {
        let temp_dir = std::env::temp_dir().join("dvs-test-init-check");
        let _ = fs::create_dir_all(&temp_dir);

        // Not initialized initially
        assert!(!is_initialized(&temp_dir));

        // Create dvs.yaml
        let config_file = temp_dir.join("dvs.yaml");
        fs::write(&config_file, "storage_dir: /storage").unwrap();

        // Now initialized
        assert!(is_initialized(&temp_dir));

        // Cleanup
        let _ = fs::remove_dir_all(&temp_dir);
    }

    #[test]
    fn test_expand_path_relative() {
        let base = PathBuf::from("/project");
        let relative = PathBuf::from("data/file.csv");
        let expanded = expand_path(&relative, &base);
        assert_eq!(expanded, PathBuf::from("/project/data/file.csv"));
    }

    #[test]
    fn test_expand_path_absolute() {
        let base = PathBuf::from("/project");
        let absolute = PathBuf::from("/other/file.csv");
        let expanded = expand_path(&absolute, &base);
        assert_eq!(expanded, PathBuf::from("/other/file.csv"));
    }

    #[test]
    fn test_expand_path_tilde() {
        let base = PathBuf::from("/project");
        let tilde = PathBuf::from("~/data/file.csv");
        let expanded = expand_path(&tilde, &base);

        // Should expand to home directory if HOME is set
        if let Some(home) = home_dir() {
            assert_eq!(expanded, home.join("data/file.csv"));
        }
    }

    #[test]
    fn test_validate_storage_dir() {
        let temp_dir = std::env::temp_dir().join("dvs-test-validate-storage");
        let _ = fs::create_dir_all(&temp_dir);

        // Valid directory
        assert!(validate_storage_dir(&temp_dir).is_ok());

        // Non-existent directory
        let nonexistent = temp_dir.join("nonexistent");
        assert!(validate_storage_dir(&nonexistent).is_err());

        // Cleanup
        let _ = fs::remove_dir_all(&temp_dir);
    }

    #[test]
    fn test_create_storage_dir() {
        let temp_dir = std::env::temp_dir().join("dvs-test-create-storage");
        let _ = fs::remove_dir_all(&temp_dir);

        let storage = temp_dir.join("deep/nested/storage");
        assert!(!storage.exists());

        // Create it
        create_storage_dir(&storage).unwrap();
        assert!(storage.exists());
        assert!(storage.is_dir());

        // Creating again should succeed
        create_storage_dir(&storage).unwrap();

        // Cleanup
        let _ = fs::remove_dir_all(&temp_dir);
    }
}
