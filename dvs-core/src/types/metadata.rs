//! DVS metadata types (.dvs files).

use serde::{Deserialize, Serialize};
use chrono::{DateTime, Utc};

/// Metadata stored in `.dvs` files alongside tracked data files.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Metadata {
    /// Blake3 hash of the file contents (64-character hex string).
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
        }
    }

    /// Load metadata from a `.dvs` file.
    pub fn load(path: &std::path::Path) -> Result<Self, crate::DvsError> {
        todo!("Load metadata from JSON file")
    }

    /// Save metadata to a `.dvs` file.
    pub fn save(&self, path: &std::path::Path) -> Result<(), crate::DvsError> {
        todo!("Save metadata to JSON file")
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
