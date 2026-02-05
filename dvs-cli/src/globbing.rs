use std::collections::HashSet;
use std::ffi::OsStr;
use std::path::PathBuf;

use anyhow::{Result, anyhow, bail};
use dvs::paths::DvsPaths;
use globset::{GlobBuilder, GlobMatcher};
use walkdir::WalkDir;

/// Builds the glob matching the rg behaviour
/// eg "*.csv" will not match `some/dir/test.csv`
fn build_glob_matcher(pattern: Option<&str>) -> Result<Option<GlobMatcher>> {
    pattern
        .map(|p| {
            GlobBuilder::new(p)
                .literal_separator(true)
                .build()
                .map(|g| g.compile_matcher())
                .map_err(Into::into)
        })
        .transpose()
}

/// Resolve paths for `add` command following ripgrep-style behavior:
/// - Explicit files: added directly (glob ignored)
/// - Explicit directories: walked and filtered by glob
/// - No paths + glob: walks cwd filtered by glob
pub fn resolve_paths_for_add(
    paths: Vec<PathBuf>,
    glob_pattern: Option<&str>,
    dvs_paths: &DvsPaths,
) -> Result<HashSet<PathBuf>> {
    let mut out = HashSet::new();
    let glob_matcher = build_glob_matcher(glob_pattern)?;
    let repo_root = dvs_paths.repo_root().canonicalize()?;
    let metadata_root = dvs_paths.metadata_folder().canonicalize()?;

    // If no paths given, default to cwd
    let paths = if paths.is_empty() {
        vec![PathBuf::from(".")]
    } else {
        paths
    };

    for path in paths {
        let full_path = dvs_paths
            .cwd()
            .join(&path)
            .canonicalize()
            .map_err(|_| anyhow!("Path not found: {}", path.display()))?;

        // Explicit file: we ignore the glob and add it to the file
        if full_path.is_file() {
            // Ensure it's in the repo
            let relative_to_root = full_path
                .strip_prefix(&repo_root)
                .map_err(|_| anyhow!("Path is outside repository: {}", path.display()))?
                .to_path_buf();
            out.insert(relative_to_root);
        } else if full_path.is_dir() {
            if let Some(matcher) = &glob_matcher {
                for entry in WalkDir::new(&full_path).into_iter().filter_map(|e| e.ok()) {
                    let entry_path = entry.path().canonicalize()?;
                    // Skip directories and metadata root folder
                    if !entry_path.is_file() || entry_path.starts_with(&metadata_root) {
                        continue;
                    }

                    // Get path relative to the walked directory for matching
                    let relative_to_dir = match entry_path.strip_prefix(&full_path) {
                        Ok(p) => p,
                        Err(_) => continue,
                    };
                    if matcher.is_match(relative_to_dir) {
                        // Return path relative to repo root
                        let relative_to_root = match entry_path.strip_prefix(&repo_root) {
                            Ok(p) => p.to_path_buf(),
                            Err(_) => continue,
                        };
                        out.insert(relative_to_root);
                    }
                }
            } else {
            }
        } else {
            bail!("Path is not a file or directory: {}", path.display());
        }
    }

    Ok(out)
}

