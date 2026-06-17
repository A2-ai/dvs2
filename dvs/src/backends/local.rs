use std::collections::HashSet;
use std::fs::OpenOptions;
use std::io::{BufReader, Write};
use std::path::{Path, PathBuf};
use std::sync::Mutex;

#[cfg(unix)]
use anyhow::anyhow;
use anyhow::{Result, bail};
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

/// Detect the current user's primary group name.
#[cfg(unix)]
fn detect_primary_group() -> Result<String> {
    use nix::unistd::Group;
    let gid = nix::unistd::getegid();
    let group = Group::from_gid(gid)?
        .ok_or_else(|| anyhow!("Could not resolve primary group for GID {}", gid))?;
    Ok(group.name)
}

fn make_readonly(path: impl AsRef<Path>) -> Result<()> {
    let mut perms = fs::metadata(path.as_ref())?.permissions();
    perms.set_readonly(true);
    fs::set_permissions(path.as_ref(), perms)?;
    Ok(())
}

const SHARED_DIRECTORY_MODE: u32 = 0o2770;
const SHARED_BLOB_MODE: u32 = 0o0440;
const SHARED_AUDIT_LOG_MODE: u32 = 0o0660;

const OPEN_DIRECTORY_MODE: u32 = 0o2777;
const OPEN_AUDIT_LOG_MODE: u32 = 0o0666;

#[cfg(unix)]
fn ensure_mode(path: impl AsRef<Path>, mode: u32) -> Result<()> {
    use std::os::unix::fs::PermissionsExt;

    let path = path.as_ref();
    let mut perms = fs::metadata(path)?.permissions();
    if perms.mode() & 0o7777 != mode {
        perms.set_mode(mode);
        fs::set_permissions(path, perms)?;
    }
    Ok(())
}

#[cfg(not(unix))]
fn ensure_mode(_path: impl AsRef<Path>, _mode: u32) -> Result<()> {
    Ok(())
}

#[derive(Debug, Serialize, Deserialize, Clone, PartialEq)]
pub struct LocalBackend {
    pub path: PathBuf,
    group: Option<String>,
    #[serde(default, skip_serializing_if = "core::ops::Not::not")]
    open: bool,
}

impl LocalBackend {
    pub fn new(path: impl AsRef<Path>, group: Option<String>) -> Result<Self> {
        let group = match group {
            Some(g) => {
                resolve_group(&g)?;
                Some(g)
            }
            None => {
                #[cfg(unix)]
                {
                    match detect_primary_group() {
                        Ok(g) => {
                            log::debug!("Auto-detected primary group: {}", g);
                            Some(g)
                        }
                        Err(e) => {
                            log::warn!("Could not detect primary group: {e}");
                            None
                        }
                    }
                }
                #[cfg(not(unix))]
                {
                    None
                }
            }
        };

        Ok(Self {
            path: path.as_ref().to_path_buf(),
            group,
            open: false,
        })
    }

    /// Apply configured group ownership to a path.
    /// No-op on non-Unix or if no group is set.
    #[cfg(unix)]
    fn apply_group(&self, path: impl AsRef<Path>) -> Result<()> {
        use std::os::unix::fs::MetadataExt;

        use nix::unistd::chown;

        if let Some(group_name) = &self.group {
            let path = path.as_ref();
            let gid = resolve_group(group_name)?;
            if fs::metadata(path)?.gid() == gid.as_raw() {
                return Ok(());
            }

            log::debug!("Setting group {} on {}", group_name, path.display());
            chown(path, None, Some(gid))?;
        }

        Ok(())
    }

    #[cfg(not(unix))]
    fn apply_group(&self, _path: impl AsRef<Path>) -> Result<()> {
        Ok(())
    }

    fn ensure_group_and_mode(&self, path: impl AsRef<Path>, mode: u32) -> Result<()> {
        if self.group.is_some() || self.open {
            let path = path.as_ref();
            self.apply_group(path)?;
            ensure_mode(path, mode)?;
        }
        Ok(())
    }

    fn dir_mode(&self) -> u32 {
        if self.open {
            OPEN_DIRECTORY_MODE
        } else {
            SHARED_DIRECTORY_MODE
        }
    }

    fn audit_mode(&self) -> u32 {
        if self.open {
            OPEN_AUDIT_LOG_MODE
        } else {
            SHARED_AUDIT_LOG_MODE
        }
    }

