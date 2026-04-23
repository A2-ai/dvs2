use std::collections::HashSet;
use std::ffi::OsStr;
use std::path::PathBuf;

use crate::paths::DvsPaths;
use anyhow::Result;
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
) -> (Vec<PathBuf>, Vec<crate::AddError>) {
    use crate::AddError;

    let mut out = HashSet::new();
    let mut errs = Vec::new();

    let glob_matcher = match build_glob_matcher(glob_pattern) {
        Ok(m) => m,
        Err(e) => {
            errs.push(AddError::GlobFailure {
                pattern: glob_pattern.unwrap_or_default().to_string(),
                reason: e.to_string(),
            });
            return (Vec::new(), errs);
        }
    };
    let repo_root = match dvs_paths.repo_root().canonicalize() {
        Ok(r) => r,
        Err(e) => {
            errs.push(AddError::PathResolution {
                path: dvs_paths.repo_root().to_path_buf(),
                reason: e.to_string(),
            });
            return (Vec::new(), errs);
        }
    };
    let metadata_root = match dvs_paths.metadata_folder().canonicalize() {
        Ok(r) => r,
        Err(e) => {
            errs.push(AddError::PathResolution {
                path: dvs_paths.metadata_folder().to_path_buf(),
                reason: e.to_string(),
            });
            return (Vec::new(), errs);
        }
    };

    // If no paths given, default to cwd
    let paths = if paths.is_empty() {
        vec![PathBuf::from(".")]
    } else {
        paths
    };

    for path in paths {
        let full_path = match dvs_paths.cwd().join(&path).canonicalize() {
            Ok(p) => p,
            Err(_) => {
                errs.push(AddError::NotFound { path: path.clone() });
                continue;
            }
        };

        // Explicit file: we ignore the glob and add it to the file
        if full_path.is_file() {
            let relative_to_root = match full_path.strip_prefix(&repo_root) {
                Ok(p) => p.to_path_buf(),
                // Outside repo: insert original user path; validate_for_add will catch it
                Err(_) => path.clone(),
            };
            out.insert(relative_to_root);
        } else if full_path.is_dir() {
            if let Some(matcher) = &glob_matcher {
                for entry in WalkDir::new(&full_path).into_iter().filter_map(|e| e.ok()) {
                    let entry_path = match entry.path().canonicalize() {
                        Ok(p) => p,
                        Err(_) => continue,
                    };
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
            }
        } else {
            errs.push(AddError::PathResolution {
                path: path.clone(),
                reason: "not a regular file or directory".to_string(),
            });
        }
    }

    let mut ok: Vec<PathBuf> = out.into_iter().collect();
    ok.sort();
    (ok, errs)
}

/// Resolve paths for `get` command by scanning tracked metadata:
/// - Explicit files or directories: filtered to tracked files under them
/// - Glob: applied to cwd-relative paths within matched files
/// - No paths + no glob: returns all tracked files under cwd
pub fn resolve_paths_for_get(
    paths: Vec<PathBuf>,
    glob_pattern: Option<&str>,
    dvs_paths: &DvsPaths,
) -> (Vec<PathBuf>, Vec<crate::GetError>) {
    use crate::GetError;

    let mut out = HashSet::new();
    let mut errs = Vec::new();

    let glob_matcher = match build_glob_matcher(glob_pattern) {
        Ok(m) => m,
        Err(e) => {
            errs.push(GetError::GlobFailure {
                pattern: glob_pattern.unwrap_or_default().to_string(),
                reason: e.to_string(),
            });
            return (Vec::new(), errs);
        }
    };
    let metadata_root = match dvs_paths.metadata_folder().canonicalize() {
        Ok(r) => r,
        Err(e) => {
            errs.push(GetError::MetadataRead {
                path: dvs_paths.metadata_folder().to_path_buf(),
                reason: e.to_string(),
            });
            return (Vec::new(), errs);
        }
    };
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
                if p.is_absolute() {
                    match p.strip_prefix(dvs_paths.repo_root()) {
                        Ok(r) => r.to_path_buf(),
                        Err(_) => p,
                    }
                } else if let Some(prefix) = cwd_prefix {
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
            .is_none_or(|g| g.is_match(&cwd_relative))
        {
            out.insert(tracked_path);
        }
    }

    let mut ok: Vec<PathBuf> = out.into_iter().collect();
    ok.sort();
    (ok, errs)
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

        let dvs_paths = DvsPaths::new(root.to_path_buf(), root.to_path_buf(), ".dvs").unwrap();
        (temp, dvs_paths)
    }

    #[test]
    fn add_explicit_file_ignores_glob() {
        let (_temp, dvs_paths) = setup_test_repo();
        let (ok, errs) =
            resolve_paths_for_add(vec![PathBuf::from("foo.txt")], Some("*.csv"), &dvs_paths);

        assert!(errs.is_empty());
        assert_eq!(ok.len(), 1);
        assert!(ok.contains(&PathBuf::from("foo.txt")));
    }

    #[test]
    fn add_directory_with_glob_filters() {
        let (_temp, dvs_paths) = setup_test_repo();
        let (ok, errs) =
            resolve_paths_for_add(vec![PathBuf::from("data")], Some("*.csv"), &dvs_paths);

        assert!(errs.is_empty());
        assert!(ok.contains(&PathBuf::from("data/a.csv")));
        assert!(!ok.contains(&PathBuf::from("data/b.txt")));
        // *.csv should not match subdir/c.csv due to literal_separator
        assert!(!ok.contains(&PathBuf::from("data/subdir/c.csv")));
    }

    #[test]
    fn add_directory_with_recursive_glob() {
        let (_temp, dvs_paths) = setup_test_repo();
        let (ok, errs) =
            resolve_paths_for_add(vec![PathBuf::from("data")], Some("**/*.csv"), &dvs_paths);

        assert!(errs.is_empty());
        assert!(ok.contains(&PathBuf::from("data/a.csv")));
        assert!(ok.contains(&PathBuf::from("data/subdir/c.csv")));
        assert!(!ok.contains(&PathBuf::from("data/b.txt")));
    }

    #[test]
    fn add_path_not_found_becomes_error_row() {
        let (_temp, dvs_paths) = setup_test_repo();
        let (ok, errs) = resolve_paths_for_add(vec![PathBuf::from("nonexistent")], None, &dvs_paths);

        assert!(ok.is_empty());
        assert_eq!(errs.len(), 1);
        assert!(matches!(errs[0], crate::AddError::NotFound { .. }));
    }

    #[test]
    fn add_mixed_inputs_collect_all_errors() {
        let (_temp, dvs_paths) = setup_test_repo();
        let (ok, errs) = resolve_paths_for_add(
            vec![
                PathBuf::from("foo.txt"),
                PathBuf::from("missing1.csv"),
                PathBuf::from("missing2.csv"),
            ],
            None,
            &dvs_paths,
        );

        assert_eq!(ok, vec![PathBuf::from("foo.txt")]);
        assert_eq!(errs.len(), 2);
        assert!(
            errs.iter()
                .all(|e| matches!(e, crate::AddError::NotFound { .. }))
        );
    }

    #[test]
    fn add_malformed_glob_becomes_error() {
        let (_temp, dvs_paths) = setup_test_repo();
        let (ok, errs) = resolve_paths_for_add(vec![], Some("[invalid"), &dvs_paths);

        assert!(ok.is_empty());
        assert_eq!(errs.len(), 1);
        assert!(matches!(errs[0], crate::AddError::GlobFailure { .. }));
    }

    #[test]
    fn get_exact_file_match() {
        let (_temp, dvs_paths) = setup_test_repo();
        let (ok, errs) = resolve_paths_for_get(vec![PathBuf::from("foo.txt")], None, &dvs_paths);

        assert!(errs.is_empty());
        assert_eq!(ok.len(), 1);
        assert!(ok.contains(&PathBuf::from("foo.txt")));
    }

    #[test]
    fn get_directory_returns_all_tracked() {
        let (_temp, dvs_paths) = setup_test_repo();
        let (ok, errs) = resolve_paths_for_get(vec![PathBuf::from("data")], None, &dvs_paths);

        assert!(errs.is_empty());
        assert!(ok.contains(&PathBuf::from("data/a.csv")));
        assert!(ok.contains(&PathBuf::from("data/subdir/c.csv")));
        // b.txt is not tracked
        assert!(!ok.contains(&PathBuf::from("data/b.txt")));
    }

    #[test]
    fn get_with_glob_filters() {
        let (_temp, dvs_paths) = setup_test_repo();
        // Empty paths defaults to cwd, then glob filters
        let (ok, errs) = resolve_paths_for_get(vec![], Some("*.txt"), &dvs_paths);

        assert!(errs.is_empty());
        assert!(ok.contains(&PathBuf::from("foo.txt")));
        assert!(!ok.contains(&PathBuf::from("data/a.csv")));
    }

    // Do we want that behaviour?
    #[test]
    fn get_no_paths_defaults_to_cwd() {
        let (_temp, dvs_paths) = setup_test_repo();
        let (ok, errs) = resolve_paths_for_get(vec![], None, &dvs_paths);

        // Should return all tracked files
        assert!(errs.is_empty());
        assert!(ok.contains(&PathBuf::from("foo.txt")));
        assert!(ok.contains(&PathBuf::from("data/a.csv")));
        assert!(ok.contains(&PathBuf::from("data/subdir/c.csv")));
    }

    #[test]
    fn get_absolute_file_path() {
        let (temp, dvs_paths) = setup_test_repo();
        let abs_path = temp.path().canonicalize().unwrap().join("foo.txt");
        let (ok, errs) = resolve_paths_for_get(vec![abs_path], None, &dvs_paths);

        assert!(errs.is_empty());
        assert_eq!(ok.len(), 1);
        assert!(ok.contains(&PathBuf::from("foo.txt")));
    }

    #[test]
    fn get_absolute_directory_path() {
        let (temp, dvs_paths) = setup_test_repo();
        let abs_path = temp.path().canonicalize().unwrap().join("data");
        let (ok, errs) = resolve_paths_for_get(vec![abs_path], None, &dvs_paths);

        assert!(errs.is_empty());
        assert!(ok.contains(&PathBuf::from("data/a.csv")));
        assert!(ok.contains(&PathBuf::from("data/subdir/c.csv")));
        assert!(!ok.contains(&PathBuf::from("foo.txt")));
    }

    #[test]
    fn add_absolute_file_path() {
        let (temp, dvs_paths) = setup_test_repo();
        let abs_path = temp.path().canonicalize().unwrap().join("foo.txt");
        let (ok, errs) = resolve_paths_for_add(vec![abs_path], None, &dvs_paths);

        assert!(errs.is_empty());
        assert_eq!(ok.len(), 1);
        assert!(ok.contains(&PathBuf::from("foo.txt")));
    }
}
