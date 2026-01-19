//! Blake3 hashing utilities.

use fs_err as fs;
use fs_err::File;
use std::io::Read;
use std::path::{Path, PathBuf};
use blake3::Hasher;
use memmap2::Mmap;
use crate::DvsError;

/// Hash threshold for memory-mapped I/O (16KB).
pub const MMAP_THRESHOLD: u64 = 16 * 1024;

/// Compute the Blake3 hash of a file.
///
/// Uses memory-mapped I/O for files >= 16KB, traditional read for smaller files.
pub fn get_file_hash(path: &Path) -> Result<String, DvsError> {
    let metadata = fs::metadata(path)?;
    let size = metadata.len();

    if size >= MMAP_THRESHOLD {
        hash_mmap(path)
    } else {
        hash_read(path)
    }
}

/// Get the storage path for a given hash.
///
/// Storage structure: `{storage_dir}/{first_2_chars}/{remaining_chars}`
pub fn storage_path_for_hash(storage_dir: &Path, hash: &str) -> PathBuf {
    let (prefix, suffix) = hash.split_at(2.min(hash.len()));
    storage_dir.join(prefix).join(suffix)
}

/// Hash a file using memory-mapped I/O.
fn hash_mmap(path: &Path) -> Result<String, DvsError> {
    let file = File::open(path)?;
    let mmap = unsafe { Mmap::map(&file)? };

    let hash = blake3::hash(&mmap);
    Ok(hash.to_hex().to_string())
}

/// Hash a file using traditional read.
fn hash_read(path: &Path) -> Result<String, DvsError> {
    let mut file = File::open(path)?;
    let mut hasher = Hasher::new();

    let mut buffer = [0u8; 8192];
    loop {
        let bytes_read = file.read(&mut buffer)?;
        if bytes_read == 0 {
            break;
        }
        hasher.update(&buffer[..bytes_read]);
    }

    Ok(hasher.finalize().to_hex().to_string())
}

/// Verify that a file matches the expected hash.
pub fn verify_hash(path: &Path, expected_hash: &str) -> Result<bool, DvsError> {
    let actual_hash = get_file_hash(path)?;
    Ok(actual_hash == expected_hash)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    #[test]
    fn test_storage_path_for_hash() {
        let storage = PathBuf::from("/storage");
        let hash = "abc123def456";
        let path = storage_path_for_hash(&storage, hash);
        assert_eq!(path, PathBuf::from("/storage/ab/c123def456"));
    }

    #[test]
    fn test_hash_small_file() {
        let temp_dir = std::env::temp_dir().join("dvs-test-hash-small");
        let _ = fs::create_dir_all(&temp_dir);
        let file_path = temp_dir.join("small.txt");

        let mut file = File::create(&file_path).unwrap();
        file.write_all(b"hello world").unwrap();
        drop(file);

        let hash = get_file_hash(&file_path).unwrap();
        // Blake3 hash of "hello world"
        assert_eq!(hash.len(), 64); // Blake3 produces 64-char hex string

        // Cleanup
        let _ = fs::remove_dir_all(&temp_dir);
    }

    #[test]
    fn test_hash_consistency() {
        let temp_dir = std::env::temp_dir().join("dvs-test-hash-consistency");
        let _ = fs::create_dir_all(&temp_dir);
        let file_path = temp_dir.join("test.txt");

        let mut file = File::create(&file_path).unwrap();
        file.write_all(b"test content for hashing").unwrap();
        drop(file);

        // Hash should be consistent across calls
        let hash1 = get_file_hash(&file_path).unwrap();
        let hash2 = get_file_hash(&file_path).unwrap();
        assert_eq!(hash1, hash2);

        // Cleanup
        let _ = fs::remove_dir_all(&temp_dir);
    }

    #[test]
    fn test_verify_hash() {
        let temp_dir = std::env::temp_dir().join("dvs-test-verify-hash");
        let _ = fs::create_dir_all(&temp_dir);
        let file_path = temp_dir.join("verify.txt");

        let mut file = File::create(&file_path).unwrap();
        file.write_all(b"verify me").unwrap();
        drop(file);

        let hash = get_file_hash(&file_path).unwrap();
        assert!(verify_hash(&file_path, &hash).unwrap());
        assert!(!verify_hash(&file_path, "wrong_hash").unwrap());

        // Cleanup
        let _ = fs::remove_dir_all(&temp_dir);
    }
}
