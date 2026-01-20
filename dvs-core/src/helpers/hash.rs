//! Multi-algorithm hashing utilities.
//!
//! Supports BLAKE3 (default), XXH3 (fast), and SHA-256 (interop) via feature flags.
//!
//! With the `mmap` feature enabled, files >= 16KB use memory-mapped I/O for performance.
//! Without the `mmap` feature, all files use streaming reads.

use crate::{DvsError, HashAlgo};
use fs_err as fs;
use fs_err::File;
#[cfg(feature = "mmap")]
use memmap2::Mmap;
use std::io::Read;
use std::path::{Path, PathBuf};

/// Hash threshold for memory-mapped I/O (16KB).
///
/// With `mmap` feature: files >= this size use mmap
/// Without `mmap` feature: all files use streaming read
#[cfg(feature = "mmap")]
pub const MMAP_THRESHOLD: u64 = 16 * 1024;

#[cfg(not(feature = "mmap"))]
pub const MMAP_THRESHOLD: u64 = u64::MAX; // Never use mmap

// ============================================================================
// Hasher trait for streaming hash computation
// ============================================================================

/// Streaming hasher interface.
///
/// Allows incremental hash computation via `update()` calls followed by `finalize()`.
pub trait Hasher: Send {
    /// Update the hasher with more data.
    fn update(&mut self, data: &[u8]);

    /// Finalize and return the hex-encoded hash.
    fn finalize(self: Box<Self>) -> String;

    /// Get the algorithm this hasher uses.
    fn algorithm(&self) -> HashAlgo;
}

// ============================================================================
// BLAKE3 hasher (feature: blake3)
// ============================================================================

#[cfg(feature = "blake3")]
mod blake3_impl {
    use super::*;

    /// BLAKE3 streaming hasher.
    pub struct Blake3Hasher {
        inner: blake3::Hasher,
    }

    impl Blake3Hasher {
        /// Create a new BLAKE3 hasher.
        pub fn new() -> Self {
            Self {
                inner: blake3::Hasher::new(),
            }
        }
    }

    impl Default for Blake3Hasher {
        fn default() -> Self {
            Self::new()
        }
    }

    impl Hasher for Blake3Hasher {
        fn update(&mut self, data: &[u8]) {
            self.inner.update(data);
        }

        fn finalize(self: Box<Self>) -> String {
            self.inner.finalize().to_hex().to_string()
        }

        fn algorithm(&self) -> HashAlgo {
            HashAlgo::Blake3
        }
    }

    /// Hash data with BLAKE3.
    pub fn hash_blake3(data: &[u8]) -> String {
        blake3::hash(data).to_hex().to_string()
    }
}

#[cfg(feature = "blake3")]
pub use blake3_impl::{hash_blake3, Blake3Hasher};

// ============================================================================
// XXH3 hasher (feature: xxh3)
// ============================================================================

#[cfg(feature = "xxh3")]
mod xxh3_impl {
    use super::*;
    use xxhash_rust::xxh3::Xxh3;

    /// XXH3 streaming hasher (fast, non-cryptographic).
    pub struct Xxh3Hasher {
        inner: Xxh3,
    }

    impl Xxh3Hasher {
        /// Create a new XXH3 hasher.
        pub fn new() -> Self {
            Self { inner: Xxh3::new() }
        }
    }

    impl Default for Xxh3Hasher {
        fn default() -> Self {
            Self::new()
        }
    }

    impl Hasher for Xxh3Hasher {
        fn update(&mut self, data: &[u8]) {
            self.inner.update(data);
        }

        fn finalize(self: Box<Self>) -> String {
            format!("{:016x}", self.inner.digest())
        }

        fn algorithm(&self) -> HashAlgo {
            HashAlgo::Xxh3
        }
    }

    /// Hash data with XXH3.
    pub fn hash_xxh3(data: &[u8]) -> String {
        format!("{:016x}", xxhash_rust::xxh3::xxh3_64(data))
    }
}

#[cfg(feature = "xxh3")]
pub use xxh3_impl::{hash_xxh3, Xxh3Hasher};

// ============================================================================
// SHA-256 hasher (feature: sha256)
// ============================================================================

#[cfg(feature = "sha256")]
mod sha256_impl {
    use super::*;
    use sha2::{Digest, Sha256};

    /// SHA-256 streaming hasher.
    pub struct Sha256Hasher {
        inner: Sha256,
    }

    impl Sha256Hasher {
        /// Create a new SHA-256 hasher.
        pub fn new() -> Self {
            Self {
                inner: Sha256::new(),
            }
        }
    }

    impl Default for Sha256Hasher {
        fn default() -> Self {
            Self::new()
        }
    }

    impl Hasher for Sha256Hasher {
        fn update(&mut self, data: &[u8]) {
            self.inner.update(data);
        }

