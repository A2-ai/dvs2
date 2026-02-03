use std::path::{Path, PathBuf};

use crate::config::Config;
use anyhow::{Result, anyhow};
use fs_err as fs;

pub const CONFIG_FILE_NAME: &str = "dvs.toml";
pub const DEFAULT_FOLDER_NAME: &str = ".dvs";

/// Finds the root of a git repository by walking up from the given directory
/// until a `.git` folder or `dvs.toml` is found
/// TODO: add more heuristics than .git
///
/// Returns `None` if no `.git` folder is found before reaching the filesystem root.
pub fn find_repo_root(start_dir: impl AsRef<Path>) -> Option<PathBuf> {
    let mut dir = start_dir.as_ref();
    log::debug!("Searching for repo root starting from {}", dir.display());

    loop {
        if dir.join(".git").exists() || dir.join(CONFIG_FILE_NAME).exists() {
            log::debug!("Found repo root at {}", dir.display());
            return Some(dir.to_path_buf());
        }

        dir = dir.parent()?;
    }
}

/// We always need to figure out where the user is in a project,
/// where the root is etc.
/// This struct handles all of it so the rest of the code doesn't have to
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DvsPaths {
    /// Canonicalized path of where the user currently is
    cwd: PathBuf,
    /// Canonicalized path where our `dvs.toml` is
    repo_root: PathBuf,
    /// Folder name for metadata, defined in the config
    metadata_folder_name: String,
}

impl DvsPaths {
    /// Create with explicit paths (for testing or R package)
    pub fn new(cwd: PathBuf, repo_root: PathBuf, metadata_folder_name: impl Into<String>) -> Self {
        Self {
            cwd,
            repo_root,
            metadata_folder_name: metadata_folder_name.into(),
        }
    }

    pub fn from_cwd(config: &Config) -> Result<Self> {
        let cwd = fs::canonicalize(std::env::current_dir()?)?;
        let repo_root = fs::canonicalize(
            find_repo_root(&cwd).ok_or_else(|| anyhow!("Not in a git repository"))?,
        )?;

        log::debug!(
            "Resolved paths: cwd={}, repo_root={}",
            cwd.display(),
            repo_root.display()
        );
        Ok(Self {
            cwd,
            repo_root,
            metadata_folder_name: config.metadata_folder_name().to_owned(),
        })
    }

    pub fn metadata_folder(&self) -> PathBuf {
        self.repo_root.join(&self.metadata_folder_name)
    }

    pub fn metadata_path(&self, relative: &Path) -> PathBuf {
        let dvs_path = self.metadata_folder().join(relative);
        let mut s = dvs_path.into_os_string();
        s.push(".dvs");
        PathBuf::from(s)
    }

    pub fn repo_root(&self) -> &Path {
        &self.repo_root
    }

    pub fn cwd(&self) -> &Path {
        &self.cwd
    }

    /// Get the path relative from repo root to cwd, or None if at repo root
    pub fn cwd_relative_to_root(&self) -> Option<&Path> {
        self.cwd
            .strip_prefix(&self.repo_root)
            .ok()
            .filter(|p| !p.as_os_str().is_empty())
    }

    /// Construct the full file path from a repo-relative path
    pub fn file_path(&self, relative: &Path) -> PathBuf {
        self.repo_root.join(relative)
    }

    pub fn validate_for_add(&self, paths: &[PathBuf]) -> Vec<(PathBuf, bool)> {
        let mut found = Vec::new();
        for path in paths {
            // For add: check if file exists on disk
            let file_path = self.file_path(path);
            let exists = file_path.is_file();
            found.push((path.clone(), exists));
        }
        found
    }

    pub fn validate_for_get(&self, paths: &[PathBuf]) -> Vec<(PathBuf, bool)> {
        let mut found = Vec::new();
        for path in paths {
            // For get: check if file is tracked (metadata exists)
            let metadata_path = self.metadata_path(path);
            let exists = metadata_path.is_file();
            found.push((path.clone(), exists));
        }
        found
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testutil::create_temp_git_repo;

    #[test]
    fn find_repo_root_at_root() {
        let (_tmp, root) = create_temp_git_repo();
        assert_eq!(find_repo_root(&root), Some(root));
    }

    #[test]
    fn find_repo_root_from_subdirectory() {
        let (_tmp, root) = create_temp_git_repo();
        let subdir = root.join("a/b/c");
        fs::create_dir_all(&subdir).unwrap();
        assert_eq!(find_repo_root(&subdir), Some(root));
    }

    #[test]
    fn find_repo_root_returns_none_without_git() {
        let tmp = tempfile::tempdir().unwrap();
        assert_eq!(find_repo_root(tmp.path()), None);
    }

    #[test]
    fn metadata_path_returns_dvs_file_path() {
        let (_tmp, root) = create_temp_git_repo();
        let paths = DvsPaths::new(root.clone(), root.clone(), ".meta");

        let result = paths.metadata_path(Path::new("sub/file.txt"));
        assert_eq!(result, root.join(".meta/sub/file.txt.dvs"));
    }
}
