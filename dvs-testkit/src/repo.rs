//! Test repository utilities.

use dvs_core::helpers::backend::{detect_backend, Backend, GitBackend};
use fs_err as fs;
use std::path::{Path, PathBuf};
use tempfile::TempDir;

/// A temporary test repository with git init and storage directory.
///
/// The repository is automatically cleaned up when dropped.
pub struct TestRepo {
    /// Temporary directory containing the repo.
    _temp: TempDir,
    /// Path to the repository root.
    root: PathBuf,
    /// Path to the storage directory.
    storage: PathBuf,
}

impl TestRepo {
    /// Create a new test repository with git init.
    pub fn new() -> Result<Self, TestRepoError> {
        let temp = TempDir::new()?;
        let root = temp.path().to_path_buf();
        let storage = root.join(".dvs-storage");

        // Initialize git repo
        git2::Repository::init(&root)?;

        // Create storage directory
        fs::create_dir_all(&storage)?;

        Ok(Self {
            _temp: temp,
            root,
            storage,
        })
    }

    /// Create a new test repository without git (DVS-only workspace).
    pub fn new_dvs_only() -> Result<Self, TestRepoError> {
        let temp = TempDir::new()?;
        let root = temp.path().to_path_buf();
        let storage = root.join(".dvs-storage");

        // Create .dvs directory to mark as DVS workspace
        fs::create_dir_all(root.join(".dvs"))?;

        // Create storage directory
        fs::create_dir_all(&storage)?;

        Ok(Self {
            _temp: temp,
            root,
            storage,
        })
    }

    /// Get the repository root path.
    pub fn root(&self) -> &Path {
        &self.root
    }

    /// Get the storage directory path.
    pub fn storage_dir(&self) -> &Path {
        &self.storage
    }

    /// Get the .dvs directory path.
    pub fn dvs_dir(&self) -> PathBuf {
        self.root.join(".dvs")
    }

    /// Get the config path (dvs.toml or dvs.yaml depending on features).
    pub fn config_path(&self) -> PathBuf {
        self.root.join(dvs_core::Config::config_filename())
    }

    /// Get the dvs.lock manifest path.
    pub fn manifest_path(&self) -> PathBuf {
        self.root.join("dvs.lock")
    }

    /// Write a file to the repository.
    pub fn write_file(&self, rel_path: &str, contents: &[u8]) -> Result<PathBuf, TestRepoError> {
        let path = self.root.join(rel_path);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::write(&path, contents)?;
        Ok(path)
    }

    /// Read a file from the repository.
    pub fn read_file(&self, rel_path: &str) -> Result<Vec<u8>, TestRepoError> {
        let path = self.root.join(rel_path);
        Ok(fs::read(&path)?)
    }

    /// Check if a file exists in the repository.
    pub fn file_exists(&self, rel_path: &str) -> bool {
        self.root.join(rel_path).exists()
    }

    /// Get the absolute path for a relative path.
    pub fn path(&self, rel_path: &str) -> PathBuf {
        self.root.join(rel_path)
    }

    /// Detect and return the backend for this repo.
    pub fn backend(&self) -> Backend {
        detect_backend(&self.root).unwrap_or_else(|_| {
            // Fallback to GitBackend if detection fails
            Backend::Git(GitBackend::new(self.root.clone()))
        })
    }

    /// List all files in the repository (excluding .git and .dvs-storage).
    pub fn list_files(&self) -> Result<Vec<PathBuf>, TestRepoError> {
        let mut files = Vec::new();
        self.list_files_recursive(&self.root, &mut files)?;
        Ok(files)
    }

    fn list_files_recursive(
        &self,
        dir: &Path,
        files: &mut Vec<PathBuf>,
    ) -> Result<(), TestRepoError> {
        for entry in fs::read_dir(dir)? {
            let entry = entry?;
            let path = entry.path();
            let name = path.file_name().unwrap_or_default().to_string_lossy();

            // Skip git and storage directories
            if name == ".git" || name == ".dvs-storage" {
                continue;
            }

            if path.is_dir() {
                self.list_files_recursive(&path, files)?;
            } else {
                // Store relative path
                if let Ok(rel) = path.strip_prefix(&self.root) {
                    files.push(rel.to_path_buf());
                }
            }
        }
        Ok(())
    }

