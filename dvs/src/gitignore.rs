use std::collections::HashSet;
use std::path::{Path, PathBuf};

use anyhow::Result;
use fs_err as fs;

/// Adds entries to `.gitignore` in the `repo_root` (already canonicalized)
///
/// Each path should be relative to the repo_root already
/// If no `.git` folder exists, this is a no-op.
pub(crate) fn add_to_gitignore(repo_root: &Path, paths: &[PathBuf]) -> Result<()> {
    if !repo_root.join(".git").exists() {
        return Ok(());
    }

    let gitignore_path = repo_root.join(".gitignore");
    let existing = if gitignore_path.is_file() {
        fs::read_to_string(&gitignore_path)?
    } else {
        String::new()
    };

    let existing_lines: HashSet<&str> = existing.lines().collect();

    let new_entries: Vec<String> = paths
        .iter()
        .map(|p| format!("/{}", p.display()))
        .filter(|entry| !existing_lines.contains(entry.as_str()))
        .collect();

    if new_entries.is_empty() {
        return Ok(());
    }

    let mut content = existing;
    // Ensure trailing newline before appending
    if !content.is_empty() && !content.ends_with('\n') {
        content.push('\n');
    }
    for entry in &new_entries {
        content.push_str(entry);
        content.push('\n');
    }

    fs::write(&gitignore_path, content)?;
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
        let content = fs::read_to_string(root.join(".gitignore")).unwrap();
        assert_eq!(content, "*.log\n/data.csv\n/models/big.bin\n");

        // Calling again with the same paths should not duplicate entries
        add_to_gitignore(&root, &paths).unwrap();
        let content = fs::read_to_string(root.join(".gitignore")).unwrap();
        assert_eq!(content, "*.log\n/data.csv\n/models/big.bin\n");
    }

    #[test]
    fn test_no_op_without_git_folder() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path();
        let paths = vec![PathBuf::from("data.csv")];
        add_to_gitignore(root, &paths).unwrap();
        assert!(!root.join(".gitignore").exists());
    }
}
