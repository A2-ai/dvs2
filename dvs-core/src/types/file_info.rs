//! File information types.

use std::path::PathBuf;

/// Information about a tracked file.
#[derive(Debug, Clone)]
pub struct FileInfo {
    /// Relative path from the working directory.
    pub relative_path: PathBuf,

    /// Absolute path on the filesystem.
    pub absolute_path: PathBuf,

    /// File size in bytes.
    pub size: u64,

    /// Blake3 hash of the file contents.
    pub blake3_checksum: String,

    /// Whether the file has associated metadata.
    pub has_metadata: bool,
}

impl FileInfo {
    /// Create new file info.
    pub fn new(
        relative_path: PathBuf,
        absolute_path: PathBuf,
        size: u64,
        blake3_checksum: String,
        has_metadata: bool,
    ) -> Self {
        Self {
            relative_path,
            absolute_path,
            size,
            blake3_checksum,
            has_metadata,
        }
    }

    /// Get file info for a path.
    pub fn from_path(path: &std::path::Path) -> Result<Self, crate::DvsError> {
        todo!("Get file info from path")
    }
}
