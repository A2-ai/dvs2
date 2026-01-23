use std::path::{Path, PathBuf};

use anyhow::{Result, bail};
use fs_err as fs;
use serde::{Deserialize, Serialize};

fn get_path_in_dvs(dvs_directory: impl AsRef<Path>, relative_path: impl AsRef<Path>) -> PathBuf {
    let dvs_path = dvs_directory.as_ref().join(relative_path.as_ref());
    let dvs_filename = format!("{}.dvs", dvs_path.display());
    PathBuf::from(dvs_filename)
}

#[derive(Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct Hashes {
    blake3: String,
    md5: String,
}

impl From<Vec<u8>> for Hashes {
    fn from(bytes: Vec<u8>) -> Self {
        let blake3_hash = format!("{}", blake3::hash(&bytes));
        let md5_hash = format!("{:x}", md5::compute(&bytes));

        Self {
            blake3: blake3_hash,
            md5: md5_hash,
        }
    }
}

/// Outcome of an add or get operation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Outcome {
    /// File was copied to/from storage.
    Copied,
    /// File was already present (no action needed).
    Present,
}

#[derive(Debug, Copy, Clone, Eq, PartialEq)]
pub enum Status {
    /// Local file not tracked in dvs
    Untracked,
    /// Local file exists and matches stored version.
    Current,
    /// Metadata exists but local file is missing.
    Absent,
    /// Local file exists but differs from stored version.
    Unsynced,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct FileMetadata {
    hashes: Hashes,
    size: u64,
    created_by: String,
    add_time: String,
    message: Option<String>,
}

impl PartialEq for FileMetadata {
    fn eq(&self, other: &Self) -> bool {
        self.hashes == other.hashes && self.size == other.size
    }
}

impl FileMetadata {
    fn get_local_storage_location(&self, path: impl AsRef<Path>) -> PathBuf {
        let (a, b) = self.hashes.md5.split_at(2);
        path.as_ref().join(a).join(b)
    }

    pub fn from_file(path: impl AsRef<Path>, message: Option<String>) -> Result<Self> {
        if !path.as_ref().is_file() {
            bail!("Path {} is not a file", path.as_ref().display());
        }

        let content = fs::read(path)?;
        let size = content.len() as u64;
        let hashes = Hashes::from(content);
        let created_by = whoami::username()?;
        let add_time = jiff::Zoned::now().to_string();

        Ok(Self {
            hashes,
            size,
            created_by,
            add_time,
            message,
        })
    }