        fn finalize(self: Box<Self>) -> String {
            format!("{:x}", self.inner.finalize())
        }

        fn algorithm(&self) -> HashAlgo {
            HashAlgo::Sha256
        }
    }

    /// Hash data with SHA-256.
    pub fn hash_sha256(data: &[u8]) -> String {
        let mut hasher = Sha256::new();
        hasher.update(data);
        format!("{:x}", hasher.finalize())
    }
}

#[cfg(feature = "sha256")]
pub use sha256_impl::{hash_sha256, Sha256Hasher};

// ============================================================================
// Multi-algorithm file hashing
// ============================================================================

/// Create a new hasher for the given algorithm.
///
/// Returns an error if the algorithm's feature is not enabled.
pub fn create_hasher(algo: HashAlgo) -> Result<Box<dyn Hasher>, DvsError> {
    match algo {
        #[cfg(feature = "blake3")]
        HashAlgo::Blake3 => Ok(Box::new(Blake3Hasher::new())),

        #[cfg(feature = "xxh3")]
        HashAlgo::Xxh3 => Ok(Box::new(Xxh3Hasher::new())),

        #[cfg(feature = "sha256")]
        HashAlgo::Sha256 => Ok(Box::new(Sha256Hasher::new())),

        #[allow(unreachable_patterns)]
        _ => Err(DvsError::config_error(format!(
            "Hash algorithm {} not enabled (missing feature)",
            algo
        ))),
    }
}

/// Get the default hash algorithm based on enabled features.
///
/// Priority: BLAKE3 > XXH3 > SHA-256
pub fn default_algorithm() -> HashAlgo {
    #[cfg(feature = "blake3")]
    return HashAlgo::Blake3;

    #[cfg(all(feature = "xxh3", not(feature = "blake3")))]
    return HashAlgo::Xxh3;

    #[cfg(all(feature = "sha256", not(feature = "blake3"), not(feature = "xxh3")))]
    return HashAlgo::Sha256;

    #[cfg(not(any(feature = "blake3", feature = "xxh3", feature = "sha256")))]
    compile_error!("At least one hash algorithm feature must be enabled");
}

/// Compute the hash of a file using the specified algorithm.
///
/// With `mmap` feature: uses memory-mapped I/O for files >= 16KB.
/// Without `mmap` feature: always uses streaming read.
pub fn get_file_hash_with_algo(path: &Path, algo: HashAlgo) -> Result<String, DvsError> {
    let metadata = fs::metadata(path)?;
    let size = metadata.len();

    #[cfg(feature = "mmap")]
    if size >= MMAP_THRESHOLD {
        return hash_file_mmap(path, algo);
    }

    // Small files or mmap feature disabled
    let _ = size; // silence unused warning when mmap disabled
    hash_file_read(path, algo)
}

/// Compute the hash of a file using the default algorithm (BLAKE3).
///
/// With `mmap` feature: uses memory-mapped I/O for files >= 16KB.
/// Without `mmap` feature: always uses streaming read.
pub fn get_file_hash(path: &Path) -> Result<String, DvsError> {
    get_file_hash_with_algo(path, default_algorithm())
}

/// Hash a file using memory-mapped I/O.
#[cfg(feature = "mmap")]
fn hash_file_mmap(path: &Path, algo: HashAlgo) -> Result<String, DvsError> {
    let file = File::open(path)?;
    let mmap = unsafe { Mmap::map(&file)? };

    hash_bytes(&mmap, algo)
}

/// Hash a file using streaming read.
fn hash_file_read(path: &Path, algo: HashAlgo) -> Result<String, DvsError> {
    let mut file = File::open(path)?;
    let mut hasher = create_hasher(algo)?;

    let mut buffer = [0u8; 8192];
    loop {
        let bytes_read = file.read(&mut buffer)?;
        if bytes_read == 0 {
            break;
        }
        hasher.update(&buffer[..bytes_read]);
    }

    Ok(hasher.finalize())
}

/// Hash bytes with the specified algorithm.
pub fn hash_bytes(data: &[u8], algo: HashAlgo) -> Result<String, DvsError> {
    match algo {
        #[cfg(feature = "blake3")]
        HashAlgo::Blake3 => Ok(hash_blake3(data)),

        #[cfg(feature = "xxh3")]
        HashAlgo::Xxh3 => Ok(hash_xxh3(data)),

        #[cfg(feature = "sha256")]
        HashAlgo::Sha256 => Ok(hash_sha256(data)),

        #[allow(unreachable_patterns)]
        _ => Err(DvsError::config_error(format!(
            "Hash algorithm {} not enabled (missing feature)",
            algo
        ))),
    }
}

/// Get the storage path for a given hash.
///
/// Storage structure: `{storage_dir}/{first_2_chars}/{remaining_chars}`
pub fn storage_path_for_hash(storage_dir: &Path, hash: &str) -> PathBuf {
    let (prefix, suffix) = hash.split_at(2.min(hash.len()));
    storage_dir.join(prefix).join(suffix)
}

