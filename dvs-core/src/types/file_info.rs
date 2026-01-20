//! File information types.

use std::path::{Path, PathBuf};

use crate::helpers::file::metadata_path_for;
use crate::helpers::hash::get_file_hash;
use crate::DvsError;

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
    ///
    /// Computes the blake3 hash and checks for metadata file existence.
    /// The relative_path is set to the filename component.
    pub fn from_path(path: &Path) -> Result<Self, DvsError> {
        let absolute_path = if path.is_absolute() {
            path.to_path_buf()
        } else {
            std::env::current_dir()
                .map_err(|e| DvsError::storage_error(format!("failed to get cwd: {e}")))?
                .join(path)
        };

        if !absolute_path.exists() {
            return Err(DvsError::file_not_found(absolute_path.display().to_string()));
        }

        let metadata = std::fs::metadata(&absolute_path)
            .map_err(|e| DvsError::storage_error(format!("failed to stat {}: {e}", path.display())))?;

        let size = metadata.len();
        let blake3_checksum = get_file_hash(&absolute_path)?;

        // Use filename as relative path if no better context
        let relative_path = path
            .file_name()
            .map(PathBuf::from)
            .unwrap_or_else(|| path.to_path_buf());

        let metadata_path = metadata_path_for(&absolute_path);
        let has_metadata = metadata_path.exists();

        Ok(Self {
            relative_path,
            absolute_path,
            size,
            blake3_checksum,
            has_metadata,
        })
    }

    /// Get file info with a specific relative path.
    ///
    /// Like `from_path` but allows specifying the relative path explicitly.
    pub fn from_path_with_relative(path: &Path, relative_path: PathBuf) -> Result<Self, DvsError> {
        let mut info = Self::from_path(path)?;
        info.relative_path = relative_path;
        Ok(info)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    #[test]
    fn test_file_info_from_path() {
        let dir = tempfile::tempdir().unwrap();
        let file_path = dir.path().join("test.txt");
        let mut file = std::fs::File::create(&file_path).unwrap();
        writeln!(file, "hello world").unwrap();
        drop(file);

        let info = FileInfo::from_path(&file_path).unwrap();

        assert_eq!(info.relative_path, PathBuf::from("test.txt"));
        assert_eq!(info.absolute_path, file_path);
        assert_eq!(info.size, 12); // "hello world\n"
        assert!(!info.blake3_checksum.is_empty());
        assert!(!info.has_metadata);
    }

    #[test]
    fn test_file_info_from_path_not_found() {
        let result = FileInfo::from_path(Path::new("/nonexistent/file.txt"));
        assert!(result.is_err());
    }

    #[test]
    fn test_file_info_with_relative() {
        let dir = tempfile::tempdir().unwrap();
        let file_path = dir.path().join("test.txt");
        std::fs::write(&file_path, "test").unwrap();

        let info =
            FileInfo::from_path_with_relative(&file_path, PathBuf::from("custom/path.txt"))
                .unwrap();

        assert_eq!(info.relative_path, PathBuf::from("custom/path.txt"));
    }
}
