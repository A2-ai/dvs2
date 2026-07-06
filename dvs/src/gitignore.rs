use std::collections::{HashMap, HashSet};
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

/// Removes entries from per-directory `.gitignore` files under `repo_root`.
///
/// This is the inverse of [`add_to_gitignore`]. Each path should be relative to
/// the repo_root already. Files are grouped by parent directory and the
/// `/filename` entry is dropped from the `.gitignore` in that directory. Other
/// lines are left untouched. A `.gitignore` that becomes empty is deleted so no
/// empty file is left behind. Entries that are not present are ignored. If no
/// `.git` folder exists, this is a no-op.
pub(crate) fn remove_from_gitignore(repo_root: &Path, paths: &[PathBuf]) -> Result<()> {
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
        if !gitignore_path.is_file() {
            continue;
        }

        let existing = fs::read_to_string(&gitignore_path)?;

        let to_remove: HashSet<String> = dir_paths
            .iter()
            .filter_map(|p| p.file_name())
            .map(|name| format!("/{}", name.to_string_lossy()))
            .collect();

        let original_line_count = existing.lines().count();
        let kept: Vec<&str> = existing
            .lines()
            .filter(|line| !to_remove.contains(*line))
            .collect();

        // None of our entries were present: leave the file untouched.
        if kept.len() == original_line_count {
            continue;
        }

        if kept.is_empty() {
            fs::remove_file(&gitignore_path)?;
        } else {
            let mut content = kept.join("\n");
            content.push('\n');
            fs::write(&gitignore_path, content)?;
        }
    }

    Ok(())
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
    fn test_remove_from_gitignore() {
        let (_tmp, root) = create_temp_git_repo();
        fs::write(root.join(".gitignore"), "*.log").unwrap();
        let paths = vec![PathBuf::from("data.csv"), PathBuf::from("models/big.bin")];
        add_to_gitignore(&root, &paths).unwrap();

        // Removing one entry keeps the unrelated line and drops only /data.csv.
        remove_from_gitignore(&root, &[PathBuf::from("data.csv")]).unwrap();
        let root_content = fs::read_to_string(root.join(".gitignore")).unwrap();
        assert_eq!(root_content, "*.log\n");

        // The nested .gitignore held only the dvs entry, so removing it deletes
        // the now-empty file.
        remove_from_gitignore(&root, &[PathBuf::from("models/big.bin")]).unwrap();
        assert!(!root.join("models/.gitignore").exists());
    }

    #[test]
    fn test_remove_missing_entry_is_noop() {
        let (_tmp, root) = create_temp_git_repo();
        fs::write(root.join(".gitignore"), "*.log\n/kept.csv\n").unwrap();

        // A path never added leaves the file untouched.
        remove_from_gitignore(&root, &[PathBuf::from("never.csv")]).unwrap();
        let content = fs::read_to_string(root.join(".gitignore")).unwrap();
        assert_eq!(content, "*.log\n/kept.csv\n");
    }

    #[test]
    fn test_remove_no_op_without_git_folder() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path();
        fs::write(root.join(".gitignore"), "/data.csv\n").unwrap();
        remove_from_gitignore(root, &[PathBuf::from("data.csv")]).unwrap();
        // Without a .git folder the update is skipped, so the entry stays.
        let content = fs::read_to_string(root.join(".gitignore")).unwrap();
        assert_eq!(content, "/data.csv\n");
    }
}