/// Verify that a file matches the expected hash.
pub fn verify_hash(path: &Path, expected_hash: &str) -> Result<bool, DvsError> {
    let actual_hash = get_file_hash(path)?;
    Ok(actual_hash == expected_hash)
}

/// Verify that a file matches the expected hash using a specific algorithm.
pub fn verify_hash_with_algo(
    path: &Path,
    expected_hash: &str,
    algo: HashAlgo,
) -> Result<bool, DvsError> {
    let actual_hash = get_file_hash_with_algo(path, algo)?;
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
    fn test_default_algorithm() {
        let algo = default_algorithm();
        // With default features, should be BLAKE3
        #[cfg(feature = "blake3")]
        assert_eq!(algo, HashAlgo::Blake3);
    }

    #[cfg(feature = "blake3")]
    #[test]
    fn test_hash_small_file_blake3() {
        let temp_dir = std::env::temp_dir().join("dvs-test-hash-small-blake3");
        let _ = fs::create_dir_all(&temp_dir);
        let file_path = temp_dir.join("small.txt");

        let mut file = File::create(&file_path).unwrap();
        file.write_all(b"hello world").unwrap();
        drop(file);

        let hash = get_file_hash_with_algo(&file_path, HashAlgo::Blake3).unwrap();
        assert_eq!(hash.len(), 64); // BLAKE3 produces 64-char hex string

        // Cleanup
        let _ = fs::remove_dir_all(&temp_dir);
    }

    #[cfg(feature = "xxh3")]
    #[test]
    fn test_hash_small_file_xxh3() {
        let temp_dir = std::env::temp_dir().join("dvs-test-hash-small-xxh3");
        let _ = fs::create_dir_all(&temp_dir);
        let file_path = temp_dir.join("small.txt");

        let mut file = File::create(&file_path).unwrap();
        file.write_all(b"hello world").unwrap();
        drop(file);

        let hash = get_file_hash_with_algo(&file_path, HashAlgo::Xxh3).unwrap();
        assert_eq!(hash.len(), 16); // XXH3 produces 16-char hex string

        // Cleanup
        let _ = fs::remove_dir_all(&temp_dir);
    }

    #[cfg(feature = "sha256")]
    #[test]
    fn test_hash_small_file_sha256() {
        let temp_dir = std::env::temp_dir().join("dvs-test-hash-small-sha256");
        let _ = fs::create_dir_all(&temp_dir);
        let file_path = temp_dir.join("small.txt");

        let mut file = File::create(&file_path).unwrap();
        file.write_all(b"hello world").unwrap();
        drop(file);

        let hash = get_file_hash_with_algo(&file_path, HashAlgo::Sha256).unwrap();
        assert_eq!(hash.len(), 64); // SHA-256 produces 64-char hex string

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

    #[cfg(feature = "blake3")]
    #[test]
    fn test_blake3_hasher_streaming() {
        let mut hasher = Blake3Hasher::new();
        hasher.update(b"hello ");
        hasher.update(b"world");
        let hash = Box::new(hasher).finalize();

        // Should match single-shot hash
        let expected = hash_blake3(b"hello world");
        assert_eq!(hash, expected);
    }

    #[cfg(feature = "xxh3")]
    #[test]
    fn test_xxh3_hasher_streaming() {
        let mut hasher = Xxh3Hasher::new();
        hasher.update(b"hello ");
        hasher.update(b"world");
        let hash = Box::new(hasher).finalize();

        // Should match single-shot hash
        let expected = hash_xxh3(b"hello world");
        assert_eq!(hash, expected);
    }

    #[cfg(feature = "sha256")]
    #[test]
    fn test_sha256_hasher_streaming() {
        let mut hasher = Sha256Hasher::new();
        hasher.update(b"hello ");
        hasher.update(b"world");
        let hash = Box::new(hasher).finalize();

        // Should match single-shot hash
        let expected = hash_sha256(b"hello world");
        assert_eq!(hash, expected);
    }

    #[test]
    fn test_create_hasher() {
        #[cfg(feature = "blake3")]
        {
            let hasher = create_hasher(HashAlgo::Blake3).unwrap();
            assert_eq!(hasher.algorithm(), HashAlgo::Blake3);
        }

        #[cfg(feature = "xxh3")]
        {
            let hasher = create_hasher(HashAlgo::Xxh3).unwrap();
            assert_eq!(hasher.algorithm(), HashAlgo::Xxh3);
        }

        #[cfg(feature = "sha256")]
        {
            let hasher = create_hasher(HashAlgo::Sha256).unwrap();
            assert_eq!(hasher.algorithm(), HashAlgo::Sha256);
        }
    }
}
