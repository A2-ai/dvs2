use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::{Path, PathBuf};

use anyhow::Result;
use fs_err as fs;

/// Adds entries to per-directory `.gitignore` files under `repo_root`.
///
/// Each path should be relative to the repo_root already. Files are grouped by
/// parent directory and a `/filename` entry is appended to the `.gitignore` in
/// that directory. Files at the repo root use the root `.gitignore`.
/// If no `.git` folder exists, this is a no-op.
pub(crate) fn add_to_gitignore(repo_root: &Path, paths: &[PathBuf]) -> Result<()> {
    if !repo_root.join(".git").exists() {
        return Ok(());
    }

    // Group paths by parent directory (empty path = repo root)
    let mut by_dir: HashMap<PathBuf, Vec<&Path>> = HashMap::new();
    for p in paths {
        let dir = p.parent().unwrap_or(Path::new("")).to_path_buf();
        by_dir.entry(dir).or_default().push(p);
    }

    for (dir, dir_paths) in &by_dir {
        let gitignore_path = repo_root.join(dir).join(".gitignore");

        let existing = if gitignore_path.is_file() {
            fs::read_to_string(&gitignore_path)?
        } else {
            String::new()
        };

        let existing_lines: HashSet<&str> = existing.lines().collect();

        let new_entries: Vec<String> = dir_paths
            .iter()
            .filter_map(|p| {
                let name = p.file_name()?;
                let entry = format!("/{}", name.to_string_lossy());
                if existing_lines.contains(entry.as_str()) {
                    None
                } else {
                    Some(entry)
                }
            })
            .collect();

        if new_entries.is_empty() {
            continue;
        }

        let mut content = existing;
        if !content.is_empty() && !content.ends_with('\n') {
            content.push('\n');
        }
        for entry in &new_entries {
            content.push_str(entry);
            content.push('\n');
        }

        // Ensure the directory exists (it may not for nested paths)
        if let Some(parent) = gitignore_path.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::write(&gitignore_path, content)?;
    }

    Ok(())
}

/// Drops the `!/x.csv.dvs` lines that kept the dvs1 metadata files `paths`
/// (relative to `repo_root`) out of their directory's `/x.csv` ignore, which
/// stays. Returns the `.gitignore` files that change, changing them unless
/// `dry_run`.
pub(crate) fn remove_dvs1_exceptions(
    repo_root: &Path,
    paths: &[PathBuf],
    dry_run: bool,
) -> Result<Vec<PathBuf>> {
    let mut by_dir: BTreeMap<PathBuf, HashSet<String>> = BTreeMap::new();
    for p in paths {
        let dir = p.parent().unwrap_or(Path::new("")).to_path_buf();
        if let Some(name) = p.file_name() {
            let line = format!("!/{}", name.to_string_lossy());
            by_dir.entry(dir).or_default().insert(line);
        }
    }

    let mut changed = Vec::new();
    for (dir, lines) in by_dir {
        let gitignore = dir.join(".gitignore");
        let gitignore_path = repo_root.join(&gitignore);
        if !gitignore_path.is_file() {
            continue;
        }
        let existing = fs::read_to_string(&gitignore_path)?;
        let kept: String = existing
            .split_inclusive('\n')
            .filter(|line| !lines.contains(line.trim_end()))
            .collect();
        if kept != existing {
            if !dry_run {
                fs::write(&gitignore_path, kept)?;
            }
            changed.push(gitignore);
        }
    }
    Ok(changed)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testutil::create_temp_git_repo;

    #[test]
    fn test_add_to_gitignore() {
        let (_tmp, root) = create_temp_git_repo();
        fs::write(root.join(".gitignore"), "*.log").unwrap();
        let paths = vec![PathBuf::from("data.csv"), PathBuf::from("models/big.bin")];
        add_to_gitignore(&root, &paths).unwrap();

        // Root file goes into root .gitignore
        let root_content = fs::read_to_string(root.join(".gitignore")).unwrap();
        assert_eq!(root_content, "*.log\n/data.csv\n");

        // Nested file goes into its parent directory's .gitignore
        let models_content = fs::read_to_string(root.join("models/.gitignore")).unwrap();
        assert_eq!(models_content, "/big.bin\n");

        // Calling again with the same paths should not duplicate entries
        add_to_gitignore(&root, &paths).unwrap();
        let root_content = fs::read_to_string(root.join(".gitignore")).unwrap();
        assert_eq!(root_content, "*.log\n/data.csv\n");
        let models_content = fs::read_to_string(root.join("models/.gitignore")).unwrap();
        assert_eq!(models_content, "/big.bin\n");
    }

    #[test]
    fn test_no_op_without_git_folder() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path();
        let paths = vec![PathBuf::from("data.csv")];
        add_to_gitignore(root, &paths).unwrap();
        assert!(!root.join(".gitignore").exists());
    }

    #[test]
    fn dvs1_exceptions_are_dropped_and_ignores_kept() {
        let (_tmp, root) = create_temp_git_repo();
        fs::write(
            root.join(".gitignore"),
            "*.log\n/x.csv\n!/x.csv.dvs\n!/y.csv.dvs",
        )
        .unwrap();
        fs::create_dir(root.join("sub")).unwrap();
        fs::write(root.join("sub/.gitignore"), "/z.csv\r\n!/z.csv.dvs\r\n").unwrap();
        let paths = [
            PathBuf::from("x.csv.dvs"),
            PathBuf::from("sub/z.csv.dvs"),
            PathBuf::from("none/w.csv.dvs"),
        ];

        let changed = remove_dvs1_exceptions(&root, &paths, true).unwrap();
        assert_eq!(
            changed,
            [PathBuf::from(".gitignore"), PathBuf::from("sub/.gitignore")]
        );
        let unchanged = fs::read_to_string(root.join(".gitignore")).unwrap();
        assert!(unchanged.contains("!/x.csv.dvs"));

        remove_dvs1_exceptions(&root, &paths, false).unwrap();
        let root_content = fs::read_to_string(root.join(".gitignore")).unwrap();
        assert_eq!(root_content, "*.log\n/x.csv\n!/y.csv.dvs");
        let sub_content = fs::read_to_string(root.join("sub/.gitignore")).unwrap();
        assert_eq!(sub_content, "/z.csv\r\n");
    }
}
