//! Object store abstraction for remote/local content-addressable storage.

use std::fs;
use std::path::{Path, PathBuf};
use crate::DvsError;
use crate::types::Oid;

/// Result type for store operations.
pub type StoreResult<T> = Result<T, DvsError>;

/// Object store trait for content-addressable storage.
///
/// Implementations include local filesystem, HTTP CAS, and cloud stores.
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
            return Err(DvsError::StorageError {
                message: format!("Object not found in local store: {}", oid),
            });
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
            return Err(DvsError::FileNotFound {
                path: src.to_path_buf(),
            });
        }

        let dest = self.object_path(oid);

        // Skip if already exists (content-addressable = immutable)
        if dest.exists() {
            return Ok(());
        }

        // Create parent directory
        if let Some(parent) = dest.parent() {
            fs::create_dir_all(parent)?;
        }

        fs::copy(src, &dest)?;
        Ok(())
    }

    fn store_type(&self) -> &'static str {
        "local"
    }
}

/// HTTP CAS (Content-Addressable Storage) client.
///
/// Accesses objects via HTTP endpoints:
/// - `HEAD {base}/objects/{algo}/{hash}` - check existence
/// - `GET  {base}/objects/{algo}/{hash}` - download
/// - `PUT  {base}/objects/{algo}/{hash}` - upload
#[derive(Debug, Clone)]
pub struct HttpStore {
    /// Base URL for the HTTP CAS.
    base_url: String,
}

impl HttpStore {
    /// Create a new HTTP store.
    pub fn new(base_url: String) -> Self {
        // Remove trailing slash
        let base_url = base_url.trim_end_matches('/').to_string();
        Self { base_url }
    }

    /// Get the URL for an OID.
    pub fn object_url(&self, oid: &Oid) -> String {
        format!("{}/objects/{}/{}", self.base_url, oid.algo, oid.hex)
    }
}

impl ObjectStore for HttpStore {
    fn has(&self, oid: &Oid) -> StoreResult<bool> {
        let url = self.object_url(oid);

        // Use a simple HTTP HEAD request via curl or similar
        // For now, we'll use std::process::Command to call curl
        let output = std::process::Command::new("curl")
            .args(["-s", "-o", "/dev/null", "-w", "%{http_code}", "-I", &url])
            .output()
            .map_err(|e| DvsError::StorageError {
                message: format!("Failed to execute curl: {}", e),
            })?;

        let status = String::from_utf8_lossy(&output.stdout);
        let code: u16 = status.trim().parse().unwrap_or(0);

        Ok(code == 200)
    }

    fn get(&self, oid: &Oid, dest: &Path) -> StoreResult<()> {
        let url = self.object_url(oid);

        // Create parent directory
        if let Some(parent) = dest.parent() {
            fs::create_dir_all(parent)?;
        }

        // Download using curl
        let status = std::process::Command::new("curl")
            .args(["-s", "-f", "-o", &dest.to_string_lossy(), &url])
            .status()
            .map_err(|e| DvsError::StorageError {
                message: format!("Failed to execute curl: {}", e),
            })?;

        if !status.success() {
            return Err(DvsError::StorageError {
                message: format!("Failed to download object from {}: HTTP error", url),
            });
        }

        Ok(())
    }

    fn put(&self, oid: &Oid, src: &Path) -> StoreResult<()> {
        if !src.exists() {
            return Err(DvsError::FileNotFound {
                path: src.to_path_buf(),
            });
        }

        let url = self.object_url(oid);

        // Upload using curl
        let status = std::process::Command::new("curl")
            .args([
                "-s",
                "-f",
                "-X",
                "PUT",
                "--data-binary",
                &format!("@{}", src.to_string_lossy()),
                &url,
            ])
            .status()
            .map_err(|e| DvsError::StorageError {
                message: format!("Failed to execute curl: {}", e),
            })?;

        if !status.success() {
            return Err(DvsError::StorageError {
                message: format!("Failed to upload object to {}: HTTP error", url),
            });
        }

        Ok(())
    }

    fn store_type(&self) -> &'static str {
        "http"
    }
}

/// Multi-store that tries stores in order.
///
/// Useful for local cache + remote fallback.
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
        Err(DvsError::StorageError {
            message: format!("Object not found in any store: {}", oid),
        })
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

    #[test]
    fn test_http_store_url() {
        let store = HttpStore::new("https://example.com/dvcs/".to_string());
        let oid = Oid::new(HashAlgo::Blake3, "abc".to_string() + &"0".repeat(61));
        let url = store.object_url(&oid);

        assert_eq!(url, format!("https://example.com/dvcs/objects/blake3/{}", oid.hex));
    }
}
