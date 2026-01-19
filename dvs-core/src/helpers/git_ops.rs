//! Git operations abstraction with libgit2 and CLI backends.
//!
//! Provides a `GitOps` trait that abstracts Git operations, with two implementations:
//! - `Git2Ops`: Uses the `git2` crate (libgit2) - default
//! - `GitCliOps`: Uses the system `git` CLI - fallback
//!
//! Backend selection:
//! - Set `DVS_GIT_BACKEND=cli` to force CLI backend
//! - Otherwise uses git2, with automatic fallback to CLI on certain errors

use std::path::{Path, PathBuf};
use std::process::Command;
use crate::DvsError;

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
///
/// Implementations provide the actual Git functionality using either
/// libgit2 or the system Git CLI.
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
// Git2Ops Implementation (libgit2)
// ============================================================================

/// Git operations using the `git2` crate (libgit2).
#[derive(Debug, Clone, Default)]
pub struct Git2Ops;

impl Git2Ops {
    /// Create a new Git2Ops instance.
    pub fn new() -> Self {
        Self
    }
}

impl GitOps for Git2Ops {
    fn discover_repo_root(&self, start: &Path) -> Result<PathBuf, DvsError> {
        let repo = git2::Repository::discover(start).map_err(|e| DvsError::GitError {
            message: format!("Failed to discover repository: {}", e),
        })?;

        let workdir = repo.workdir().ok_or_else(|| DvsError::GitError {
            message: "Repository has no working directory (bare repository)".to_string(),
        })?;

        Ok(workdir.to_path_buf())
    }

    fn head_info(&self, repo_root: &Path) -> Result<HeadInfo, DvsError> {
        let repo = git2::Repository::open(repo_root).map_err(|e| DvsError::GitError {
            message: format!("Failed to open repository: {}", e),
        })?;

        let head = match repo.head() {
            Ok(h) => h,
            Err(e) if e.code() == git2::ErrorCode::UnbornBranch => {
                // No commits yet
                return Ok(HeadInfo {
                    oid: None,
                    branch: None,
                    is_detached: false,
                });
            }
            Err(e) => {
                return Err(DvsError::GitError {
                    message: format!("Failed to get HEAD: {}", e),
                });
            }
        };

        let oid = head.target().map(|o| o.to_string());
        let is_detached = repo.head_detached().unwrap_or(false);

        let branch = if is_detached {
            None
        } else {
            head.shorthand().map(|s| s.to_string())
        };

        Ok(HeadInfo {
            oid,
            branch,
            is_detached,
        })
    }

