use std::path::{Path, PathBuf};

use crate::audit::{AuditEntry, AuditFile};
use crate::backends::Backend;
use crate::hashes::Hashes;
use crate::paths::DvsPaths;
use anyhow::{Context, Result, bail};
use fs_err as fs;
use serde::{Deserialize, Serialize};
use uuid::Uuid;
use walkdir::WalkDir;

/// Outcome of an add or get operation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Outcome {
    /// File was copied to/from storage.
    Copied,
    /// File was already present (no action needed).
    Present,
}

#[derive(Debug, Copy, Clone, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
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

/// The dvs metadata for a given file
#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct FileMetadata {
    pub hashes: Hashes,
    pub size: u64,
    pub created_by: String,
    pub add_time: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
}

impl PartialEq for FileMetadata {
    fn eq(&self, other: &Self) -> bool {
        self.hashes == other.hashes && self.size == other.size
    }
}

impl FileMetadata {
    pub fn from_file(path: impl AsRef<Path>, message: Option<String>) -> Result<Self> {
        if !path.as_ref().is_file() {
            bail!("Path {} is not a file", path.as_ref().display());
        }

        let content = fs::read(path.as_ref())?;
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
    pub fn save(
        &self,
        operation_id: Uuid,
        source_file: impl AsRef<Path>,
        backend: &dyn Backend,
        paths: &DvsPaths,
        relative_path: impl AsRef<Path>,
    ) -> Result<Outcome> {
        let dvs_file_path = paths.metadata_path(relative_path.as_ref());
        let dvs_file_exists = dvs_file_path.is_file();
        let storage_exists = backend.exists(&self.hashes)?;

        log::debug!(
            "Saving {}: metadata_exists={}, storage_exists={}",
            relative_path.as_ref().display(),
            dvs_file_exists,
            storage_exists
        );

        if dvs_file_exists && storage_exists {
            // we read the file anyway to make sure it's not 2 files having the same hash
            let existing: FileMetadata = serde_json::from_reader(fs::File::open(&dvs_file_path)?)?;
            if existing == *self {
                log::debug!(
                    "File {} is already in sync",
                    relative_path.as_ref().display()
                );
                return Ok(Outcome::Present);
            }
        }

        // We do an atomic update, either everything works or we error
        // 1. Create metadata dirs first
        if let Some(parent) = dvs_file_path.parent() {
            fs::create_dir_all(parent)?;
        }

        // 2. Read old storage content for rollback (if any)
        let old_storage_content = backend.read(&self.hashes)?;

        // 3. Store file to backend
        let storage_res = backend.store(&self.hashes, source_file.as_ref());

        // 4. Then metadata
        let old_metadata_content = fs::read(&dvs_file_path).ok();
        log::debug!("Writing metadata to {}", dvs_file_path.display());
        let metadata_res = fs::write(
            &dvs_file_path,
            serde_json::to_string_pretty(self).expect("valid json"),
        );

        match (storage_res, metadata_res) {
            (Ok(_), Ok(_)) => {
                let audit_entry = AuditEntry::new_add(
                    operation_id,
                    AuditFile {
                        path: relative_path.as_ref().to_path_buf(),
                        hashes: self.hashes.clone(),
                    },
                );
                if let Err(e) = backend.log_audit(&audit_entry) {
                    log::error!("Failed to write audit log {audit_entry:?}: {e}");
                }
                Ok(Outcome::Copied)
            }
            (Err(e), Ok(_)) => {
                log::warn!(
                    "Storage failed, rolling back metadata for {}",
                    relative_path.as_ref().display()
                );
                if let Some(old) = old_metadata_content {
                    fs::write(&dvs_file_path, &old)?;
                } else {
                    fs::remove_file(&dvs_file_path)?;
                }
                Err(e)
            }
            (Ok(_), Err(_)) => {
                log::warn!(
                    "Metadata write failed, rolling back storage for {}",
                    relative_path.as_ref().display()
                );
                if let Some(old) = old_storage_content {
                    backend.store_bytes(&self.hashes, &old)?;
                } else {
                    backend.remove(&self.hashes)?;
                }
                bail!("Failed to write metadata file: {dvs_file_path:?}")
            }
            (Err(e), Err(_)) => {
                log::warn!(
                    "Both storage and metadata failed, rolling back for {}",
                    relative_path.as_ref().display()
                );
                if let Some(old) = old_metadata_content {
                    fs::write(&dvs_file_path, &old)?;
                } else {
                    fs::remove_file(&dvs_file_path)?;
                }
                if let Some(old) = old_storage_content {
                    backend.store_bytes(&self.hashes, &old)?;
                } else {
                    backend.remove(&self.hashes)?;
                }
                bail!("Failed to write metadata file: {dvs_file_path:?}: {e}")
            }
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FileStatus {
    pub path: PathBuf,
    pub status: Status,
}

fn get_file_status(paths: &DvsPaths, relative_path: impl AsRef<Path>) -> Result<Status> {
    let dvs_file_path = paths.metadata_path(relative_path.as_ref());
    if !dvs_file_path.is_file() {
        return Ok(Status::Untracked);
    }
    let existing_metadata: FileMetadata = serde_json::from_reader(fs::File::open(dvs_file_path)?)?;
    // If we have read the metadata, but we can't find the original file
    let file_path = paths.file_path(relative_path.as_ref());
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

pub fn get_status(paths: &DvsPaths) -> Result<Vec<FileStatus>> {
    let dvs_directory = paths.metadata_folder();
    log::debug!("Scanning metadata folder: {}", dvs_directory.display());
    let mut results = Vec::new();
    for entry in WalkDir::new(&dvs_directory)
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
        let dvs_path = entry.path();
        // Strip dvs_directory prefix and .dvs suffix to get relative path
        let relative = dvs_path.strip_prefix(&dvs_directory)?.with_extension("");
        let status = get_file_status(paths, &relative)?;
        results.push(FileStatus {
            path: relative.to_path_buf(),
            status,
        });
    }
    log::debug!("Found {} tracked files", results.len());
    Ok(results)
}

fn get_file(
    backend: &dyn Backend,
    paths: &DvsPaths,
    relative_path: impl AsRef<Path>,
) -> Result<Outcome> {
    log::debug!("Retrieving file: {}", relative_path.as_ref().display());
    let dvs_file_path = paths.metadata_path(relative_path.as_ref());
    if !dvs_file_path.is_file() {
        bail!(
            "File {} is not tracked by DVS",
            relative_path.as_ref().display()
        );
    }

    let metadata: FileMetadata = serde_json::from_reader(fs::File::open(&dvs_file_path)?)?;
    log::debug!(
        "Read metadata for {}: {}",
        relative_path.as_ref().display(),
        metadata.hashes
    );

    if !backend.exists(&metadata.hashes)? {
        bail!("Storage file missing for hash: {}", metadata.hashes);
    }

    let target_path = paths.file_path(relative_path.as_ref());

    // Check if target already exists and matches
    if target_path.is_file() {
        let current = FileMetadata::from_file(&target_path, None)?;
        if current == metadata {
            log::debug!(
                "File {} already present locally and matches",
                relative_path.as_ref().display()
            );
            return Ok(Outcome::Present);
        }
    }

    // Retrieve from backend to target path
    log::debug!(
        "Copying {} from storage to {}",
        metadata.hashes,
        target_path.display()
    );
    backend
        .retrieve(&metadata.hashes, &target_path)
        .with_context(|| format!("Failed to retrieve {}", relative_path.as_ref().display()))?;
    let actual = FileMetadata::from_file(&target_path, None)?;
    if actual.hashes != metadata.hashes {
        fs::remove_file(&target_path)?;
        bail!("Retrieved file does not match expected hash");
    }
    Ok(Outcome::Copied)
}

/// Result of adding a single file.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AddResult {
    pub path: PathBuf,
    pub outcome: Outcome,
}

/// Result of getting a single file.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GetResult {
    pub path: PathBuf,
    pub outcome: Outcome,
}

/// Adds files matching a glob pattern to DVS.
///
/// The pattern is matched against files relative to cwd.
/// Files are stored with paths relative to repo_root.
pub fn add_files(
    files: Vec<PathBuf>,
    paths: &DvsPaths,
    backend: &dyn Backend,
    message: Option<String>,
) -> Result<Vec<AddResult>> {
    let matched_paths = paths.validate_for_add(&files);
    let missing: Vec<_> = matched_paths
        .iter()
        .filter(|(_, exists)| !*exists)
        .map(|(p, _)| p.display().to_string())
        .collect();
    if !missing.is_empty() {
        bail!("The following files were not found: {}", missing.join(", "));
    }

    let mut results = Vec::new();
    let operation_id = Uuid::new_v4();

    for (relative_path, _) in matched_paths {
        let full_path = paths.file_path(&relative_path);

        let metadata = FileMetadata::from_file(&full_path, message.clone())?;
        let outcome = metadata.save(operation_id, &full_path, backend, paths, &relative_path)?;
        log::info!(
            "Successfully added {} ({:?})",
            relative_path.display(),
            outcome
        );
        results.push(AddResult {
            path: relative_path,
            outcome,
        });
    }

    Ok(results)
}

/// Gets files matching a glob pattern from DVS storage.
///
/// The pattern is matched against tracked files (paths in metadata folder).
/// The pattern is adjusted based on cwd relative to repo root.
pub fn get_files(
    files: Vec<PathBuf>,
    paths: &DvsPaths,
    backend: &dyn Backend,
) -> Result<Vec<GetResult>> {
    let matched_paths = paths.validate_for_get(&files);
    let missing: Vec<_> = matched_paths
        .iter()
        .filter(|(_, exists)| !*exists)
        .map(|(p, _)| p.display().to_string())
        .collect();
    if !missing.is_empty() {
        bail!("The following files were not found: {}", missing.join(", "));
    }

    let mut results = Vec::new();

    for (relative_path, _) in matched_paths {
        let outcome = get_file(backend, paths, &relative_path)?;
        log::info!(
            "Successfully retrieved {} ({:?})",
            relative_path.display(),
            outcome
        );
        results.push(GetResult {
            path: relative_path,
            outcome,
        });
    }

    Ok(results)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testutil::{create_file, create_temp_git_repo, init_dvs_repo};

    fn make_paths(root: &Path, config: &crate::config::Config) -> DvsPaths {
        DvsPaths::new(
            root.to_path_buf(),
            root.to_path_buf(),
            config.metadata_folder_name(),
        )
    }

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
        let (config, dvs_dir) = init_dvs_repo(&root);
        let backend = config.backend();
        let paths = make_paths(&root, &config);
        let file_path = create_file(&root, "data.bin", b"binary data");

        let metadata = FileMetadata::from_file(&file_path, None).unwrap();
        let outcome = metadata
            .save(Uuid::new_v4(), &file_path, backend, &paths, "data.bin")
            .unwrap();

        assert_eq!(outcome, Outcome::Copied);
        // Metadata file should exist
        assert!(dvs_dir.join("data.bin.dvs").is_file());
        assert!(backend.exists(&metadata.hashes).unwrap());
    }

    #[test]
    fn save_local_returns_present_when_already_stored() {
        let (_tmp, root) = create_temp_git_repo();
        let (config, _dvs_dir) = init_dvs_repo(&root);
        let backend = config.backend();
        let paths = make_paths(&root, &config);
        let file_path = create_file(&root, "data.bin", b"binary data");

        let metadata = FileMetadata::from_file(&file_path, None).unwrap();
        metadata
            .save(Uuid::new_v4(), &file_path, backend, &paths, "data.bin")
            .unwrap();

        // Second save should return Present
        let outcome = metadata
            .save(Uuid::new_v4(), &file_path, backend, &paths, "data.bin")
            .unwrap();
        assert_eq!(outcome, Outcome::Present);
    }

    #[test]
    fn get_file_status_returns_untracked_for_new_file() {
        let (_tmp, root) = create_temp_git_repo();
        let (config, _dvs_dir) = init_dvs_repo(&root);
        let paths = make_paths(&root, &config);
        create_file(&root, "new.txt", b"content");

        let status = get_file_status(&paths, "new.txt").unwrap();
        assert_eq!(status, Status::Untracked);
    }

    #[test]
    fn get_file_status_returns_current_for_synced_file() {
        let (_tmp, root) = create_temp_git_repo();
        let (config, _dvs_dir) = init_dvs_repo(&root);
        let backend = config.backend();
        let paths = make_paths(&root, &config);
        let file_path = create_file(&root, "synced.txt", b"content");

        let metadata = FileMetadata::from_file(&file_path, None).unwrap();
        metadata
            .save(Uuid::new_v4(), &file_path, backend, &paths, "synced.txt")
            .unwrap();

        let status = get_file_status(&paths, "synced.txt").unwrap();
        assert_eq!(status, Status::Current);
    }

    #[test]
    fn get_file_status_returns_absent_when_file_deleted() {
        let (_tmp, root) = create_temp_git_repo();
        let (config, _dvs_dir) = init_dvs_repo(&root);
        let backend = config.backend();
        let paths = make_paths(&root, &config);
        let file_path = create_file(&root, "deleted.txt", b"content");

        let metadata = FileMetadata::from_file(&file_path, None).unwrap();
        metadata
            .save(Uuid::new_v4(), &file_path, backend, &paths, "deleted.txt")
            .unwrap();

        // Delete the original file
        fs::remove_file(&file_path).unwrap();

        let status = get_file_status(&paths, "deleted.txt").unwrap();
        assert_eq!(status, Status::Absent);
    }

    #[test]
    fn get_file_status_returns_unsynced_when_file_modified() {
        let (_tmp, root) = create_temp_git_repo();
        let (config, _dvs_dir) = init_dvs_repo(&root);
        let backend = config.backend();
        let paths = make_paths(&root, &config);
        let file_path = create_file(&root, "modified.txt", b"original");

        let metadata = FileMetadata::from_file(&file_path, None).unwrap();
        metadata
            .save(Uuid::new_v4(), &file_path, backend, &paths, "modified.txt")
            .unwrap();

        // Modify the file
        fs::write(&file_path, b"changed content").unwrap();

        let status = get_file_status(&paths, "modified.txt").unwrap();
        assert_eq!(status, Status::Unsynced);
    }

    #[test]
    fn get_file_retrieves_from_storage() {
        let (_tmp, root) = create_temp_git_repo();
        let (config, _dvs_dir) = init_dvs_repo(&root);
        let backend = config.backend();
        let paths = make_paths(&root, &config);
        let file_path = create_file(&root, "retrieve.txt", b"stored content");

        let metadata = FileMetadata::from_file(&file_path, None).unwrap();
        metadata
            .save(Uuid::new_v4(), &file_path, backend, &paths, "retrieve.txt")
            .unwrap();

        // Delete the original file
        fs::remove_file(&file_path).unwrap();
        assert!(!file_path.exists());

        // Retrieve it
        let outcome = get_file(backend, &paths, "retrieve.txt").unwrap();
        assert_eq!(outcome, Outcome::Copied);
        assert!(file_path.exists());
        assert_eq!(fs::read(&file_path).unwrap(), b"stored content");
    }

    #[test]
    fn get_file_returns_present_when_already_current() {
        let (_tmp, root) = create_temp_git_repo();
        let (config, _dvs_dir) = init_dvs_repo(&root);
        let backend = config.backend();
        let paths = make_paths(&root, &config);
        let file_path = create_file(&root, "present.txt", b"content");

        let metadata = FileMetadata::from_file(&file_path, None).unwrap();
        metadata
            .save(Uuid::new_v4(), &file_path, backend, &paths, "present.txt")
            .unwrap();

        // File still exists and matches - should return Present
        let outcome = get_file(backend, &paths, "present.txt").unwrap();
        assert_eq!(outcome, Outcome::Present);
    }

    #[test]
    fn get_file_fails_for_untracked_file() {
        let (_tmp, root) = create_temp_git_repo();
        let (config, _dvs_dir) = init_dvs_repo(&root);
        let backend = config.backend();
        let paths = make_paths(&root, &config);

        let result = get_file(backend, &paths, "untracked.txt");
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("not tracked"));
    }

