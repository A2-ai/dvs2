//! Object store abstraction for local content-addressable storage.

use crate::types::Oid;
use crate::DvsError;
use fs_err as fs;
use std::path::{Path, PathBuf};

/// Result type for store operations.
pub type StoreResult<T> = Result<T, DvsError>;

/// Object store trait for content-addressable storage.
///
/// Currently implements local filesystem storage.
pub trait ObjectStore: Send + Sync {
    /// Check if an object exists in the store.
    fn has(&self, oid: &Oid) -> StoreResult<bool>;

    /// Download an object from the store to a local path.
    fn get(&self, oid: &Oid, dest: &Path) -> StoreResult<()>;

    /// Upload an object to the store from a local path.
    fn put(&self, oid: &Oid, src: &Path) -> StoreResult<()>;

    /// Get the store type name for logging.
    fn store_type(&self) -> &'static str;
}

/// Local filesystem object store.
///
/// Stores objects in a directory structure: `{root}/{algo}/{prefix}/{suffix}`
#[derive(Debug, Clone)]
pub struct LocalStore {
    /// Root directory for object storage.
    root: PathBuf,
}

impl LocalStore {
    /// Create a new local store.
    pub fn new(root: PathBuf) -> Self {
        Self { root }
    }

    /// Get the full path for an OID.
    pub fn object_path(&self, oid: &Oid) -> PathBuf {
        self.root.join(oid.storage_subpath())
    }
}

impl ObjectStore for LocalStore {
    fn has(&self, oid: &Oid) -> StoreResult<bool> {
        Ok(self.object_path(oid).exists())
    }

    fn get(&self, oid: &Oid, dest: &Path) -> StoreResult<()> {
        let src = self.object_path(oid);
        if !src.exists() {
            return Err(DvsError::storage_error(format!(
                "Object not found in local store: {}",
                oid
            )));
        }

        // Create parent directory
        if let Some(parent) = dest.parent() {
            fs::create_dir_all(parent)?;
        }

        fs::copy(&src, dest)?;
        Ok(())
    }

    fn put(&self, oid: &Oid, src: &Path) -> StoreResult<()> {
        if !src.exists() {
            return Err(DvsError::file_not_found(src));
        }

        let dest = self.object_path(oid);

        // Skip if already exists (content-addressable = immutable)
        // This is a quick check to avoid unnecessary work, but we still
        // use atomic operations below to handle race conditions.
        if dest.exists() {
            return Ok(());
        }

        // Create parent directory
        if let Some(parent) = dest.parent() {
            fs::create_dir_all(parent)?;
        }

        // Use atomic copy: write to temp file, then rename.
        // This prevents race conditions where two processes try to create
        // the same object simultaneously.
        //
        // Generate a unique temp filename in the same directory to ensure
        // the rename operation is atomic (same filesystem).
        let temp_name = format!(
            ".tmp.{}.{}.{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or(0),
            &oid.hex
        );
        let temp_path = dest
            .parent()
            .map(|p| p.join(&temp_name))
            .unwrap_or_else(|| PathBuf::from(&temp_name));

        // Copy to temp file
        fs::copy(src, &temp_path)?;

        // Atomic rename to final destination
        // If the file already exists (created by another process), rename fails
        // on some systems. We handle this by checking if dest exists after
        // a failed rename.
        match fs::rename(&temp_path, &dest) {
            Ok(()) => Ok(()),
            Err(e) => {
                // Clean up temp file
                let _ = fs::remove_file(&temp_path);

                // If the destination now exists, another process won the race.
                // This is fine for content-addressable storage since the content
                // is identical.
                if dest.exists() {
                    Ok(())
                } else {
                    Err(DvsError::storage_error(format!(
                        "Failed to store object {}: {}",
                        oid, e
                    )))
                }
            }
        }
    }

    fn store_type(&self) -> &'static str {
        "local"
    }
}

