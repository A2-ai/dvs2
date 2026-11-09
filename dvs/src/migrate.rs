//! `dvs migrate`: converts a dvs1 or dvs2-local working tree to dvs2 on a
//! server or on local storage. It changes files only; committing is the user's step.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use anyhow::Result;
use fs_err as fs;
use serde::Serialize;
use url::Url;
use uuid::Uuid;
use walkdir::WalkDir;

use crate::backends::Backend;
use crate::backends::local::stored_compression;
use crate::backends::server::{ServerBackend, UploadManifest, UploadManifestEntry, coded};
use crate::config::{Config, dvs1_root};
use crate::error::{Error, ErrorCode, backend_error_code};
use crate::init::create_metadata_folder;
use crate::legacy::{self, DVS1_CONFIG_FILE, Dvs1Config, Dvs1Sidecar, Sidecar, is_blake3};
use crate::paths::CONFIG_FILE_NAME;
use crate::{Compression, DvsPaths, FileMetadata, Hashes, ProjectPath, StoreRequest};

const UPLOAD_NOTE: &str = "Not found in the admin's migration; found in local storage and \
                           uploaded by the user with `dvs migrate server --upload-missing`.";

pub enum Destination {
    Server {
        name: String,
        url: Url,
        /// Where `--upload-missing` looks for content. `None`: the storage this run replaces
        storage: Option<PathBuf>,
    },
    /// `None`: dvs.yaml's `storage_dir`
    Local { path: Option<PathBuf> },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub enum Source {
    #[serde(rename = "dvs1")]
    Dvs1,
    #[serde(rename = "dvs2-local")]
    Dvs2Local,
    #[serde(rename = "dvs2-server")]
    Dvs2Server,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum LeftKind {
    /// Not a dvs1 sidecar dvs can convert
    Unrecognized,
    /// The file also has a dvs2 sidecar with another hash
    Conflict,
    /// A dvs2 sidecar that could not be read
    Unreadable,
}

/// A sidecar left as it was, for the user to resolve.
#[derive(Debug, Clone, Serialize)]
pub struct LeftSidecar {
    pub sidecar: PathBuf,
    pub kind: LeftKind,
    pub message: String,
}

/// Content a sidecar names that the server does not have.
#[derive(Debug, Clone, Serialize)]
pub struct MissingHash {
    pub hash: String,
    pub paths: Vec<ProjectPath>,
    pub in_local_storage: bool,
    pub uploaded: bool,
    pub code: Option<ErrorCode>,
    pub message: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct MigrateReport {
    pub source: Source,
    pub project_root: PathBuf,
    /// False when the tree already was what was asked for
    pub changed: bool,
    pub config_written: bool,
    /// Files whose dvs1 sidecar became a dvs2 one
    pub converted: Vec<ProjectPath>,
    /// Files whose dvs1 sidecar was dropped: a dvs2 one with the same hash exists
    pub duplicates: Vec<ProjectPath>,
    pub left: Vec<LeftSidecar>,
    pub compression_rewritten: usize,
    /// Repo-relative files deleted by the clean-up
    pub removed: Vec<PathBuf>,
    /// The local storage searched for the content the server lacks
    pub storage: Option<PathBuf>,
    pub missing: Vec<MissingHash>,
    /// The server check failed after the conversion, which stands
    pub check_error: Option<Error>,
    pub warnings: Vec<String>,
}

impl MigrateReport {
    /// Something needs the user: a sidecar left in place, or a failed check or upload.
    pub fn is_partial(&self) -> bool {
        !self.left.is_empty()
            || self.check_error.is_some()
            || self.missing.iter().any(|m| m.message.is_some())
    }
}

fn same_path(a: &Path, b: &Path) -> bool {
    let canonical = |p: &Path| fs::canonicalize(p).unwrap_or_else(|_| p.to_path_buf());
    canonical(a) == canonical(b)
}

fn same_url(a: &Url, b: &Url) -> bool {
    a.as_str().trim_end_matches('/') == b.as_str().trim_end_matches('/')
}

/// The config `destination` asks for, and whether it differs from dvs.toml.
fn target_config(
    root: &Path,
    existing: Option<&Config>,
    dvs1: Option<&Dvs1Config>,
    destination: &Destination,
    warnings: &mut Vec<String>,
) -> Result<(Config, bool)> {
    let refuse = |message: String| Err(coded(ErrorCode::InvalidArguments, message));
    let dvs1_storage = dvs1.map(|d| root.join(&d.storage_dir));
    match (existing, destination) {
        (Some(config), Destination::Server { name, url, .. }) => match config.backend() {
            Backend::Server(s) if s.name == *name && same_url(&s.url, url) => {
                Ok((config.clone(), false))
            }
            Backend::Server(s) => refuse(format!(
                "{CONFIG_FILE_NAME} already uses the project {} on {}: dvs migrate does not move a project between servers",
                s.name, s.url
            )),
            Backend::Local(_) => {
                let mut config = config.clone();
                config.backend =
                    Backend::Server(ServerBackend::new(name.clone(), None, url.clone()));
                config.set_compression(None);
                Ok((config, true))
            }
        },
        (Some(config), Destination::Local { path }) => match config.backend() {
            Backend::Server(s) => refuse(format!(
                "{CONFIG_FILE_NAME} already uses the project {} on {}: there is nothing to migrate to local storage",
                s.name, s.url
            )),
            Backend::Local(l) => match path.clone().or(dvs1_storage) {
                Some(p) if !same_path(&p, &l.path) => refuse(format!(
                    "{CONFIG_FILE_NAME} already uses dvs2 local storage at {}: dvs migrate does not move it to {}",
                    l.path.display(),
                    p.display()
                )),
                _ => Ok((config.clone(), false)),
            },
        },
        (None, Destination::Server { name, url, .. }) => {
            let mut config = Config::new_server(name.clone(), url.clone(), None);
            config.set_compression(None);
            Ok((config, true))
        }
        (None, Destination::Local { path }) => {
            let dvs1 = dvs1.expect("a tree without dvs.toml has dvs.yaml");
            let storage = path
                .clone()
                .or(dvs1_storage)
                .expect("dvs.yaml has storage_dir");
            if !storage.is_dir() {
                return refuse(format!(
                    "the storage {} is not a directory",
                    storage.display()
                ));
            }
            let storage = fs::canonicalize(storage)?;
            // dvs1 v0.0.1 writes `group: ''` for no group
            let group = dvs1.group.clone().filter(|g| !g.is_empty());
            let mut config = Config::new_local(&storage, group.clone())?;
            if group.is_none() {
                // `new_local` would pick the user's primary group and then
                // chmod the shared store on the next add; dvs1 set none
                config.backend = Backend::Local(serde_json::from_value(
                    serde_json::json!({ "path": storage }),
                )?);
            }
            if let Some(permissions) = dvs1.permissions {
                warnings.push(format!(
                    "{DVS1_CONFIG_FILE} sets permissions {permissions}: dvs2 local storage sets its own file modes, so it is not carried over"
                ));
            }
            Ok((config, true))
        }
    }
}

struct Dvs1Found {
    sidecar: PathBuf,
    data: ProjectPath,
    metadata: FileMetadata,
}

/// Every `*.dvs` file outside `.git` and the metadata folder, sorted into the
/// dvs1 sidecars to convert and the ones to leave.
fn scan_dvs1(root: &Path, metadata_folder: &Path) -> Result<(Vec<Dvs1Found>, Vec<LeftSidecar>)> {
    let mut found = Vec::new();
    let mut left = Vec::new();
    let entries = WalkDir::new(root)
        .sort_by_file_name()
        .into_iter()
        .filter_entry(|e| e.path() != metadata_folder && e.file_name() != ".git")
        .filter_map(|e| e.ok());
    for entry in entries {
        if !entry.file_type().is_file() {
            continue;
        }
        let sidecar = entry.path().strip_prefix(root)?.to_path_buf();
        let Some(data) = legacy::sidecar_data_path(&sidecar) else {
            continue;
        };
        let unrecognized = |message: String| LeftSidecar {
            sidecar: sidecar.clone(),
            kind: LeftKind::Unrecognized,
            message,
        };
        let parsed = match legacy::classify_sidecar(&fs::read(entry.path())?) {
            Ok(Sidecar::Dvs1(m)) => to_metadata(&m),
            Ok(Sidecar::Dvs2(_)) => Err(format!(
                "a dvs2 sidecar outside {}",
                metadata_folder.display()
            )),
            Err(e) => Err(e.message),
        };
        match parsed {
            Ok(metadata) => found.push(Dvs1Found {
                sidecar,
                data,
                metadata,
            }),
            Err(message) => left.push(unrecognized(message)),
        }
    }
    Ok((found, left))
}

fn to_metadata(m: &Dvs1Sidecar) -> Result<FileMetadata, String> {
    if !is_blake3(&m.blake3) {
        return Err(format!("invalid blake3 hash {:?}", m.blake3));
    }
    m.to_metadata().map_err(|e| e.to_string())
}

fn read_metadata(path: &Path) -> Result<FileMetadata> {
    serde_json::from_slice(&fs::read(path)?).map_err(|e| {
        coded(
            ErrorCode::BadMetadata,
            format!("cannot read {}: {e}", path.display()),
        )
    })
}

/// Where a file's content may still be in local storage, and how it is stored
/// there. `None`: unknown, told from the blob's size.
#[derive(Clone)]
struct Origin {
    store: PathBuf,
    compression: Option<Compression>,
}

/// Sets every dvs2 sidecar's compression to the server's. Returns the count
/// rewritten and each file's compression before.
fn rewrite_compression(
    paths: &DvsPaths,
    compression: Compression,
    left: &mut Vec<LeftSidecar>,
) -> Result<(usize, BTreeMap<PathBuf, Compression>)> {
    let mut rewritten = 0;
    let mut before = BTreeMap::new();
    for data in paths.tracked_paths() {
        let path = paths.metadata_path(&data);
        let mut metadata = match read_metadata(&path) {
            Ok(m) => m,
            Err(e) => {
                left.push(LeftSidecar {
                    sidecar: path.strip_prefix(paths.repo_root())?.to_path_buf(),
                    kind: LeftKind::Unreadable,
                    message: e.to_string(),
                });
                continue;
            }
        };
        before.insert(data.clone(), metadata.compression);
        if metadata.compression != compression {
            metadata.compression = compression;
            metadata.write(paths, &data)?;
            rewritten += 1;
        }
    }
    Ok((rewritten, before))
}

/// A migration that passed every refusal, before anything is changed or sent.
pub struct Prepared {
    root: PathBuf,
    dvs1: Option<Dvs1Config>,
    source: Source,
    config: Config,
    config_written: bool,
    replaced_store: Option<PathBuf>,
    paths: DvsPaths,
    found: Vec<Dvs1Found>,
    left: Vec<LeftSidecar>,
    warnings: Vec<String>,
    /// Given with `--storage`
    storage: Option<PathBuf>,
    upload_missing: bool,
}

/// Reads the working tree at or above `cwd` and refuses what `destination`
/// cannot take, without touching the tree or the network.
pub fn prepare(cwd: &Path, destination: &Destination, upload_missing: bool) -> Result<Prepared> {
    let dvs1_root = dvs1_root(cwd);
    let root = cwd
        .ancestors()
        .find(|d| d.join(CONFIG_FILE_NAME).is_file() || Some(*d) == dvs1_root)
        .ok_or_else(|| {
            coded(
                ErrorCode::NotInProject,
                format!("No {CONFIG_FILE_NAME} or {DVS1_CONFIG_FILE} here or in a parent folder"),
            )
        })?;
    let root = fs::canonicalize(root)?;
    let dvs1_config_path = root.join(DVS1_CONFIG_FILE);
    let dvs1 = if dvs1_config_path.is_file() {
        Some(Dvs1Config::from_yaml(&fs::read_to_string(
            &dvs1_config_path,
        )?)?)
    } else {
        None
    };
    let existing = if root.join(CONFIG_FILE_NAME).is_file() {
        Config::find(&root).transpose()?
    } else {
        None
    };
    let source = match existing.as_ref().map(Config::backend) {
        None => Source::Dvs1,
        Some(Backend::Local(_)) => Source::Dvs2Local,
        Some(Backend::Server(_)) => Source::Dvs2Server,
    };
    let mut warnings = Vec::new();
    let (config, config_written) = target_config(
        &root,
        existing.as_ref(),
        dvs1.as_ref(),
        destination,
        &mut warnings,
    )?;
    let replaced_store = match existing.as_ref().map(Config::backend) {
        Some(Backend::Local(l)) if config_written => Some(l.path.clone()),
        _ => None,
    };

    let paths = DvsPaths::new(root.clone(), root.clone(), config.metadata_folder_name())?;
    let (found, left) = scan_dvs1(&root, &paths.metadata_folder())?;
    if source == Source::Dvs2Local
        && matches!(destination, Destination::Local { .. })
        && dvs1.is_none()
        && found.is_empty()
        && left.is_empty()
    {
        return Err(coded(
            ErrorCode::InvalidArguments,
            format!(
                "{CONFIG_FILE_NAME} already uses dvs2 local storage and there is no {DVS1_CONFIG_FILE} or dvs1 sidecar: there is nothing to migrate"
            ),
        ));
    }
    let storage = match destination {
        Destination::Server { storage, .. } => storage.clone(),
        Destination::Local { .. } => None,
    };
    if let Some(storage) = &storage
        && !storage.is_dir()
    {
        return Err(coded(
            ErrorCode::InvalidArguments,
            format!("the storage {} is not a directory", storage.display()),
        ));
    }
    if upload_missing && storage.is_none() && dvs1.is_none() && replaced_store.is_none() {
        return Err(coded(
            ErrorCode::InvalidArguments,
            format!(
                "--upload-missing needs --storage: there is no {DVS1_CONFIG_FILE} or dvs2 local storage here to find the content in"
            ),
        ));
    }
    Ok(Prepared {
        root,
        dvs1,
        source,
        config,
        config_written,
        replaced_store,
        paths,
        found,
        left,
        warnings,
        storage,
        upload_missing,
    })
}

/// Converts the prepared tree, then, for a server, lists the content it lacks
/// (uploading what local storage has with `upload_missing`). A failure of that
/// check leaves the conversion standing.
pub fn migrate(prepared: Prepared, clean: bool) -> Result<MigrateReport> {
    let Prepared {
        root,
        dvs1,
        source,
        config,
        config_written,
        replaced_store,
        paths,
        found,
        mut left,
        warnings,
        storage,
        upload_missing,
    } = prepared;
    let dvs1_config_path = root.join(DVS1_CONFIG_FILE);

    let server = match config.backend() {
        Backend::Server(s) => Some(s),
        Backend::Local(_) => None,
    };
    let mut server_compression = None;
    if let Some(server) = server
        && (config_written || !found.is_empty())
    {
        let access = server.access()?.ok_or_else(|| {
            coded(
                ErrorCode::AccessDenied,
                format!("you are not a member of the project {}", server.name),
            )
        })?;
        server_compression = Some(match access.compression {
            Some(compression) => compression,
            // Older servers name the compression only in a manifest answer
            None => {
                server
                    .manifest(&UploadManifest {
                        operation_id: Uuid::new_v4(),
                        message: None,
                        entries: Vec::new(),
                        git_branch: None,
                        git_commit: None,
                    })?
                    .compression
            }
        });
    }

    // Where each file's content may still be, for --upload-missing
    let mut origins = BTreeMap::new();
    let mut compression_rewritten = 0;
    if let (Some(compression), Some(store)) = (server_compression, &replaced_store) {
        let before;
        (compression_rewritten, before) = rewrite_compression(&paths, compression, &mut left)?;
        origins = store_origins(before.into_keys(), store);
    }

    let dvs1_store = dvs1.as_ref().map(|d| root.join(&d.storage_dir));
    let mut converted = Vec::new();
    let mut duplicates = Vec::new();
    let mut to_remove = Vec::new();
    for Dvs1Found {
        sidecar,
        data,
        mut metadata,
    } in found
    {
        let target = paths.metadata_path(&data.to_native());
        if target.is_file() {
            match read_metadata(&target) {
                Ok(dvs2) if dvs2.hashes.blake3 == metadata.hashes.blake3 => {
                    if let Some(store) = &dvs1_store {
                        origins.entry(data.to_native()).or_insert(Origin {
                            store: store.clone(),
                            compression: Some(Compression::None),
                        });
                    }
                    duplicates.push(data);
                    to_remove.push(sidecar);
                }
                Ok(dvs2) => left.push(LeftSidecar {
                    sidecar,
                    kind: LeftKind::Conflict,
                    message: format!(
                        "its hash {} differs from {}'s {}",
                        metadata.hashes.blake3,
                        target.strip_prefix(&root)?.display(),
                        dvs2.hashes.blake3
                    ),
                }),
                Err(e) => left.push(LeftSidecar {
                    sidecar,
                    kind: LeftKind::Conflict,
                    message: e.to_string(),
                }),
            }
            continue;
        }
        if let Some(compression) = server_compression {
            metadata.compression = compression;
        }
        metadata.write(&paths, data.to_native())?;
        // A leftover dvs1 sidecar's content may have been re-added, possibly
        // compressed, to the dvs2 store this run replaces
        let origin = match (&dvs1_store, &replaced_store) {
            (Some(store), _) => Some((store, Some(Compression::None))),
            (None, Some(store)) => Some((store, None)),
            (None, None) => None,
        };
        if let Some((store, compression)) = origin {
            origins.insert(
                data.to_native(),
                Origin {
                    store: store.clone(),
                    compression,
                },
            );
        }
        converted.push(data);
        to_remove.push(sidecar);
    }

    if let Some(store) = &storage {
        origins.extend(storage_origins(&paths, store));
    }

    if config_written {
        create_metadata_folder(&root, &config)?;
        config.save(&root)?;
    }

    let mut removed = Vec::new();
    if clean {
        if dvs1_config_path.is_file() {
            to_remove.push(PathBuf::from(DVS1_CONFIG_FILE));
        }
        for rel in to_remove {
            fs::remove_file(root.join(&rel))?;
            removed.push(rel);
        }
    }

    let mut report = MigrateReport {
        source,
        changed: config_written
            || !converted.is_empty()
            || compression_rewritten > 0
            || !removed.is_empty(),
        project_root: root,
        config_written,
        converted,
        duplicates,
        left,
        compression_rewritten,
        removed,
        storage: server.and(storage.or(dvs1_store).or(replaced_store)),
        missing: Vec::new(),
        check_error: None,
        warnings,
    };
    if let Some(server) = server
        && let Err(e) = check_server(server, &paths, &origins, upload_missing, &mut report)
    {
        report.check_error = Some(Error::new(
            backend_error_code(&e).unwrap_or(ErrorCode::OperationFailed),
            format!("{e:#}"),
        ));
    }
    Ok(report)
}

/// Lists in `report` every sidecar's hash the server lacks, uploading those
/// local storage has when asked: one ordinary add per file, as the user, with
/// a note. Only reaching the server fails it, keeping the rows listed so far.
fn check_server(
    server: &ServerBackend,
    paths: &DvsPaths,
    origins: &BTreeMap<PathBuf, Origin>,
    upload_missing: bool,
    report: &mut MigrateReport,
) -> Result<()> {
    let mut by_hash: BTreeMap<String, Vec<(PathBuf, FileMetadata)>> = BTreeMap::new();
    for data in paths.tracked_paths() {
        match read_metadata(&paths.metadata_path(&data)) {
            Ok(m) if is_blake3(&m.hashes.blake3) => {
                by_hash
                    .entry(m.hashes.blake3.clone())
                    .or_default()
                    .push((data, m));
            }
            Ok(m) => report.warnings.push(format!(
                "{} has an invalid blake3 hash {:?}: not checked",
                data.display(),
                m.hashes.blake3
            )),
            Err(e) => report.warnings.push(e.to_string()),
        }
    }

    for (hash, files) in by_hash {
        let hashes = Hashes {
            blake3: hash.clone(),
            md5: None,
        };
        if server.exists(&hashes)? {
            continue;
        }
        let mut local = None;
        let mut read_error = None;
        for (data, metadata) in &files {
            let Some(origin) = origins.get(data) else {
                continue;
            };
            match local_blob(origin, &hash, metadata.size) {
                Ok(Some(found)) => {
                    local = Some(found);
                    break;
                }
                Ok(None) => {}
                Err(e) => {
                    read_error.get_or_insert(e);
                }
            }
        }
        let mut row = MissingHash {
            hash: hash.clone(),
            paths: files
                .iter()
                .filter_map(|(data, _)| ProjectPath::from_path(data).ok())
                .collect(),
            in_local_storage: local.is_some(),
            uploaded: false,
            code: None,
            message: None,
        };
        if let (None, Some(e)) = (&local, read_error) {
            row.code = Some(backend_error_code(&e).unwrap_or(ErrorCode::FileFailed));
            row.message = Some(format!("could not read local storage: {e:#}"));
        }
        if let (true, Some((blob, compression))) = (upload_missing, local) {
            match upload(server, paths, &hashes, &blob, compression, &files) {
                Ok(()) => row.uploaded = true,
                Err(e) => {
                    row.code = Some(backend_error_code(&e).unwrap_or(ErrorCode::FileFailed));
                    row.message = Some(format!("{e:#}"));
                }
            }
        }
        report.missing.push(row);
    }
    Ok(())
}

/// An explicit storage may hold any tracked file, in whatever form it was stored.
fn storage_origins(paths: &DvsPaths, store: &Path) -> BTreeMap<PathBuf, Origin> {
    store_origins(paths.tracked_paths(), store)
}

/// A dvs2 store's blobs are told apart from their bytes: a sidecar's compression
/// may not be how the blob was stored.
fn store_origins(
    files: impl IntoIterator<Item = PathBuf>,
    store: &Path,
) -> BTreeMap<PathBuf, Origin> {
    files
        .into_iter()
        .map(|data| {
            let origin = Origin {
                store: store.to_path_buf(),
                compression: None,
            };
            (data, origin)
        })
        .collect()
}

/// The blob in `origin`'s store and how it is stored.
fn local_blob(origin: &Origin, hash: &str, size: u64) -> Result<Option<(PathBuf, Compression)>> {
    let blob = legacy::blob_path(&origin.store, hash);
    if !blob.is_file() {
        return Ok(None);
    }
    let compression = match origin.compression {
        Some(compression) => compression,
        None => stored_compression(&blob, size, hash)?,
    };
    Ok(Some((blob, compression)))
}

fn upload(
    server: &ServerBackend,
    paths: &DvsPaths,
    hashes: &Hashes,
    blob: &Path,
    compression: Compression,
    files: &[(PathBuf, FileMetadata)],
) -> Result<()> {
    // The server takes the bytes uncompressed
    let decompressed;
    let source = match compression {
        Compression::None => blob,
        Compression::Zstd => {
            decompressed = tempfile::NamedTempFile::new()?.into_temp_path();
            compression.decompress(blob, &decompressed, None)?;
            &decompressed
        }
    };
    let checkout = crate::git::current_checkout(paths.repo_root());
    let mut stored = false;
    for (data, metadata) in files {
        let path = ProjectPath::from_path(data).map_err(|e| anyhow::anyhow!("{e}"))?;
        let message = format!(
            "{UPLOAD_NOTE} Its sidecar records it as added by {} at {}.",
            metadata.created_by, metadata.add_time
        );
        let operation_id = Uuid::new_v4();
        let answer = server.manifest(&UploadManifest {
            operation_id,
            message: Some(message.clone()),
            entries: vec![UploadManifestEntry {
                path: path.clone(),
                size: metadata.size,
                hash: hashes.blake3.clone(),
            }],
            git_branch: checkout.branch.clone(),
            git_commit: checkout.commit.clone(),
        })?;
        if !stored && answer.missing.contains(&hashes.blake3) {
            server.store(StoreRequest {
                hashes,
                source,
                compression: answer.compression,
                path,
                operation_id,
                message: Some(&message),
                on_bytes: None,
            })?;
            stored = true;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testutil::{create_file, create_temp_git_repo, init_dvs_repo};

    fn hash(c: char) -> String {
        c.to_string().repeat(64)
    }

    fn v001(hash: &str) -> String {
        format!(
            r#"{{"blake3_checksum": "{hash}", "file_size_bytes": 3, "time_stamp": "2023-04-05T14:11:12.345Z", "message": "", "saved_by": "jdoe"}}"#
        )
    }

    fn v002(hash: &str) -> String {
        format!(
            r#"{{"blake3_checksum": "{hash}", "size": 3, "add_time": "2024-01-02T03:04:05Z", "message": "m", "saved_by": "ann"}}"#
        )
    }

    /// A dvs1 tree: dvs.yaml naming a sibling store, and two sidecars.
    fn dvs1_repo() -> (tempfile::TempDir, PathBuf, PathBuf) {
        let (tmp, root) = create_temp_git_repo();
        let store = root.parent().unwrap().join("store");
        fs::create_dir(&store).unwrap();
        let yaml = format!(
            "storage_dir: {}\npermissions: 777\ngroup: ''\n",
            store.display()
        );
        create_file(&root, DVS1_CONFIG_FILE, yaml.as_bytes());
        create_file(&root, "a.csv.dvs", v001(&hash('a')).as_bytes());
        create_file(&root, "data/b.csv.dvs", v002(&hash('b')).as_bytes());
        (tmp, root, store)
    }

    fn local() -> Destination {
        Destination::Local { path: None }
    }

    fn migrate(
        cwd: &Path,
        destination: &Destination,
        clean: bool,
        upload_missing: bool,
    ) -> Result<MigrateReport> {
        super::migrate(prepare(cwd, destination, upload_missing)?, clean)
    }

    #[test]
    fn local_converts_dvs1_and_cleans_up() {
        let (_tmp, root, store) = dvs1_repo();
        let report = migrate(&root.join("data"), &local(), true, false).unwrap();

        assert_eq!(report.source, Source::Dvs1);
        assert_eq!(report.converted.len(), 2);
        assert_eq!(report.removed.len(), 3);
        assert!(!root.join(DVS1_CONFIG_FILE).exists());
        assert!(!root.join("a.csv.dvs").exists());
        let config = Config::find(&root).unwrap().unwrap();
        let Backend::Local(l) = config.backend() else {
            panic!("not local");
        };
        assert_eq!(l.path, fs::canonicalize(&store).unwrap());
        // An empty dvs1 group is no group, not the user's primary one
        assert!(
            !fs::read_to_string(root.join(CONFIG_FILE_NAME))
                .unwrap()
                .contains("group")
        );
        assert!(report.warnings[0].contains("permissions 777"));

        let m = read_metadata(&root.join(".dvs/data/b.csv.dvs")).unwrap();
        assert_eq!(m.hashes.blake3, hash('b'));
        assert_eq!(m.created_by, "ann");
        assert_eq!(m.compression, Compression::None);
    }

    #[test]
    fn no_clean_keeps_dvs1_files() {
        let (_tmp, root, _store) = dvs1_repo();
        let report = migrate(&root, &local(), false, false).unwrap();
        assert!(report.removed.is_empty());
        assert!(root.join(DVS1_CONFIG_FILE).is_file());
        assert!(root.join("a.csv.dvs").is_file());
        assert!(root.join(".dvs/a.csv.dvs").is_file());
    }

    #[test]
    fn a_second_run_changes_nothing() {
        let (_tmp, root, _store) = dvs1_repo();
        migrate(&root, &local(), false, false).unwrap();
        let report = migrate(&root, &local(), false, false).unwrap();
        assert_eq!(report.source, Source::Dvs2Local);
        assert!(!report.changed);
    }

    #[test]
    fn dvs1_and_dvs2_sidecars_of_one_file() {
        let (_tmp, root, _store) = dvs1_repo();
        migrate(&root, &local(), false, false).unwrap();
        // b keeps its hash; a now differs from its dvs2 sidecar
        create_file(&root, "a.csv.dvs", v001(&hash('c')).as_bytes());
        let report = migrate(&root, &local(), true, false).unwrap();

        assert_eq!(report.duplicates[0].as_str(), "data/b.csv");
        assert!(!root.join("data/b.csv.dvs").exists());
        assert_eq!(report.left[0].kind, LeftKind::Conflict);
        assert!(root.join("a.csv.dvs").is_file());
        assert!(report.is_partial());
    }

    #[test]
    fn unrecognized_sidecars_stay() {
        let (_tmp, root, _store) = dvs1_repo();
        create_file(&root, "bad.dvs", b"{}");
        create_file(&root, "short.dvs", v002("abc").as_bytes());
        let report = migrate(&root, &local(), true, false).unwrap();
        let left: Vec<_> = report.left.iter().map(|l| l.sidecar.clone()).collect();
        assert_eq!(left, [PathBuf::from("bad.dvs"), PathBuf::from("short.dvs")]);
        assert!(report.left[1].message.contains("invalid blake3 hash"));
        assert!(root.join("bad.dvs").is_file() && root.join("short.dvs").is_file());
    }

    #[test]
    fn refuses_dvs2_local_to_local() {
        let (tmp, root) = create_temp_git_repo();
        // A dvs.yaml above the git repo is not this repo's
        create_file(tmp.path(), DVS1_CONFIG_FILE, b"storage_dir: /x\n");
        let err = migrate(&root, &local(), true, false).unwrap_err();
        assert_eq!(backend_error_code(&err), Some(ErrorCode::NotInProject));

        init_dvs_repo(&root);
        let other = tmp.path().join("other");
        fs::create_dir(&other).unwrap();
        let err = migrate(
            &root,
            &Destination::Local { path: Some(other) },
            true,
            false,
        )
        .unwrap_err();
        assert_eq!(backend_error_code(&err), Some(ErrorCode::InvalidArguments));

        // Same storage, but no dvs1 leftovers to convert
        let err = migrate(&root, &local(), true, false).unwrap_err();
        assert_eq!(backend_error_code(&err), Some(ErrorCode::InvalidArguments));
        assert!(err.to_string().contains("nothing to migrate"), "{err}");

        // A dvs1 sidecar left behind is still converted
        create_file(&root, "a.csv.dvs", v001(&hash('a')).as_bytes());
        let report = migrate(&root, &local(), true, false).unwrap();
        assert_eq!(report.converted.len(), 1);
    }

    #[test]
    fn upload_missing_finds_content_in_the_given_storage() {
        let (_tmp, root, store) = dvs1_repo();
        migrate(&root, &local(), true, false).unwrap();
        let mut config = Config::find(&root).unwrap().unwrap();
        config.backend = Backend::Server(ServerBackend::new(
            "p".into(),
            None,
            Url::parse("https://h").unwrap(),
        ));
        config.save(&root).unwrap();
        let server = |storage: Option<PathBuf>| Destination::Server {
            name: "p".into(),
            url: Url::parse("https://h").unwrap(),
            storage,
        };

        // No dvs.yaml is left to name the storage
        let err = prepare(&root, &server(None), true).err().unwrap();
        assert_eq!(backend_error_code(&err), Some(ErrorCode::InvalidArguments));
        assert!(err.to_string().contains("--storage"), "{err}");
        prepare(&root, &server(Some(store.clone())), true).unwrap();

        let paths = DvsPaths::new(root.clone(), root.clone(), ".dvs").unwrap();
        let origins = storage_origins(&paths, &store);
        // A raw file that is itself a zstd frame, and a compressed blob of the
        // file's exact size, so neither magic bytes nor size alone tell them apart
        let raw = zstd::encode_all(&b"inner"[..], 0).unwrap();
        let noise: Vec<u8> = (0..256u32)
            .map(|i| (i.wrapping_mul(2654435761) >> 13) as u8)
            .collect();
        let (plain, compressed) = (0..64)
            .map(|zeros| [noise.clone(), vec![0; zeros]].concat())
            .map(|plain| (zstd::encode_all(&plain[..], 0).unwrap(), plain))
            .find(|(compressed, plain)| compressed.len() == plain.len())
            .map(|(compressed, plain)| (plain, compressed))
            .unwrap();
        for (content, blob_content, expected) in [
            (raw.clone(), raw, Compression::None),
            (plain, compressed, Compression::Zstd),
        ] {
            let blake3 = blake3::hash(&content).to_hex().to_string();
            let blob = create_file(
                &store,
                &format!("{}/{}", &blake3[..2], &blake3[2..]),
                &blob_content,
            );
            let found = local_blob(&origins[Path::new("a.csv")], &blake3, content.len() as u64);
            assert_eq!(found.unwrap(), Some((blob, expected)));
        }
        assert_eq!(
            local_blob(&origins[Path::new("data/b.csv")], &hash('b'), 3).unwrap(),
            None
        );

        // A sidecar that says zstd for a blob stored raw
        let raw = zstd::encode_all(&b"other"[..], 0).unwrap();
        let blake3 = blake3::hash(&raw).to_hex().to_string();
        let blob = create_file(&store, &format!("{}/{}", &blake3[..2], &blake3[2..]), &raw);
        let mut m = read_metadata(&paths.metadata_path(Path::new("a.csv"))).unwrap();
        m.compression = Compression::Zstd;
        m.write(&paths, "a.csv").unwrap();
        let (_, before) = rewrite_compression(&paths, Compression::None, &mut Vec::new()).unwrap();
        assert_eq!(before[Path::new("a.csv")], Compression::Zstd);
        let origins = store_origins(before.into_keys(), &store);
        let found = local_blob(&origins[Path::new("a.csv")], &blake3, raw.len() as u64);
        assert_eq!(found.unwrap(), Some((blob, Compression::None)));
    }

    /// A server that has no content: every request gets a 404.
    fn empty_server() -> Url {
        use std::io::{BufRead, BufReader, Write};
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let url = Url::parse(&format!("http://{}/", listener.local_addr().unwrap())).unwrap();
        std::thread::spawn(move || {
            for stream in listener.incoming().flatten() {
                let mut reader = BufReader::new(&stream);
                let mut line = String::new();
                while reader.read_line(&mut line).is_ok_and(|n| n > 2) {
                    line.clear();
                }
                let _ = (&stream).write_all(
                    b"HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
                );
            }
        });
        url
    }

    #[cfg(unix)]
    #[test]
    fn an_unreadable_blob_does_not_stop_the_check() {
        use std::os::unix::fs::PermissionsExt;
        let (_tmp, root, store) = dvs1_repo();
        migrate(&root, &local(), true, false).unwrap();
        let url = empty_server();
        crate::auth::store_token(&url, "t").unwrap();
        let mut config = Config::find(&root).unwrap().unwrap();
        config.backend = Backend::Server(ServerBackend::new("p".into(), None, url.clone()));
        config.save(&root).unwrap();
        let a = create_file(&store, &format!("aa/{}", &hash('a')[2..]), b"aaa");
        create_file(&store, &format!("bb/{}", &hash('b')[2..]), b"bbb");
        fs::set_permissions(&a, std::fs::Permissions::from_mode(0o000)).unwrap();
        if fs::File::open(&a).is_ok() {
            // Root reads it anyway
            return;
        }

        let destination = Destination::Server {
            name: "p".into(),
            url,
            storage: Some(store),
        };
        let report = migrate(&root, &destination, true, false).unwrap();
        assert!(report.check_error.is_none(), "{:?}", report.check_error);
        let rows: Vec<_> = report
            .missing
            .iter()
            .map(|m| (m.hash.clone(), m.in_local_storage, m.message.is_some()))
            .collect();
        assert_eq!(rows, [(hash('a'), false, true), (hash('b'), true, false)]);
        assert!(report.is_partial());
    }

    #[test]
    fn rewrites_compression_for_the_server() {
        let (_tmp, root) = create_temp_git_repo();
        init_dvs_repo(&root);
        let paths = DvsPaths::new(root.clone(), root.clone(), ".dvs").unwrap();
        let m = legacy::classify_sidecar(v001(&hash('a')).as_bytes()).unwrap();
        let Sidecar::Dvs1(m) = m else { panic!() };
        m.to_metadata().unwrap().write(&paths, "a.csv").unwrap();

        let mut left = Vec::new();
        let (n, before) = rewrite_compression(&paths, Compression::Zstd, &mut left).unwrap();
        assert_eq!(n, 1);
        assert_eq!(before[Path::new("a.csv")], Compression::None);
        let m = read_metadata(&root.join(".dvs/a.csv.dvs")).unwrap();
        assert_eq!(m.compression, Compression::Zstd);
    }
}
