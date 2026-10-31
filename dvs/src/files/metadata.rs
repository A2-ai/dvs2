use std::path::Path;

use anyhow::{Result, anyhow, bail};
use fs_err as fs;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::error::{Error, ErrorCode};
use crate::paths::ProjectPath;
use crate::{Backend, Compression, DvsPaths, Hashes, Outcome, StoreRequest, StoreResult};

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

    /// Writes the metadata file, creating its directory if needed.
    pub fn write(&self, paths: &DvsPaths, relative_path: impl AsRef<Path>) -> Result<()> {
        let dvs_file_path = paths.metadata_path(relative_path.as_ref());
        if let Some(parent) = dvs_file_path.parent() {
            fs::create_dir_all(parent)?;
        }
        log::debug!("Writing metadata to {}", dvs_file_path.display());
        write_atomic(
            &dvs_file_path,
            serde_json::to_string_pretty(self)
                .expect("valid json")
                .as_bytes(),
        )
    }

    /// Whether storage has the content and the metadata file already says
    /// exactly this, so saving would change nothing.
    pub(crate) fn is_saved(
        &self,
        backend: &Backend,
        paths: &DvsPaths,
        relative_path: &Path,
    ) -> Result<bool> {
        let dvs_file_path = paths.metadata_path(relative_path);
        let dvs_file_exists = dvs_file_path.is_file();
        // Only check if file exists on the server if it exists locally
        // Backend will dedup anyway
        let storage_exists = dvs_file_exists && backend.exists(&self.hashes)?;

        log::debug!(
            "Saving {}: metadata_exists={}, storage_exists={}",
            relative_path.display(),
            dvs_file_exists,
            storage_exists
        );

        if dvs_file_exists && storage_exists {
            // we read the file anyway to make sure it's not 2 files having the same hash
            let existing: FileMetadata = serde_json::from_reader(fs::File::open(&dvs_file_path)?)
                .map_err(|e| {
                Error::new(
                    ErrorCode::BadMetadata,
                    format!("cannot read {}: {e}", dvs_file_path.display()),
                )
            })?;
            if existing == *self {
                log::debug!("File {} is already in sync", relative_path.display());
                return Ok(true);
            }
        }
        Ok(false)
    }

    /// Returns whether the file already existed in the dvs folder and therefore is an update
    /// and what storage reported, if it stored anything.
    /// Copies the source file to storage and saves metadata atomically (both succeed or neither).
    pub fn save(
        &mut self,
        operation_id: Uuid,
        source_file: impl AsRef<Path>,
        backend: &Backend,
        paths: &DvsPaths,
        relative_path: impl AsRef<Path>,
        on_bytes: Option<&(dyn Fn(u64) + Send + Sync)>,
    ) -> Result<(Outcome, Option<StoreResult>)> {
        if self.is_saved(backend, paths, relative_path.as_ref())? {
            return Ok((Outcome::Present, None));
        }

        let stored = self.store_and_write(
            operation_id,
            source_file,
            backend,
            paths,
            relative_path,
            on_bytes,
        )?;
        Ok((Outcome::Copied, Some(stored)))
    }

    /// Stores the file then writes the metadata, atomically: either both succeed or
    /// neither is left behind. Returns what storage reported.
    pub fn store_and_write(
        &mut self,
        operation_id: Uuid,
        source_file: impl AsRef<Path>,
        backend: &Backend,
        paths: &DvsPaths,
        relative_path: impl AsRef<Path>,
        on_bytes: Option<&(dyn Fn(u64) + Send + Sync)>,
    ) -> Result<StoreResult> {
        let dvs_file_path = paths.metadata_path(relative_path.as_ref());

        // 1. Store file to backend. The backend records the add in its audit log
        // and dedups the blob internally if it is already present.
        let path = ProjectPath::from_path(relative_path.as_ref()).map_err(|e| anyhow!("{e}"))?;
        let store_res = backend.store(StoreRequest {
            hashes: &self.hashes,
            source: source_file.as_ref(),
            compression: self.compression,
            path,
            operation_id,
            message: self.message.as_deref(),
            on_bytes,
        });

        // The server compresses with the project's setting, which may not be the one
        // we asked for
        if let Ok(res) = &store_res {
            self.compression = res.compression;
        }

        // 2. Then metadata
        let old_metadata_content = fs::read(&dvs_file_path).ok();
        let metadata_res = self.write(paths, relative_path.as_ref());

        match (store_res, metadata_res) {
            (Ok(res), Ok(_)) => Ok(res),
            (Err(e), Ok(_)) => {
                log::warn!(
                    "Storage failed, rolling back metadata for {}",
                    relative_path.as_ref().display()
                );
                if let Some(old) = old_metadata_content {
                    let _ = write_atomic(&dvs_file_path, &old);
                } else {
                    let _ = fs::remove_file(&dvs_file_path);
                }
                Err(e)
            }
            (Ok(res), Err(_)) => {
                log::warn!(
                    "Metadata write failed, rolling back storage for {}",
                    relative_path.as_ref().display()
                );
                if let Some(old) = old_metadata_content {
                    let _ = write_atomic(&dvs_file_path, &old);
                } else {
                    let _ = fs::remove_file(&dvs_file_path);
                }
                // Remove the blob only if this call actually created it
                if res.created {
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
                    let _ = write_atomic(&dvs_file_path, &old);
                } else {
                    let _ = fs::remove_file(&dvs_file_path);
                }
                bail!("Failed to write metadata file: {dvs_file_path:?}: {e}")
            }
        }
    }
}

