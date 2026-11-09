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
use crate::backends::server::{ServerBackend, UploadManifest, UploadManifestEntry, coded};
use crate::config::{Config, dvs1_root};
use crate::error::{Error, ErrorCode, backend_error_code};
use crate::init::create_metadata_folder;
use crate::legacy::{self, DVS1_CONFIG_FILE, Dvs1Config, Sidecar, is_blake3};
use crate::paths::CONFIG_FILE_NAME;
use crate::{Compression, DvsPaths, FileMetadata, Hashes, ProjectPath, StoreRequest};

const UPLOAD_NOTE: &str = "Not found in the admin's migration; found in local storage and \
                           uploaded by the user with `dvs migrate server --upload-missing`.";

pub enum Destination {
    Server {
        name: String,
        url: Url,
        /// Where `--upload-missing` looks for content first
        storage: Option<PathBuf>,
    },
    /// `None`: dvs.yaml's `storage_dir`
    Local { path: Option<PathBuf> },
}

#[derive(Debug, Clone, Copy, Default)]
pub struct MigrateOptions {
    /// Remove dvs.yaml and the converted dvs1 sidecars
    pub clean: bool,
    /// Upload the content the server lacks that local storage has
    pub upload_missing: bool,
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
    #[serde(flatten)]
    pub state: MissingState,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum MissingState {
    /// Found in local storage and uploaded with `--upload-missing`
    Uploaded,
    /// Found in local storage, not uploaded
    InLocalStorage,
    NotLocal,
    /// Reading local storage or the upload failed
    Failed {
        code: ErrorCode,
        message: String,
    },
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
    /// The local storage searched first for the content the server lacks
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
            || self
                .missing
                .iter()
                .any(|m| matches!(m.state, MissingState::Failed { .. }))
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
            Ok(Sidecar::Dvs1(m)) => m.to_metadata().map_err(|e| e.to_string()),
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

fn read_metadata(path: &Path) -> Result<FileMetadata> {
    serde_json::from_slice(&fs::read(path)?).map_err(|e| {
        coded(
            ErrorCode::BadMetadata,
            format!("cannot read {}: {e}", path.display()),
        )
    })
}

/// A migration that passed every refusal, before anything is changed or sent.
pub struct Prepared {
    root: PathBuf,
    source: Source,
    config: Config,
    config_written: bool,
    replaced_store: Option<PathBuf>,
    /// Where `--upload-missing` looks for content, in order
    stores: Vec<PathBuf>,
    paths: DvsPaths,
    found: Vec<Dvs1Found>,
    left: Vec<LeftSidecar>,
    warnings: Vec<String>,
    options: MigrateOptions,
}

/// Reads the working tree at or above `cwd` and refuses what `destination`
/// cannot take, without touching the tree or the network.
pub fn prepare(cwd: &Path, destination: &Destination, options: MigrateOptions) -> Result<Prepared> {
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
    // The storage given with `--storage` first, then dvs.yaml's `storage_dir`,
    // then the dvs2 local storage this run replaces
    let dvs1_store = dvs1.as_ref().map(|d| root.join(&d.storage_dir));
    let stores: Vec<PathBuf> = [storage, dvs1_store, replaced_store.clone()]
        .into_iter()
        .flatten()
        .collect();
    if options.upload_missing && stores.is_empty() {
        return Err(coded(
            ErrorCode::InvalidArguments,
            format!(
                "--upload-missing needs --storage: there is no {DVS1_CONFIG_FILE} or dvs2 local storage here to find the content in"
            ),
        ));
    }
    Ok(Prepared {
        root,
        source,
        config,
        config_written,
        replaced_store,
        stores,
        paths,
        found,
        left,
        warnings,
        options,
    })
}

/// Converts the prepared tree, then, for a server, lists the content it lacks
/// (uploading what local storage has with `upload_missing`). A failure of that
/// check leaves the conversion standing.
pub fn migrate(prepared: Prepared) -> Result<MigrateReport> {
    let Prepared {
        root,
        source,
        config,
        config_written,
        replaced_store,
        stores,
        paths,
        found,
        mut left,
        warnings,
        options,
    } = prepared;
    let server = match config.backend() {
        Backend::Server(s) => Some(s),
        Backend::Local(_) => None,
    };

    // Only needed when there are sidecars to write
    let compression = match server {
        Some(server) if config_written || !found.is_empty() => Some(server_compression(server)?),
        _ => None,
    };

    let mut compression_rewritten = 0;
    if let (Some(compression), Some(_)) = (compression, &replaced_store) {
        compression_rewritten = rewrite_compression(&paths, compression, &mut left)?;
    }

    let conversion = convert_dvs1(&paths, found, compression, &mut left)?;

    if config_written {
        write_config(&root, &config)?;
    }

    let removed = if options.clean {
        clean_up(&root, conversion.to_remove)?
    } else {
        Vec::new()
    };

    let mut report = MigrateReport {
        source,
        changed: config_written
            || !conversion.converted.is_empty()
            || compression_rewritten > 0
            || !removed.is_empty(),
        project_root: root,
        config_written,
        converted: conversion.converted,
        duplicates: conversion.duplicates,
        left,
        compression_rewritten,
        removed,
        storage: None,
        missing: Vec::new(),
        check_error: None,
        warnings,
    };
    if let Some(server) = server {
        report.storage = stores.first().cloned();
        if let Err(e) = check_server(server, &paths, &stores, options.upload_missing, &mut report) {
            report.check_error = Some(Error::new(
                backend_error_code(&e).unwrap_or(ErrorCode::OperationFailed),
                format!("{e:#}"),
            ));
        }
    }
    Ok(report)
}

/// The compression the server stores the project's content with.
fn server_compression(server: &ServerBackend) -> Result<Compression> {
    let access = server.access()?.ok_or_else(|| {
        coded(
            ErrorCode::AccessDenied,
            format!("you are not a member of the project {}", server.name),
        )
    })?;
    if let Some(compression) = access.compression {
        return Ok(compression);
    }
    // Older servers name the compression only in a manifest answer
    let answer = server.manifest(&UploadManifest {
        operation_id: Uuid::new_v4(),
        message: None,
        entries: Vec::new(),
        git_branch: None,
        git_commit: None,
    })?;
    Ok(answer.compression)
}

/// Sets every dvs2 sidecar's compression to the server's. Returns the count rewritten.
fn rewrite_compression(
    paths: &DvsPaths,
    compression: Compression,
    left: &mut Vec<LeftSidecar>,
) -> Result<usize> {
    let mut rewritten = 0;
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
        if metadata.compression != compression {
            metadata.compression = compression;
            metadata.write(paths, &data)?;
            rewritten += 1;
        }
    }
    Ok(rewritten)
}

struct Conversion {
    converted: Vec<ProjectPath>,
    duplicates: Vec<ProjectPath>,
    /// The dvs1 sidecars the clean-up removes
    to_remove: Vec<PathBuf>,
}

/// Writes a dvs2 sidecar for each dvs1 one, with the server's compression if
/// given. A file that already has a dvs2 sidecar keeps it.
fn convert_dvs1(
    paths: &DvsPaths,
    found: Vec<Dvs1Found>,
    compression: Option<Compression>,
    left: &mut Vec<LeftSidecar>,
) -> Result<Conversion> {
    let mut conversion = Conversion {
        converted: Vec::new(),
        duplicates: Vec::new(),
        to_remove: Vec::new(),
    };
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
                    conversion.duplicates.push(data);
                    conversion.to_remove.push(sidecar);
                }
                Ok(dvs2) => left.push(LeftSidecar {
                    sidecar,
                    kind: LeftKind::Conflict,
                    message: format!(
                        "its hash {} differs from {}'s {}",
                        metadata.hashes.blake3,
                        target.strip_prefix(paths.repo_root())?.display(),
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
        if let Some(compression) = compression {
            metadata.compression = compression;
        }
        metadata.write(paths, data.to_native())?;
        conversion.converted.push(data);
        conversion.to_remove.push(sidecar);
    }
    Ok(conversion)
}

fn write_config(root: &Path, config: &Config) -> Result<()> {
    create_metadata_folder(root, config)?;
    config.save(root)
}

/// Deletes dvs.yaml and `sidecars`. Returns what it deleted, repo-relative.
fn clean_up(root: &Path, sidecars: Vec<PathBuf>) -> Result<Vec<PathBuf>> {
    let mut to_remove = sidecars;
    if root.join(DVS1_CONFIG_FILE).is_file() {
        to_remove.push(PathBuf::from(DVS1_CONFIG_FILE));
    }
    for rel in &to_remove {
        fs::remove_file(root.join(rel))?;
    }
    Ok(to_remove)
}

/// Lists in `report` every sidecar's hash the server lacks, uploading those
/// `stores` have when asked: one ordinary add per file, as the user, with a
/// note. Only reaching the server fails it, keeping the rows listed so far.
fn check_server(
    server: &ServerBackend,
    paths: &DvsPaths,
    stores: &[PathBuf],
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
        let failed = |message: String, e: &anyhow::Error| MissingState::Failed {
            code: backend_error_code(e).unwrap_or(ErrorCode::FileFailed),
            message,
        };
        let size = files[0].1.size;
        let state = match legacy::find_blob(stores, &hash, size) {
            None => MissingState::NotLocal,
            Some((_, Err(e))) => failed(format!("could not read local storage: {e:#}"), &e),
            Some((blob, Ok(compression))) if upload_missing => {
                match upload(server, paths, &hashes, &blob, compression, &files) {
                    Ok(()) => MissingState::Uploaded,
                    Err(e) => failed(format!("upload failed: {e:#}"), &e),
                }
            }
            Some(_) => MissingState::InLocalStorage,
        };
        report.missing.push(MissingHash {
            hash,
            paths: files
                .iter()
                .filter_map(|(data, _)| ProjectPath::from_path(data).ok())
                .collect(),
            state,
        });
    }
    Ok(())
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

    fn options(clean: bool, upload_missing: bool) -> MigrateOptions {
        MigrateOptions {
            clean,
            upload_missing,
        }
    }

    fn migrate(
        cwd: &Path,
        destination: &Destination,
        clean: bool,
        upload_missing: bool,
    ) -> Result<MigrateReport> {
        super::migrate(prepare(cwd, destination, options(clean, upload_missing))?)
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

    fn server(storage: Option<PathBuf>) -> Destination {
        Destination::Server {
            name: "p".into(),
            url: Url::parse("https://h").unwrap(),
            storage,
        }
    }

    #[test]
    fn upload_missing_needs_a_storage() {
        let (_tmp, root, store) = dvs1_repo();
        migrate(&root, &local(), true, false).unwrap();
        let mut config = Config::find(&root).unwrap().unwrap();
        config.backend = Backend::Server(ServerBackend::new(
            "p".into(),
            None,
            Url::parse("https://h").unwrap(),
        ));
        config.save(&root).unwrap();

        // No dvs.yaml is left to name the storage
        let err = prepare(&root, &server(None), options(true, true))
            .err()
            .unwrap();
        assert_eq!(backend_error_code(&err), Some(ErrorCode::InvalidArguments));
        assert!(err.to_string().contains("--storage"), "{err}");
        prepare(&root, &server(Some(store)), options(true, true)).unwrap();
    }

    #[test]
    fn stores_are_searched_given_storage_first() {
        let (tmp, root, store) = dvs1_repo();
        let given = tmp.path().join("given");
        fs::create_dir(&given).unwrap();
        let prepared = prepare(&root, &server(Some(given.clone())), options(true, true)).unwrap();
        assert_eq!(prepared.stores, [given, store]);
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
        let rows: Vec<_> = report.missing.iter().map(|m| &m.state).collect();
        assert_eq!(report.missing[0].hash, hash('a'));
        assert!(
            matches!(rows[0], MissingState::Failed { message, .. } if message.starts_with("could not read local storage")),
            "{rows:?}"
        );
        assert_eq!(rows[1], &MissingState::InLocalStorage);
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
        let n = rewrite_compression(&paths, Compression::Zstd, &mut left).unwrap();
        assert_eq!(n, 1);
        let m = read_metadata(&root.join(".dvs/a.csv.dvs")).unwrap();
        assert_eq!(m.compression, Compression::Zstd);
    }
}
