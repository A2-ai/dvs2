use std::path::Path;

use anyhow::{Context, Result, bail};
use fs_err as fs;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::{Compression, Hashes};

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

    /// The `.dvs` file at `path`.
    pub fn read(path: &Path) -> Result<Self> {
        Ok(serde_json::from_reader(fs::File::open(path)?)?)
    }

    /// Write the `.dvs` file at `path` through a temporary file and a rename, so a
    /// failed write leaves the old metadata in place.
    pub fn write(&self, path: &Path) -> Result<()> {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        let tmp = path.with_extension(format!("dvs-tmp.{}", Uuid::new_v4()));
        let result = fs::write(
            &tmp,
            serde_json::to_string_pretty(self).expect("valid json"),
        )
        .and_then(|()| fs::rename(&tmp, path));
        if result.is_err() {
            let _ = fs::remove_file(&tmp);
        }
        result.with_context(|| format!("Failed to write metadata file {}", path.display()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testutil::{create_file, create_temp_git_repo};

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
    fn write_replaces_metadata_atomically() {
        let (_tmp, root) = create_temp_git_repo();
        let file_path = create_file(&root, "data.bin", b"binary data");
        let target = root.join(".dvs").join("sub").join("data.bin.dvs");

        let first = FileMetadata::from_file(&file_path, Compression::Zstd, None).unwrap();
        first.write(&target).unwrap();
        assert_eq!(FileMetadata::read(&target).unwrap(), first);

        fs::write(&file_path, b"other data").unwrap();
        let second = FileMetadata::from_file(&file_path, Compression::Zstd, None).unwrap();
        second.write(&target).unwrap();
        assert_eq!(FileMetadata::read(&target).unwrap(), second);
        let leftovers = fs::read_dir(target.parent().unwrap()).unwrap().count();
        assert_eq!(leftovers, 1, "no temporary file is left behind");
    }

    #[test]
    fn a_failed_write_keeps_the_old_metadata() {
        let (_tmp, root) = create_temp_git_repo();
        let file_path = create_file(&root, "data.bin", b"binary data");
        let metadata = FileMetadata::from_file(&file_path, Compression::Zstd, None).unwrap();
        // A directory where the file should go: the rename fails
        let target = root.join(".dvs").join("data.bin.dvs");
        fs::create_dir_all(target.join("occupied")).unwrap();
        assert!(metadata.write(&target).is_err());
        assert!(target.join("occupied").is_dir());
        let entries = fs::read_dir(root.join(".dvs")).unwrap().count();
        assert_eq!(entries, 1, "no temporary file is left behind");
    }
}
