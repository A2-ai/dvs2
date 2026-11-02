use std::collections::HashSet;
use std::io::{BufReader, Write};
use std::path::{Path, PathBuf};
use std::sync::Mutex;

#[cfg(unix)]
use anyhow::anyhow;
use anyhow::{Result, bail};
use fs_err as fs;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::audit::{AuditEntry, AuditFile, parse_audit_log};
use crate::backends::{RetrieveRequest, StoreRequest, StoreResult};
use crate::{Compression, Hashes};

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

/// Whether this user owns `path`, and so may change its group and mode.
#[cfg(unix)]
fn owned_by_me(path: &Path) -> bool {
    use std::os::unix::fs::MetadataExt;
    fs::metadata(path).is_ok_and(|m| m.uid() == nix::unistd::geteuid().as_raw())
}

#[cfg(not(unix))]
fn owned_by_me(_path: &Path) -> bool {
    true
}

/// The rule dvs-server applies to project names.
const MAX_PROJECT_NAME_LEN: usize = 100;
const RESERVED_PROJECT_NAMES: &[&str] = &["project"];

/// A project name that breaks the rule dvs-server applies.
#[derive(Debug)]
pub struct InvalidProjectName(pub String);

impl std::fmt::Display for InvalidProjectName {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if RESERVED_PROJECT_NAMES.contains(&self.0.as_str()) {
            write!(
                f,
                "invalid project name {:?}: this name is reserved",
                self.0
            )
        } else {
            write!(
                f,
                "invalid project name {:?}: use 1 to {MAX_PROJECT_NAME_LEN} lowercase letters, \
                 digits and dashes, starting with a letter or a digit",
                self.0
            )
        }
    }
}

impl std::error::Error for InvalidProjectName {}

/// Checks a project name against the rule of dvs-server, `^[a-z0-9][a-z0-9-]{0,99}$`.
pub fn validate_project_name(name: &str) -> Result<(), InvalidProjectName> {
    let mut chars = name.chars();
    let valid = chars
        .next()
        .is_some_and(|c| c.is_ascii_lowercase() || c.is_ascii_digit())
        && chars.all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
        && name.len() <= MAX_PROJECT_NAME_LEN
        && !RESERVED_PROJECT_NAMES.contains(&name);
    if valid {
        Ok(())
    } else {
        Err(InvalidProjectName(name.to_string()))
    }
}

/// The project name made from a folder name: lowercase, with every run of
/// other characters turned into one dash. `None` when nothing valid is left.
pub fn project_name_from_folder(folder: &str) -> Option<String> {
    let mut name = String::new();
    for c in folder.chars().flat_map(char::to_lowercase) {
        if c.is_ascii_lowercase() || c.is_ascii_digit() {
            name.push(c);
        } else if !name.is_empty() && !name.ends_with('-') {
            name.push('-');
        }
    }
    name.truncate(MAX_PROJECT_NAME_LEN);
    let name = name.trim_end_matches('-').to_string();
    validate_project_name(&name).is_ok().then_some(name)
}

/// The folder of a project in the storage directory, as dvs-server names it.
pub fn project_folder_name(name: &str) -> String {
    blake3::hash(name.as_bytes()).to_string()
}