    #[test]
    fn get_status_returns_all_tracked_files() {
        let (_tmp, root) = create_temp_git_repo();
        let (config, _dvs_dir) = init_dvs_repo(&root);
        let backend = config.backend();
        let paths = make_paths(&root, &config);

        // Add multiple files
        for name in ["a.txt", "b.txt", "subdir/c.txt"] {
            let file_path = create_file(&root, name, name.as_bytes());
            let metadata = FileMetadata::from_file(&file_path, None).unwrap();
            metadata
                .save(Uuid::new_v4(), &file_path, backend, &paths, name)
                .unwrap();
        }

        let statuses = get_status(&paths).unwrap();
        assert_eq!(statuses.len(), 3);

        // All should be Current
        for status in &statuses {
            assert_eq!(status.status, Status::Current);
        }
    }

    #[test]
    fn add_files_errors_when_not_found() {
        let (_tmp, root) = create_temp_git_repo();
        let (config, _dvs_dir) = init_dvs_repo(&root);
        let backend = config.backend();
        let paths = make_paths(&root, &config);

        create_file(&root, "a.txt", b"a");

        let result = add_files(vec!["nonexistent.csv".into()], &paths, backend, None);
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("not found"));
    }

    #[test]
    fn get_files_errors_when_not_found() {
        let (_tmp, root) = create_temp_git_repo();
        let (config, _dvs_dir) = init_dvs_repo(&root);
        let backend = config.backend();
        let paths = make_paths(&root, &config);

        create_file(&root, "a.txt", b"a");
        add_files(vec!["a.txt".into()], &paths, backend, None).unwrap();

        let result = get_files(vec!["nonexistent.csv".into()], &paths, backend);
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("not found"));
    }

    fn run_add_get_roundtrip(file_paths: Vec<PathBuf>, expected_files: &[&str]) {
        let (_tmp, root) = create_temp_git_repo();
        let (config, _dvs_dir) = init_dvs_repo(&root);
        let backend = config.backend();
        let paths = make_paths(&root, &config);

        create_file(&root, "a.txt", b"a");
        create_file(&root, "b.txt", b"b");
        create_file(&root, "c.csv", b"c");

        // Add files
        let results = add_files(file_paths.clone(), &paths, backend, None).unwrap();
        assert_eq!(results.len(), expected_files.len());
        for result in &results {
            assert_eq!(result.outcome, Outcome::Copied);
        }

        // Verify correct files are tracked
        let statuses = get_status(&paths).unwrap();
        assert_eq!(statuses.len(), expected_files.len());
        let tracked_names: Vec<_> = statuses.iter().map(|s| s.path.to_str().unwrap()).collect();
        for expected in expected_files {
            assert!(
                tracked_names.contains(expected),
                "Expected {expected} to be tracked"
            );
        }

        // Delete tracked files
        for name in expected_files {
            fs::remove_file(root.join(name)).unwrap();
        }

        // Get files back
        let results = get_files(file_paths, &paths, backend).unwrap();
        assert_eq!(results.len(), expected_files.len());
        for result in &results {
            assert_eq!(result.outcome, Outcome::Copied);
        }

        // Verify files restored
        for name in expected_files {
            assert!(root.join(name).exists(), "Expected {name} to be restored");
        }
    }

    #[test]
    fn add_get_roundtrip_with_explicit_paths() {
        let paths: Vec<PathBuf> = vec!["a.txt".into(), "c.csv".into()];
        run_add_get_roundtrip(paths, &["a.txt", "c.csv"]);
    }

    #[test]
    fn save_local_updates_metadata_when_content_matches_different_file() {
        // - Add file A with content "foo" (hash H1)
        // - Add file B with content "bar" (hash H2)
        // - Change file B's content to "foo" (now hash H1)
        // - Run `add` on B
        // => B's metadata is updated to hash H1
        let (_tmp, root) = create_temp_git_repo();
        let (config, dvs_dir) = init_dvs_repo(&root);
        let backend = config.backend();
        let paths = make_paths(&root, &config);

        // Add file A with content "foo" (hash H1)
        let file_a = create_file(&root, "a.txt", b"foo");
        let metadata_a = FileMetadata::from_file(&file_a, None).unwrap();
        metadata_a
            .save(Uuid::new_v4(), &file_a, backend, &paths, "a.txt")
            .unwrap();
        let hash_h1 = metadata_a.hashes.md5.clone();

        // Add file B with content "bar" (hash H2)
        let file_b = create_file(&root, "b.txt", b"bar");
        let metadata_b = FileMetadata::from_file(&file_b, None).unwrap();
        metadata_b
            .save(Uuid::new_v4(), &file_b, backend, &paths, "b.txt")
            .unwrap();
        let hash_h2 = metadata_b.hashes.md5.clone();
        assert_ne!(hash_h1, hash_h2);

        // Change file B's content to "foo" (now hash H1)
        fs::write(&file_b, b"foo").unwrap();

        // Run add on B with new content
        let metadata_b_new = FileMetadata::from_file(&file_b, None).unwrap();
        assert_eq!(metadata_b_new.hashes.md5, hash_h1);

        metadata_b_new
            .save(Uuid::new_v4(), &file_b, backend, &paths, "b.txt")
            .unwrap();

        // Verify metadata was updated
        let dvs_file = dvs_dir.join("b.txt.dvs");
        let stored: FileMetadata =
            serde_json::from_reader(fs::File::open(&dvs_file).unwrap()).unwrap();

        assert_eq!(
            stored.hashes.md5, hash_h1,
            "Metadata should be updated to new hash"
        );

        let status = get_file_status(&paths, "b.txt").unwrap();
        assert_eq!(status, Status::Current);
    }

    #[test]
    fn get_file_errors_on_corrupted_storage() {
        let (_tmp, root) = create_temp_git_repo();
        let (config, _dvs_dir) = init_dvs_repo(&root);
        let backend = config.backend();
        let paths = make_paths(&root, &config);

        // Add a file
        let file_path = create_file(&root, "data.txt", b"original content");
        let metadata = FileMetadata::from_file(&file_path, None).unwrap();
        metadata
            .save(Uuid::new_v4(), &file_path, backend, &paths, "data.txt")
            .unwrap();

        // Delete the local file
        fs::remove_file(&file_path).unwrap();

        // Corrupt the storage file
        let storage_path = root
            .join(".storage")
            .join(&metadata.hashes.blake3[..2])
            .join(&metadata.hashes.blake3[2..]);
        fs::write(&storage_path, b"corrupted content").unwrap();

        // get_file should error on hash mismatch
        let result = get_file(backend, &paths, "data.txt");
        assert!(result.is_err());
    }
}
