use std::path::{Path, PathBuf};

use crate::config::Config;
use anyhow::Result;
use fs_err as fs;

pub const CONFIG_FILE_NAME: &str = "dvs.toml";
pub const DEFAULT_FOLDER_NAME: &str = ".dvs";

/// Result of validating a file path for `get`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GetPathStatus {
    /// Metadata exists: file is tracked
    Tracked,
    /// No metadata and no file on disk
    NotFound,
    /// File exists on disk but is not tracked by DVS
    NotTracked,
}

/// Finds the root of a project by walking up from the given directory
/// until a `dvs.toml` is found
///
/// Returns the `start_dir` if no `dvs.toml` has been found.
pub fn find_repo_root(start_dir: impl AsRef<Path>) -> PathBuf {
    let mut dir = start_dir.as_ref();
    log::debug!("Searching for repo root starting from {}", dir.display());

    loop {
        if dir.join(CONFIG_FILE_NAME).exists() {
            log::debug!("Found repo root at {}", dir.display());
            return dir.to_path_buf();
        }

        if let Some(parent) = dir.parent() {
            dir = parent;
        } else {
            break;
        }
    }

    start_dir.as_ref().to_path_buf()
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
        let repo_root = fs::canonicalize(find_repo_root(&cwd))?;

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

    pub fn cache_folder(&self) -> PathBuf {
        self.metadata_folder().join(".cache")
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

    pub fn validate_for_get(&self, paths: &[PathBuf]) -> Vec<(PathBuf, GetPathStatus)> {
        let mut found = Vec::new();
        for path in paths {
            let metadata_path = self.metadata_path(path);
            let validation = if metadata_path.is_file() {
                GetPathStatus::Tracked
            } else if self.file_path(path).is_file() {
                GetPathStatus::NotTracked
            } else {
                GetPathStatus::NotFound
            };
            found.push((path.clone(), validation));
        }
        found
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testutil::create_temp_git_repo;

    #[test]
    fn metadata_path_returns_dvs_file_path() {
        let (_tmp, root) = create_temp_git_repo();
        let paths = DvsPaths::new(root.clone(), root.clone(), ".meta");

        let result = paths.metadata_path(Path::new("sub/file.txt"));
        assert_eq!(result, root.join(".meta/sub/file.txt.dvs"));
    }
}
