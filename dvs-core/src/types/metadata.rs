//! DVS metadata types (.dvs files).

use fs_err as fs;
use serde::{Deserialize, Serialize};
use chrono::{DateTime, Utc};
use crate::HashAlgo;

/// Metadata stored in `.dvs` files alongside tracked data files.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Metadata {
    /// Hash of the file contents (hex string).
    /// Field name kept as `blake3_checksum` for backward compatibility.
    pub blake3_checksum: String,

    /// File size in bytes.
    pub size: u64,

    /// Timestamp when the file was added/updated.
    pub add_time: DateTime<Utc>,

    /// User-provided message describing this version.
    #[serde(default)]
    pub message: String,

    /// Username of the person who added this file.
    pub saved_by: String,

    /// Hash algorithm used for this file.
    /// Defaults to Blake3 for backward compatibility with existing metadata.
    #[serde(default = "default_hash_algo", skip_serializing_if = "is_blake3")]
    pub hash_algo: HashAlgo,
}

/// Default hash algorithm for backward compatibility.
fn default_hash_algo() -> HashAlgo {
    HashAlgo::Blake3
}

/// Check if algorithm is Blake3 (for skip_serializing_if).
fn is_blake3(algo: &HashAlgo) -> bool {
    *algo == HashAlgo::Blake3
}

impl Metadata {
    /// Create new metadata for a file.
    pub fn new(
        blake3_checksum: String,
        size: u64,
        message: Option<String>,
        saved_by: String,
    ) -> Self {
        Self {
            blake3_checksum,
            size,
            add_time: Utc::now(),
            message: message.unwrap_or_default(),
            saved_by,
            hash_algo: HashAlgo::Blake3,
        }
    }

    /// Create new metadata with a specific hash algorithm.
    pub fn with_algo(
        checksum: String,
        size: u64,
        message: Option<String>,
        saved_by: String,
        hash_algo: HashAlgo,
    ) -> Self {
        Self {
            blake3_checksum: checksum,
            size,
            add_time: Utc::now(),
            message: message.unwrap_or_default(),
            saved_by,
            hash_algo,
        }
    }

    /// Get the checksum (alias for blake3_checksum field).
    pub fn checksum(&self) -> &str {
        &self.blake3_checksum
    }

    /// Load metadata from a `.dvs` file.
    pub fn load(path: &std::path::Path) -> Result<Self, crate::DvsError> {
        let contents = fs::read_to_string(path)?;
        let metadata: Metadata = serde_json::from_str(&contents)?;
        Ok(metadata)
    }

    /// Save metadata to a `.dvs` file.
    pub fn save(&self, path: &std::path::Path) -> Result<(), crate::DvsError> {
        let json = serde_json::to_string_pretty(self)?;
        fs::write(path, json)?;
        Ok(())
    }

    /// Get the metadata file path for a given data file.
    pub fn metadata_path(data_path: &std::path::Path) -> std::path::PathBuf {
        let mut path = data_path.to_path_buf();
        let filename = path.file_name().unwrap().to_string_lossy();
        path.set_file_name(format!("{}.dvs", filename));
        path
    }

    /// Get the data file path from a metadata file path.
    pub fn data_path(metadata_path: &std::path::Path) -> Option<std::path::PathBuf> {
        let filename = metadata_path.file_name()?.to_string_lossy();
        if filename.ends_with(".dvs") {
            let mut path = metadata_path.to_path_buf();
            path.set_file_name(&filename[..filename.len() - 4]);
            Some(path)
        } else {
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_metadata_new() {
        let metadata = Metadata::new(
            "abc123".to_string(),
            1024,
            Some("test message".to_string()),
            "testuser".to_string(),
        );
        assert_eq!(metadata.blake3_checksum, "abc123");
        assert_eq!(metadata.size, 1024);
        assert_eq!(metadata.message, "test message");
        assert_eq!(metadata.saved_by, "testuser");
    }

    #[test]
    fn test_metadata_roundtrip() {
        let temp_dir = std::env::temp_dir().join("dvs-test-metadata-roundtrip");
        let _ = fs::create_dir_all(&temp_dir);

        let meta_path = temp_dir.join("test.csv.dvs");
        let metadata = Metadata::new(
            "deadbeef1234567890abcdef".to_string(),
            2048,
            Some("first version".to_string()),
            "alice".to_string(),
        );

        // Save
        metadata.save(&meta_path).unwrap();

        // Load
        let loaded = Metadata::load(&meta_path).unwrap();
        assert_eq!(loaded.blake3_checksum, metadata.blake3_checksum);
        assert_eq!(loaded.size, metadata.size);
        assert_eq!(loaded.message, metadata.message);
        assert_eq!(loaded.saved_by, metadata.saved_by);

        // Cleanup
        let _ = fs::remove_dir_all(&temp_dir);
    }

    #[test]
    fn test_metadata_path() {
        let data = std::path::PathBuf::from("/data/file.csv");
        let meta = Metadata::metadata_path(&data);
        assert_eq!(meta, std::path::PathBuf::from("/data/file.csv.dvs"));
    }

    #[test]
    fn test_data_path() {
        let meta = std::path::PathBuf::from("/data/file.csv.dvs");
        let data = Metadata::data_path(&meta).unwrap();
        assert_eq!(data, std::path::PathBuf::from("/data/file.csv"));

        // Non-.dvs file should return None
        let not_meta = std::path::PathBuf::from("/data/file.csv");
        assert!(Metadata::data_path(&not_meta).is_none());
    }

    #[test]
    fn test_metadata_no_message() {
        let metadata = Metadata::new(
            "hash".to_string(),
            100,
            None,
            "user".to_string(),
        );
        assert_eq!(metadata.message, "");
    }
}
