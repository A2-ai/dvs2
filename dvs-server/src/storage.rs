//! Storage backend abstraction.
//!
//! Provides content-addressable storage for DVS objects using the
//! `{algo}/{prefix}/{suffix}` path format compatible with dvs-core.

use std::path::PathBuf;
use fs_err as fs;
use dvs_core::Oid;
use crate::ServerError;

/// Storage backend trait for file storage operations.
///
/// All operations use [`Oid`] which includes the hash algorithm prefix.
pub trait StorageBackend: Send + Sync {
    /// Check if an object with the given OID exists.
    fn exists(&self, oid: &Oid) -> Result<bool, ServerError>;

    /// Get the path to an object by OID.
    fn get_path(&self, oid: &Oid) -> Result<PathBuf, ServerError>;

    /// Read object data by OID.
    fn get(&self, oid: &Oid) -> Result<Vec<u8>, ServerError>;

    /// Store object data with a known OID.
    ///
    /// The caller must have already computed the OID. This is idempotent -
    /// storing the same data twice is a no-op.
    fn put(&self, oid: &Oid, data: &[u8]) -> Result<(), ServerError>;

    /// Delete an object by OID.
    fn delete(&self, oid: &Oid) -> Result<(), ServerError>;

    /// Get storage statistics.
    fn stats(&self) -> Result<StorageStats, ServerError>;
}

/// Storage statistics.
#[derive(Debug, Clone, Default, serde::Serialize)]
pub struct StorageStats {
    /// Total bytes used.
    pub bytes_used: u64,
    /// Number of objects stored.
    pub object_count: u64,
    /// Total capacity (if known).
    pub capacity: Option<u64>,
}

/// Local filesystem storage backend.
///
/// Stores objects in a content-addressable layout:
/// `{root}/{algo}/{prefix}/{suffix}`
///
/// For example, a BLAKE3 hash `abc123...` would be stored at:
/// `{root}/blake3/ab/c123...`
pub struct LocalStorage {
    root: PathBuf,
}

impl LocalStorage {
    /// Create a new local storage backend.
    ///
    /// Creates the root directory if it doesn't exist.
    pub fn new(root: PathBuf) -> Result<Self, ServerError> {
        if !root.exists() {
            fs::create_dir_all(&root).map_err(|e| {
                ServerError::StorageError(format!("failed to create storage root: {e}"))
            })?;
        }
        Ok(Self { root })
    }

    /// Get the root directory.
    pub fn root(&self) -> &PathBuf {
        &self.root
    }

    /// Get storage path for an OID.
    ///
    /// Returns `{root}/{algo}/{prefix}/{suffix}` where prefix is first 2 chars.
    fn oid_to_path(&self, oid: &Oid) -> PathBuf {
        let (prefix, suffix) = oid.storage_path_components();
        self.root
            .join(oid.algo.prefix())
            .join(prefix)
            .join(suffix)
    }

    /// Ensure parent directories exist for a path.
    fn ensure_parent_dirs(&self, path: &std::path::Path) -> Result<(), ServerError> {
        if let Some(parent) = path.parent() {
            if !parent.exists() {
                fs::create_dir_all(parent).map_err(|e| {
                    ServerError::StorageError(format!("failed to create directory: {e}"))
                })?;
            }
        }
        Ok(())
    }
}

impl StorageBackend for LocalStorage {
    fn exists(&self, oid: &Oid) -> Result<bool, ServerError> {
        let path = self.oid_to_path(oid);
        Ok(path.exists())
    }

    fn get_path(&self, oid: &Oid) -> Result<PathBuf, ServerError> {
        let path = self.oid_to_path(oid);
        if path.exists() {
            Ok(path)
        } else {
            Err(ServerError::NotFound(format!("object not found: {oid}")))
        }
    }

    fn get(&self, oid: &Oid) -> Result<Vec<u8>, ServerError> {
        let path = self.get_path(oid)?;
        fs::read(&path).map_err(|e| {
            ServerError::StorageError(format!("failed to read object {oid}: {e}"))
        })
    }

    fn put(&self, oid: &Oid, data: &[u8]) -> Result<(), ServerError> {
        let path = self.oid_to_path(oid);

        // Idempotent: skip if already exists
        if path.exists() {
            return Ok(());
        }

        self.ensure_parent_dirs(&path)?;

        // Write to temp file then rename for atomicity
        let temp_path = path.with_extension("tmp");
        fs::write(&temp_path, data).map_err(|e| {
            ServerError::StorageError(format!("failed to write object {oid}: {e}"))
        })?;

        fs::rename(&temp_path, &path).map_err(|e| {
            // Clean up temp file on rename failure
            let _ = fs::remove_file(&temp_path);
            ServerError::StorageError(format!("failed to commit object {oid}: {e}"))
        })?;

        Ok(())
    }

    fn delete(&self, oid: &Oid) -> Result<(), ServerError> {
        let path = self.oid_to_path(oid);
        if path.exists() {
            fs::remove_file(&path).map_err(|e| {
                ServerError::StorageError(format!("failed to delete object {oid}: {e}"))
            })?;
        }
        Ok(())
    }

