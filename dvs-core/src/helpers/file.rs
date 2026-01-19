//! File metadata utilities.

use std::path::Path;
use crate::{Metadata, DvsError};

/// Save metadata to a `.dvs` file.
pub fn save_metadata(_metadata: &Metadata, _path: &Path) -> Result<(), DvsError> {
    todo!("Save metadata to file")
}

/// Load metadata from a `.dvs` file.
pub fn load_metadata(_path: &Path) -> Result<Metadata, DvsError> {
    todo!("Load metadata from file")
}

/// Check if metadata files exist for all given paths.
pub fn check_meta_files_exist(_paths: &[std::path::PathBuf]) -> Result<(), DvsError> {
    todo!("Check if metadata files exist")
}

/// Get the username of the current user (for `saved_by` field).
pub fn get_current_username() -> Result<String, DvsError> {
    todo!("Get current username")
}

/// Get file size in bytes.
pub fn get_file_size(_path: &Path) -> Result<u64, DvsError> {
    todo!("Get file size")
}
