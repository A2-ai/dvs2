//! Repository backend abstraction.
//!
//! Provides a unified interface for Git-backed and DVS-only workspaces.
//! The backend handles repo root detection, path normalization, ignore
//! file handling, and optional branch information.

use super::git_ops::select_git_backend;
use super::ignore::{
    add_dvsignore_pattern, add_gitignore_pattern, load_dvs_ignore_patterns, load_gitignore_patterns,
};
use crate::DvsError;
use std::path::{Path, PathBuf};

/// Backend trait for repository/workspace operations.
///
/// Implementations provide repo-root lookup, path normalization,
/// ignore handling, and optional branch info.
pub trait RepoBackend: Send + Sync {
    /// Get the root directory of the repository/workspace.
    fn root(&self) -> &Path;

    /// Normalize a path relative to the repository root.
    fn normalize(&self, path: &Path) -> Result<PathBuf, DvsError>;

    /// Add a pattern to the appropriate ignore file.
    ///
    /// - Git backend: adds to `.gitignore`
    /// - DVS backend: adds to `.dvsignore`
    fn add_ignore(&self, pattern: &str) -> Result<(), DvsError>;

    /// Check if a path is ignored.
    ///
    /// - Git backend: checks `.gitignore` (and git's ignore rules)
    /// - DVS backend: checks `.dvsignore` and `.ignore`
    fn is_ignored(&self, path: &Path) -> Result<bool, DvsError>;

    /// Get the current branch name, if applicable.
    ///
    /// Returns `Ok(None)` for DVS-only workspaces.
    fn current_branch(&self) -> Result<Option<String>, DvsError>;

    /// Get the backend type name for logging/debugging.
    fn backend_type(&self) -> &'static str;
}

/// Backend enum for runtime dispatch.
///
/// Prefers Git-backed projects but falls back to DVS-only workspace.
#[derive(Debug)]
pub enum Backend {
    /// Git repository backend.
    Git(GitBackend),
    /// DVS-only workspace backend.
    Dvs(DvsBackend),
}

impl Backend {
    /// Get a reference to the underlying backend trait object.
    pub fn as_backend(&self) -> &dyn RepoBackend {
        match self {
            Backend::Git(b) => b,
            Backend::Dvs(b) => b,
        }
    }
}

// Delegate RepoBackend methods to the inner backend
impl RepoBackend for Backend {
    fn root(&self) -> &Path {
        match self {
            Backend::Git(b) => b.root(),
            Backend::Dvs(b) => b.root(),
        }
    }

    fn normalize(&self, path: &Path) -> Result<PathBuf, DvsError> {
        match self {
            Backend::Git(b) => b.normalize(path),
            Backend::Dvs(b) => b.normalize(path),
        }
    }

    fn add_ignore(&self, pattern: &str) -> Result<(), DvsError> {
        match self {
            Backend::Git(b) => b.add_ignore(pattern),
            Backend::Dvs(b) => b.add_ignore(pattern),
        }
    }

    fn is_ignored(&self, path: &Path) -> Result<bool, DvsError> {
        match self {
            Backend::Git(b) => b.is_ignored(path),
            Backend::Dvs(b) => b.is_ignored(path),
        }
    }

    fn current_branch(&self) -> Result<Option<String>, DvsError> {
        match self {
            Backend::Git(b) => b.current_branch(),
            Backend::Dvs(b) => b.current_branch(),
        }
    }

    fn backend_type(&self) -> &'static str {
        match self {
            Backend::Git(b) => b.backend_type(),
            Backend::Dvs(b) => b.backend_type(),
        }
    }
}

// ============================================================================
// Git Backend
// ============================================================================

/// Git repository backend.
///
/// Uses `.git` for root detection, `.gitignore` for ignore handling,
/// and provides branch information via the `git_ops` module.
#[derive(Debug)]
pub struct GitBackend {
    root: PathBuf,
}

impl GitBackend {
    /// Create a new Git backend for the given repository root.
    pub fn new(root: PathBuf) -> Self {
        Self { root }
    }

    /// Find Git root by walking up from the given path.
    ///
    /// Uses the git_ops module (libgit2 or CLI fallback) for discovery.
    pub fn find_root(start: &Path) -> Option<PathBuf> {
        let backend = select_git_backend();
        backend.discover_repo_root(start).ok()
    }

    /// Find Git root using simple filesystem check (no git2/cli).
    ///
    /// This is a fallback method that doesn't require git to be installed.
    pub fn find_root_simple(start: &Path) -> Option<PathBuf> {
        let mut current = start.to_path_buf();
        loop {
            if current.join(".git").exists() {
                return Some(current);
            }
            if !current.pop() {
                return None;
            }
        }
    }
}

