//! File metadata utilities.

use crate::{DvsError, Metadata};
use fs_err as fs;
use std::path::Path;

/// Save metadata to a `.dvs` file.
pub fn save_metadata(metadata: &Metadata, path: &Path) -> Result<(), DvsError> {
    metadata.save(path)
}

/// Load metadata from a `.dvs` file.
pub fn load_metadata(path: &Path) -> Result<Metadata, DvsError> {
    Metadata::load(path)
}

/// Check if metadata files exist for all given paths.
pub fn check_meta_files_exist(paths: &[std::path::PathBuf]) -> Result<(), DvsError> {
    let mut missing = Vec::new();

    for path in paths {
        let meta_path = Metadata::metadata_path(path);
        if !meta_path.exists() {
            missing.push(path.clone());
        }
    }

    if missing.is_empty() {
        Ok(())
    } else {
        Err(DvsError::batch_error(format!(
            "Missing metadata for {} files: {}",
            missing.len(),
            missing
                .iter()
                .take(5)
                .map(|p| p.display().to_string())
                .collect::<Vec<_>>()
                .join(", ")
        )))
    }
}

/// Get the username of the current user (for `saved_by` field).
pub fn get_current_username() -> Result<String, DvsError> {
    // Try multiple methods to get the username

    // Method 1: $USER environment variable (most common)
    if let Ok(user) = std::env::var("USER") {
        if !user.is_empty() {
            return Ok(user);
        }
    }

    // Method 2: $USERNAME environment variable (Windows)
    if let Ok(user) = std::env::var("USERNAME") {
        if !user.is_empty() {
            return Ok(user);
        }
    }

    // Method 3: $LOGNAME environment variable
    if let Ok(user) = std::env::var("LOGNAME") {
        if !user.is_empty() {
            return Ok(user);
        }
    }

    // Method 4: whoami via libc (Unix only)
    #[cfg(unix)]
    {
        let uid = unsafe { libc::getuid() };
        let pw = unsafe { libc::getpwuid(uid) };
        if !pw.is_null() {
            let name = unsafe { std::ffi::CStr::from_ptr((*pw).pw_name) };
            if let Ok(s) = name.to_str() {
                return Ok(s.to_string());
            }
        }
    }

    // Fallback: use "unknown"
    Ok("unknown".to_string())
}

/// Get file size in bytes.
pub fn get_file_size(path: &Path) -> Result<u64, DvsError> {
    let metadata = fs::metadata(path)?;
    Ok(metadata.len())
}

/// Check if a file exists.
pub fn file_exists(path: &Path) -> bool {
    path.exists() && path.is_file()
}

/// Get the metadata path for a data file.
pub fn metadata_path_for(data_path: &Path) -> std::path::PathBuf {
    Metadata::metadata_path(data_path)
}

/// Get the data path from a metadata file path.
pub fn data_path_for(metadata_path: &Path) -> Option<std::path::PathBuf> {
    Metadata::data_path(metadata_path)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    #[test]
    fn test_get_file_size() {
        let temp_dir = std::env::temp_dir().join("dvs-test-file-size");
        let _ = fs::create_dir_all(&temp_dir);

        let path = temp_dir.join("size.txt");
        let mut file = fs::File::create(&path).unwrap();
        file.write_all(b"hello world").unwrap();
        drop(file);

        let size = get_file_size(&path).unwrap();
        assert_eq!(size, 11);

        let _ = fs::remove_dir_all(&temp_dir);
    }

    #[test]
    fn test_get_current_username() {
        // This should not fail - it should always return something
        let username = get_current_username().unwrap();
        assert!(!username.is_empty());
    }

    #[test]
    fn test_file_exists() {
        let temp_dir = std::env::temp_dir().join("dvs-test-file-exists");
        let _ = fs::create_dir_all(&temp_dir);

        let path = temp_dir.join("exists.txt");
        assert!(!file_exists(&path));

        fs::write(&path, b"test").unwrap();
        assert!(file_exists(&path));

        // Directory should not count as file
        assert!(!file_exists(&temp_dir));

        let _ = fs::remove_dir_all(&temp_dir);
    }

    #[test]
    fn test_metadata_path_for() {
        let data_path = std::path::PathBuf::from("/data/file.csv");
        let meta_path = metadata_path_for(&data_path);
        assert_eq!(meta_path, std::path::PathBuf::from("/data/file.csv.dvs"));
    }

    #[test]
    fn test_data_path_for() {
        let meta_path = std::path::PathBuf::from("/data/file.csv.dvs");
        let data_path = data_path_for(&meta_path).unwrap();
        assert_eq!(data_path, std::path::PathBuf::from("/data/file.csv"));

        // Non-.dvs file should return None
        let not_meta = std::path::PathBuf::from("/data/file.csv");
        assert!(data_path_for(&not_meta).is_none());
    }
}
