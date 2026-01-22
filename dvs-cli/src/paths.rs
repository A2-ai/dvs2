//! Path resolution utilities for the CLI.
//!
//! Handles working directory changes, tilde expansion, and path normalization.

use std::io::{self, BufRead};
use std::path::{Path, PathBuf};

use crate::commands::{CliError, Result};

/// Read file paths from stdin, one per line.
/// Empty lines and lines starting with # are ignored.
pub fn read_paths_from_stdin() -> Result<Vec<PathBuf>> {
    let stdin = io::stdin();
    let paths: Vec<PathBuf> = stdin
        .lock()
        .lines()
        .filter_map(|line| {
            let line = line.ok()?;
            let trimmed = line.trim();
            // Skip empty lines and comments
            if trimmed.is_empty() || trimmed.starts_with('#') {
                None
            } else {
                Some(PathBuf::from(trimmed))
            }
        })
        .collect();
    Ok(paths)
}

/// Collect files from args or stdin (batch mode).
pub fn collect_files(files: Vec<PathBuf>, batch: bool) -> Result<Vec<PathBuf>> {
    if batch {
        read_paths_from_stdin()
    } else {
        Ok(files)
    }
}

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
    std::env::set_current_dir(&resolved)
        .map_err(|e| CliError::Path(format!("Failed to change to {}: {}", resolved.display(), e)))
}

/// Resolve a path, expanding ~ and making it absolute.
pub fn resolve_path(path: &Path) -> Result<PathBuf> {
    let path_str = path.to_string_lossy();

    // Expand tilde
    let expanded = if let Some(rest) = path_str.strip_prefix("~/") {
        if let Some(home) = home_dir() {
            home.join(rest)
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
        let cwd = std::env::current_dir()
            .map_err(|e| CliError::Path(format!("Failed to get current directory: {}", e)))?;
        Ok(cwd.join(expanded))
    }
}

/// Get the home directory.
fn home_dir() -> Option<PathBuf> {
    // Try HOME environment variable first
    std::env::var("HOME").ok().map(PathBuf::from).or({
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

    #[test]
    fn test_collect_files_non_batch() {
        let files = vec![PathBuf::from("a.txt"), PathBuf::from("b.txt")];
        let result = collect_files(files.clone(), false).unwrap();
        assert_eq!(result, files);
    }

    #[test]
    fn test_collect_files_empty() {
        let files: Vec<PathBuf> = vec![];
        let result = collect_files(files, false).unwrap();
        assert!(result.is_empty());
    }
}