    fn status_info(&self, repo_root: &Path) -> Result<StatusInfo, DvsError> {
        let repo = git2::Repository::open(repo_root).map_err(|e| DvsError::GitError {
            message: format!("Failed to open repository: {}", e),
        })?;

        let mut opts = git2::StatusOptions::new();
        opts.include_untracked(true)
            .recurse_untracked_dirs(false)
            .exclude_submodules(true);

        let statuses = repo.statuses(Some(&mut opts)).map_err(|e| DvsError::GitError {
            message: format!("Failed to get status: {}", e),
        })?;

        let mut is_dirty = false;
        let mut has_untracked = false;

        for entry in statuses.iter() {
            let status = entry.status();
            if status.contains(git2::Status::WT_NEW) {
                has_untracked = true;
            }
            if status.intersects(
                git2::Status::INDEX_NEW
                    | git2::Status::INDEX_MODIFIED
                    | git2::Status::INDEX_DELETED
                    | git2::Status::INDEX_RENAMED
                    | git2::Status::INDEX_TYPECHANGE
                    | git2::Status::WT_MODIFIED
                    | git2::Status::WT_DELETED
                    | git2::Status::WT_RENAMED
                    | git2::Status::WT_TYPECHANGE,
            ) {
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
        let repo = git2::Repository::open(repo_root).map_err(|e| DvsError::GitError {
            message: format!("Failed to open repository: {}", e),
        })?;

        let config = repo.config().map_err(|e| DvsError::GitError {
            message: format!("Failed to get config: {}", e),
        })?;

        match config.get_string(key) {
            Ok(value) => Ok(Some(value)),
            Err(e) if e.code() == git2::ErrorCode::NotFound => Ok(None),
            Err(e) => Err(DvsError::GitError {
                message: format!("Failed to get config value '{}': {}", key, e),
            }),
        }
    }

    fn remote_url(&self, repo_root: &Path, name: &str) -> Result<Option<String>, DvsError> {
        let repo = git2::Repository::open(repo_root).map_err(|e| DvsError::GitError {
            message: format!("Failed to open repository: {}", e),
        })?;

        let remote_result = repo.find_remote(name);
        match remote_result {
            Ok(remote) => {
                let url = remote.url().map(|s| s.to_string());
                Ok(url)
            }
            Err(e) if e.code() == git2::ErrorCode::NotFound => Ok(None),
            Err(e) => Err(DvsError::GitError {
                message: format!("Failed to find remote '{}': {}", name, e),
            }),
        }
    }

    fn create_tag_lightweight(
        &self,
        repo_root: &Path,
        name: &str,
        target_oid: &str,
    ) -> Result<(), DvsError> {
        let repo = git2::Repository::open(repo_root).map_err(|e| DvsError::GitError {
            message: format!("Failed to open repository: {}", e),
        })?;

        let oid = git2::Oid::from_str(target_oid).map_err(|e| DvsError::GitError {
            message: format!("Invalid OID '{}': {}", target_oid, e),
        })?;

        let obj = repo.find_object(oid, None).map_err(|e| DvsError::GitError {
            message: format!("Failed to find object '{}': {}", target_oid, e),
        })?;

        repo.tag_lightweight(name, &obj, false)
            .map_err(|e| DvsError::GitError {
                message: format!("Failed to create tag '{}': {}", name, e),
            })?;

        Ok(())
    }

    fn backend_name(&self) -> &'static str {
        "git2"
    }
}

// ============================================================================
// GitCliOps Implementation (system git CLI)
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
            .map_err(|e| DvsError::GitError {
                message: format!("Failed to execute git: {}", e),
            })?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(DvsError::GitError {
                message: format!("git {} failed: {}", args.join(" "), stderr.trim()),
            });
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
            .map_err(|e| DvsError::GitError {
                message: format!("Failed to execute git: {}", e),
            })?;