    /// Returns whether the file already existed in the dvs folder and therefore is an update.
    /// Copies the source file to storage and saves metadata atomically (both succeed or neither).
    pub fn save_local(
        &self,
        source_file: impl AsRef<Path>,
        storage_root: impl AsRef<Path>,
        dvs_directory: impl AsRef<Path>,
        relative_path: impl AsRef<Path>,
    ) -> Result<Outcome> {
        let dvs_file_path = get_path_in_dvs(&dvs_directory, &relative_path);
        let dvs_file_exists = dvs_file_path.is_file();
        let storage_path = self.get_local_storage_location(storage_root.as_ref());
        let storage_exists = storage_path.exists();

        if dvs_file_exists && storage_exists {
            log::info!("File {} is already in sync", storage_path.display());
            return Ok(Outcome::Present);
        }

        // We do an atomic update, either everything works or we error
        // 1. Create the dirs first
        if let Some(parent) = storage_path.parent() {
            fs::create_dir_all(parent)?;
        }
        if let Some(parent) = dvs_file_path.parent() {
            fs::create_dir_all(parent)?;
        }

        // 2. Copy file to storage first
        let old_storage_content = fs::read(&storage_path).ok();
        let storage_res = fs::copy(&source_file, &storage_path);

        // 3. Then metadata
        let old_metadata_content = fs::read(&dvs_file_path).ok();
        let metadata_res = fs::write(
            &dvs_file_path,
            serde_json::to_string(self).expect("valid json"),
        );

        match (storage_res, metadata_res) {
            (Ok(_), Ok(_)) => Ok(Outcome::Copied),
            (Err(e), Ok(_)) => {
                if let Some(old) = old_metadata_content {
                    fs::write(&dvs_file_path, &old)?;
                } else {
                    fs::remove_file(&dvs_file_path)?;
                }
                Err(e.into())
            }
            (Ok(_), Err(_)) => {
                if let Some(old) = old_storage_content {
                    fs::write(&storage_path, &old)?;
                } else {
                    fs::remove_file(&storage_path)?;
                }
                bail!("Failed to write metadata file: {dvs_file_path:?}")
            }
            (Err(e), Err(_)) => {
                if let Some(old) = old_metadata_content {
                    fs::write(&dvs_file_path, &old)?;
                } else {
                    fs::remove_file(&dvs_file_path)?;
                }
                if let Some(old) = old_storage_content {
                    fs::write(&storage_path, &old)?;
                } else {
                    fs::remove_file(&storage_path)?;
                }
                bail!(
                    "Failed to write metadata file: {dvs_file_path:?} and file {storage_path:?}: {e}"
                )
            }
        }
    }
}

#[derive(Debug)]
pub struct FileStatus {
    pub path: PathBuf,
    pub status: Status,
}

pub fn get_file_status(
    repo_root: impl AsRef<Path>,
    dvs_directory: impl AsRef<Path>,
    relative_path: impl AsRef<Path>,
) -> Result<Status> {
    let dvs_file_path = get_path_in_dvs(&dvs_directory, &relative_path);
    if !dvs_file_path.is_file() {
        return Ok(Status::Untracked);
    }
    let existing_metadata: FileMetadata = serde_json::from_reader(fs::File::open(dvs_file_path)?)?;
    // If we have read the metadata, but we can't find the original file
    let file_path = repo_root.as_ref().join(relative_path.as_ref());
    if !file_path.is_file() {
        return Ok(Status::Absent);
    }
    let current_metadata = FileMetadata::from_file(&file_path, None)?;
    if existing_metadata == current_metadata {
        Ok(Status::Current)
    } else {
        Ok(Status::Unsynced)
    }
}

pub fn get_status(
    repo_root: impl AsRef<Path>,
    dvs_directory: impl AsRef<Path>,
) -> Result<Vec<FileStatus>> {
    let pattern = format!("{}/**/*.dvs", dvs_directory.as_ref().display());
    let mut results = Vec::new();
    for entry in glob::glob(&pattern)? {
        let dvs_path = entry?;
        // Strip dvs_directory prefix and .dvs suffix to get relative path
        let relative = dvs_path
            .strip_prefix(dvs_directory.as_ref())?
            .with_extension("");
        let status = get_file_status(&repo_root, &dvs_directory, &relative)?;
        results.push(FileStatus {
            path: relative.to_path_buf(),
            status,
        });
    }
    Ok(results)
}

/// Retrieves a file from local storage to the target path.
/// Returns `Outcome::Present` if file already exists and matches, `Outcome::Copied` if copied.
pub fn get_file(
    storage_root: impl AsRef<Path>,
    dvs_directory: impl AsRef<Path>,
    repo_root: impl AsRef<Path>,
    relative_path: impl AsRef<Path>,
) -> Result<Outcome> {
    let dvs_file_path = get_path_in_dvs(&dvs_directory, &relative_path);
    if !dvs_file_path.is_file() {
        bail!(
            "File {} is not tracked by DVS",
            relative_path.as_ref().display()
        );
    }

    let metadata: FileMetadata = serde_json::from_reader(fs::File::open(&dvs_file_path)?)?;
    let storage_path = metadata.get_local_storage_location(storage_root.as_ref());

    if !storage_path.is_file() {
        bail!("Storage file missing: {}", storage_path.display());
    }

    let target_path = repo_root.as_ref().join(relative_path.as_ref());

    // Check if target already exists and matches
    if target_path.is_file() {
        let current = FileMetadata::from_file(&target_path, None)?;
        if current == metadata {
            return Ok(Outcome::Present);
        }
    }

    // Create parent directories if needed
    if let Some(parent) = target_path.parent() {
        fs::create_dir_all(parent)?;
    }

    fs::copy(&storage_path, &target_path)?;
    Ok(Outcome::Copied)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testutil::{create_file, create_temp_git_repo, init_dvs_repo};

    #[test]
    fn file_metadata_from_file_creates_hashes_and_message() {
        let (_tmp, root) = create_temp_git_repo();
        let file_path = create_file(&root, "test.txt", b"hello world");

        let metadata =
            FileMetadata::from_file(&file_path, Some("test message".to_string())).unwrap();

        assert_eq!(metadata.hashes.blake3.len(), 64);
        assert_eq!(metadata.hashes.md5, "5eb63bbbe01eeed093cb22bb8f5acdc3");
        assert_eq!(metadata.size, 11);
        assert_eq!(metadata.message, Some("test message".to_string()));
    }

    #[test]
    fn file_metadata_from_nonexistent_file_fails() {
        let tmp = tempfile::tempdir().unwrap();
        let result = FileMetadata::from_file(tmp.path().join("nonexistent.txt"), None);
        assert!(result.is_err());
    }

    #[test]
    fn save_local_creates_storage_and_metadata() {
        let (_tmp, root) = create_temp_git_repo();
        let (storage_dir, dvs_dir) = init_dvs_repo(&root);
        let file_path = create_file(&root, "data.bin", b"binary data");

        let metadata = FileMetadata::from_file(&file_path, None).unwrap();
        let outcome = metadata
            .save_local(&file_path, &storage_dir, &dvs_dir, "data.bin")
            .unwrap();

        assert_eq!(outcome, Outcome::Copied);
        // Metadata file should exist
        assert!(dvs_dir.join("data.bin.dvs").is_file());
        // Storage file should exist (md5 prefix/suffix structure)
        let (prefix, suffix) = metadata.hashes.md5.split_at(2);
        assert!(storage_dir.join(prefix).join(suffix).is_file());
    }

    #[test]
    fn save_local_returns_present_when_already_stored() {
        let (_tmp, root) = create_temp_git_repo();
        let (storage_dir, dvs_dir) = init_dvs_repo(&root);
        let file_path = create_file(&root, "data.bin", b"binary data");

        let metadata = FileMetadata::from_file(&file_path, None).unwrap();
        metadata
            .save_local(&file_path, &storage_dir, &dvs_dir, "data.bin")
            .unwrap();

        // Second save should return Present
        let outcome = metadata
            .save_local(&file_path, &storage_dir, &dvs_dir, "data.bin")
            .unwrap();
        assert_eq!(outcome, Outcome::Present);
    }

    #[test]
    fn get_file_status_returns_untracked_for_new_file() {
        let (_tmp, root) = create_temp_git_repo();
        let (_storage_dir, dvs_dir) = init_dvs_repo(&root);
        create_file(&root, "new.txt", b"content");

        let status = get_file_status(&root, &dvs_dir, "new.txt").unwrap();
        assert_eq!(status, Status::Untracked);
    }

    #[test]
    fn get_file_status_returns_current_for_synced_file() {
        let (_tmp, root) = create_temp_git_repo();
        let (storage_dir, dvs_dir) = init_dvs_repo(&root);
        let file_path = create_file(&root, "synced.txt", b"content");

        let metadata = FileMetadata::from_file(&file_path, None).unwrap();
        metadata
            .save_local(&file_path, &storage_dir, &dvs_dir, "synced.txt")
            .unwrap();

        let status = get_file_status(&root, &dvs_dir, "synced.txt").unwrap();
        assert_eq!(status, Status::Current);
    }

    #[test]
    fn get_file_status_returns_absent_when_file_deleted() {
        let (_tmp, root) = create_temp_git_repo();
        let (storage_dir, dvs_dir) = init_dvs_repo(&root);
        let file_path = create_file(&root, "deleted.txt", b"content");

        let metadata = FileMetadata::from_file(&file_path, None).unwrap();
        metadata
            .save_local(&file_path, &storage_dir, &dvs_dir, "deleted.txt")
            .unwrap();

        // Delete the original file
        fs::remove_file(&file_path).unwrap();

        let status = get_file_status(&root, &dvs_dir, "deleted.txt").unwrap();
        assert_eq!(status, Status::Absent);
    }

    #[test]
    fn get_file_status_returns_unsynced_when_file_modified() {
        let (_tmp, root) = create_temp_git_repo();
        let (storage_dir, dvs_dir) = init_dvs_repo(&root);
        let file_path = create_file(&root, "modified.txt", b"original");

        let metadata = FileMetadata::from_file(&file_path, None).unwrap();
        metadata
            .save_local(&file_path, &storage_dir, &dvs_dir, "modified.txt")
            .unwrap();

        // Modify the file
        fs::write(&file_path, b"changed content").unwrap();

        let status = get_file_status(&root, &dvs_dir, "modified.txt").unwrap();
        assert_eq!(status, Status::Unsynced);
    }

    #[test]
    fn get_file_retrieves_from_storage() {
        let (_tmp, root) = create_temp_git_repo();
        let (storage_dir, dvs_dir) = init_dvs_repo(&root);
        let file_path = create_file(&root, "retrieve.txt", b"stored content");

        let metadata = FileMetadata::from_file(&file_path, None).unwrap();
        metadata
            .save_local(&file_path, &storage_dir, &dvs_dir, "retrieve.txt")
            .unwrap();

        // Delete the original file
        fs::remove_file(&file_path).unwrap();
        assert!(!file_path.exists());

        // Retrieve it
        let outcome = get_file(&storage_dir, &dvs_dir, &root, "retrieve.txt").unwrap();
        assert_eq!(outcome, Outcome::Copied);
        assert!(file_path.exists());
        assert_eq!(fs::read(&file_path).unwrap(), b"stored content");
    }

    #[test]
    fn get_file_returns_present_when_already_current() {
        let (_tmp, root) = create_temp_git_repo();
        let (storage_dir, dvs_dir) = init_dvs_repo(&root);
        let file_path = create_file(&root, "present.txt", b"content");

        let metadata = FileMetadata::from_file(&file_path, None).unwrap();
        metadata
            .save_local(&file_path, &storage_dir, &dvs_dir, "present.txt")
            .unwrap();

        // File still exists and matches - should return Present
        let outcome = get_file(&storage_dir, &dvs_dir, &root, "present.txt").unwrap();
        assert_eq!(outcome, Outcome::Present);
    }

    #[test]
    fn get_file_fails_for_untracked_file() {
        let (_tmp, root) = create_temp_git_repo();
        let (storage_dir, dvs_dir) = init_dvs_repo(&root);

        let result = get_file(&storage_dir, &dvs_dir, &root, "untracked.txt");
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("not tracked"));
    }

    #[test]
    fn get_status_returns_all_tracked_files() {
        let (_tmp, root) = create_temp_git_repo();
        let (storage_dir, dvs_dir) = init_dvs_repo(&root);

        // Add multiple files
        for name in ["a.txt", "b.txt", "subdir/c.txt"] {
            let file_path = create_file(&root, name, name.as_bytes());
            let metadata = FileMetadata::from_file(&file_path, None).unwrap();
            metadata
                .save_local(&file_path, &storage_dir, &dvs_dir, name)
                .unwrap();
        }

        let statuses = get_status(&root, &dvs_dir).unwrap();
        assert_eq!(statuses.len(), 3);

        // All should be Current
        for status in &statuses {
            assert_eq!(status.status, Status::Current);
        }
    }
}