#[derive(Debug, Serialize, Deserialize, Clone, PartialEq)]
pub struct LocalBackend {
    /// The storage directory. Without a project name it holds the blobs and
    /// the audit log itself, the layout from before projects.
    pub path: PathBuf,
    /// The project's name: its blobs and audit log are in
    /// `path/<blake3 of the name>/`, so several projects can share `path`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    name: Option<String>,
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
            name: None,
            group,
            open: false,
        })
    }

    /// The project name, `None` for a store from before projects.
    pub fn name(&self) -> Option<&str> {
        self.name.as_deref()
    }

    pub fn set_name(&mut self, name: Option<String>) {
        self.name = name;
    }

    /// Where this project's blobs and audit log are.
    pub fn project_dir(&self) -> PathBuf {
        match &self.name {
            Some(name) => self.path.join(project_folder_name(name)),
            None => self.path.clone(),
        }
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
        Ok(self.project_dir().join(prefix).join(suffix))
    }

    pub fn log_audit_entry(&self, entry: &AuditEntry) -> Result<()> {
        let _guard = AUDIT_LOG_LOCK
            .lock()
            .expect("audit log lock should not be poisoned");
        log::debug!("Appending {entry:?} to audit log");

        let project_dir = self.project_dir();
        fs::create_dir_all(&project_dir)?;
        self.ensure_group_and_mode(&project_dir, self.dir_mode())?;
        let audit_path = project_dir.join(AUDIT_LOG_FILENAME);
        let mut file = fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&audit_path)?;
        self.ensure_group_and_mode(&audit_path, self.audit_mode())?;
        let json = serde_json::to_string(entry)?;
        writeln!(file, "{}", json)?;
        Ok(())
    }

    /// Returns `true` when the project (or, without a name, the store) exists.
    pub fn init(&self, _: Compression) -> Result<bool> {
        let project_dir = self.project_dir();
        if project_dir.join(AUDIT_LOG_FILENAME).exists() {
            return Ok(true);
        }

        // Set the storage directory's group and mode only while it holds no
        // project: other projects' users rely on what it has, and another
        // user who owns it would make the change fail
        let fresh_root = !self.path.exists() || fs::read_dir(&self.path)?.next().is_none();
        log::debug!("Creating storage directory: {}", project_dir.display());
        fs::create_dir_all(&project_dir)?;
        if self.name.is_some() && fresh_root && owned_by_me(&self.path) {
            self.ensure_group_and_mode(&self.path, self.dir_mode())?;
        }
        self.ensure_group_and_mode(&project_dir, self.dir_mode())?;
        log::info!("Initialized local storage at {}", project_dir.display());
        Ok(false)
    }

    pub fn store(&self, req: StoreRequest<'_>) -> Result<StoreResult> {
        let StoreRequest {
            hashes,
            source,
            compression,
            path: rel_path,
            operation_id,
            message,
            on_bytes,
            ..
        } = req;
        let blob_path = self.hash_to_path(hashes)?;

        // Store the blob unless it is already present (dedup).
        let created = !blob_path.is_file();
        let stored_size = if !created {
            fs::metadata(&blob_path)?.len()
        } else {
            if let Some(parent) = blob_path.parent() {
                fs::create_dir_all(parent)?;
                self.ensure_group_and_mode(self.project_dir(), self.dir_mode())?;
                self.ensure_group_and_mode(parent, self.dir_mode())?;
            }

            // Unique tmp name
            let tmp_path = blob_path.with_extension(format!("tmp.{}", Uuid::new_v4()));

            // Always clean up the tmp file even if an error occurred
            let result: Result<u64> = (|| {
                let stored_size = compression.compress(source, &tmp_path, on_bytes)?;
                if self.use_shared_blob_mode() {
                    self.ensure_group_and_mode(&tmp_path, SHARED_BLOB_MODE)?;
                } else {
                    make_readonly(&tmp_path)?;
                }
                fs::rename(&tmp_path, &blob_path)?;
                Ok(stored_size)
            })();
            if result.is_err() {
                let _ = fs::remove_file(&tmp_path);
            }
            result?
        };

        let entry = AuditEntry::new_add(
            operation_id,
            AuditFile {
                path: rel_path.to_native(),
                hashes: hashes.clone(),
            },
            compression,
            message.map(|m| m.to_string()),
        );
        self.log_audit_entry(&entry)?;

        Ok(StoreResult {
            stored_size,
            compression,
            created,
        })
    }

    pub fn retrieve(&self, req: RetrieveRequest<'_>) -> Result<bool> {
        let RetrieveRequest {
            hashes,
            target,
            compression,
            on_bytes,
            ..
        } = req;
        let path = self.hash_to_path(hashes)?;
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

    pub fn exists(&self, hash: &Hashes) -> Result<bool> {
        Ok(self.hash_to_path(hash)?.is_file())
    }

    pub fn remove(&self, hash: &Hashes) -> Result<()> {
        let path = self.hash_to_path(hash)?;
        if path.is_file() {
            log::debug!("Removing {path:?} from storage");
            fs::remove_file(path)?;
        }
        Ok(())
    }

    /// An empty `files` slice returns the full audit log. See
    /// [`parse_audit_log`] for the filtering rules.
    pub fn get_audit_entries(&self, files: &[PathBuf]) -> Result<Vec<AuditEntry>> {
        let files_to_include: HashSet<_> = HashSet::from_iter(files.iter().cloned());
        let audit_path = self.project_dir().join(AUDIT_LOG_FILENAME);
        let f = fs::File::open(&audit_path)?;
        let entries = parse_audit_log(BufReader::new(f), &files_to_include)?;
        Ok(entries)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ProjectPath;

    /// A second project never changes the storage directory that the
    /// first one's users rely on.
    #[cfg(unix)]
    #[test]
    fn a_storage_directory_in_use_keeps_its_group_and_mode() {
        use std::os::unix::fs::PermissionsExt;
        let tmp = tempfile::tempdir().unwrap();
        let storage = tmp.path().join("store");
        fs::create_dir(&storage).unwrap();
        fs::write(storage.join("other"), b"").unwrap();
        fs::set_permissions(&storage, std::fs::Permissions::from_mode(0o755)).unwrap();

        let mut backend = LocalBackend::new(&storage, None).unwrap();
        assert!(backend.group.is_some(), "the mode path runs with a group");
        backend.set_name(Some("pk2".into()));
        backend.init(Compression::Zstd).unwrap();

        let mode = fs::metadata(&storage).unwrap().permissions().mode() & 0o7777;
        assert_eq!(mode, 0o755);
        let project = fs::metadata(backend.project_dir()).unwrap();
        assert_eq!(project.permissions().mode() & 0o7777, SHARED_DIRECTORY_MODE);
    }

    #[test]
    fn folder_names_make_server_names() {
        for (folder, name) in [
            ("analysis", Some("analysis")),
            ("My_Analysis 2", Some("my-analysis-2")),
            ("--pk--", Some("pk")),
            ("Étude", Some("tude")),
            ("___", None),
            ("project", None),
        ] {
            assert_eq!(
                project_name_from_folder(folder).as_deref(),
                name,
                "{folder}"
            );
        }
        assert!(project_name_from_folder(&"a".repeat(150)).is_some_and(|n| n.len() == 100));
    }
    use crate::audit::{Action, AuditEntry, AuditFile, parse_audit_log};
    use crate::config::Compression;
    use crate::hashes::Hashes;
    use std::io::Cursor;

    /// Shorthand for a project path in tests.
    fn project_path(s: &str) -> ProjectPath {
        ProjectPath::from_canonical(s).unwrap()
    }

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

        backend.init(Compression::None).unwrap();
        assert!(storage_path.is_dir());
    }

    #[test]
    fn store_creates_hash_prefixed_path() {
        let tmp = tempfile::tempdir().unwrap();
        let storage = tmp.path().join("storage");
        let backend = LocalBackend::new(&storage, None).unwrap();
        backend.init(Compression::None).unwrap();

        // Create source file
        let source = tmp.path().join("source.txt");
        fs::write(&source, b"test content").unwrap();

        let hash = test_hash("d41d8cd98f00b204e9800998ecf8427e");
        backend
            .store(StoreRequest::new_local(
                &hash,
                &source,
                project_path("source.txt"),
                Compression::None,
            ))
            .unwrap();

        let stored = storage.join("d4").join("1d8cd98f00b204e9800998ecf8427e");
        assert!(stored.is_file());
        assert_eq!(fs::read(&stored).unwrap(), b"test content");
    }

    #[test]
    fn retrieve_copies_to_target() {
        let tmp = tempfile::tempdir().unwrap();
        let storage = tmp.path().join("storage");
        let backend = LocalBackend::new(&storage, None).unwrap();
        backend.init(Compression::None).unwrap();

        // Store content via store()
        let hash = test_hash("abc123def456789012345678901234ab");
        let source = tmp.path().join("source.txt");
        fs::write(&source, b"stored content").unwrap();
        backend
            .store(StoreRequest::new_local(
                &hash,
                &source,
                project_path("source.txt"),
                Compression::None,
            ))
            .unwrap();

        // Retrieve to new location
        let target = tmp.path().join("retrieved.txt");
        let result = backend
            .retrieve(RetrieveRequest::new_local(
                &hash,
                &target,
                project_path("source.txt"),
                Compression::None,
            ))
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
        backend.init(Compression::None).unwrap();

        let target = tmp.path().join("target.txt");
        let result = backend
            .retrieve(RetrieveRequest::new_local(
                &test_hash("1234567890123456789012"),
                &target,
                project_path("source.txt"),
                Compression::None,
            ))
            .unwrap();

        assert!(!result);
        assert!(!target.exists());
    }

    #[test]
    fn exists_returns_true_for_stored() {
        let tmp = tempfile::tempdir().unwrap();
        let storage = tmp.path().join("storage");
        let backend = LocalBackend::new(&storage, None).unwrap();
        backend.init(Compression::None).unwrap();

        let hash = test_hash("abc123def456789012345678901234ab");
        assert!(!backend.exists(&hash).unwrap());
        let source = tmp.path().join("source.txt");
        fs::write(&source, b"content").unwrap();
        backend
            .store(StoreRequest::new_local(
                &hash,
                &source,
                project_path("source.txt"),
                Compression::None,
            ))
            .unwrap();
        assert!(backend.exists(&hash).unwrap());
    }

    #[test]
    fn remove_deletes_stored_file() {
        let tmp = tempfile::tempdir().unwrap();
        let storage = tmp.path().join("storage");
        let backend = LocalBackend::new(&storage, None).unwrap();
        backend.init(Compression::None).unwrap();

        let hash = test_hash("abc123def456789012345678901234ab");
        let source = tmp.path().join("source.txt");
        fs::write(&source, b"content").unwrap();
        backend
            .store(StoreRequest::new_local(
                &hash,
                &source,
                project_path("source.txt"),
                Compression::None,
            ))
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
        backend.init(Compression::None).unwrap();

        let hash = test_hash("abc123def456789012345678901234ab");
        let source = tmp.path().join("source.txt");
        fs::write(&source, b"content").unwrap();
        backend
            .store(StoreRequest::new_local(
                &hash,
                &source,
                project_path("source.txt"),
                Compression::None,
            ))
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
        backend.init(Compression::None).unwrap();

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
                message: None,
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
                message: None,
            },
        };

        backend.log_audit_entry(&entry1).unwrap();
        backend.log_audit_entry(&entry2).unwrap();

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
        backend.init(Compression::None).unwrap();

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
                                message: None,
                            },
                        };
                        backend.log_audit_entry(&entry).unwrap();
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
                message: None,
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

        backend.init(Compression::None).unwrap();
        let storage_mode = fs::metadata(&storage).unwrap().permissions().mode();
        assert_eq!(storage_mode & 0o7777, SHARED_DIRECTORY_MODE);

        let hash = test_hash("abc123def456789012345678901234ab");
        let source = tmp.path().join("source.txt");
        fs::write(&source, b"content").unwrap();
        backend
            .store(StoreRequest::new_local(
                &hash,
                &source,
                project_path("source.txt"),
                Compression::None,
            ))
            .unwrap();

        let prefix_dir = storage.join("ab");
        let prefix_mode = fs::metadata(&prefix_dir).unwrap().permissions().mode();
        assert_eq!(prefix_mode & 0o7777, SHARED_DIRECTORY_MODE);

        let stored = prefix_dir.join("c123def456789012345678901234ab");
        let file_mode = fs::metadata(&stored).unwrap().permissions().mode();
        assert_eq!(file_mode & 0o777, SHARED_BLOB_MODE);

        backend.log_audit_entry(&test_audit_entry()).unwrap();
        let audit_path = storage.join("audit.log.jsonl");
        let audit_mode = fs::metadata(&audit_path).unwrap().permissions().mode();
        assert_eq!(audit_mode & 0o777, SHARED_AUDIT_LOG_MODE);
    }
}