        if !output.status.success() {
            return Err(DvsError::GitError {
                message: "Not a git repository".to_string(),
            });
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

/// Select the Git backend based on environment variable.
///
/// - If `DVS_GIT_BACKEND=cli`, returns `GitCliOps`
/// - Otherwise returns `Git2Ops`
pub fn select_git_backend() -> Box<dyn GitOps> {
    match std::env::var("DVS_GIT_BACKEND").as_deref() {
        Ok("cli") => Box::new(GitCliOps::new()),
        _ => Box::new(Git2Ops::new()),
    }
}

/// Get the default Git backend (git2).
pub fn default_git_backend() -> Git2Ops {
    Git2Ops::new()
}

/// Get the CLI Git backend.
pub fn cli_git_backend() -> GitCliOps {
    GitCliOps::new()
}

/// Execute a Git operation with automatic fallback to CLI on certain errors.
///
/// Tries the git2 backend first, and falls back to CLI if the error
/// suggests an unsupported repository layout (worktrees, etc.).
pub fn with_fallback<T, F>(op: F) -> Result<T, DvsError>
where
    F: Fn(&dyn GitOps) -> Result<T, DvsError>,
{
    let git2_backend = Git2Ops::new();
    match op(&git2_backend) {
        Ok(result) => Ok(result),
        Err(e) => {
            // Check if this is a fallback-worthy error
            let error_msg = format!("{}", e);
            if should_fallback(&error_msg) {
                eprintln!(
                    "dvs: git2 backend failed ({}), falling back to CLI",
                    error_msg
                );
                let cli_backend = GitCliOps::new();
                op(&cli_backend)
            } else {
                Err(e)
            }
        }
    }
}

/// Check if an error message suggests we should fall back to CLI.
fn should_fallback(error_msg: &str) -> bool {
    let fallback_patterns = [
        "unsupported",
        "worktree",
        "submodule",
        "sparse",
        "could not find repository",
    ];
    let lower = error_msg.to_lowercase();
    fallback_patterns.iter().any(|p| lower.contains(p))
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
    fn test_git2_backend_name() {
        let backend = Git2Ops::new();
        assert_eq!(backend.backend_name(), "git2");
    }

    #[test]
    fn test_cli_backend_name() {
        let backend = GitCliOps::new();
        assert_eq!(backend.backend_name(), "cli");
    }

    #[test]
    fn test_select_default_backend() {
        // Without env var, should select git2
        std::env::remove_var("DVS_GIT_BACKEND");
        let backend = select_git_backend();
        assert_eq!(backend.backend_name(), "git2");
    }

    #[test]
    fn test_should_fallback() {
        assert!(should_fallback("unsupported repository layout"));
        assert!(should_fallback("worktree not found"));
        assert!(should_fallback("submodule error"));
        assert!(!should_fallback("file not found"));
        assert!(!should_fallback("permission denied"));
    }

    #[test]
    fn test_git2_discover_repo_root() {
        // Test in the current repo (which should be a git repo)
        let cwd = std::env::current_dir().unwrap();
        let backend = Git2Ops::new();
        let result = backend.discover_repo_root(&cwd);
        assert!(result.is_ok());
        let root = result.unwrap();
        assert!(root.join(".git").exists());
    }

    #[test]
    fn test_git2_head_info() {
        let cwd = std::env::current_dir().unwrap();
        let backend = Git2Ops::new();
        if let Ok(root) = backend.discover_repo_root(&cwd) {
            let result = backend.head_info(&root);
            assert!(result.is_ok());
            // Should have some head info in a real repo
            let info = result.unwrap();
            // At minimum, we should not panic
            let _ = info.oid;
            let _ = info.branch;
            let _ = info.is_detached;
        }
    }

    #[test]
    fn test_git2_status_info() {
        let cwd = std::env::current_dir().unwrap();
        let backend = Git2Ops::new();
        if let Ok(root) = backend.discover_repo_root(&cwd) {
            let result = backend.status_info(&root);
            assert!(result.is_ok());
        }
    }

    #[test]
    fn test_git2_config_value() {
        let cwd = std::env::current_dir().unwrap();
        let backend = Git2Ops::new();
        if let Ok(root) = backend.discover_repo_root(&cwd) {
            // user.name might or might not be set, but should not error
            let result = backend.config_value(&root, "user.name");
            assert!(result.is_ok());
        }
    }

    #[test]
    fn test_git2_remote_url() {
        let cwd = std::env::current_dir().unwrap();
        let backend = Git2Ops::new();
        if let Ok(root) = backend.discover_repo_root(&cwd) {
            // origin might or might not exist
            let result = backend.remote_url(&root, "origin");
            assert!(result.is_ok());
        }
    }

    #[test]
    fn test_cli_discover_repo_root() {
        let cwd = std::env::current_dir().unwrap();
        let backend = GitCliOps::new();
        let result = backend.discover_repo_root(&cwd);
        // Should work if git is installed
        if result.is_ok() {
            let root = result.unwrap();
            assert!(root.join(".git").exists());
        }
    }

    #[test]
    fn test_cli_head_info() {
        let cwd = std::env::current_dir().unwrap();
        let backend = GitCliOps::new();
        if let Ok(root) = backend.discover_repo_root(&cwd) {
            let result = backend.head_info(&root);
            // Should work if git is installed
            if result.is_ok() {
                let info = result.unwrap();
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