/// Multi-store that tries stores in order.
///
/// Useful for combining multiple storage backends.
pub struct ChainStore {
    stores: Vec<Box<dyn ObjectStore>>,
}

impl ChainStore {
    /// Create a new chain store.
    pub fn new(stores: Vec<Box<dyn ObjectStore>>) -> Self {
        Self { stores }
    }
}

impl ObjectStore for ChainStore {
    fn has(&self, oid: &Oid) -> StoreResult<bool> {
        for store in &self.stores {
            if store.has(oid)? {
                return Ok(true);
            }
        }
        Ok(false)
    }

    fn get(&self, oid: &Oid, dest: &Path) -> StoreResult<()> {
        for store in &self.stores {
            if store.has(oid)? {
                return store.get(oid, dest);
            }
        }
        Err(DvsError::storage_error(format!(
            "Object not found in any store: {}",
            oid
        )))
    }

    fn put(&self, oid: &Oid, src: &Path) -> StoreResult<()> {
        // Put to all stores
        for store in &self.stores {
            store.put(oid, src)?;
        }
        Ok(())
    }

    fn store_type(&self) -> &'static str {
        "chain"
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::HashAlgo;
    use std::io::Write;

    fn test_oid() -> Oid {
        Oid::new(HashAlgo::Blake3, "a".repeat(64))
    }

    #[test]
    fn test_local_store_object_path() {
        let store = LocalStore::new(PathBuf::from("/storage"));
        let oid = test_oid();
        let path = store.object_path(&oid);

        assert!(path.to_string_lossy().contains("blake3"));
        assert!(path.to_string_lossy().contains("aa")); // first 2 chars
    }

    #[test]
    fn test_local_store_has() {
        let temp_dir = std::env::temp_dir().join("dvs-test-store-has");
        let _ = fs::remove_dir_all(&temp_dir);
        fs::create_dir_all(&temp_dir).unwrap();

        let store = LocalStore::new(temp_dir.clone());
        let oid = test_oid();

        // Not found initially
        assert!(!store.has(&oid).unwrap());

        // Create the object
        let object_path = store.object_path(&oid);
        fs::create_dir_all(object_path.parent().unwrap()).unwrap();
        fs::write(&object_path, b"content").unwrap();

        // Now found
        assert!(store.has(&oid).unwrap());

        let _ = fs::remove_dir_all(&temp_dir);
    }

    #[test]
    fn test_local_store_put_get() {
        let temp_dir = std::env::temp_dir().join("dvs-test-store-put-get");
        let _ = fs::remove_dir_all(&temp_dir);
        fs::create_dir_all(&temp_dir).unwrap();

        let store = LocalStore::new(temp_dir.join("storage"));
        let oid = test_oid();

        // Create source file
        let src = temp_dir.join("source.txt");
        let mut file = fs::File::create(&src).unwrap();
        file.write_all(b"test content").unwrap();
        drop(file);

        // Put
        store.put(&oid, &src).unwrap();
        assert!(store.has(&oid).unwrap());

        // Get
        let dest = temp_dir.join("dest.txt");
        store.get(&oid, &dest).unwrap();
        assert!(dest.exists());

        let content = fs::read_to_string(&dest).unwrap();
        assert_eq!(content, "test content");

        let _ = fs::remove_dir_all(&temp_dir);
    }

    #[test]
    fn test_local_store_put_idempotent() {
        let temp_dir = std::env::temp_dir().join("dvs-test-store-idempotent");
        let _ = fs::remove_dir_all(&temp_dir);
        fs::create_dir_all(&temp_dir).unwrap();

        let store = LocalStore::new(temp_dir.join("storage"));
        let oid = test_oid();

        // Create source file
        let src = temp_dir.join("source.txt");
        fs::write(&src, b"content").unwrap();

        // Put twice should succeed
        store.put(&oid, &src).unwrap();
        store.put(&oid, &src).unwrap();

        let _ = fs::remove_dir_all(&temp_dir);
    }
}
