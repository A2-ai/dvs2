use std::path::{Path, PathBuf};

use crate::config::Config;
use anyhow::{Result, anyhow, bail};
use fs_err as fs;
use globset::Glob;
use walkdir::WalkDir;

pub const CONFIG_FILE_NAME: &str = "dvs.toml";
pub const DEFAULT_FOLDER_NAME: &str = ".dvs";

/// We can pass either a glob or a list of paths to dvs to handle.
/// This enum is here to auto-convert properly
#[derive(Debug, Clone)]
pub enum PathInput {
    /// A glob pattern to expand using the library
    Glob(String),
    /// Explicit list of paths (relative to repo root)
    Paths(Vec<PathBuf>),
}

impl From<&str> for PathInput {
    fn from(s: &str) -> Self {
        PathInput::Glob(s.to_string())
    }
}

impl From<String> for PathInput {
    fn from(s: String) -> Self {
        PathInput::Glob(s)
    }
}

impl From<Vec<PathBuf>> for PathInput {
    fn from(paths: Vec<PathBuf>) -> Self {
        PathInput::Paths(paths)
    }
}

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

    fn resolve(&self, input: &PathInput, tracked: bool) -> Result<Vec<PathBuf>> {
        match input {
            PathInput::Glob(pattern) => {
                if tracked {
                    self.expand_glob_tracked(pattern)
                } else {
                    self.expand_glob(pattern)
                }
            }
            PathInput::Paths(paths) => {
                for path in paths {
                    if tracked {
                        // For get: check if file is tracked (metadata exists)
                        let metadata_path = self.metadata_path(path);
                        if !metadata_path.is_file() {
                            bail!("Path {} is not tracked by DVS", path.display());
                        }
                    } else {
                        // For add: check if file exists on disk
                        let full = self.file_path(path);
                        if !full.is_file() {
                            bail!("Path {} does not exist or is not a file", path.display());
                        }
                    }
                }
                log::debug!("Passed paths: {:?}", paths);
                Ok(paths.clone())
            }
        }
    }

    pub fn resolve_for_add(&self, input: &PathInput) -> Result<Vec<PathBuf>> {
        self.resolve(input, false)
    }

    pub fn resolve_tracked(&self, input: &PathInput) -> Result<Vec<PathBuf>> {
        self.resolve(input, true)
    }

    /// Expand glob pattern against files on disk.
    /// Pattern matched relative to cwd, returns paths relative to repo_root.
    fn expand_glob(&self, pattern: &str) -> Result<Vec<PathBuf>> {
        log::debug!(
            "Expanding glob pattern '{}' from cwd {}",
            pattern,
            self.cwd.display()
        );
        let glob = Glob::new(pattern)
            .map_err(|e| anyhow::anyhow!("Invalid glob pattern '{}': {}", pattern, e))?
            .compile_matcher();

        let mut paths = Vec::new();

        for entry in WalkDir::new(&self.cwd)
            .into_iter()
            .filter_entry(|e| !e.path().starts_with(self.metadata_folder()))
            .filter_map(|e| e.ok())
            .filter(|e| e.file_type().is_file())
        {
            // Check if path matches pattern (relative to cwd)
            let matches = entry
                .path()
                .strip_prefix(&self.cwd)
                .map(|rel| glob.is_match(rel))
                .unwrap_or(false);

            if matches {
                // Convert to repo-relative path
                let relative = entry.path().strip_prefix(&self.repo_root).map_err(|_| {
                    anyhow::anyhow!("Path {} is outside repository", entry.path().display())
                })?;
                paths.push(relative.to_path_buf());
            }
        }

        log::debug!(
            "Pattern '{}' matched {:?}",
            pattern,
            paths.iter().map(|p| p.display()).collect::<Vec<_>>()
        );
        Ok(paths)
    }

    /// Expand glob pattern against tracked files (in .dvs/).
    /// Pattern adjusted for cwd (e.g., "*.txt" in subdir becomes "subdir/*.txt").
    /// Returns paths relative to repo_root.
    fn expand_glob_tracked(&self, pattern: &str) -> Result<Vec<PathBuf>> {
        log::debug!("Expanding glob pattern '{}' against tracked files", pattern);
        // Adjust pattern based on cwd relative to repo root
        let effective_pattern = match self.cwd.strip_prefix(&self.repo_root).ok() {
            Some(rel) if !rel.as_os_str().is_empty() => {
                let adjusted = format!("{}/{}", rel.display(), pattern);
                log::debug!(
                    "Adjusted pattern to '{}' for cwd-relative matching",
                    adjusted
                );
                adjusted
            }
            _ => pattern.to_string(),
        };

        let glob = Glob::new(&effective_pattern)
            .map_err(|e| anyhow::anyhow!("Invalid glob pattern '{}': {}", effective_pattern, e))?
            .compile_matcher();

        let dvs_dir = self.metadata_folder();
        let mut paths = Vec::new();

        for entry in WalkDir::new(&dvs_dir)
            .into_iter()
            .filter_map(|e| e.ok())
            .filter(|e| e.file_type().is_file())
            .filter(|e| {
                e.path()
                    .extension()
                    .map(|ext| ext == "dvs")
                    .unwrap_or(false)
            })
        {
            // Strip .dvs dir prefix and .dvs extension
            if let Ok(rel) = entry.path().strip_prefix(&dvs_dir) {
                let rel_no_ext = rel.with_extension("");
                if glob.is_match(&rel_no_ext) {
                    paths.push(rel_no_ext);
                }
            }
        }

        log::debug!(
            "Pattern '{}' matched {:?}",
            pattern,
            paths.iter().map(|p| p.display()).collect::<Vec<_>>()
        );
        Ok(paths)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testutil::create_temp_git_repo;
    use uuid::Uuid;

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

    #[test]
    fn expand_glob_returns_repo_relative_paths() {
        let (_tmp, root) = create_temp_git_repo();
        crate::testutil::create_file(&root, "a.txt", b"a");
        crate::testutil::create_file(&root, "sub/b.txt", b"b");

        let cwd = root.join("sub");
        let paths = DvsPaths::new(cwd, root.clone(), ".dvs");

        // From sub/, "*.txt" should only match sub/b.txt
        let result = paths.expand_glob("*.txt").unwrap();
        assert_eq!(result, vec![PathBuf::from("sub/b.txt")]);
    }

    #[test]
    fn expand_glob_tracked_adjusts_pattern_for_cwd() {
        let (_tmp, root) = create_temp_git_repo();
        let dvs_dir = root.join(".dvs");
        fs::create_dir_all(dvs_dir.join("sub")).unwrap();
        crate::testutil::create_file(&dvs_dir, "root.txt.dvs", b"{}");
        crate::testutil::create_file(&dvs_dir, "sub/nested.txt.dvs", b"{}");

        let cwd = root.join("sub");
        let paths = DvsPaths::new(cwd, root.clone(), ".dvs");

        // From sub/, "*.txt" should match sub/*.txt only
        let result = paths.expand_glob_tracked("*.txt").unwrap();
        assert_eq!(result, vec![PathBuf::from("sub/nested.txt")]);
    }

    #[test]
    fn expand_glob_excludes_metadata_folder() {
        let (_tmp, root) = create_temp_git_repo();
        let (config, _dvs_dir) = crate::testutil::init_dvs_repo(&root);

        // Add a regular file via DVS (creates .dvs/data.txt.dvs metadata)
        let file_path = crate::testutil::create_file(&root, "data.txt", b"content");
        let metadata = crate::FileMetadata::from_file(&file_path, None).unwrap();
        let paths = DvsPaths::new(root.clone(), root.clone(), config.metadata_folder_name());
        metadata
            .save(
                Uuid::new_v4(),
                &file_path,
                config.backend(),
                &paths,
                "data.txt",
            )
            .unwrap();

        // Also create a regular file in a subdir
        crate::testutil::create_file(&root, "subdir/other.txt", b"other");

        // expand_glob should NOT include .dvs/* paths
        let result = paths.expand_glob("**/*").unwrap();

        // Should include data.txt and subdir/other.txt, but NOT .dvs/data.txt.dvs
        assert!(result.iter().any(|p| p == Path::new("data.txt")));
        assert!(result.iter().any(|p| p == Path::new("subdir/other.txt")));
        assert!(!result.iter().any(|p| p.starts_with(".dvs")));
    }
}