    /// List all .dvs metadata files.
    pub fn list_metadata_files(&self) -> Result<Vec<PathBuf>, TestRepoError> {
        let files = self.list_files()?;
        Ok(files
            .into_iter()
            .filter(|p| p.extension().is_some_and(|e| e == "dvs"))
            .collect())
    }

    /// List all objects in the storage directory.
    pub fn list_storage_objects(&self) -> Result<Vec<PathBuf>, TestRepoError> {
        let mut objects = Vec::new();
        if self.storage.exists() {
            self.list_storage_recursive(&self.storage, &mut objects)?;
        }
        Ok(objects)
    }

    fn list_storage_recursive(
        &self,
        dir: &Path,
        objects: &mut Vec<PathBuf>,
    ) -> Result<(), TestRepoError> {
        for entry in fs::read_dir(dir)? {
            let entry = entry?;
            let path = entry.path();

            if path.is_dir() {
                self.list_storage_recursive(&path, objects)?;
            } else {
                // Store relative path from storage root
                if let Ok(rel) = path.strip_prefix(&self.storage) {
                    objects.push(rel.to_path_buf());
                }
            }
        }
        Ok(())
    }
}

/// Error type for TestRepo operations.
#[derive(Debug)]
pub enum TestRepoError {
    /// I/O error.
    Io(std::io::Error),
    /// Git error.
    Git(git2::Error),
}

impl std::fmt::Display for TestRepoError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            TestRepoError::Io(e) => write!(f, "I/O error: {}", e),
            TestRepoError::Git(e) => write!(f, "Git error: {}", e),
        }
    }
}

impl std::error::Error for TestRepoError {}

impl From<std::io::Error> for TestRepoError {
    fn from(e: std::io::Error) -> Self {
        TestRepoError::Io(e)
    }
}

impl From<git2::Error> for TestRepoError {
    fn from(e: git2::Error) -> Self {
        TestRepoError::Git(e)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_new_repo() {
        let repo = TestRepo::new().unwrap();
        assert!(repo.root().exists());
        assert!(repo.root().join(".git").exists());
        assert!(repo.storage_dir().exists());
    }

    #[test]
    fn test_new_dvs_only() {
        let repo = TestRepo::new_dvs_only().unwrap();
        assert!(repo.root().exists());
        assert!(!repo.root().join(".git").exists());
        assert!(repo.dvs_dir().exists());
        assert!(repo.storage_dir().exists());
    }

    #[test]
    fn test_write_read_file() {
        let repo = TestRepo::new().unwrap();
        let content = b"hello world";

        repo.write_file("test.txt", content).unwrap();
        assert!(repo.file_exists("test.txt"));

        let read = repo.read_file("test.txt").unwrap();
        assert_eq!(read, content);
    }

    #[test]
    fn test_write_nested_file() {
        let repo = TestRepo::new().unwrap();
        repo.write_file("a/b/c/test.txt", b"nested").unwrap();
        assert!(repo.file_exists("a/b/c/test.txt"));
    }

    #[test]
    fn test_list_files() {
        let repo = TestRepo::new().unwrap();
        repo.write_file("file1.txt", b"1").unwrap();
        repo.write_file("dir/file2.txt", b"2").unwrap();
        repo.write_file("file3.csv.dvs", b"{}").unwrap();

        let files = repo.list_files().unwrap();
        assert_eq!(files.len(), 3);
    }

    #[test]
    fn test_list_metadata_files() {
        let repo = TestRepo::new().unwrap();
        repo.write_file("data.csv", b"a,b").unwrap();
        repo.write_file("data.csv.dvs", b"{}").unwrap();
        repo.write_file("other.txt", b"text").unwrap();

        let meta = repo.list_metadata_files().unwrap();
        assert_eq!(meta.len(), 1);
        assert!(meta[0].to_string_lossy().contains("data.csv.dvs"));
    }
}
