use std::fs::OpenOptions;
use std::io::Write;
use std::path::{Path, PathBuf};

use anyhow::{Result, anyhow, bail};
use fs_err as fs;
use serde::{Deserialize, Serialize};

use crate::audit::AuditEntry;
use crate::backends::Backend;
use crate::{HashAlg, Hashes};

const AUDIT_LOG_FILENAME: &str = "audit.log.jsonl";

/// Parse a permission string as an octal mode.
/// Returns the mode as an u32.
fn parse_permissions(perms: &str) -> Result<u32> {
    let mode = u32::from_str_radix(perms, 8).map_err(|_| {
        anyhow!(
            "Invalid permission mode '{}': must be octal (e.g., '770')",
            perms
        )
    })?;
    if mode > 0o7777 {
        bail!(
            "Invalid permission mode '{}': value {} exceeds maximum 7777",
            perms,
            mode
        );
    }
    Ok(mode)
}

/// Resolve a group name to its GID.
#[cfg(unix)]
fn resolve_group(group_name: &str) -> Result<nix::unistd::Gid> {
    use nix::unistd::Group;
    let group =
        Group::from_name(group_name)?.ok_or_else(|| anyhow!("Group '{}' not found", group_name))?;
    Ok(nix::unistd::Gid::from_raw(group.gid.as_raw()))
}

#[cfg(not(unix))]
fn resolve_group(_: &str) -> Result<()> {
    Ok(())
}

#[derive(Debug, Serialize, Deserialize, Clone, PartialEq)]
pub struct LocalBackend {
    pub path: PathBuf,
    permissions: Option<String>,
    group: Option<String>,
    hash_alg: HashAlg,
}

impl LocalBackend {
    pub fn new(
        path: impl AsRef<Path>,
        permissions: Option<String>,
        group: Option<String>,
    ) -> Result<Self> {
        // Validate permissions and group before creating config
        if let Some(ref perms) = permissions {
            parse_permissions(perms)?;
        }
        if let Some(ref grp) = group {
            resolve_group(grp)?;
        }

        Ok(Self {
            path: path.as_ref().to_path_buf(),
            permissions,
            group,
            hash_alg: HashAlg::Blake3,
        })
    }

    /// Apply configured permissions and group to a path.
    /// No-op on non-Unix or if neither permissions nor group are set.
    #[cfg(unix)]
    pub fn apply_perms(&self, path: impl AsRef<Path>) -> Result<()> {
        use nix::unistd::chown;
        use std::os::unix::fs::PermissionsExt;

        let path = path.as_ref();

        if let Some(perms) = &self.permissions {
            log::debug!("Setting permissions {} on {}", perms, path.display());
            let mode = parse_permissions(perms)?;
            let permissions = std::fs::Permissions::from_mode(mode);
            fs::set_permissions(path, permissions)?;
        }

        if let Some(group_name) = &self.group {
            log::debug!("Setting group {} on {}", group_name, path.display());
            let gid = resolve_group(group_name)?;
            chown(path, None, Some(gid))?;
        }

        Ok(())
    }

    #[cfg(not(unix))]
    pub fn apply_perms(&self, _path: impl AsRef<Path>) -> Result<()> {
        Ok(())
    }

    fn hash_to_path(&self, hashes: &Hashes) -> Result<PathBuf> {
        let hash = hashes.get_by_alg(self.hash_alg);
        if hash.len() < 3 || !hash.chars().all(|c| c.is_ascii_hexdigit()) {
            bail!("Invalid hash: {}", hash);
        }
        let (prefix, suffix) = hash.split_at(2);
        Ok(self.path.join(prefix).join(suffix))
    }
}

impl Backend for LocalBackend {
    fn init(&self) -> Result<()> {
        log::debug!("Creating storage directory: {}", self.path.display());
        fs::create_dir_all(&self.path)?;
        self.apply_perms(&self.path)?;
        log::info!("Initialized local storage at {}", self.path.display());
        Ok(())
    }