    // On non-Unix, a configured group is intentionally a no-op. We still need
    // blobs to become read-only eg on Windows, so this cannot be just `self.group.is_some()`.
    fn use_shared_blob_mode(&self) -> bool {
        #[cfg(unix)]
        {
            self.group.is_some()
        }
        #[cfg(not(unix))]
        {
            false
        }
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
    fn is_initialized(&self) -> Result<bool> {
        Ok(self.path.join(AUDIT_LOG_FILENAME).exists())
    }

    fn local_path(&self) -> Option<&Path> {
        Some(&self.path)
    }

    fn init(&self) -> Result<()> {
        log::debug!("Creating storage directory: {}", self.path.display());
        fs::create_dir_all(&self.path)?;
        self.ensure_group_and_mode(&self.path, self.dir_mode())?;
        log::info!("Initialized local storage at {}", self.path.display());
        Ok(())
    }

    fn store(
        &self,
        hash: &Hashes,
        source: &Path,
        compression: Compression,
        on_bytes: Option<&(dyn Fn(u64) + Send + Sync)>,
    ) -> Result<u64> {
        let path = self.hash_to_path(hash)?;
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
            self.ensure_group_and_mode(&self.path, self.dir_mode())?;
            self.ensure_group_and_mode(parent, self.dir_mode())?;
        }
        // Serialize writers targeting the same blob with an exclusive lock on a
        // per-hash lockfile. Two parallel writes of identical content hash to the
        // same final path; without this they share `<hash>.tmp` and race on the
        // rename (one wins, the loser's tmp is already gone -> #216). Distinct
        // content uses a different lockfile, so unrelated writes still run in
        // parallel. The lock covers in-process threads and concurrent dvs
        // processes (advisory flock; best-effort on NFS).
        let lock = std::fs::File::create(path.with_extension("lock"))?;
        lock.lock()?;

        // Another writer may have already stored this exact blob while we waited.
        if path.is_file() {
            return Ok(fs::metadata(&path)?.len());
        }

        let tmp_path = path.with_extension("tmp");
        let stored_size = compression.compress(source, &tmp_path, on_bytes)?;

        if self.use_shared_blob_mode() {
            self.ensure_group_and_mode(&tmp_path, SHARED_BLOB_MODE)?;
        } else {
            make_readonly(&tmp_path)?;
        }
        fs::rename(&tmp_path, &path)?;
        Ok(stored_size)
    }

    fn retrieve(
        &self,
        hash: &Hashes,
        target: &Path,
        compression: Compression,
        on_bytes: Option<&(dyn Fn(u64) + Send + Sync)>,
    ) -> Result<bool> {
        let path = self.hash_to_path(hash)?;
        if path.is_file() {
            if let Some(parent) = target.parent() {
                fs::create_dir_all(parent)?;
            }
            compression.decompress(&path, target, on_bytes)?;
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

        fs::create_dir_all(&self.path)?;
        self.ensure_group_and_mode(&self.path, self.dir_mode())?;
        let audit_path = self.path.join(AUDIT_LOG_FILENAME);
        let mut file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&audit_path)?;
        self.ensure_group_and_mode(&audit_path, self.audit_mode())?;
        let json = serde_json::to_string(entry)?;
        writeln!(file, "{}", json)?;
        Ok(())
    }

    /// An empty `files` slice returns the full audit log. See
    /// [`parse_audit_log`] for the filtering rules.
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
        backend
            .store(&hash, &source, Compression::None, None)
            .unwrap();

