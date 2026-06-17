use std::collections::HashSet;
use std::path::{Path, PathBuf};

use crate::paths::{DvsPaths, PathFilter, normalize_path};
use anyhow::Result;
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
///
/// An explicitly provided path that resolves to no files (missing on disk, a
/// bare directory, or a directory whose glob matches nothing) is carried forward
/// as a repo-root-relative path so `add_files` reports it per-path (NotFound /
/// IsDirectory) and the command exits non-zero. The valid paths are still added.
pub fn resolve_paths_for_add(
    paths: Vec<PathBuf>,
    glob_pattern: Option<&str>,
    dvs_paths: &DvsPaths,
) -> Result<HashSet<PathBuf>> {
    let mut out = HashSet::new();
    let glob_matcher = build_glob_matcher(glob_pattern)?;
    let repo_root = dvs_paths.repo_root().canonicalize()?;
    let metadata_root = dvs_paths.metadata_folder().canonicalize()?;

    // No paths given defaults to cwd; that synthesized path is not carried forward.
    let explicit = !paths.is_empty();
    let paths = if paths.is_empty() {
        vec![PathBuf::from(".")]
    } else {
        paths
    };

    for path in paths {
        let before = out.len();

        if let Ok(full_path) = dvs_paths.cwd().join(&path).canonicalize() {
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
                        let Ok(entry_path) = entry.path().canonicalize() else {
                            continue;
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
            }
        }

        if explicit && out.len() == before {
            out.insert(carry_forward_path(&path, dvs_paths));
        }
    }

    Ok(out)
}

/// Translate a cwd-relative user path to a normalized repo-root-relative path so
/// it can be carried into `validate_for_add` / `validate_for_get` for reporting.
fn carry_forward_path(path: &Path, dvs_paths: &DvsPaths) -> PathBuf {
    let joined = match dvs_paths.cwd_relative_to_root() {
        Some(prefix) => prefix.join(path),
        None => path.to_path_buf(),
    };
    normalize_path(joined).unwrap_or_else(|| path.to_path_buf())
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
    let mut out = HashSet::new();
    let glob_matcher = build_glob_matcher(glob_pattern)?;

    // Explicit args that match no tracked file are invalid: carry them forward so
    // get_files reports them per-path (NotTracked / NotFound) and the exit is
    // non-zero. Computed before `paths` is moved into the filter.
    let unmatched = unmatched_tracked_args(&paths, glob_pattern, recursive, dvs_paths)?;

    let filter = if paths.is_empty() {
        PathFilter::cwd_scoped(recursive, dvs_paths)
    } else {
        PathFilter::from_user_paths(paths, recursive, dvs_paths)
    };

    for tracked_path in dvs_paths.tracked_paths() {
        if filter.matches(&tracked_path, glob_matcher.as_ref()) {
            out.insert(tracked_path);
        }
    }

    out.extend(unmatched);
    Ok(out)
}

/// For each explicitly provided user path, return the ones that match no tracked
/// file (after glob/recursive filtering), normalized to repo-root-relative form.
/// Empty `user_paths` (no explicit args, e.g. glob-only or cwd default) returns
/// empty. Shared by `get` (carried forward) and `status` (reported as errors) so
/// an invalid argument yields a non-zero exit without aborting the valid work.
pub fn unmatched_tracked_args(
    user_paths: &[PathBuf],
    glob_pattern: Option<&str>,
    recursive: bool,
    dvs_paths: &DvsPaths,
) -> Result<Vec<PathBuf>> {
    if user_paths.is_empty() {
        return Ok(Vec::new());
    }
    let glob_matcher = build_glob_matcher(glob_pattern)?;
    let tracked = dvs_paths.tracked_paths();

    let mut unmatched = Vec::new();
    for path in user_paths {
        let filter = PathFilter::from_user_paths(vec![path.clone()], recursive, dvs_paths);
        let matched = tracked
            .iter()
            .any(|t| filter.matches(t, glob_matcher.as_ref()));
        if !matched {
            if filter.paths.is_empty() {
                unmatched.push(path.clone());
            } else {
                unmatched.extend(filter.paths);
            }
        }
    }
    Ok(unmatched)
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
    fn add_missing_path_is_carried_forward() {
        let (_temp, dvs_paths) = setup_test_repo();
        // A missing path is no longer a hard error: it is carried forward so
        // add_files reports it per-path (NotFound) and the command exits non-zero.
        let result =
            resolve_paths_for_add(vec![PathBuf::from("nonexistent")], None, &dvs_paths).unwrap();
        assert!(result.contains(&PathBuf::from("nonexistent")));
    }

    #[test]
    fn add_valid_plus_missing_keeps_valid_and_carries_missing() {
        let (_temp, dvs_paths) = setup_test_repo();
        let result = resolve_paths_for_add(
            vec![PathBuf::from("foo.txt"), PathBuf::from("nonexistent")],
            None,
            &dvs_paths,
        )
        .unwrap();
        assert!(result.contains(&PathBuf::from("foo.txt")));
        assert!(result.contains(&PathBuf::from("nonexistent")));
    }

    #[test]
    fn add_bare_directory_is_carried_forward() {
        let (_temp, dvs_paths) = setup_test_repo();
        // Bare directory (no glob) resolves to no files; carried forward so
        // add_files reports it as IsDirectory rather than silently dropping it.
        let result = resolve_paths_for_add(vec![PathBuf::from("data")], None, &dvs_paths).unwrap();
        assert!(result.contains(&PathBuf::from("data")));
    }

    #[test]
    fn get_untracked_explicit_path_is_carried_forward() {
        let (_temp, dvs_paths) = setup_test_repo();
        // bar.csv exists on disk but is not tracked; an explicit untracked path
        // is carried forward so get_files reports it (NotTracked) and exits 1.
        let result = resolve_paths_for_get(
            vec![PathBuf::from("foo.txt"), PathBuf::from("bar.csv")],
            None,
            &dvs_paths,
            false,
        )
        .unwrap();
        assert!(result.contains(&PathBuf::from("foo.txt")));
        assert!(result.contains(&PathBuf::from("bar.csv")));
    }

    #[test]
    fn unmatched_args_flags_only_unmatched() {
        let (_temp, dvs_paths) = setup_test_repo();
        let unmatched = unmatched_tracked_args(
            &[PathBuf::from("foo.txt"), PathBuf::from("bar.csv")],
            None,
            false,
            &dvs_paths,
        )
        .unwrap();
        // foo.txt is tracked (not flagged); bar.csv is untracked (flagged).
        assert_eq!(unmatched, vec![PathBuf::from("bar.csv")]);
    }

    #[test]
    fn unmatched_args_empty_for_no_explicit_paths() {
        let (_temp, dvs_paths) = setup_test_repo();
        let unmatched = unmatched_tracked_args(&[], Some("*.zzz"), false, &dvs_paths).unwrap();
        assert!(unmatched.is_empty());
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
