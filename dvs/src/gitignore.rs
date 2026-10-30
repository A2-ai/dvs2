use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

use anyhow::Result;
use fs_err as fs;

/// Adds entries to per-directory `.gitignore` files under `repo_root`.
///
/// Each path should be relative to the repo_root already. Files are grouped by
/// parent directory and a `/filename` entry is appended to the `.gitignore` in
/// that directory. Files at the repo root use the root `.gitignore`.
/// If neither `repo_root` nor a directory above it has a `.git`, this is a no-op.
pub(crate) fn add_to_gitignore(repo_root: &Path, paths: &[PathBuf]) -> Result<()> {
    if !in_git_work_tree(repo_root) {
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

/// Whether `dir` is in a git working tree: it or a directory above it has a
/// `.git` (a folder, or a file in a worktree or submodule). A project can sit
/// in a subfolder of the repository, as an RStudio project often does.
fn in_git_work_tree(dir: &Path) -> bool {
    dir.ancestors().any(|d| d.join(".git").exists())
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
    fn test_project_in_a_subfolder_of_the_repo() {
        let (_tmp, repo) = create_temp_git_repo();
        let project = repo.join("analysis");
        fs::create_dir(&project).unwrap();
        let paths = vec![PathBuf::from("data.csv"), PathBuf::from("raw/dm.csv")];
        add_to_gitignore(&project, &paths).unwrap();

        // Each entry goes next to its file, inside the project
        let root_content = fs::read_to_string(project.join(".gitignore")).unwrap();
        assert_eq!(root_content, "/data.csv\n");
        let raw_content = fs::read_to_string(project.join("raw/.gitignore")).unwrap();
        assert_eq!(raw_content, "/dm.csv\n");
        assert!(!repo.join(".gitignore").exists());
    }

    #[test]
    fn test_git_file_marks_a_worktree() {
        let tmp = tempfile::tempdir().unwrap();
        let root = fs::canonicalize(tmp.path()).unwrap();
        fs::write(root.join(".git"), "gitdir: /elsewhere/.git/worktrees/w\n").unwrap();
        add_to_gitignore(&root, &[PathBuf::from("data.csv")]).unwrap();
        assert_eq!(
            fs::read_to_string(root.join(".gitignore")).unwrap(),
            "/data.csv\n"
        );
    }

    #[test]
    fn test_no_op_without_git_folder() {
        let tmp = tempfile::tempdir().unwrap();
        let root = fs::canonicalize(tmp.path()).unwrap();
        // A temp dir inside some git repo cannot show the no-op
        if in_git_work_tree(&root) {
            return;
        }
        let paths = vec![PathBuf::from("data.csv")];
        add_to_gitignore(&root, &paths).unwrap();
        assert!(!root.join(".gitignore").exists());
    }
}