/// Writes a metadata file through a temp file next to it and a rename, so a
/// kill never leaves a half-written or empty record. The temp name does not
/// end in `.dvs`, so a leftover one is never read as a record.
fn write_atomic(path: &Path, contents: &[u8]) -> Result<()> {
    let tmp_path = path.with_extension(format!("dvs-tmp.{}", Uuid::new_v4()));
    let result = fs::write(&tmp_path, contents).and_then(|()| fs::rename(&tmp_path, path));
    if result.is_err() {
        let _ = fs::remove_file(&tmp_path);
    }
    Ok(result?)
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

    fn dir_names(dir: &Path) -> Vec<String> {
        let mut names: Vec<_> = fs::read_dir(dir)
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        names.sort();
        names
    }

    // A record is replaced through a temp file and a rename, and no temp file
    // is left next to it
    #[test]
    fn write_replaces_a_record_and_leaves_no_temp_file() {
        let (_tmp, root) = create_temp_git_repo();
        let (config, dvs_dir) = init_dvs_repo(&root);
        let paths = make_paths(&root, &config);
        let one = create_file(&root, "data/one.bin", b"one");
        let two = create_file(&root, "data/two.bin", b"two, longer");

        let first = FileMetadata::from_file(&one, Compression::Zstd, None).unwrap();
        first.write(&paths, "data/one.bin").unwrap();
        let second = FileMetadata::from_file(&two, Compression::Zstd, None).unwrap();
        second.write(&paths, "data/one.bin").unwrap();

        let record = dvs_dir.join("data/one.bin.dvs");
        let read: FileMetadata = serde_json::from_reader(fs::File::open(&record).unwrap()).unwrap();
        assert_eq!(read, second);
        assert_eq!(dir_names(&dvs_dir.join("data")), ["one.bin.dvs"]);
    }

    // When the rename fails, the temp file is removed
    #[test]
    fn a_failed_write_leaves_no_temp_file() {
        let (_tmp, root) = create_temp_git_repo();
        let (config, dvs_dir) = init_dvs_repo(&root);
        let paths = make_paths(&root, &config);
        let file = create_file(&root, "a.bin", b"a");
        // A directory where the record goes, which a file cannot replace
        fs::create_dir_all(dvs_dir.join("a.bin.dvs/inside")).unwrap();
        let before = dir_names(&dvs_dir);

        let metadata = FileMetadata::from_file(&file, Compression::Zstd, None).unwrap();
        assert!(metadata.write(&paths, "a.bin").is_err());
        assert_eq!(dir_names(&dvs_dir), before);
    }

    #[test]
    fn save_local_creates_storage_and_metadata() {
        let (_tmp, root) = create_temp_git_repo();
        let (config, dvs_dir) = init_dvs_repo(&root);
        let backend = config.backend();
        let paths = make_paths(&root, &config);
        let file_path = create_file(&root, "data.bin", b"binary data");

        let mut metadata = FileMetadata::from_file(&file_path, Compression::Zstd, None).unwrap();
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
    fn save_reports_an_unreadable_record_as_bad_metadata() {
        let (_tmp, root) = create_temp_git_repo();
        let (config, dvs_dir) = init_dvs_repo(&root);
        let backend = config.backend();
        let paths = make_paths(&root, &config);
        let file_path = create_file(&root, "data.bin", b"binary data");
        let mut metadata = FileMetadata::from_file(&file_path, Compression::Zstd, None).unwrap();
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
        let conflicted = "<<<<<<< HEAD\n{}\n=======\n{}\n>>>>>>> theirs\n";
        fs::write(dvs_dir.join("data.bin.dvs"), conflicted).unwrap();

        let error = metadata
            .save(
                Uuid::new_v4(),
                &file_path,
                backend,
                &paths,
                "data.bin",
                None,
            )
            .unwrap_err();
        assert_eq!(
            crate::backend_error_code(&error),
            Some(ErrorCode::BadMetadata)
        );
    }

    #[test]
    fn save_local_returns_present_when_already_stored() {
        let (_tmp, root) = create_temp_git_repo();
        let (config, _dvs_dir) = init_dvs_repo(&root);
        let backend = config.backend();
        let paths = make_paths(&root, &config);
        let file_path = create_file(&root, "data.bin", b"binary data");

        let mut metadata = FileMetadata::from_file(&file_path, Compression::Zstd, None).unwrap();
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
        let mut metadata_a = FileMetadata::from_file(&file_a, Compression::Zstd, None).unwrap();
        metadata_a
            .save(Uuid::new_v4(), &file_a, backend, &paths, "a.txt", None)
            .unwrap();
        let hash_h1 = metadata_a.hashes.blake3.clone();

        // Add file B with content "bar" (hash H2)
        let file_b = create_file(&root, "b.txt", b"bar");
        let mut metadata_b = FileMetadata::from_file(&file_b, Compression::Zstd, None).unwrap();
        metadata_b
            .save(Uuid::new_v4(), &file_b, backend, &paths, "b.txt", None)
            .unwrap();
        let hash_h2 = metadata_b.hashes.blake3.clone();
        assert_ne!(hash_h1, hash_h2);

        // Change file B's content to "foo" (now hash H1)
        fs::write(&file_b, b"foo").unwrap();

        // Run add on B with new content
        let mut metadata_b_new = FileMetadata::from_file(&file_b, Compression::Zstd, None).unwrap();
        assert_eq!(metadata_b_new.hashes.blake3, hash_h1);

        metadata_b_new
            .save(Uuid::new_v4(), &file_b, backend, &paths, "b.txt", None)
            .unwrap();

        // Verify metadata was updated
        let dvs_file = dvs_dir.join("b.txt.dvs");
        let stored: FileMetadata =
            serde_json::from_reader(fs::File::open(&dvs_file).unwrap()).unwrap();

        assert_eq!(
            stored.hashes.blake3, hash_h1,
            "Metadata should be updated to new hash"
        );
    }
}
