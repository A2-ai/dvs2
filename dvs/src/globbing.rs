use std::collections::HashSet;
use std::path::PathBuf;

use crate::paths::{DvsPaths, PathFilter};
use anyhow::{Result, anyhow, bail};
use globset::{GlobBuilder, GlobMatcher};
use walkdir::WalkDir;

/// Builds the glob matching the rg behaviour
/// eg "*.csv" will not match `some/dir/test.csv`
pub(crate) fn build_glob_matcher(pattern: Option<&str>) -> Result<Option<GlobMatcher>> {
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
            let relative_to_root = match full_path.strip_prefix(&repo_root) {
                Ok(p) => p.to_path_buf(),
                // Outside repo: insert original user path; validate_for_add will catch it
                Err(_) => path.clone(),
            };
            out.insert(relative_to_root);
        } else if full_path.is_dir() {
            if !full_path.starts_with(&repo_root) {
                // A dir without a glob: it will be rejected later
                out.insert(path.clone());
            } else if let Some(matcher) = &glob_matcher {
                for entry in WalkDir::new(&full_path).into_iter().filter_map(|e| e.ok()) {
                    let entry_path = entry.path().canonicalize()?;
                    // Skip directories and metadata root folder
                    if !entry_path.is_file() || entry_path.starts_with(&metadata_root) {
                        continue;
                    }

                    // Get path relative to the walked directory for matching
                    let relative_to_dir = match entry_path.strip_prefix(&full_path) {
                        Ok(p) => p,
                        Err(_) => {
                            // A symlink outside of the repo from a glob is a warning only
                            if !entry_path.starts_with(&repo_root) {
                                log::warn!(
                                    "Skipping {}: symlink resolves outside the project root ({})",
                                    entry.path().display(),
                                    entry_path.display()
                                );
                            }
                            continue;
                        }
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
                let relative_to_root = match full_path.strip_prefix(&repo_root) {
                    Ok(p) => p.to_path_buf(),
                    Err(_) => path.clone(),
                };
                out.insert(relative_to_root);
            }
        } else {
            bail!("Path is not a file or directory: {}", path.display());
        }
    }

    Ok(out)
}