        let stored = storage.join("d4").join("1d8cd98f00b204e9800998ecf8427e");
        assert!(stored.is_file());
        assert_eq!(fs::read(&stored).unwrap(), b"test content");
    }

    #[test]
    fn concurrent_store_of_identical_content_does_not_race() {
        use std::sync::Arc;
        use std::thread;

        let tmp = tempfile::tempdir().unwrap();
        let storage = tmp.path().join("storage");
        let backend = Arc::new(LocalBackend::new(&storage, None).unwrap());
        backend.init().unwrap();

        // Identical content => identical hash => same final blob path for every
        // writer. Pre-fix, parallel writers shared `<hash>.tmp` and the losers
        // failed the rename (#216).
        let hash = test_hash("d41d8cd98f00b204e9800998ecf8427e");

        let handles: Vec<_> = (0..8)
            .map(|i| {
                let backend = Arc::clone(&backend);
                let src = tmp.path().join(format!("src{i}.bin"));
                fs::write(&src, b"identical content").unwrap();
                let hash = hash.clone();
                thread::spawn(move || backend.store(&hash, &src, Compression::None, None))
            })
            .collect();

        for h in handles {
            h.join()
                .unwrap()
                .expect("every concurrent store must succeed");
        }

        let stored = storage.join("d4").join("1d8cd98f00b204e9800998ecf8427e");
        assert!(stored.is_file());
        assert_eq!(fs::read(&stored).unwrap(), b"identical content");
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
        backend
            .store(&hash, &source, Compression::None, None)
            .unwrap();

        // Retrieve to new location
        let target = tmp.path().join("retrieved.txt");
        let result = backend
            .retrieve(&hash, &target, Compression::None, None)
            .unwrap();

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
                None,
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
        backend
            .store(&hash, &source, Compression::None, None)
            .unwrap();
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
        backend
            .store(&hash, &source, Compression::None, None)
            .unwrap();
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
        backend
            .store(&hash, &source, Compression::None, None)
            .unwrap();

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
            timestamp: jiff::Timestamp::from_second(1000000000).unwrap(),
            user: "alice".to_string(),
            action: Action::Add {
                file: AuditFile {
                    path: PathBuf::from("file1.txt"),
                    hashes: hash.clone(),
                },
                compression: Compression::Zstd,
            },
        };

        let entry2 = AuditEntry {
            operation_id: "op-2".to_string(),
            timestamp: jiff::Timestamp::from_second(2000000000).unwrap(),
            user: "bob".to_string(),
            action: Action::Add {
                file: AuditFile {
                    path: PathBuf::from("file2.txt"),
                    hashes: hash.clone(),
                },
                compression: Compression::Zstd,
            },
        };

        backend.log_audit(&entry1).unwrap();
        backend.log_audit(&entry2).unwrap();

        let audit_path = storage.join("audit.log.jsonl");
        assert!(audit_path.is_file());

        let content = fs::read(&audit_path).unwrap();
        let entries = parse_audit_log(Cursor::new(content), &HashSet::new()).unwrap();
        assert_eq!(entries.len(), 2);

        assert_eq!(entries[0].operation_id, "op-1");
        // i64-seconds wire format still deserializes into a typed Timestamp
        assert_eq!(entries[0].timestamp.as_second(), 1000000000);
        assert_eq!(entries[0].user, "alice");

        assert_eq!(entries[1].operation_id, "op-2");
        assert_eq!(entries[1].timestamp.as_second(), 2000000000);
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
                            timestamp: jiff::Timestamp::from_second(
                                (worker * entries_per_worker + idx) as i64,
                            )
                            .unwrap(),
                            user: format!("user-{worker}"),
                            action: Action::Add {
                                file: AuditFile {
                                    path: PathBuf::from(format!("file-{worker}-{idx}.txt")),
                                    hashes: hash.clone(),
                                },
                                compression: Compression::Zstd,
                            },
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

    #[cfg(unix)]
    fn test_audit_entry() -> AuditEntry {
        AuditEntry {
            operation_id: "op-1".to_string(),
            timestamp: jiff::Timestamp::from_second(1000000000).unwrap(),
            user: "alice".to_string(),
            action: Action::Add {
                file: AuditFile {
                    path: PathBuf::from("file1.txt"),
                    hashes: test_hash("abc123def456789012345678901234ab"),
                },
                compression: Compression::Zstd,
            },
        }
    }

    #[cfg(unix)]
    #[test]
    fn permissions_with_group_use_shared_modes() {
        use nix::unistd::{Group, getegid};
        use std::os::unix::fs::PermissionsExt;

        let current_group_name = Group::from_gid(getegid()).unwrap().unwrap().name;

        let tmp = tempfile::tempdir().unwrap();
        let storage = tmp.path().join("storage");
        let backend = LocalBackend::new(&storage, Some(current_group_name)).unwrap();

        backend.init().unwrap();
        let storage_mode = fs::metadata(&storage).unwrap().permissions().mode();
        assert_eq!(storage_mode & 0o7777, SHARED_DIRECTORY_MODE);

        let hash = test_hash("abc123def456789012345678901234ab");
        let source = tmp.path().join("source.txt");
        fs::write(&source, b"content").unwrap();
        backend
            .store(&hash, &source, Compression::None, None)
            .unwrap();

        let prefix_dir = storage.join("ab");
        let prefix_mode = fs::metadata(&prefix_dir).unwrap().permissions().mode();
        assert_eq!(prefix_mode & 0o7777, SHARED_DIRECTORY_MODE);

        let stored = prefix_dir.join("c123def456789012345678901234ab");
        let file_mode = fs::metadata(&stored).unwrap().permissions().mode();
        assert_eq!(file_mode & 0o777, SHARED_BLOB_MODE);

        backend.log_audit(&test_audit_entry()).unwrap();
        let audit_path = storage.join("audit.log.jsonl");
        let audit_mode = fs::metadata(&audit_path).unwrap().permissions().mode();
        assert_eq!(audit_mode & 0o777, SHARED_AUDIT_LOG_MODE);
    }
}
