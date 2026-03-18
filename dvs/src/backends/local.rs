use std::collections::HashSet;
use std::fs::OpenOptions;
use std::io::{BufReader, Write};
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use anyhow::{Result, anyhow, bail};
use fs_err as fs;
use serde::{Deserialize, Serialize};

use crate::Hashes;
use crate::audit::{AuditEntry, parse_audit_log};
use crate::backends::Backend;
use crate::config::Compression;

const AUDIT_LOG_FILENAME: &str = "audit.log.jsonl";
/// Only protects the current dvs process, not concurrent dvs processes
static AUDIT_LOG_LOCK: Mutex<()> = Mutex::new(());

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

fn make_readonly(path: impl AsRef<Path>) -> Result<()> {
    let mut perms = fs::metadata(path.as_ref())?.permissions();
    perms.set_readonly(true);
    fs::set_permissions(path.as_ref(), perms)?;
    Ok(())
}

#[derive(Debug, Serialize, Deserialize, Clone, PartialEq)]
pub struct LocalBackend {
    pub path: PathBuf,
    group: Option<String>,
}

impl LocalBackend {
    pub fn new(path: impl AsRef<Path>, group: Option<String>) -> Result<Self> {
        if let Some(ref grp) = group {
            resolve_group(grp)?;
        }

        Ok(Self {
            path: path.as_ref().to_path_buf(),
            group,
        })
    }

    /// Apply configured group ownership to a path.
    /// No-op on non-Unix or if no group is set.
    #[cfg(unix)]
    fn apply_group(&self, path: impl AsRef<Path>) -> Result<()> {
        use nix::unistd::chown;

        if let Some(group_name) = &self.group {
            log::debug!(
                "Setting group {} on {}",
                group_name,
                path.as_ref().display()
            );
            let gid = resolve_group(group_name)?;
            chown(path.as_ref(), None, Some(gid))?;
        }

        Ok(())
    }

    #[cfg(not(unix))]
    fn apply_group(&self, _path: impl AsRef<Path>) -> Result<()> {
        Ok(())
    }

    fn hash_to_path(&self, hashes: &Hashes) -> Result<PathBuf> {
        let hash = hashes.get_blake3();
        if hash.len() < 3 || !hash.chars().all(|c| c.is_ascii_hexdigit()) {
            bail!("Invalid hash: {hash}");
        }
        let (prefix, suffix) = hash.split_at(2);
        Ok(self.path.join(prefix).join(suffix))
    }
}

impl Backend for LocalBackend {
    fn init(&self) -> Result<()> {
        log::debug!("Creating storage directory: {}", self.path.display());
        fs::create_dir_all(&self.path)?;
        self.apply_group(&self.path)?;
        log::info!("Initialized local storage at {}", self.path.display());
        Ok(())
    }

