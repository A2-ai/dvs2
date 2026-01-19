//! Blake3 hashing utilities.

use std::path::Path;
use crate::DvsError;

/// Compute the Blake3 hash of a file.
///
/// Uses memory-mapped I/O for files >= 16KB, traditional read for smaller files.
pub fn get_file_hash(path: &Path) -> Result<String, DvsError> {
    todo!("Compute file hash using blake3")
}

/// Get the storage path for a given hash.
///
/// Storage structure: `{storage_dir}/{first_2_chars}/{remaining_62_chars}`
pub fn storage_path_for_hash(storage_dir: &Path, hash: &str) -> std::path::PathBuf {
    todo!("Compute storage path from hash")
}

/// Hash threshold for memory-mapped I/O (16KB).
pub const MMAP_THRESHOLD: u64 = 16 * 1024;

/// Hash a file using memory-mapped I/O.
fn hash_mmap(path: &Path) -> Result<String, DvsError> {
    todo!("Hash file using mmap")
}

/// Hash a file using traditional read.
fn hash_read(path: &Path) -> Result<String, DvsError> {
    todo!("Hash file using read")
}
