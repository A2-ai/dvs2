//! Git operations using the system git CLI.

use crate::DvsError;
use std::path::{Path, PathBuf};
use std::process::Command;

// ============================================================================
// Types
// ============================================================================

/// Information about the repository HEAD.
#[derive(Debug, Clone, Default)]
pub struct HeadInfo {
    /// Current commit OID (hex string), if any.
    pub oid: Option<String>,
    /// Current branch name, if on a branch.
    pub branch: Option<String>,
    /// Whether HEAD is detached (not on a branch).
    pub is_detached: bool,
}

/// Information about the repository status.
#[derive(Debug, Clone, Default)]
pub struct StatusInfo {
    /// Whether there are uncommitted changes (staged or unstaged).
    pub is_dirty: bool,
    /// Whether there are untracked files.
    pub has_untracked: bool,
}

// ============================================================================
// GitOps Trait
// ============================================================================

/// Trait for Git operations.
pub trait GitOps: Send + Sync {
    /// Discover the repository root from a starting path.
    fn discover_repo_root(&self, start: &Path) -> Result<PathBuf, DvsError>;

    /// Get information about HEAD (commit OID, branch name, detached state).
    fn head_info(&self, repo_root: &Path) -> Result<HeadInfo, DvsError>;

    /// Get repository status (dirty, untracked files).
    fn status_info(&self, repo_root: &Path) -> Result<StatusInfo, DvsError>;

    /// Get a Git config value.
    fn config_value(&self, repo_root: &Path, key: &str) -> Result<Option<String>, DvsError>;

    /// Get the URL of a remote.
    fn remote_url(&self, repo_root: &Path, name: &str) -> Result<Option<String>, DvsError>;

    /// Create a lightweight tag pointing to a commit.
    fn create_tag_lightweight(
        &self,
        repo_root: &Path,
        name: &str,
        target_oid: &str,
    ) -> Result<(), DvsError>;

    /// Get the backend name for logging.
    fn backend_name(&self) -> &'static str;
}

// ============================================================================
// GitCliOps Implementation
// ============================================================================

/// Git operations using the system `git` CLI.
#[derive(Debug, Clone, Default)]
pub struct GitCliOps;

impl GitCliOps {
    /// Create a new GitCliOps instance.
    pub fn new() -> Self {
        Self
    }

    /// Run a git command and return stdout as a string.
    fn run_git(&self, repo_root: &Path, args: &[&str]) -> Result<String, DvsError> {
        let output = Command::new("git")
            .arg("-C")
            .arg(repo_root)
            .args(args)
            .output()
            .map_err(|e| DvsError::git_error(format!("Failed to execute git: {}", e)))?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(DvsError::git_error(format!(
                "git {} failed: {}",
                args.join(" "),
                stderr.trim()
            )));
        }

        Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
    }

    /// Run a git command, returning None if it fails (for optional values).
    fn run_git_optional(&self, repo_root: &Path, args: &[&str]) -> Option<String> {
        let output = Command::new("git")
            .arg("-C")
            .arg(repo_root)
            .args(args)
            .output()
            .ok()?;

        if output.status.success() {
            let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
            if stdout.is_empty() {
                None
            } else {
                Some(stdout)
            }
        } else {
            None
        }
    }
}

impl GitOps for GitCliOps {
    fn discover_repo_root(&self, start: &Path) -> Result<PathBuf, DvsError> {
        let output = Command::new("git")
            .arg("-C")
            .arg(start)
            .args(["rev-parse", "--show-toplevel"])
            .output()
            .map_err(|e| DvsError::git_error(format!("Failed to execute git: {}", e)))?;

        if !output.status.success() {
            return Err(DvsError::git_error("Not a git repository"));
        }

        let path = String::from_utf8_lossy(&output.stdout).trim().to_string();
        Ok(PathBuf::from(path))
    }