    fn store(&self, hash: &Hashes, source: &Path, compression: Compression) -> Result<u64> {
        let path = self.hash_to_path(hash)?;
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
            self.apply_group(parent)?;
        }
        let tmp_path = path.with_extension("tmp");
        let stored_size = compression.compress(source, &tmp_path)?;
        fs::rename(&tmp_path, &path)?;
        make_readonly(&path)?;
        self.apply_group(&path)?;
        Ok(stored_size)
    }

    fn retrieve(&self, hash: &Hashes, target: &Path, compression: Compression) -> Result<bool> {
        let path = self.hash_to_path(hash)?;
        if path.is_file() {
            if let Some(parent) = target.parent() {
                fs::create_dir_all(parent)?;
            }
            compression.decompress(&path, target)?;
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
            log::debug!("Removing {path:?} from storage");
            fs::remove_file(path)?;
        }
        Ok(())
    }

    fn log_audit(&self, entry: &AuditEntry) -> Result<()> {
        let _guard = AUDIT_LOG_LOCK
            .lock()
            .expect("audit log lock should not be poisoned");
        log::debug!("Appending {entry:?} to audit log");
        let audit_path = self.path.join(AUDIT_LOG_FILENAME);
        let mut file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&audit_path)?;
        let json = serde_json::to_string(entry)?;
        writeln!(file, "{}", json)?;
        self.apply_group(&audit_path)?;
        Ok(())
    }

    fn read_audit_file(&self, files: &[PathBuf]) -> Result<Vec<AuditEntry>> {
        let files_to_include: HashSet<_> = HashSet::from_iter(files.iter().cloned());
        let audit_path = self.path.join(AUDIT_LOG_FILENAME);
        let f = fs::File::open(&audit_path)?;
        let entries = parse_audit_log(BufReader::new(f), &files_to_include)?;
        Ok(entries)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::audit::{Action, AuditEntry, AuditFile, parse_audit_log};
    use crate::config::Compression;
    use crate::hashes::Hashes;
    use std::io::Cursor;

    fn test_hash(hash: &str) -> Hashes {
        Hashes {
            blake3: hash.to_string(),
            md5: None,
        }
    }

    #[test]
    fn hash_to_path_rejects_bad_hash() {
        let backend = LocalBackend::new("/tmp/storage", None).unwrap();

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

        let backend = LocalBackend::new(&storage_path, None).unwrap();
        assert!(!storage_path.exists());

        backend.init().unwrap();
        assert!(storage_path.is_dir());
    }

    #[test]
    fn store_creates_hash_prefixed_path() {
        let tmp = tempfile::tempdir().unwrap();
        let storage = tmp.path().join("storage");
        let backend = LocalBackend::new(&storage, None).unwrap();
        backend.init().unwrap();

        // Create source file
        let source = tmp.path().join("source.txt");
        fs::write(&source, b"test content").unwrap();

        let hash = test_hash("d41d8cd98f00b204e9800998ecf8427e");
        backend.store(&hash, &source, Compression::None).unwrap();

        let stored = storage.join("d4").join("1d8cd98f00b204e9800998ecf8427e");
        assert!(stored.is_file());
        assert_eq!(fs::read(&stored).unwrap(), b"test content");
    }

    #[test]
    fn retrieve_copies_to_target() {
        let tmp = tempfile::tempdir().unwrap();
        let storage = tmp.path().join("storage");
        let backend = LocalBackend::new(&storage, None).unwrap();
        backend.init().unwrap();

        // Store content via store()
        let hash = test_hash("abc123def456789012345678901234ab");
        let source = tmp.path().join("source.txt");
        fs::write(&source, b"stored content").unwrap();
        backend.store(&hash, &source, Compression::None).unwrap();

        // Retrieve to new location
        let target = tmp.path().join("retrieved.txt");
        let result = backend.retrieve(&hash, &target, Compression::None).unwrap();

        // file was copied if result == true
        assert!(result);
        assert_eq!(fs::read(&target).unwrap(), b"stored content");
    }

    #[test]
    fn retrieve_returns_false_when_missing() {
        let tmp = tempfile::tempdir().unwrap();
        let storage = tmp.path().join("storage");
        let backend = LocalBackend::new(&storage, None).unwrap();
        backend.init().unwrap();

        let target = tmp.path().join("target.txt");
        let result = backend
            .retrieve(
                &test_hash("1234567890123456789012"),
                &target,
                Compression::None,
            )
            .unwrap();

        assert!(!result);
        assert!(!target.exists());
    }

    #[test]
    fn exists_returns_true_for_stored() {
        let tmp = tempfile::tempdir().unwrap();
        let storage = tmp.path().join("storage");
        let backend = LocalBackend::new(&storage, None).unwrap();
        backend.init().unwrap();

        let hash = test_hash("abc123def456789012345678901234ab");
        assert!(!backend.exists(&hash).unwrap());
        let source = tmp.path().join("source.txt");
        fs::write(&source, b"content").unwrap();
        backend.store(&hash, &source, Compression::None).unwrap();
        assert!(backend.exists(&hash).unwrap());
    }

    #[test]
    fn remove_deletes_stored_file() {
        let tmp = tempfile::tempdir().unwrap();
        let storage = tmp.path().join("storage");
        let backend = LocalBackend::new(&storage, None).unwrap();
        backend.init().unwrap();

        let hash = test_hash("abc123def456789012345678901234ab");
        let source = tmp.path().join("source.txt");
        fs::write(&source, b"content").unwrap();
        backend.store(&hash, &source, Compression::None).unwrap();
        assert!(backend.exists(&hash).unwrap());

        backend.remove(&hash).unwrap();
        assert!(!backend.exists(&hash).unwrap());
        // removing something that doesn't exist is a noop
        backend.remove(&hash).unwrap();
    }

    #[test]
    fn stored_files_are_readonly() {
        let tmp = tempfile::tempdir().unwrap();
        let storage = tmp.path().join("storage");
        let backend = LocalBackend::new(&storage, None).unwrap();
        backend.init().unwrap();

        let hash = test_hash("abc123def456789012345678901234ab");
        let source = tmp.path().join("source.txt");
        fs::write(&source, b"content").unwrap();
        backend.store(&hash, &source, Compression::None).unwrap();

        let stored = storage.join("ab").join("c123def456789012345678901234ab");
        let perms = fs::metadata(&stored).unwrap().permissions();
        assert!(perms.readonly());
    }

    #[test]
    fn log_audit_appends_to_jsonl() {
        let tmp = tempfile::tempdir().unwrap();
        let storage = tmp.path().join("storage");
        let backend = LocalBackend::new(&storage, None).unwrap();
        backend.init().unwrap();

        let hash = test_hash("abc123def456789012345678901234ab");

        let entry1 = AuditEntry {
            operation_id: "op-1".to_string(),
            timestamp: 1000000000,
            user: "alice".to_string(),
            file: AuditFile {
                path: PathBuf::from("file1.txt"),
                hashes: hash.clone(),
            },
            action: Action::Add,
        };

        let entry2 = AuditEntry {
            operation_id: "op-2".to_string(),
            timestamp: 2000000000,
            user: "bob".to_string(),
            file: AuditFile {
                path: PathBuf::from("file2.txt"),
                hashes: hash.clone(),
            },
            action: Action::Add,
        };

        backend.log_audit(&entry1).unwrap();
        backend.log_audit(&entry2).unwrap();

        let audit_path = storage.join("audit.log.jsonl");
        assert!(audit_path.is_file());

        let content = fs::read(&audit_path).unwrap();
        let entries = parse_audit_log(Cursor::new(content), &HashSet::new()).unwrap();
        assert_eq!(entries.len(), 2);

        assert_eq!(entries[0].operation_id, "op-1");
        assert_eq!(entries[0].timestamp, 1000000000);
        assert_eq!(entries[0].user, "alice");

        assert_eq!(entries[1].operation_id, "op-2");
        assert_eq!(entries[1].timestamp, 2000000000);
        assert_eq!(entries[1].user, "bob");
    }

    #[test]
    fn log_audit_is_valid_jsonl_under_concurrency() {
        let tmp = tempfile::tempdir().unwrap();
        let storage = tmp.path().join("storage");
        let backend = LocalBackend::new(&storage, None).unwrap();
        backend.init().unwrap();

        let hash = test_hash("abc123def456789012345678901234ab");
        let workers = 4;
        let entries_per_worker = 64;

        std::thread::scope(|scope| {
            let backend = &backend;
            for worker in 0..workers {
                let hash = hash.clone();
                scope.spawn(move || {
                    for idx in 0..entries_per_worker {
                        let entry = AuditEntry {
                            operation_id: format!("op-{worker}-{idx}"),
                            timestamp: (worker * entries_per_worker + idx) as i64,
                            user: format!("user-{worker}"),
                            file: AuditFile {
                                path: PathBuf::from(format!("file-{worker}-{idx}.txt")),
                                hashes: hash.clone(),
                            },
                            action: Action::Add,
                        };
                        backend.log_audit(&entry).unwrap();
                    }
                });
            }
        });

        let audit_path = storage.join("audit.log.jsonl");
        let content = fs::read(&audit_path).unwrap();
        let entries = parse_audit_log(Cursor::new(content), &HashSet::new()).unwrap();
        assert_eq!(entries.len(), workers * entries_per_worker);
    }
}
