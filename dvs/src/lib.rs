pub mod config;
pub mod file;
pub mod init;

pub use file::{FileMetadata, FileStatus, Hashes, Outcome, Status};
pub use file::{get_file, get_file_status, get_status};

#[cfg(test)]
pub mod testutil {
    use crate::config::Config;
    use crate::init::init;
    use fs_err as fs;
    use std::path::{Path, PathBuf};
    use tempfile::TempDir;

    /// Creates a temporary directory with a .git folder (simulating a git repo).
    /// Returns the TempDir (owns the directory) and the path to the repo root.
    ///
    /// IMPORTANT: Keep the TempDir alive for the duration of the test,
    /// otherwise the directory gets deleted.
    pub fn create_temp_git_repo() -> (TempDir, PathBuf) {
        let tmp = tempfile::tempdir().unwrap();
        let repo_root = tmp.path().to_path_buf();
        fs::create_dir(repo_root.join(".git")).unwrap();
        (tmp, repo_root)
    }

    /// Creates a file with the given content at the specified path.
    /// Creates parent directories if needed.
    /// Returns the full path to the created file.
    pub fn create_file(dir: &Path, relative_path: &str, content: &[u8]) -> PathBuf {
        let path = dir.join(relative_path);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).ok();
        }
        fs::write(&path, content).unwrap();
        path
    }

    /// Initializes a DVS repository in the given directory.
    /// Creates storage at `{repo_root}/.storage` and metadata at `{repo_root}/.dvs`.
    /// Returns (storage_dir, dvs_metadata_dir).
    pub fn init_dvs_repo(repo_root: &Path) -> (PathBuf, PathBuf) {
        let storage_dir = repo_root.join(".storage");
        let config = Config::new_local(&storage_dir);
        init(repo_root, config).unwrap();
        let dvs_dir = repo_root.join(".dvs");
        (storage_dir, dvs_dir)
    }
}