    fn head_info(&self, repo_root: &Path) -> Result<HeadInfo, DvsError> {
        // Get HEAD commit OID
        let oid = self.run_git_optional(repo_root, &["rev-parse", "HEAD"]);

        // Get branch name (fails if detached)
        let branch = self.run_git_optional(repo_root, &["symbolic-ref", "--short", "HEAD"]);

        // Determine if detached
        let is_detached = oid.is_some() && branch.is_none();

        Ok(HeadInfo {
            oid,
            branch,
            is_detached,
        })
    }

    fn status_info(&self, repo_root: &Path) -> Result<StatusInfo, DvsError> {
        let output = self.run_git(repo_root, &["status", "--porcelain"])?;

        let mut is_dirty = false;
        let mut has_untracked = false;

        for line in output.lines() {
            if line.starts_with("??") {
                has_untracked = true;
            } else if !line.is_empty() {
                is_dirty = true;
            }
            if is_dirty && has_untracked {
                break;
            }
        }

        Ok(StatusInfo {
            is_dirty,
            has_untracked,
        })
    }

    fn config_value(&self, repo_root: &Path, key: &str) -> Result<Option<String>, DvsError> {
        Ok(self.run_git_optional(repo_root, &["config", "--get", key]))
    }

    fn remote_url(&self, repo_root: &Path, name: &str) -> Result<Option<String>, DvsError> {
        Ok(self.run_git_optional(repo_root, &["remote", "get-url", name]))
    }

    fn create_tag_lightweight(
        &self,
        repo_root: &Path,
        name: &str,
        target_oid: &str,
    ) -> Result<(), DvsError> {
        self.run_git(repo_root, &["tag", name, target_oid])?;
        Ok(())
    }

    fn backend_name(&self) -> &'static str {
        "cli"
    }
}

// ============================================================================
// Backend Selection
// ============================================================================

/// Select the Git backend. Always returns GitCliOps.
pub fn select_git_backend() -> Box<dyn GitOps> {
    Box::new(GitCliOps::new())
}

/// Get the default Git backend.
pub fn default_git_backend() -> GitCliOps {
    GitCliOps::new()
}

/// Get the CLI Git backend.
pub fn cli_git_backend() -> GitCliOps {
    GitCliOps::new()
}

/// Execute a Git operation.
pub fn with_fallback<T, F>(op: F) -> Result<T, DvsError>
where
    F: Fn(&dyn GitOps) -> Result<T, DvsError>,
{
    let cli_backend = GitCliOps::new();
    op(&cli_backend)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_head_info_default() {
        let info = HeadInfo::default();
        assert!(info.oid.is_none());
        assert!(info.branch.is_none());
        assert!(!info.is_detached);
    }

    #[test]
    fn test_status_info_default() {
        let info = StatusInfo::default();
        assert!(!info.is_dirty);
        assert!(!info.has_untracked);
    }

    #[test]
    fn test_cli_backend_name() {
        let backend = GitCliOps::new();
        assert_eq!(backend.backend_name(), "cli");
    }

    #[test]
    fn test_select_default_backend() {
        let backend = select_git_backend();
        assert_eq!(backend.backend_name(), "cli");
    }

    #[test]
    fn test_cli_discover_repo_root() {
        let cwd = std::env::current_dir().unwrap();
        let backend = GitCliOps::new();
        // Should work if git is installed
        if let Ok(root) = backend.discover_repo_root(&cwd) {
            assert!(root.join(".git").exists());
        }
    }

    #[test]
    fn test_cli_head_info() {
        let cwd = std::env::current_dir().unwrap();
        let backend = GitCliOps::new();
        if let Ok(root) = backend.discover_repo_root(&cwd) {
            // Should work if git is installed
            if let Ok(info) = backend.head_info(&root) {
                let _ = info.oid;
                let _ = info.branch;
                let _ = info.is_detached;
            }
        }
    }

    #[test]
    fn test_with_fallback() {
        let cwd = std::env::current_dir().unwrap();
        let result = with_fallback(|backend| backend.discover_repo_root(&cwd));
        assert!(result.is_ok());
    }
}