/// Resolve paths for `get` command by scanning tracked metadata:
/// - Explicit files or directories: filtered to tracked files under them
/// - Glob: matched relative to each path argument, or relative to cwd when no paths are given
/// - No paths + no glob: returns all tracked files directly under cwd
pub fn resolve_paths_for_get(
    paths: Vec<PathBuf>,
    glob_pattern: Option<&str>,
    dvs_paths: &DvsPaths,
    recursive: bool,
) -> Result<HashSet<PathBuf>> {
    let glob_matcher = build_glob_matcher(glob_pattern)?;
    let tracked = dvs_paths.tracked_paths();

    // No explicit paths: scope to cwd and return whatever matches.
    if paths.is_empty() {
        let filter = PathFilter::cwd_scoped(recursive, dvs_paths);
        return Ok(tracked
            .into_iter()
            .filter(|t| filter.matches(t, glob_matcher.as_ref()))
            .collect());
    }

    // Explicit paths: every one must resolve to at least one tracked file,
    // otherwise we refuse the whole batch rather than silently skipping it.
    let mut out = HashSet::new();
    let mut missing = Vec::new();
    for path in paths {
        let filter = PathFilter::from_user_paths(vec![path.clone()], recursive, dvs_paths);
        let mut matched_any = false;
        for tracked_path in &tracked {
            if filter.matches(tracked_path, glob_matcher.as_ref()) {
                out.insert(tracked_path.clone());
                matched_any = true;
            }
        }
        if !matched_any {
            missing.push(format!("  {}", path.display()));
        }
    }
    if !missing.is_empty() {
        bail!(
            "The following paths are not tracked by DVS:\n{}",
            missing.join("\n")
        );
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

        let dvs_paths = DvsPaths::new(root.to_path_buf(), root.to_path_buf(), ".dvs").unwrap();
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
    fn add_directory_without_glob() {
        let (_temp, dvs_paths) = setup_test_repo();
        let result = resolve_paths_for_add(
            vec![PathBuf::from("data"), PathBuf::from("foo.txt")],
            None,
            &dvs_paths,
        )
        .unwrap();

        // We keep it, it will be rejected later down the line since it's a dir
        assert!(result.contains(&PathBuf::from("data")));
        assert!(result.contains(&PathBuf::from("foo.txt")));
    }

    #[test]
    fn add_path_not_found_errors() {
        let (_temp, dvs_paths) = setup_test_repo();
        let result = resolve_paths_for_add(vec![PathBuf::from("nonexistent")], None, &dvs_paths);

        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("Path not found"));
    }

    #[test]
    #[cfg(unix)]
    fn add_walk_skips_symlink_resolving_outside_repo() {
        use std::os::unix::fs::symlink;

        let (_temp, dvs_paths) = setup_test_repo();
        let outside = TempDir::new().unwrap();
        let outside_file = outside.path().join("secret.csv");
        File::create(&outside_file).unwrap();
        // A symlink inside data/ that points outside the repo.
        symlink(&outside_file, dvs_paths.repo_root().join("data/link.csv")).unwrap();

        let result =
            resolve_paths_for_add(vec![PathBuf::from("data")], Some("*.csv"), &dvs_paths).unwrap();

        assert!(result.contains(&PathBuf::from("data/a.csv")));
        assert!(
            !result
                .iter()
                .any(|p| p.ends_with("link.csv") || p.ends_with("secret.csv")),
        );
    }

    #[test]
    fn get_exact_file_match() {
        let (_temp, dvs_paths) = setup_test_repo();
        let result =
            resolve_paths_for_get(vec![PathBuf::from("foo.txt")], None, &dvs_paths, false).unwrap();

        assert_eq!(result.len(), 1);
        assert!(result.contains(&PathBuf::from("foo.txt")));
    }

    #[test]
    fn get_untracked_explicit_path_errors() {
        let (_temp, dvs_paths) = setup_test_repo();
        // bar.csv exists on disk but is not tracked; an explicit untracked path
        // must error rather than be silently dropped.
        let result = resolve_paths_for_get(
            vec![PathBuf::from("foo.txt"), PathBuf::from("bar.csv")],
            None,
            &dvs_paths,
            false,
        );

        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(err.contains("not tracked"), "unexpected error: {err}");
        assert!(err.contains("bar.csv"), "unexpected error: {err}");
    }

    #[test]
    fn get_explicit_file_ignores_glob() {
        let (_temp, dvs_paths) = setup_test_repo();
        let result = resolve_paths_for_get(
            vec![PathBuf::from("foo.txt")],
            Some("*.csv"),
            &dvs_paths,
            false,
        )
        .unwrap();

        assert_eq!(result.len(), 1);
        assert!(result.contains(&PathBuf::from("foo.txt")));
    }

    #[test]
    fn get_directory_recursive_returns_all_tracked() {
        let (_temp, dvs_paths) = setup_test_repo();
        let result =
            resolve_paths_for_get(vec![PathBuf::from("data")], None, &dvs_paths, true).unwrap();

        assert!(result.contains(&PathBuf::from("data/a.csv")));
        assert!(result.contains(&PathBuf::from("data/subdir/c.csv")));
        // b.txt is not tracked
        assert!(!result.contains(&PathBuf::from("data/b.txt")));
    }

    #[test]
    fn get_directory_non_recursive_excludes_subdirs() {
        let (_temp, dvs_paths) = setup_test_repo();
        let result =
            resolve_paths_for_get(vec![PathBuf::from("data")], None, &dvs_paths, false).unwrap();

        assert!(result.contains(&PathBuf::from("data/a.csv")));
        assert!(!result.contains(&PathBuf::from("data/subdir/c.csv")));
        assert!(!result.contains(&PathBuf::from("data/b.txt")));
    }

    #[test]
    fn get_with_glob_filters() {
        let (_temp, dvs_paths) = setup_test_repo();
        // Empty paths defaults to cwd, then glob filters
        let result = resolve_paths_for_get(vec![], Some("*.txt"), &dvs_paths, false).unwrap();

        assert!(result.contains(&PathBuf::from("foo.txt")));
        assert!(!result.contains(&PathBuf::from("data/a.csv")));
    }

    #[test]
    fn get_no_paths_non_recursive_returns_direct_children() {
        let (_temp, dvs_paths) = setup_test_repo();
        // No explicit paths scopes to cwd (the repo root here). Without
        // `recursive`, only files directly under cwd are returned.
        let result = resolve_paths_for_get(vec![], None, &dvs_paths, false).unwrap();
        assert!(result.contains(&PathBuf::from("foo.txt")));
        assert!(!result.contains(&PathBuf::from("data/a.csv")));
        assert!(!result.contains(&PathBuf::from("data/subdir/c.csv")));
    }

    #[test]
    fn get_no_paths_recursive_returns_all_under_cwd() {
        let (_temp, dvs_paths) = setup_test_repo();
        let result = resolve_paths_for_get(vec![], None, &dvs_paths, true).unwrap();
        assert!(result.contains(&PathBuf::from("foo.txt")));
        assert!(result.contains(&PathBuf::from("data/a.csv")));
        assert!(result.contains(&PathBuf::from("data/subdir/c.csv")));
    }

    #[test]
    fn get_dot_recursive_returns_all_tracked() {
        let (_temp, dvs_paths) = setup_test_repo();
        // `dvs get . --recursive` should match every tracked path: normalize_path
        // strips the CurDir component, leaving an empty PathBuf that
        // Path::starts_with treats as a prefix of any path.
        let result =
            resolve_paths_for_get(vec![PathBuf::from(".")], None, &dvs_paths, true).unwrap();
        assert!(result.contains(&PathBuf::from("foo.txt")));
        assert!(result.contains(&PathBuf::from("data/a.csv")));
        assert!(result.contains(&PathBuf::from("data/subdir/c.csv")));
    }

    #[test]
    fn get_absolute_file_path() {
        let (temp, dvs_paths) = setup_test_repo();
        let abs_path = temp.path().canonicalize().unwrap().join("foo.txt");
        let result = resolve_paths_for_get(vec![abs_path], None, &dvs_paths, false).unwrap();

        assert_eq!(result.len(), 1);
        assert!(result.contains(&PathBuf::from("foo.txt")));
    }

    #[test]
    fn get_absolute_directory_path() {
        let (temp, dvs_paths) = setup_test_repo();
        let abs_path = temp.path().canonicalize().unwrap().join("data");
        let result = resolve_paths_for_get(vec![abs_path], None, &dvs_paths, true).unwrap();

        assert!(result.contains(&PathBuf::from("data/a.csv")));
        assert!(result.contains(&PathBuf::from("data/subdir/c.csv")));
        assert!(!result.contains(&PathBuf::from("foo.txt")));
    }

    #[test]
    fn get_absolute_directory_path_non_recursive() {
        let (temp, dvs_paths) = setup_test_repo();
        let abs_path = temp.path().canonicalize().unwrap().join("data");
        let result = resolve_paths_for_get(vec![abs_path], None, &dvs_paths, false).unwrap();

        assert!(result.contains(&PathBuf::from("data/a.csv")));
        assert!(!result.contains(&PathBuf::from("data/subdir/c.csv")));
        assert!(!result.contains(&PathBuf::from("foo.txt")));
    }

    #[test]
    fn get_directory_glob_is_dir_relative() {
        let (_temp, dvs_paths) = setup_test_repo();
        let result = resolve_paths_for_get(
            vec![PathBuf::from("data")],
            Some("*.csv"),
            &dvs_paths,
            false,
        )
        .unwrap();

        assert!(result.contains(&PathBuf::from("data/a.csv")));
        assert!(!result.contains(&PathBuf::from("data/subdir/c.csv")));
        assert!(!result.contains(&PathBuf::from("foo.txt")));
    }

    #[test]
    fn get_directory_recursive_glob_reaches_subdirs() {
        let (_temp, dvs_paths) = setup_test_repo();
        let result = resolve_paths_for_get(
            vec![PathBuf::from("data")],
            Some("**/*.csv"),
            &dvs_paths,
            false,
        )
        .unwrap();

        assert!(result.contains(&PathBuf::from("data/a.csv")));
        assert!(result.contains(&PathBuf::from("data/subdir/c.csv")));
        assert!(!result.contains(&PathBuf::from("foo.txt")));
    }

    #[test]
    fn add_absolute_file_path() {
        let (temp, dvs_paths) = setup_test_repo();
        let abs_path = temp.path().canonicalize().unwrap().join("foo.txt");
        let result = resolve_paths_for_add(vec![abs_path], None, &dvs_paths).unwrap();

        assert_eq!(result.len(), 1);
        assert!(result.contains(&PathBuf::from("foo.txt")));
    }
}
