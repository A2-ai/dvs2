//! Path resolution utilities for the CLI.
//!
//! Handles working directory changes, tilde expansion, and path normalization.

use std::path::{Path, PathBuf};

use crate::commands::{CliError, Result};

/// Change the current working directory.
pub fn set_cwd(path: &Path) -> Result<()> {
    // Resolve the path first
    let resolved = resolve_path(path)?;

    // Check it exists and is a directory
    if !resolved.exists() {
        return Err(CliError::Path(format!(
            "Directory does not exist: {}",
            resolved.display()
        )));
    }

    if !resolved.is_dir() {
        return Err(CliError::Path(format!(
            "Not a directory: {}",
            resolved.display()
        )));
    }

    // Change directory
    std::env::set_current_dir(&resolved).map_err(|e| {
        CliError::Path(format!(
            "Failed to change to {}: {}",
            resolved.display(),
            e
        ))
    })
}

/// Resolve a path, expanding ~ and making it absolute.
pub fn resolve_path(path: &Path) -> Result<PathBuf> {
    let path_str = path.to_string_lossy();

    // Expand tilde
    let expanded = if path_str.starts_with("~/") {
        if let Some(home) = home_dir() {
            home.join(&path_str[2..])
        } else {
            path.to_path_buf()
        }
    } else if path_str == "~" {
        home_dir().unwrap_or_else(|| path.to_path_buf())
    } else {
        path.to_path_buf()
    };

    // Make absolute
    if expanded.is_absolute() {
        Ok(expanded)
    } else {
        let cwd = std::env::current_dir().map_err(|e| {
            CliError::Path(format!("Failed to get current directory: {}", e))
        })?;
        Ok(cwd.join(expanded))
    }
}

/// Get the home directory.
fn home_dir() -> Option<PathBuf> {
    // Try HOME environment variable first
    std::env::var("HOME")
        .ok()
        .map(PathBuf::from)
        .or_else(|| {
            // Fall back to platform-specific methods
            #[cfg(unix)]
            {
                // On Unix, we could use getpwuid, but HOME is almost always set
                None
            }
            #[cfg(windows)]
            {
                std::env::var("USERPROFILE").ok().map(PathBuf::from)
            }
            #[cfg(not(any(unix, windows)))]
            {
                None
            }
        })
}

/// Normalize a path by resolving . and .. components.
#[allow(dead_code)]
pub fn normalize_path(path: &Path) -> PathBuf {
    let mut components = Vec::new();

    for component in path.components() {
        match component {
            std::path::Component::ParentDir => {
                components.pop();
            }
            std::path::Component::CurDir => {}
            c => components.push(c),
        }
    }

    components.iter().collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_resolve_path_absolute() {
        let path = PathBuf::from("/absolute/path");
        let resolved = resolve_path(&path).unwrap();
        assert!(resolved.is_absolute());
        assert_eq!(resolved, path);
    }

    #[test]
    fn test_resolve_path_tilde() {
        if let Some(home) = home_dir() {
            let path = PathBuf::from("~/test");
            let resolved = resolve_path(&path).unwrap();
            assert_eq!(resolved, home.join("test"));
        }
    }

    #[test]
    fn test_normalize_path() {
        let path = PathBuf::from("/a/b/../c/./d");
        let normalized = normalize_path(&path);
        assert_eq!(normalized, PathBuf::from("/a/c/d"));
    }
}
