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
    pub add_time: jiff::Timestamp,
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
            created_by: whoami::username().unwrap_or_else(|_| "unknown".to_string()),
            add_time: jiff::Timestamp::now(),
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
        let add_time = jiff::Timestamp::now();

        Ok(Self {
            hashes,
            size,
            created_by,
            add_time,
            message,
            compression,
        })
    }

    /// Whether re-adding unchanged content (`existing == self`) must still
    /// rewrite state: the compression setting differs, or a new `Some` message
    /// differs from the recorded one. A `None` message never counts as a
    /// change, so it never clears an existing message.
    pub(crate) fn updates_settings_of(&self, existing: &FileMetadata) -> bool {
        self.compression != existing.compression
            || (self.message.is_some() && self.message != existing.message)
    }

    /// Returns whether the file already existed in the dvs folder and therefore is an update
    /// and the compressed size if applicable.
    /// Copies the source file to storage and saves metadata atomically (both succeed or neither).
    pub fn save(
        &self,
        operation_id: Uuid,
        source_file: impl AsRef<Path>,
        backend: &dyn Backend,
        paths: &DvsPaths,
        relative_path: impl AsRef<Path>,
        on_bytes: Option<&(dyn Fn(u64) + Send + Sync)>,
    ) -> Result<(Outcome, Option<u64>)> {
        let dvs_file_path = paths.metadata_path(relative_path.as_ref());
        let dvs_file_exists = dvs_file_path.is_file();
        let storage_exists = backend.exists(&self.hashes)?;

        log::debug!(
            "Saving {}: metadata_exists={}, storage_exists={}",
            relative_path.as_ref().display(),
            dvs_file_exists,
            storage_exists
        );

        // The metadata to write. It diverges from `self` only when re-adding
        // unchanged content without a message: the existing message is kept
        // (a `None` message never clears one).
        let mut to_write = self.clone();
        // Set when unchanged content must be re-stored because the compression
        // setting changed. Holds the previous compression for rollback.
        let mut recompress_from: Option<Compression> = None;

        if dvs_file_exists && storage_exists {
            // we read the file anyway to make sure it's not 2 files having the same hash
            let existing: FileMetadata = serde_json::from_reader(fs::File::open(&dvs_file_path)?)?;
            if existing == *self {
                // Same content. Re-add only when the settings changed: a
                // different compression, or a new `Some` message.
                if !self.updates_settings_of(&existing) {
                    log::debug!(
                        "File {} is already in sync",
                        relative_path.as_ref().display()
                    );
                    return Ok((Outcome::Present, None));
                }
                if existing.compression != self.compression {
                    recompress_from = Some(existing.compression);
                }
                if to_write.message.is_none() {
                    to_write.message = existing.message.clone();
                }
            }
        }

        // We do an atomic update, either everything works or we error
        // 1. Create metadata dirs first
        if let Some(parent) = dvs_file_path.parent() {
            fs::create_dir_all(parent)?;
        }

        // 2. Store file to backend if it doesn't already exist, or re-store it
        // when the compression setting changed. Re-storing is remove-then-store
        // rather than rename-over: `store` may treat an existing destination
        // blob as a concurrent winner and skip the write. The blob is briefly
        // missing, so a concurrent get during re-compression can transiently
        // fail. Acceptable.
        let (storage_res, stored_size) = if recompress_from.is_some() {
            match backend.remove(&self.hashes).and_then(|_| {
                backend.store(
                    &self.hashes,
                    source_file.as_ref(),
                    self.compression,
                    on_bytes,
                )
            }) {
                Ok(size) => (Ok(()), Some(size)),
                Err(e) => (Err(e), None),
            }
        } else if storage_exists {
            (Ok(()), None)
        } else {
            match backend.store(
                &self.hashes,
                source_file.as_ref(),
                self.compression,
                on_bytes,
            ) {
                Ok(size) => (Ok(()), Some(size)),
                Err(e) => (Err(e), None),
            }
        };

        // Best-effort rollback for a failed re-compression: put the blob back
        // with its previous compression. Content is unchanged, so the source
        // file has the right bytes.
        let restore_old_blob = |old: Compression| {
            let _ = backend.remove(&self.hashes);
            if let Err(e) = backend.store(&self.hashes, source_file.as_ref(), old, None) {
                log::error!(
                    "Failed to restore blob with previous compression for {}: {e}",
                    relative_path.as_ref().display()
                );
            }
        };

        // 3. Then metadata
        let old_metadata_content = fs::read(&dvs_file_path).ok();
        log::debug!("Writing metadata to {}", dvs_file_path.display());
        let metadata_res = fs::write(
            &dvs_file_path,
            serde_json::to_string_pretty(&to_write).expect("valid json"),
        );

        match (storage_res, metadata_res) {
            (Ok(_), Ok(_)) => {
                let audit_entry = AuditEntry::new_add(
                    operation_id,
                    AuditFile {
                        path: relative_path.as_ref().to_path_buf(),
                        hashes: self.hashes.clone(),
                    },
                    self.compression,
                );
                if let Err(e) = backend.log_audit(&audit_entry) {
                    log::error!("Failed to write audit log {audit_entry:?}: {e}");
                }
                Ok((Outcome::Copied, stored_size))
            }
            (Err(e), Ok(_)) => {
                log::warn!(
                    "Storage failed, rolling back metadata for {}",
                    relative_path.as_ref().display()
                );
                if let Some(old) = old_metadata_content {
                    let _ = fs::write(&dvs_file_path, &old);
                } else {
                    let _ = fs::remove_file(&dvs_file_path);
                }
                if let Some(old) = recompress_from {
                    restore_old_blob(old);
                }
                Err(e)
            }
            (Ok(_), Err(_)) => {
                log::warn!(
                    "Metadata write failed, rolling back storage for {}",
                    relative_path.as_ref().display()
                );
                if let Some(old) = old_metadata_content {
                    let _ = fs::write(&dvs_file_path, &old);
                } else {
                    let _ = fs::remove_file(&dvs_file_path);
                }
                if let Some(old) = recompress_from {
                    restore_old_blob(old);
                } else if stored_size.is_some() {
                    // Remove the blob only if this call actually stored it
                    let _ = backend.remove(&self.hashes);
                }
                bail!("Failed to write metadata file: {dvs_file_path:?}")
            }
            (Err(e), Err(_)) => {
                log::warn!(
                    "Both storage and metadata failed, rolling back for {}",
                    relative_path.as_ref().display()
                );
                if let Some(old) = old_metadata_content {
                    let _ = fs::write(&dvs_file_path, &old);
                } else {
                    let _ = fs::remove_file(&dvs_file_path);
                }
                if let Some(old) = recompress_from {
                    restore_old_blob(old);
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
        .unwrap()
    }

    /// Path of the stored blob for `metadata` in the test storage dir
    /// (`{repo_root}/../.storage/<first 2 hash chars>/<rest>`).
    fn blob_path(root: &Path, metadata: &FileMetadata) -> std::path::PathBuf {
        let hash = &metadata.hashes.blake3;
        root.parent()
            .unwrap()
            .join(".storage")
            .join(&hash[..2])
            .join(&hash[2..])
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

    /// Locks in the `.dvs` JSON wire format for `add_time`: RFC 3339 with a `Z`
    /// suffix (UTC). Changing `FileMetadata.add_time` from `String` to
    /// `jiff::Timestamp` must keep the serialized form identical so existing
    /// `.dvs` files round-trip unchanged.
    #[test]
    fn file_metadata_add_time_serde_roundtrip_rfc3339() {
        let blake3 = "a".repeat(64);
        let json = format!(
            r#"{{
                "hashes": {{"blake3": "{blake3}"}},
                "size": 11,
                "created_by": "tester",
                "add_time": "2024-01-02T03:04:05Z",
                "compression": "none"
            }}"#
        );
        let meta: FileMetadata =
            serde_json::from_str(&json).expect("parse FileMetadata with RFC 3339 timestamp");
        assert_eq!(meta.add_time.to_string(), "2024-01-02T03:04:05Z");

        let reserialized = serde_json::to_string(&meta).expect("serialize FileMetadata");
        assert!(
            reserialized.contains("\"add_time\":\"2024-01-02T03:04:05Z\""),
            "add_time must serialize as RFC 3339 string; got: {reserialized}"
        );
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
        let (outcome, stored_size) = metadata
            .save(
                Uuid::new_v4(),
                &file_path,
                backend,
                &paths,
                "data.bin",
                None,
            )
            .unwrap();

        assert_eq!(outcome, Outcome::Copied);
        assert!(stored_size.is_some());
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
            .save(
                Uuid::new_v4(),
                &file_path,
                backend,
                &paths,
                "data.bin",
                None,
            )
            .unwrap();

        // Second save should return Present
        let (outcome, stored_size) = metadata
            .save(
                Uuid::new_v4(),
                &file_path,
                backend,
                &paths,
                "data.bin",
                None,
            )
            .unwrap();
        assert_eq!(outcome, Outcome::Present);
        assert!(stored_size.is_none());
    }

    #[test]
    fn save_readd_with_new_compression_reencodes_blob() {
        let (_tmp, root) = create_temp_git_repo();
        let (config, dvs_dir) = init_dvs_repo(&root);
        let backend = config.backend();
        let paths = make_paths(&root, &config);
        let content: &[u8] = b"compressible content compressible content compressible content";
        let file_path = create_file(&root, "data.bin", content);

        let metadata = FileMetadata::from_file(&file_path, Compression::None, None).unwrap();
        metadata
            .save(
                Uuid::new_v4(),
                &file_path,
                backend,
                &paths,
                "data.bin",
                None,
            )
            .unwrap();
        let blob = blob_path(&root, &metadata);
        assert_eq!(
            fs::read(&blob).unwrap(),
            content,
            "uncompressed blob should be the raw content"
        );

        // Re-add unchanged content with a different compression setting.
        let readd = FileMetadata::from_file(&file_path, Compression::Zstd, None).unwrap();
        let (outcome, stored_size) = readd
            .save(
                Uuid::new_v4(),
                &file_path,
                backend,
                &paths,
                "data.bin",
                None,
            )
            .unwrap();
        assert_eq!(outcome, Outcome::Copied);
        assert!(stored_size.is_some());
        assert_ne!(
            fs::read(&blob).unwrap(),
            content,
            "blob should have been re-stored compressed"
        );
        let on_disk: FileMetadata =
            serde_json::from_reader(fs::File::open(dvs_dir.join("data.bin.dvs")).unwrap()).unwrap();
        assert_eq!(on_disk.compression, Compression::Zstd);

        // A get must round-trip: metadata says zstd and the blob really is zstd.
        fs::remove_file(&file_path).unwrap();
        let results =
            crate::get_files(vec!["data.bin".into()], &paths, backend, false, None).unwrap();
        assert_eq!(results.len(), 1);
        assert!(matches!(
            results[0].detail,
            crate::GetDetail::Success {
                outcome: Outcome::Copied,
                ..
            }
        ));
        assert_eq!(fs::read(&file_path).unwrap(), content);
    }

    #[test]
    fn save_readd_with_new_message_updates_metadata_only() {
        let (_tmp, root) = create_temp_git_repo();
        let (config, dvs_dir) = init_dvs_repo(&root);
        let backend = config.backend();
        let paths = make_paths(&root, &config);
        let file_path = create_file(&root, "data.bin", b"binary data");

        let metadata = FileMetadata::from_file(&file_path, Compression::Zstd, None).unwrap();
        metadata
            .save(
                Uuid::new_v4(),
                &file_path,
                backend,
                &paths,
                "data.bin",
                None,
            )
            .unwrap();
        let blob = blob_path(&root, &metadata);
        let blob_before = fs::read(&blob).unwrap();

        // Re-add unchanged content with a new message.
        let readd =
            FileMetadata::from_file(&file_path, Compression::Zstd, Some("a message".to_string()))
                .unwrap();
        let (outcome, stored_size) = readd
            .save(
                Uuid::new_v4(),
                &file_path,
                backend,
                &paths,
                "data.bin",
                None,
            )
            .unwrap();
        assert_eq!(outcome, Outcome::Copied);
        assert!(
            stored_size.is_none(),
            "a message-only update must not touch the blob"
        );
        assert_eq!(fs::read(&blob).unwrap(), blob_before);
        let on_disk: FileMetadata =
            serde_json::from_reader(fs::File::open(dvs_dir.join("data.bin.dvs")).unwrap()).unwrap();
        assert_eq!(on_disk.message, Some("a message".to_string()));

        // Re-adding with the same message is a no-op again.
        let again =
            FileMetadata::from_file(&file_path, Compression::Zstd, Some("a message".to_string()))
                .unwrap();
        let (outcome, stored_size) = again
            .save(
                Uuid::new_v4(),
                &file_path,
                backend,
                &paths,
                "data.bin",
                None,
            )
            .unwrap();
        assert_eq!(outcome, Outcome::Present);
        assert!(stored_size.is_none());
    }

    #[test]
    fn save_readd_without_message_keeps_existing_message_untouched() {
        let (_tmp, root) = create_temp_git_repo();
        let (config, dvs_dir) = init_dvs_repo(&root);
        let backend = config.backend();
        let paths = make_paths(&root, &config);
        let file_path = create_file(&root, "data.bin", b"binary data");

        let metadata =
            FileMetadata::from_file(&file_path, Compression::Zstd, Some("original".to_string()))
                .unwrap();
        metadata
            .save(
                Uuid::new_v4(),
                &file_path,
                backend,
                &paths,
                "data.bin",
                None,
            )
            .unwrap();
        let dvs_file = dvs_dir.join("data.bin.dvs");
        let bytes_before = fs::read(&dvs_file).unwrap();

        // A re-add without a message must not clear the existing one and must
        // not rewrite the metadata file at all.
        let readd = FileMetadata::from_file(&file_path, Compression::Zstd, None).unwrap();
        let (outcome, stored_size) = readd
            .save(
                Uuid::new_v4(),
                &file_path,
                backend,
                &paths,
                "data.bin",
                None,
            )
            .unwrap();
        assert_eq!(outcome, Outcome::Present);
        assert!(stored_size.is_none());
        assert_eq!(
            fs::read(&dvs_file).unwrap(),
            bytes_before,
            ".dvs file must be byte-identical after a message-less re-add"
        );
    }

    #[test]
    fn save_plain_readd_leaves_metadata_bytes_unchanged() {
        let (_tmp, root) = create_temp_git_repo();
        let (config, dvs_dir) = init_dvs_repo(&root);
        let backend = config.backend();
        let paths = make_paths(&root, &config);
        let file_path = create_file(&root, "data.bin", b"binary data");

        let metadata = FileMetadata::from_file(&file_path, Compression::Zstd, None).unwrap();
        metadata
            .save(
                Uuid::new_v4(),
                &file_path,
                backend,
                &paths,
                "data.bin",
                None,
            )
            .unwrap();
        let dvs_file = dvs_dir.join("data.bin.dvs");
        let bytes_before = fs::read(&dvs_file).unwrap();

        // A fresh re-add (new add_time, same settings) must not rewrite the
        // metadata file. Bulk re-adds must not dirty every .dvs file in git.
        let readd = FileMetadata::from_file(&file_path, Compression::Zstd, None).unwrap();
        let (outcome, stored_size) = readd
            .save(
                Uuid::new_v4(),
                &file_path,
                backend,
                &paths,
                "data.bin",
                None,
            )
            .unwrap();
        assert_eq!(outcome, Outcome::Present);
        assert!(stored_size.is_none());
        assert_eq!(
            fs::read(&dvs_file).unwrap(),
            bytes_before,
            ".dvs file must be byte-identical after a plain re-add"
        );
    }
}