pub fn resolve_paths_for_get(
    paths: Vec<PathBuf>,
    glob_pattern: Option<&str>,
    dvs_paths: &DvsPaths,
) -> Result<HashSet<PathBuf>> {
    let mut out = HashSet::new();
    let glob_matcher = build_glob_matcher(glob_pattern)?;
    let metadata_root = dvs_paths.metadata_folder().canonicalize()?;
    // Get cwd-relative prefix for converting user paths to repo-root-relative
    let cwd_prefix = dvs_paths.cwd_relative_to_root();

    // Convert user paths to repo-relative directory filters
    // If no paths given, default to cwd (or repo root if at root)
    let dir_filters: Vec<PathBuf> = if paths.is_empty() {
        vec![cwd_prefix.map(|p| p.to_path_buf()).unwrap_or_default()]
    } else {
        paths
            .into_iter()
            .map(|p| {
                if let Some(prefix) = cwd_prefix {
                    prefix.join(&p)
                } else {
                    p
                }
            })
            .collect()
    };

    // Walk all metadata files
    for entry in WalkDir::new(&metadata_root)
        .into_iter()
        .filter_map(|e| e.ok())
    {
        let entry_path = entry.path();

        // Skip directories and non .dvs files
        if !entry_path.is_file() || entry_path.extension() != Some(OsStr::new("dvs")) {
            continue;
        }
        // Get repo-relative tracked path (strip metadata folder and .dvs extension)
        let relative_to_metadata = match entry_path.strip_prefix(&metadata_root) {
            Ok(p) => p,
            Err(_) => continue,
        };
        let tracked_path = relative_to_metadata.with_extension("");

        // Filter: must be under one of user's directories (or exact match)
        let under_filter = dir_filters
            .iter()
            .any(|dir| tracked_path.starts_with(dir) || &tracked_path == dir);
        if !under_filter {
            continue;
        }

        // Get cwd-relative path for glob matching
        let cwd_relative = if let Some(prefix) = cwd_prefix {
            match tracked_path.strip_prefix(prefix) {
                Ok(p) => p.to_path_buf(),
                Err(_) => continue, // File not under cwd
            }
        } else {
            tracked_path.clone()
        };

        // Apply glob if present, otherwise match all
        if glob_matcher
            .as_ref()
            .map_or(true, |g| g.is_match(&cwd_relative))
        {
            out.insert(tracked_path);
        }
    }

    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs::{self, File};
    use tempfile::TempDir;

    /// Creates a test repo structure with files and metadata
    fn setup_test_repo() -> (TempDir, DvsPaths) {
        let temp = TempDir::new().unwrap();
        let root = temp.path();

        // Create .git to mark repo root
        fs::create_dir(root.join(".git")).unwrap();

        // Create files
        fs::create_dir_all(root.join("data/subdir")).unwrap();
        File::create(root.join("foo.txt")).unwrap();
        File::create(root.join("bar.csv")).unwrap();
        File::create(root.join("data/a.csv")).unwrap();
        File::create(root.join("data/b.txt")).unwrap();
        File::create(root.join("data/subdir/c.csv")).unwrap();

        // Create .dvs metadata folder with tracked files
        fs::create_dir_all(root.join(".dvs/data/subdir")).unwrap();
        File::create(root.join(".dvs/foo.txt.dvs")).unwrap();
        File::create(root.join(".dvs/data/a.csv.dvs")).unwrap();
        File::create(root.join(".dvs/data/subdir/c.csv.dvs")).unwrap();

        let dvs_paths = DvsPaths::new(root.to_path_buf(), root.to_path_buf(), ".dvs");
        (temp, dvs_paths)
    }

    #[test]
    fn add_explicit_file_ignores_glob() {
        let (_temp, dvs_paths) = setup_test_repo();
        let result =
            resolve_paths_for_add(vec![PathBuf::from("foo.txt")], Some("*.csv"), &dvs_paths)
                .unwrap();

        assert_eq!(result.len(), 1);
        assert!(result.contains(&PathBuf::from("foo.txt")));
    }

    #[test]
    fn add_directory_with_glob_filters() {
        let (_temp, dvs_paths) = setup_test_repo();
        let result =
            resolve_paths_for_add(vec![PathBuf::from("data")], Some("*.csv"), &dvs_paths).unwrap();

        assert!(result.contains(&PathBuf::from("data/a.csv")));
        assert!(!result.contains(&PathBuf::from("data/b.txt")));
        // *.csv should not match subdir/c.csv due to literal_separator
        assert!(!result.contains(&PathBuf::from("data/subdir/c.csv")));
    }

    #[test]
    fn add_directory_with_recursive_glob() {
        let (_temp, dvs_paths) = setup_test_repo();
        let result =
            resolve_paths_for_add(vec![PathBuf::from("data")], Some("**/*.csv"), &dvs_paths)
                .unwrap();

        assert!(result.contains(&PathBuf::from("data/a.csv")));
        assert!(result.contains(&PathBuf::from("data/subdir/c.csv")));
        assert!(!result.contains(&PathBuf::from("data/b.txt")));
    }

    #[test]
    fn add_path_not_found_errors() {
        let (_temp, dvs_paths) = setup_test_repo();
        let result = resolve_paths_for_add(vec![PathBuf::from("nonexistent")], None, &dvs_paths);

        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("Path not found"));
    }

    #[test]
    fn get_exact_file_match() {
        let (_temp, dvs_paths) = setup_test_repo();
        let result =
            resolve_paths_for_get(vec![PathBuf::from("foo.txt")], None, &dvs_paths).unwrap();

        assert_eq!(result.len(), 1);
        assert!(result.contains(&PathBuf::from("foo.txt")));
    }

    #[test]
    fn get_directory_returns_all_tracked() {
        let (_temp, dvs_paths) = setup_test_repo();
        let result = resolve_paths_for_get(vec![PathBuf::from("data")], None, &dvs_paths).unwrap();

        assert!(result.contains(&PathBuf::from("data/a.csv")));
        assert!(result.contains(&PathBuf::from("data/subdir/c.csv")));
        // b.txt is not tracked
        assert!(!result.contains(&PathBuf::from("data/b.txt")));
    }

    #[test]
    fn get_with_glob_filters() {
        let (_temp, dvs_paths) = setup_test_repo();
        // Empty paths defaults to cwd, then glob filters
        let result = resolve_paths_for_get(vec![], Some("*.txt"), &dvs_paths).unwrap();

        assert!(result.contains(&PathBuf::from("foo.txt")));
        assert!(!result.contains(&PathBuf::from("data/a.csv")));
    }

    // Do we want that behaviour?
    #[test]
    fn get_no_paths_defaults_to_cwd() {
        let (_temp, dvs_paths) = setup_test_repo();
        let result = resolve_paths_for_get(vec![], None, &dvs_paths).unwrap();

        // Should return all tracked files
        assert!(result.contains(&PathBuf::from("foo.txt")));
        assert!(result.contains(&PathBuf::from("data/a.csv")));
        assert!(result.contains(&PathBuf::from("data/subdir/c.csv")));
    }
}
