use std::path::Path;

use anyhow::{Result, bail};
use fs_err as fs;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::audit::{AuditEntry, AuditFile};
use crate::{Backend, Compression, DvsPaths, Hashes, Outcome};

/// The dvs metadata for a given file
#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct FileMetadata {
    pub hashes: Hashes,
    pub size: u64,
    pub created_by: String,
    pub add_time: String,
    pub compression: Compression,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
}

impl PartialEq for FileMetadata {
    fn eq(&self, other: &Self) -> bool {
        self.hashes == other.hashes && self.size == other.size
    }
}

impl FileMetadata {
    pub fn from_hashes(
        hashes: Hashes,
        size: u64,
        compression: Compression,
        message: Option<String>,
    ) -> Self {
        Self {
            hashes,
            size,
            created_by: whoami::username().unwrap_or_default(),
            add_time: jiff::Timestamp::now().to_string(),
            compression,
            message,
        }
    }

    pub fn from_file(
        path: impl AsRef<Path>,
        compression: Compression,
        message: Option<String>,
    ) -> Result<Self> {
        if !path.as_ref().is_file() {
            bail!("Path {} is not a file", path.as_ref().display());
        }

        let (hashes, size) = Hashes::compute_from_path(path.as_ref(), &[])?;
        let created_by = whoami::username()?;
        let add_time = jiff::Timestamp::now().to_string();

        Ok(Self {
            hashes,
            size,
            created_by,
            add_time,
            message,
            compression,
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
        let storage_res = backend.store(&self.hashes, source_file.as_ref(), self.compression);

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

        let metadata = FileMetadata::from_file(
            &file_path,
            Compression::Zstd,
            Some("test message".to_string()),
        )
        .unwrap();

        assert_eq!(metadata.hashes.blake3.len(), 64);
        assert_eq!(metadata.size, 11);
        assert_eq!(metadata.message, Some("test message".to_string()));
    }

    #[test]
    fn file_metadata_from_nonexistent_file_fails() {
        let tmp = tempfile::tempdir().unwrap();
        let result =
            FileMetadata::from_file(tmp.path().join("nonexistent.txt"), Compression::Zstd, None);
        assert!(result.is_err());
    }

    #[test]
    fn save_local_creates_storage_and_metadata() {
        let (_tmp, root) = create_temp_git_repo();
        let (config, dvs_dir) = init_dvs_repo(&root);
        let backend = config.backend();
        let paths = make_paths(&root, &config);
        let file_path = create_file(&root, "data.bin", b"binary data");

        let metadata = FileMetadata::from_file(&file_path, Compression::Zstd, None).unwrap();
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

        let metadata = FileMetadata::from_file(&file_path, Compression::Zstd, None).unwrap();
        metadata
            .save(Uuid::new_v4(), &file_path, backend, &paths, "data.bin")
            .unwrap();

        // Second save should return Present
        let outcome = metadata
            .save(Uuid::new_v4(), &file_path, backend, &paths, "data.bin")
            .unwrap();
        assert_eq!(outcome, Outcome::Present);
    }
}