impl RepoBackend for GitBackend {
    fn root(&self) -> &Path {
        &self.root
    }

    fn normalize(&self, path: &Path) -> Result<PathBuf, DvsError> {
        // Canonicalize both paths to resolve symlinks and ..
        let abs_path = if path.is_absolute() {
            path.to_path_buf()
        } else {
            std::env::current_dir()?.join(path)
        };

        // Use pathdiff to get relative path from root
        match pathdiff::diff_paths(&abs_path, &self.root) {
            Some(rel) => Ok(rel),
            None => Err(DvsError::file_outside_repo(abs_path)),
        }
    }

    fn add_ignore(&self, pattern: &str) -> Result<(), DvsError> {
        add_gitignore_pattern(&self.root, pattern)
    }

    fn is_ignored(&self, path: &Path) -> Result<bool, DvsError> {
        let patterns = load_gitignore_patterns(&self.root)?;
        let rel_path = self.normalize(path)?;
        Ok(patterns.is_ignored(&rel_path))
    }

    fn current_branch(&self) -> Result<Option<String>, DvsError> {
        // Use git_ops for robust HEAD parsing
        let backend = select_git_backend();
        let head_info = backend.head_info(&self.root)?;
        Ok(head_info.branch)
    }

    fn backend_type(&self) -> &'static str {
        "git"
    }
}

// ============================================================================
// DVS Backend
// ============================================================================

/// DVS-only workspace backend.
///
/// Uses config file (`dvs.yaml` or `dvs.json`) or `.dvs/` for root detection,
/// `.dvsignore` and `.ignore` for ignore handling. Does not provide branch information.
#[derive(Debug)]
pub struct DvsBackend {
    root: PathBuf,
}

impl DvsBackend {
    /// Create a new DVS backend for the given workspace root.
    pub fn new(root: PathBuf) -> Self {
        Self { root }
    }

    /// Find DVS workspace root by walking up from the given path.
    ///
    /// Looks for config file (`dvs.yaml` or `dvs.json`) or `.dvs/` directory.
    pub fn find_root(start: &Path) -> Option<PathBuf> {
        use crate::Config;
        let mut current = start.to_path_buf();
        loop {
            if current.join(Config::config_filename()).exists() || current.join(".dvs").is_dir() {
                return Some(current);
            }
            if !current.pop() {
                return None;
            }
        }
    }

    /// Get the path to the `.dvsignore` file.
    pub fn dvsignore_path(&self) -> PathBuf {
        self.root.join(".dvsignore")
    }

    /// Get the path to the `.ignore` file.
    pub fn ignore_path(&self) -> PathBuf {
        self.root.join(".ignore")
    }
}

impl RepoBackend for DvsBackend {
    fn root(&self) -> &Path {
        &self.root
    }

    fn normalize(&self, path: &Path) -> Result<PathBuf, DvsError> {
        // Canonicalize both paths to resolve symlinks and ..
        let abs_path = if path.is_absolute() {
            path.to_path_buf()
        } else {
            std::env::current_dir()?.join(path)
        };

        // Use pathdiff to get relative path from root
        match pathdiff::diff_paths(&abs_path, &self.root) {
            Some(rel) => Ok(rel),
            None => Err(DvsError::file_outside_repo(abs_path)),
        }
    }

    fn add_ignore(&self, pattern: &str) -> Result<(), DvsError> {
        add_dvsignore_pattern(&self.root, pattern)
    }

    fn is_ignored(&self, path: &Path) -> Result<bool, DvsError> {
        let patterns = load_dvs_ignore_patterns(&self.root)?;
        let rel_path = self.normalize(path)?;
        Ok(patterns.is_ignored(&rel_path))
    }

    fn current_branch(&self) -> Result<Option<String>, DvsError> {
        // DVS-only workspaces don't have branches
        Ok(None)
    }

    fn backend_type(&self) -> &'static str {
        "dvs"
    }
}

// ============================================================================
// Backend Detection
// ============================================================================

/// Detect the appropriate backend for the given starting path.
///
/// Detection order:
/// 1. Git repository (looks for `.git`)
/// 2. DVS workspace (looks for config file or `.dvs/`)
///
/// # Errors
///
/// Returns `DvsError::NotInitialized` if no workspace is found.
pub fn detect_backend(start: &Path) -> Result<Backend, DvsError> {
    // Try Git first
    if let Some(git_root) = GitBackend::find_root(start) {
        return Ok(Backend::Git(GitBackend::new(git_root)));
    }

    // Fall back to DVS-only workspace
    if let Some(dvs_root) = DvsBackend::find_root(start) {
        return Ok(Backend::Dvs(DvsBackend::new(dvs_root)));
    }

    Err(DvsError::not_initialized())
}