    fn store(&self, hash: &Hashes, source: &Path) -> Result<()> {
        let path = self.hash_to_path(hash)?;
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
            self.apply_perms(parent)?;
        }
        fs::copy(source, &path)?;
        self.apply_perms(&path)?;
        Ok(())
    }

    fn store_bytes(&self, hash: &Hashes, content: &[u8]) -> Result<()> {
        let path = self.hash_to_path(hash)?;
        log::debug!("Storing {} bytes to {}", content.len(), path.display());
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
            self.apply_perms(parent)?;
        }
        fs::write(&path, content)?;
        self.apply_perms(&path)?;
        Ok(())
    }

    fn retrieve(&self, hash: &Hashes, target: &Path) -> Result<bool> {
        let path = self.hash_to_path(hash)?;
        if path.is_file() {
            if let Some(parent) = target.parent() {
                fs::create_dir_all(parent)?;
            }
            fs::copy(&path, target)?;
            Ok(true)
        } else {
            Ok(false)
        }
    }

    fn exists(&self, hash: &Hashes) -> Result<bool> {
        Ok(self.hash_to_path(hash)?.is_file())
    }

    fn remove(&self, hash: &Hashes) -> Result<()> {
        let path = self.hash_to_path(hash)?;
        if path.is_file() {
            log::debug!("Removing {} from storage", hash);
            fs::remove_file(path)?;
        }
        Ok(())
    }

    fn read(&self, hash: &Hashes) -> Result<Option<Vec<u8>>> {
        let path = self.hash_to_path(hash)?;
        if path.is_file() {
            Ok(Some(fs::read(&path)?))
        } else {
            Ok(None)
        }
    }

    fn log_audit(&self, entry: &AuditEntry) -> Result<()> {
        let audit_path = self.path.join(AUDIT_LOG_FILENAME);
        let mut file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&audit_path)?;
        let json = serde_json::to_string(entry)?;
        writeln!(file, "{}", json)?;
        self.apply_perms(&audit_path)?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::hashes::Hashes;

    fn test_hash(hash: &str) -> Hashes {
        Hashes {
            blake3: hash.to_string(),
            md5: hash.to_string(),
        }
    }

    #[test]
    fn hash_to_path_rejects_bad_hash() {
        let backend = LocalBackend::new("/tmp/storage", None, None).unwrap();

        // These should error or be sanitized
        assert!(
            backend
                .hash_to_path(&test_hash("../../etc/passwd"))
                .is_err()
        );
        assert!(backend.hash_to_path(&test_hash("../escape")).is_err());
        assert!(
            backend
                .hash_to_path(&test_hash("d41d8cd98f00b204e9800998ecf8427e"))
                .is_ok()
        );
    }

    #[test]
    fn init_creates_storage_directory() {
        let tmp = tempfile::tempdir().unwrap();
        let storage_path = tmp.path().join("storage");

        let backend = LocalBackend::new(&storage_path, None, None).unwrap();
        assert!(!storage_path.exists());

        backend.init().unwrap();
        assert!(storage_path.is_dir());
    }

    #[test]
    fn store_creates_hash_prefixed_path() {
        let tmp = tempfile::tempdir().unwrap();
        let storage = tmp.path().join("storage");
        let backend = LocalBackend::new(&storage, None, None).unwrap();
        backend.init().unwrap();

        // Create source file
        let source = tmp.path().join("source.txt");
        fs::write(&source, b"test content").unwrap();

        let hash = test_hash("d41d8cd98f00b204e9800998ecf8427e");
        backend.store(&hash, &source).unwrap();

        let stored = storage.join("d4").join("1d8cd98f00b204e9800998ecf8427e");
        assert!(stored.is_file());
        assert_eq!(fs::read(&stored).unwrap(), b"test content");
    }

    #[test]
    fn retrieve_copies_to_target() {
        let tmp = tempfile::tempdir().unwrap();
        let storage = tmp.path().join("storage");
        let backend = LocalBackend::new(&storage, None, None).unwrap();
        backend.init().unwrap();

        // Store content
        let hash = test_hash("abc123def456789012345678901234ab");
        backend.store_bytes(&hash, b"stored content").unwrap();

        // Retrieve to new location
        let target = tmp.path().join("retrieved.txt");
        let result = backend.retrieve(&hash, &target).unwrap();

        // file was copied if result == true
        assert!(result);
        assert_eq!(fs::read(&target).unwrap(), b"stored content");
    }

    #[test]
    fn retrieve_returns_false_when_missing() {
        let tmp = tempfile::tempdir().unwrap();
        let storage = tmp.path().join("storage");
        let backend = LocalBackend::new(&storage, None, None).unwrap();
        backend.init().unwrap();

        let target = tmp.path().join("target.txt");
        let result = backend
            .retrieve(&test_hash("1234567890123456789012"), &target)
            .unwrap();

        assert!(!result);
        assert!(!target.exists());
    }

    #[test]
    fn exists_returns_true_for_stored() {
        let tmp = tempfile::tempdir().unwrap();
        let storage = tmp.path().join("storage");
        let backend = LocalBackend::new(&storage, None, None).unwrap();
        backend.init().unwrap();

        let hash = test_hash("abc123def456789012345678901234ab");
        assert!(!backend.exists(&hash).unwrap());
        backend.store_bytes(&hash, b"content").unwrap();
        assert!(backend.exists(&hash).unwrap());
    }

    #[test]
    fn remove_deletes_stored_file() {
        let tmp = tempfile::tempdir().unwrap();
        let storage = tmp.path().join("storage");
        let backend = LocalBackend::new(&storage, None, None).unwrap();
        backend.init().unwrap();

        let hash = test_hash("abc123def456789012345678901234ab");
        backend.store_bytes(&hash, b"content").unwrap();
        assert!(backend.exists(&hash).unwrap());

        backend.remove(&hash).unwrap();
        assert!(!backend.exists(&hash).unwrap());
        // removing something that doesn't exist is a noop
        backend.remove(&hash).unwrap();
    }

    #[test]
    fn read_returns_content() {
        let tmp = tempfile::tempdir().unwrap();
        let storage = tmp.path().join("storage");
        let backend = LocalBackend::new(&storage, None, None).unwrap();
        backend.init().unwrap();

        let hash = test_hash("abc123def456789012345678901234ab");
        backend.store_bytes(&hash, b"read me").unwrap();

        let content = backend.read(&hash).unwrap();
        assert_eq!(content, Some(b"read me".to_vec()));
        // None if hash is not found
        let content = backend.read(&test_hash("1234567890123456789012")).unwrap();
        assert_eq!(content, None);
    }

    #[cfg(unix)]
    #[test]
    fn apply_perms_works() {
        use std::os::unix::fs::PermissionsExt;

        let tmp = tempfile::tempdir().unwrap();
        let storage = tmp.path().join("storage");
        let backend = LocalBackend::new(&storage, Some("750".to_string()), None).unwrap();
        backend.init().unwrap();

        let hash = test_hash("abc123def456789012345678901234ab");
        backend.store_bytes(&hash, b"content").unwrap();

        let stored = storage.join("ab").join("c123def456789012345678901234ab");
        let mode = fs::metadata(&stored).unwrap().permissions().mode();
        assert_eq!(mode & 0o777, 0o750);
    }
}