    fn stats(&self) -> Result<StorageStats, ServerError> {
        let mut stats = StorageStats::default();

        // Walk the storage directory and count objects
        fn walk_dir(dir: &PathBuf, stats: &mut StorageStats) -> std::io::Result<()> {
            if !dir.is_dir() {
                return Ok(());
            }
            for entry in fs::read_dir(dir)? {
                let entry = entry?;
                let path = entry.path();
                if path.is_dir() {
                    walk_dir(&path, stats)?;
                } else if path.is_file() {
                    stats.object_count += 1;
                    stats.bytes_used += entry.metadata()?.len();
                }
            }
            Ok(())
        }

        walk_dir(&self.root, &mut stats).map_err(|e| {
            ServerError::StorageError(format!("failed to compute stats: {e}"))
        })?;

        Ok(stats)
    }
}

/// Validate an OID string format.
///
/// Expected format: `{algo}:{hex}` where algo is blake3, sha256, or xxh3.
pub fn validate_oid(oid_str: &str) -> bool {
    Oid::parse(oid_str).is_ok()
}

/// Parse an OID from algorithm and hash components.
///
/// This is used when the algo and hash come from separate URL path segments.
pub fn parse_oid(algo: &str, hash: &str) -> Result<Oid, ServerError> {
    let oid_str = format!("{algo}:{hash}");
    Oid::parse(&oid_str).map_err(|e| {
        ServerError::StorageError(format!("invalid OID format: {e}"))
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use dvs_core::HashAlgo;

    #[test]
    fn test_local_storage_new() {
        let temp_dir = std::env::temp_dir().join("dvs-server-test-storage");
        let _ = fs::remove_dir_all(&temp_dir);

        let storage = LocalStorage::new(temp_dir.clone()).unwrap();
        assert!(temp_dir.exists());
        assert_eq!(storage.root(), &temp_dir);

        let _ = fs::remove_dir_all(&temp_dir);
    }

    #[test]
    fn test_local_storage_put_get() {
        let temp_dir = std::env::temp_dir().join("dvs-server-test-put-get");
        let _ = fs::remove_dir_all(&temp_dir);

        let storage = LocalStorage::new(temp_dir.clone()).unwrap();
        let data = b"hello world";
        let hash = blake3::hash(data).to_hex().to_string();
        let oid = Oid::blake3(hash);

        // Put object
        storage.put(&oid, data).unwrap();
        assert!(storage.exists(&oid).unwrap());

        // Get object
        let retrieved = storage.get(&oid).unwrap();
        assert_eq!(retrieved, data);

        // Idempotent put
        storage.put(&oid, data).unwrap();

        let _ = fs::remove_dir_all(&temp_dir);
    }

    #[test]
    fn test_local_storage_delete() {
        let temp_dir = std::env::temp_dir().join("dvs-server-test-delete");
        let _ = fs::remove_dir_all(&temp_dir);

        let storage = LocalStorage::new(temp_dir.clone()).unwrap();
        let data = b"to be deleted";
        let hash = blake3::hash(data).to_hex().to_string();
        let oid = Oid::blake3(hash);

        storage.put(&oid, data).unwrap();
        assert!(storage.exists(&oid).unwrap());

        storage.delete(&oid).unwrap();
        assert!(!storage.exists(&oid).unwrap());

        // Delete non-existent is OK
        storage.delete(&oid).unwrap();

        let _ = fs::remove_dir_all(&temp_dir);
    }

    #[test]
    fn test_local_storage_stats() {
        let temp_dir = std::env::temp_dir().join("dvs-server-test-stats");
        let _ = fs::remove_dir_all(&temp_dir);

        let storage = LocalStorage::new(temp_dir.clone()).unwrap();

        // Empty stats
        let stats = storage.stats().unwrap();
        assert_eq!(stats.object_count, 0);
        assert_eq!(stats.bytes_used, 0);

        // Add some objects
        let data1 = b"object one";
        let data2 = b"object two longer";
        let oid1 = Oid::blake3(blake3::hash(data1).to_hex().to_string());
        let oid2 = Oid::blake3(blake3::hash(data2).to_hex().to_string());

        storage.put(&oid1, data1).unwrap();
        storage.put(&oid2, data2).unwrap();

        let stats = storage.stats().unwrap();
        assert_eq!(stats.object_count, 2);
        assert_eq!(stats.bytes_used, (data1.len() + data2.len()) as u64);

        let _ = fs::remove_dir_all(&temp_dir);
    }

    #[test]
    fn test_validate_oid() {
        // BLAKE3: 64 hex chars
        let blake3_hash = "a".repeat(64);
        assert!(validate_oid(&format!("blake3:{blake3_hash}")));

        // SHA256: 64 hex chars
        let sha256_hash = "b".repeat(64);
        assert!(validate_oid(&format!("sha256:{sha256_hash}")));

        // XXH3: 16 hex chars
        let xxh3_hash = "c".repeat(16);
        assert!(validate_oid(&format!("xxh3:{xxh3_hash}")));

        // Invalid formats
        assert!(!validate_oid("invalid"));
        assert!(!validate_oid("unknown:abc123"));
        assert!(!validate_oid("blake3:tooshort"));
    }

    #[test]
    fn test_parse_oid() {
        let blake3_hash = "a".repeat(64);
        let oid = parse_oid("blake3", &blake3_hash).unwrap();
        assert_eq!(oid.algo, HashAlgo::Blake3);
        assert_eq!(oid.hex, blake3_hash);

        // Invalid algo
        let err = parse_oid("invalid", &blake3_hash);
        assert!(err.is_err());

        // Invalid hex length
        let err = parse_oid("blake3", "tooshort");
        assert!(err.is_err());
    }
}