/// Detect backend, starting from the current working directory.
pub fn detect_backend_cwd() -> Result<Backend, DvsError> {
    let cwd = std::env::current_dir()?;
    detect_backend(&cwd)
}

#[cfg(test)]
mod tests {
    use super::*;
    use fs_err as fs;

    #[test]
    fn test_git_backend_type() {
        let backend = GitBackend::new(PathBuf::from("/tmp/test"));
        assert_eq!(backend.backend_type(), "git");
    }

    #[test]
    fn test_dvs_backend_type() {
        let backend = DvsBackend::new(PathBuf::from("/tmp/test"));
        assert_eq!(backend.backend_type(), "dvs");
    }

    #[test]
    fn test_dvs_backend_no_branch() {
        let backend = DvsBackend::new(PathBuf::from("/tmp/test"));
        assert_eq!(backend.current_branch().unwrap(), None);
    }

    #[test]
    fn test_git_find_root() {
        // Test in the current repo (which should be a git repo)
        let cwd = std::env::current_dir().unwrap();
        let root = GitBackend::find_root(&cwd);
        assert!(root.is_some());
        let root = root.unwrap();
        assert!(root.join(".git").exists());
    }

    #[test]
    fn test_git_backend_current_branch() {
        // Test reading branch from actual git repo
        let cwd = std::env::current_dir().unwrap();
        if let Some(root) = GitBackend::find_root(&cwd) {
            let backend = GitBackend::new(root);
            // Should return Ok (either Some(branch) or None for detached HEAD)
            let result = backend.current_branch();
            assert!(result.is_ok());
        }
    }

    #[test]
    fn test_detect_backend_git() {
        // Should detect git backend in current repo
        let cwd = std::env::current_dir().unwrap();
        let backend = detect_backend(&cwd);
        assert!(backend.is_ok());
        let backend = backend.unwrap();
        assert_eq!(backend.backend_type(), "git");
    }

    #[test]
    fn test_backend_normalize_relative_path() {
        // Test normalizing a relative path
        let root = std::env::current_dir().unwrap();
        let backend = GitBackend::new(root.clone());

        // A path within the repo should normalize successfully
        let normalized = backend.normalize(&root.join("src/lib.rs"));
        assert!(normalized.is_ok());
        let normalized = normalized.unwrap();
        // Should be relative path
        assert!(!normalized.is_absolute());
    }

    #[test]
    fn test_dvs_backend_paths() {
        let root = PathBuf::from("/tmp/test-dvs-workspace");
        let backend = DvsBackend::new(root.clone());

        assert_eq!(backend.dvsignore_path(), root.join(".dvsignore"));
        assert_eq!(backend.ignore_path(), root.join(".ignore"));
    }

    #[test]
    fn test_detect_backend_no_workspace() {
        // A path with no git or dvs workspace should fail
        let result = detect_backend(Path::new("/"));
        assert!(result.is_err());
    }

    #[test]
    fn test_dvs_find_root_with_dvs_yaml() {
        use crate::Config;

        // Create a temp directory with config file
        let temp_dir = std::env::temp_dir().join("dvs-test-find-root");
        let _ = fs::create_dir_all(&temp_dir);

        #[cfg(feature = "yaml-config")]
        fs::write(
            temp_dir.join(Config::config_filename()),
            "storage_dir: /tmp/storage",
        )
        .unwrap();

        #[cfg(all(feature = "toml-config", not(feature = "yaml-config")))]
        fs::write(
            temp_dir.join(Config::config_filename()),
            "storage_dir = \"/tmp/storage\"",
        )
        .unwrap();

        #[cfg(all(not(feature = "yaml-config"), not(feature = "toml-config")))]
        fs::write(
            temp_dir.join(Config::config_filename()),
            r#"{"storage_dir": "/tmp/storage"}"#,
        )
        .unwrap();

        let root = DvsBackend::find_root(&temp_dir);
        assert!(root.is_some());
        assert_eq!(root.unwrap(), temp_dir);

        // Cleanup
        let _ = fs::remove_dir_all(&temp_dir);
    }

    #[test]
    fn test_dvs_find_root_with_dvs_dir() {
        // Create a temp directory with .dvs/
        let temp_dir = std::env::temp_dir().join("dvs-test-find-root-dir");
        let _ = fs::create_dir_all(temp_dir.join(".dvs"));

        let root = DvsBackend::find_root(&temp_dir);
        assert!(root.is_some());
        assert_eq!(root.unwrap(), temp_dir);

        // Cleanup
        let _ = fs::remove_dir_all(&temp_dir);
    }
}
