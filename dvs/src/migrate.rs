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
use crate::config::{Config, dvs1_config_path, dvs1_root};
use crate::error::{Error, ErrorCode, backend_error_code};
use crate::init::create_metadata_folder;
use crate::legacy::{
    self, DVS1_CONFIG_FILE, DVS1_CONFIG_FILES, Dvs1Config, Dvs1Sidecar, Sidecar, SidecarShape,
    is_blake3,
};
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
            // A devious checkout can have its sidecars without its config
            let Some(storage) = path.clone().or(dvs1_storage) else {
                return refuse(format!(
                    "there is no {DVS1_CONFIG_FILE} or .dvs.yaml to name the storage: give its path to `dvs migrate local`"
                ));
            };
            if !storage.is_dir() {
                return refuse(format!(
                    "the storage {} is not a directory",
                    storage.display()
                ));
            }
            let storage = fs::canonicalize(storage)?;
            // dvs1 v0.0.1 writes `group: ''` for no group
            let group = dvs1.and_then(|d| d.group.clone()).filter(|g| !g.is_empty());
            let mut config = Config::new_local(&storage, group.clone())?;
            if group.is_none() {
                // `new_local` would pick the user's primary group and then
                // chmod the shared store on the next add; dvs1 set none
                config.backend = Backend::Local(serde_json::from_value(
                    serde_json::json!({ "path": storage }),
                )?);
            }
            if let Some(permissions) = dvs1.and_then(|d| d.permissions) {
                warnings.push(format!(
                    "{DVS1_CONFIG_FILE} sets permissions {permissions}: dvs2 local storage sets its own file modes, so it is not carried over"
                ));
            }
            Ok((config, true))
        }
    }
}

struct Dvs1Found {
    /// The sidecar converted, then any other of the file's with the same hash:
    /// the clean-up removes them together
    sidecars: Vec<PathBuf>,
    data: ProjectPath,
    metadata: FileMetadata,
}

struct Dvs1Scanned {
    sidecar: PathBuf,
    data: ProjectPath,
    parsed: Dvs1Sidecar,
}

/// Every `*.dvs` and devious `*.dvsmeta` file outside `.git` and the metadata
/// folder, sorted into the dvs1 sidecars to convert and the ones to leave.
fn scan_dvs1(root: &Path, metadata_folder: &Path) -> Result<(Vec<Dvs1Scanned>, Vec<LeftSidecar>)> {
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
        match legacy::classify_sidecar(&fs::read(entry.path())?) {
            Ok(Sidecar::Dvs1(parsed)) => found.push(Dvs1Scanned {
                sidecar,
                data,
                parsed,
            }),
            Ok(Sidecar::Dvs2(_)) => left.push(unrecognized(format!(
                "a dvs2 sidecar outside {}",
                metadata_folder.display()
            ))),
            Err(e) => left.push(unrecognized(e.message)),
        }
    }
    Ok((found, left))
}

const SIZE_UNKNOWN: &str =
    "size unknown: no blob in local storage and the working file does not match its hash";

/// The dvs2 sidecar of each file the scanned ones name. A file with both a
/// `.dvs` and a `.dvsmeta` sidecar is converted once if they name one hash,
/// else left as a conflict (D20).
fn resolve_dvs1(
    root: &Path,
    scanned: Vec<Dvs1Scanned>,
    stores: &[PathBuf],
    left: &mut Vec<LeftSidecar>,
    warnings: &mut Vec<String>,
) -> Vec<Dvs1Found> {
    let now = jiff::Timestamp::now();
    let user = whoami::username().unwrap_or_else(|_| "unknown".to_string());
    let mut by_data: BTreeMap<ProjectPath, Vec<Dvs1Scanned>> = BTreeMap::new();
    for s in scanned {
        by_data.entry(s.data.clone()).or_default().push(s);
    }
    let mut found = Vec::new();
    for (data, mut group) in by_data {
        if group
            .iter()
            .any(|s| s.parsed.blake3 != group[0].parsed.blake3)
        {
            let hashes: Vec<_> = group
                .iter()
                .map(|s| format!("{} names {}", s.sidecar.display(), s.parsed.blake3))
                .collect();
            for s in group {
                left.push(LeftSidecar {
                    sidecar: s.sidecar,
                    kind: LeftKind::Conflict,
                    message: format!(
                        "{} has dvs1 sidecars with different hashes: {}",
                        data.as_str(),
                        hashes.join(", ")
                    ),
                });
            }
            continue;
        }
        // The `.dvs` one first whatever the walk's order; the first that
        // converts stands for all
        group.sort_by_key(|s| {
            (
                s.sidecar.extension() != Some("dvs".as_ref()),
                s.sidecar.clone(),
            )
        });
        let sidecars: Vec<_> = group.iter().map(|s| s.sidecar.clone()).collect();
        let mut failures = Vec::new();
        let mut converted = None;
        for s in group {
            match resolve_one(root, &data, s.parsed, stores, now, &user) {
                Ok((metadata, notes)) => {
                    warnings.extend(notes);
                    converted = Some(metadata);
                    break;
                }
                Err(message) => failures.push((s.sidecar, message)),
            }
        }
        match converted {
            Some(metadata) => found.push(Dvs1Found {
                sidecars,
                data,
                metadata,
            }),
            None => left.extend(failures.into_iter().map(|(sidecar, message)| LeftSidecar {
                sidecar,
                kind: LeftKind::Unrecognized,
                message,
            })),
        }
    }
    found
}

/// The dvs2 sidecar of one dvs1 one, with a note per value filled in. A
/// devious sidecar may lack its time and user, which become the conversion's
/// time and the current user. A devious v0 one may lack its size, which only
/// its content gives (`content_size`); without it the sidecar is not converted.
fn resolve_one(
    root: &Path,
    data: &ProjectPath,
    mut parsed: Dvs1Sidecar,
    stores: &[PathBuf],
    now: jiff::Timestamp,
    user: &str,
) -> std::result::Result<(FileMetadata, Vec<String>), String> {
    let mut notes = Vec::new();
    let devious = matches!(
        parsed.shape,
        SidecarShape::DeviousV0 | SidecarShape::DeviousV1 | SidecarShape::DeviousV2
    );
    // devious before v0.5 wrote no size: 0 is unknown, not empty
    if parsed.shape == SidecarShape::DeviousV0 && parsed.size == 0 {
        parsed.size = content_size(root, data, &parsed.blake3, stores).ok_or(SIZE_UNKNOWN)?;
    }
    if devious {
        let no_time = parsed.recorded_at.is_empty();
        let no_user = parsed.saved_by.is_empty();
        let using = match (no_time, no_user) {
            (true, true) => Some(("time or user", format!("now and {user}"))),
            (true, false) => Some(("time", "now".to_string())),
            (false, true) => Some(("user", user.to_string())),
            (false, false) => None,
        };
        if let Some((what, using)) = using {
            notes.push(format!(
                "{}: no recorded {what} in its devious sidecar; using {using}",
                data.as_str()
            ));
        }
        if no_time {
            parsed.recorded_at = now.to_string();
        }
        if no_user {
            parsed.saved_by = user.to_string();
        }
    }
    let metadata = parsed.to_metadata().map_err(|e| e.to_string())?;
    Ok((metadata, notes))
}

/// The size of `hash`'s content: its uncompressed blob in `stores`, else the
/// working file if its content hashes to `hash`. Reads a blob's size and first
/// bytes, and hashes the working file only when no blob gives the size.
fn content_size(root: &Path, data: &ProjectPath, hash: &str, stores: &[PathBuf]) -> Option<u64> {
    // With no size to compare, only an uncompressed blob gives one
    if let Some((blob, Ok(Compression::None))) = legacy::find_blob(stores, hash, 0) {
        return fs::metadata(blob).ok().map(|m| m.len());
    }
    let file = root.join(data.to_native());
    if !file.is_file() {
        return None;
    }
    // A working file changed since it was added is not this content
    Hashes::compute_from_path(&file, &[])
        .ok()
        .filter(|(hashes, _)| hashes.blake3 == hash)
        .map(|(_, size)| size)
}

/// dvs2 local storage reads only `<hh>/<rest>` (D13), so a version whose
/// content `store` holds only in an older devious layout would be lost (R28).
/// Content found nowhere is left to the usual missing handling.
fn refuse_older_layouts(store: &Path, found: &[Dvs1Found]) -> Result<()> {
    let hashes: std::collections::BTreeSet<&str> = found
        .iter()
        .map(|f| f.metadata.hashes.blake3.as_str())
        .filter(|hash| {
            // `<hh>/<rest>` first, then the older layouts
            let paths = legacy::blob_paths(store, hash);
            !paths[0].is_file() && paths[1..].iter().any(|p| p.is_file())
        })
        .collect();
    if hashes.is_empty() {
        return Ok(());
    }
    Err(coded(
        ErrorCode::InvalidArguments,
        format!(
            "dvs migrate local: {} stored in an older devious layout (<hash>.dvsfile or <hash>) \
             that dvs2 local storage cannot read; run dvs migrate server --upload-missing, \
             or move the store into the <hh>/<rest> layout first",
            match hashes.len() {
                1 => "1 version is".to_string(),
                n => format!("{n} versions are"),
            }
        ),
    ))
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
                format!(
                    "No {CONFIG_FILE_NAME}, {DVS1_CONFIG_FILE} or .dvs.yaml here or in a parent folder"
                ),
            )
        })?;
    let root = fs::canonicalize(root)?;
    let dvs1 = match dvs1_config_path(&root) {
        Some(path) => Some(Dvs1Config::from_yaml(&fs::read_to_string(path)?)?),
        None => None,
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
    let (scanned, mut left) = scan_dvs1(&root, &paths.metadata_folder())?;
    if source == Source::Dvs2Local
        && matches!(destination, Destination::Local { .. })
        && dvs1.is_none()
        && scanned.is_empty()
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
    // A size missing from a devious sidecar is also looked up in the store
    // `dvs migrate local` points at
    let mut size_stores = stores.clone();
    if let Backend::Local(l) = config.backend() {
        size_stores.push(l.path.clone());
    }
    let found = resolve_dvs1(&root, scanned, &size_stores, &mut left, &mut warnings);
    if let (Destination::Local { .. }, Backend::Local(l)) = (destination, config.backend()) {
        refuse_older_layouts(&l.path, &found)?;
    }
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
        sidecars,
        data,
        mut metadata,
    } in found
    {
        let target = paths.metadata_path(&data.to_native());
        if target.is_file() {
            let message = match read_metadata(&target) {
                Ok(dvs2) if dvs2.hashes.blake3 == metadata.hashes.blake3 => {
                    conversion.duplicates.push(data);
                    conversion.to_remove.extend(sidecars);
                    continue;
                }
                Ok(dvs2) => format!(
                    "its hash {} differs from {}'s {}",
                    metadata.hashes.blake3,
                    target.strip_prefix(paths.repo_root())?.display(),
                    dvs2.hashes.blake3
                ),
                Err(e) => e.to_string(),
            };
            left.extend(sidecars.into_iter().map(|sidecar| LeftSidecar {
                sidecar,
                kind: LeftKind::Conflict,
                message: message.clone(),
            }));
            continue;
        }
        if let Some(compression) = compression {
            metadata.compression = compression;
        }
        metadata.write(paths, data.to_native())?;
        conversion.converted.push(data);
        conversion.to_remove.extend(sidecars);
    }
    Ok(conversion)
}

fn write_config(root: &Path, config: &Config) -> Result<()> {
    create_metadata_folder(root, config)?;
    config.save(root)
}

/// Deletes dvs.yaml or devious's .dvs.yaml and `sidecars`. Returns what it
/// deleted, repo-relative.
fn clean_up(root: &Path, sidecars: Vec<PathBuf>) -> Result<Vec<PathBuf>> {
    let mut to_remove = sidecars;
    for name in DVS1_CONFIG_FILES {
        if root.join(name).is_file() {
            to_remove.push(PathBuf::from(name));
        }
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
    use crate::testutil::{create_file, create_temp_git_repo, git, init_dvs_repo};

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

    // devious sidecars as its tags wrote them. v0.4.1's Go `Metadata` had only
    // `FileHash` and no json tags, written with `json.NewEncoder`.
    fn devious_v0(hash: &str) -> String {
        format!("{{\"FileHash\":\"{hash}\"}}\n")
    }

    // v1.0 (`json.MarshalIndent`, `user`)
    fn devious_v1(hash: &str) -> String {
        format!(
            "{{\n  \"blake3_checksum\": \"{hash}\",\n  \"file_size_bytes\": 22600,\n  \
             \"timestamp\": \"2023-08-15T11:47:21.737105368-04:00\",\n  \
             \"message\": \"\",\n  \"user\": \"andriygm\"\n}}"
        )
    }

    // v2.0.0's testing/266-536x354.jpg.dvs, with another hash
    fn devious_v2(hash: &str) -> String {
        format!(
            "{{\n  \"blake3_checksum\": \"{hash}\",\n  \"file_size_bytes\": 22600,\n  \
             \"timestamp\": \"2023-10-11T11:47:21.737105368-04:00\",\n  \
             \"message\": \"\",\n  \"saved_by\": \"andriygm\"\n}}"
        )
    }

    /// A devious v1 tree: `.dvs.yaml` with `storage-dir`, a `.dvsmeta` sidecar,
    /// and devious's root .gitignore entry for it.
    fn devious_repo() -> (tempfile::TempDir, PathBuf, PathBuf) {
        let (tmp, root) = create_temp_git_repo();
        let store = root.parent().unwrap().join("store");
        fs::create_dir(&store).unwrap();
        let yaml = format!("storage-dir: {}\n", store.display());
        create_file(&root, ".dvs.yaml", yaml.as_bytes());
        create_file(
            &root,
            "data/a.jpg.dvsmeta",
            devious_v1(&hash('a')).as_bytes(),
        );
        create_file(&root, ".gitignore", b"\n\n# Devious entry\n/data/a.jpg");
        (tmp, root, store)
    }

    #[test]
    fn devious_converts_and_cleans_up() {
        let (_tmp, root, store) = devious_repo();
        create_file(&root, "b.jpg.dvs", devious_v2(&hash('b')).as_bytes());
        let report = migrate(&root, &local(), true, false).unwrap();

        assert_eq!(report.source, Source::Dvs1);
        assert_eq!(report.converted.len(), 2);
        assert!(!root.join(".dvs.yaml").exists());
        assert!(!root.join("data/a.jpg.dvsmeta").exists());
        assert!(!root.join("b.jpg.dvs").exists());
        let Backend::Local(l) = Config::find(&root).unwrap().unwrap().backend().clone() else {
            panic!("not local");
        };
        assert_eq!(l.path, fs::canonicalize(&store).unwrap());

        let m = read_metadata(&root.join(".dvs/data/a.jpg.dvs")).unwrap();
        assert_eq!(m.hashes.blake3, hash('a'));
        assert_eq!(m.created_by, "andriygm");
        assert_eq!(m.add_time.to_string(), "2023-08-15T15:47:21.737105368Z");
        assert_eq!(m.size, 22600);
    }

    #[test]
    fn devious_gitignore_entries_stay() {
        let (_tmp, root, _store) = devious_repo();
        migrate(&root, &local(), true, false).unwrap();
        assert_eq!(
            fs::read_to_string(root.join(".gitignore")).unwrap(),
            "\n\n# Devious entry\n/data/a.jpg"
        );
    }

    #[test]
    fn devious_config_names_the_storage_to_upload_from() {
        let (_tmp, root, store) = devious_repo();
        let prepared = prepare(&root, &server(None), options(true, true)).unwrap();
        assert_eq!(prepared.stores, [store]);
    }

    #[test]
    fn devious_v0_without_time_or_user_gets_now_and_the_user() {
        let (_tmp, root, _store) = devious_repo();
        create_file(
            &root,
            "b.jpg.dvsmeta",
            devious_v0(&blake3_of(b"b")).as_bytes(),
        );
        create_file(&root, "b.jpg", b"b");
        let before = jiff::Timestamp::now();
        let report = migrate(&root, &local(), true, false).unwrap();

        let user = whoami::username().unwrap();
        assert_eq!(
            report.warnings,
            [format!(
                "b.jpg: no recorded time or user in its devious sidecar; using now and {user}"
            )]
        );
        let m = read_metadata(&root.join(".dvs/b.jpg.dvs")).unwrap();
        assert_eq!(m.created_by, user);
        assert!(m.add_time >= before);
    }

    fn blake3_of(content: &[u8]) -> String {
        blake3::hash(content).to_string()
    }

    #[test]
    fn local_refuses_content_only_in_an_older_devious_layout() {
        let (_tmp, root, store) = devious_repo();
        create_file(&store, &format!("{}.dvsfile", hash('a')), b"a");
        let before: Vec<_> = WalkDir::new(root.parent().unwrap())
            .sort_by_file_name()
            .into_iter()
            .map(|e| e.unwrap().into_path())
            .collect();

        let err = migrate(&root, &local(), true, false).unwrap_err();
        assert_eq!(backend_error_code(&err), Some(ErrorCode::InvalidArguments));
        assert!(
            err.to_string()
                .starts_with("dvs migrate local: 1 version is stored in an older devious layout"),
            "{err}"
        );
        let after: Vec<_> = WalkDir::new(root.parent().unwrap())
            .sort_by_file_name()
            .into_iter()
            .map(|e| e.unwrap().into_path())
            .collect();
        assert_eq!(before, after);
    }

    #[test]
    fn local_converts_content_in_the_nested_layout() {
        let (_tmp, root, store) = devious_repo();
        let a = hash('a');
        create_file(&store, &format!("{}/{}", &a[..2], &a[2..]), b"a");
        create_file(&store, &format!("{a}.dvsfile"), b"a");
        let report = migrate(&root, &local(), true, false).unwrap();
        assert_eq!(report.converted.len(), 1);
    }

    #[test]
    fn devious_v0_size_comes_from_the_blob_or_the_file() {
        let (_tmp, root, store) = devious_repo();
        let (b, c) = (blake3_of(b"bbbb"), blake3_of(b"ccccc"));
        create_file(&root, "b.jpg.dvsmeta", devious_v0(&b).as_bytes());
        create_file(&root, "c.jpg.dvsmeta", devious_v0(&c).as_bytes());
        // c is only in the working tree
        create_file(&store, &format!("{}/{}", &b[..2], &b[2..]), b"bbbb");
        create_file(&root, "c.jpg", b"ccccc");
        let report = migrate(&root, &local(), true, false).unwrap();

        assert!(report.left.is_empty(), "{:?}", report.left);
        let size = |name: &str| read_metadata(&root.join(".dvs").join(name)).unwrap().size;
        assert_eq!(size("b.jpg.dvs"), 4);
        assert_eq!(size("c.jpg.dvs"), 5);
    }

    #[test]
    fn devious_v0_size_prefers_the_blob() {
        let (_tmp, root, store) = devious_repo();
        let b = blake3_of(b"bbbbbb");
        create_file(&root, "b.jpg.dvsmeta", devious_v0(&b).as_bytes());
        create_file(&store, &format!("{}/{}", &b[..2], &b[2..]), b"bbbb");
        create_file(&root, "b.jpg", b"bbbbbb");
        migrate(&root, &local(), true, false).unwrap();
        assert_eq!(read_metadata(&root.join(".dvs/b.jpg.dvs")).unwrap().size, 4);
    }

    #[test]
    fn devious_v0_size_ignores_a_changed_working_file() {
        let (_tmp, root, _store) = devious_repo();
        create_file(&root, "b.jpg.dvsmeta", devious_v0(&hash('b')).as_bytes());
        create_file(&root, "b.jpg", b"edited since it was added");
        let report = migrate(&root, &local(), true, false).unwrap();
        assert_eq!(report.left[0].sidecar, PathBuf::from("b.jpg.dvsmeta"));
        assert!(!root.join(".dvs/b.jpg.dvs").exists());
    }

    #[test]
    fn devious_v0_of_unknown_size_is_left() {
        let (_tmp, root, _store) = devious_repo();
        create_file(&root, "b.jpg.dvsmeta", devious_v0(&hash('b')).as_bytes());
        let report = migrate(&root, &local(), true, false).unwrap();

        assert_eq!(report.converted.len(), 1);
        assert_eq!(report.left.len(), 1);
        assert_eq!(report.left[0].kind, LeftKind::Unrecognized);
        assert_eq!(report.left[0].message, SIZE_UNKNOWN);
        assert!(root.join("b.jpg.dvsmeta").is_file());
        assert!(!root.join(".dvs/b.jpg.dvs").exists());
        assert!(report.is_partial());
    }

    #[test]
    fn devious_v1_and_v2_without_user_or_time_get_the_fallback() {
        let (_tmp, root, _store) = devious_repo();
        let no_user = devious_v1(&hash('b')).replace("\"andriygm\"", "\"\"");
        let no_time =
            devious_v2(&hash('c')).replace("\"2023-10-11T11:47:21.737105368-04:00\"", "\"\"");
        create_file(&root, "b.jpg.dvsmeta", no_user.as_bytes());
        create_file(&root, "c.jpg.dvs", no_time.as_bytes());
        let before = jiff::Timestamp::now();
        let report = migrate(&root, &local(), true, false).unwrap();

        let user = whoami::username().unwrap();
        assert_eq!(
            report.warnings,
            [
                format!("b.jpg: no recorded user in its devious sidecar; using {user}"),
                "c.jpg: no recorded time in its devious sidecar; using now".to_string(),
            ]
        );
        let b = read_metadata(&root.join(".dvs/b.jpg.dvs")).unwrap();
        assert_eq!(b.created_by, user);
        let c = read_metadata(&root.join(".dvs/c.jpg.dvs")).unwrap();
        assert_eq!(c.created_by, "andriygm");
        assert!(c.add_time >= before);
    }

    #[test]
    fn devious_no_clean_keeps_its_config() {
        let (_tmp, root, _store) = devious_repo();
        let report = migrate(&root, &local(), false, false).unwrap();
        assert!(report.removed.is_empty());
        assert!(root.join(".dvs.yaml").is_file());
        assert!(root.join("data/a.jpg.dvsmeta").is_file());
    }

    #[test]
    fn an_unclassified_dvsmeta_stays() {
        let (_tmp, root, _store) = devious_repo();
        create_file(&root, "b.jpg.dvsmeta", b"not json");
        let report = migrate(&root, &local(), true, false).unwrap();
        assert_eq!(report.left[0].sidecar, PathBuf::from("b.jpg.dvsmeta"));
        assert!(report.left[0].message.starts_with("not JSON"));
        assert!(root.join("b.jpg.dvsmeta").is_file());
    }

    #[test]
    fn dvs_and_dvsmeta_with_one_hash_convert_once() {
        let (_tmp, root, _store) = devious_repo();
        // The same file renamed by devious v2's migrate, by another user
        let v2 = devious_v2(&hash('a')).replace("andriygm", "ann");
        create_file(&root, "data/a.jpg.dvs", v2.as_bytes());
        let report = migrate(&root, &local(), true, false).unwrap();

        assert!(report.left.is_empty(), "{:?}", report.left);
        assert_eq!(report.converted.len(), 1);
        assert!(!root.join("data/a.jpg.dvs").exists());
        assert!(!root.join("data/a.jpg.dvsmeta").exists());
        // The `.dvs` one, though the `.dvsmeta` one sorts no earlier
        let m = read_metadata(&root.join(".dvs/data/a.jpg.dvs")).unwrap();
        assert_eq!(m.created_by, "ann");
    }

    #[test]
    fn dvs_and_dvsmeta_with_two_hashes_are_a_conflict() {
        let (_tmp, root, _store) = devious_repo();
        create_file(&root, "data/a.jpg.dvs", devious_v2(&hash('b')).as_bytes());
        let report = migrate(&root, &local(), true, false).unwrap();

        assert!(report.converted.is_empty());
        let left: Vec<_> = report.left.iter().map(|l| (&l.sidecar, l.kind)).collect();
        assert_eq!(
            left,
            [
                (&PathBuf::from("data/a.jpg.dvs"), LeftKind::Conflict),
                (&PathBuf::from("data/a.jpg.dvsmeta"), LeftKind::Conflict),
            ]
        );
        assert!(root.join("data/a.jpg.dvs").is_file());
        assert!(root.join("data/a.jpg.dvsmeta").is_file());
        assert!(!root.join(".dvs/data/a.jpg.dvs").exists());
    }

    #[test]
    fn devious_sidecars_without_config_need_a_local_path() {
        let (_tmp, root, store) = devious_repo();
        fs::remove_dir(root.join(".git")).unwrap();
        fs::remove_file(root.join(".dvs.yaml")).unwrap();
        // Only git's index says the checkout has devious sidecars
        git(&root, &["init", "-q"]);
        git(&root, &["add", "data/a.jpg.dvsmeta"]);
        let err = prepare(&root, &local(), options(true, false))
            .err()
            .unwrap();
        assert_eq!(backend_error_code(&err), Some(ErrorCode::InvalidArguments));
        assert!(err.to_string().contains("give its path"), "{err}");

        let path = Some(store);
        let report = migrate(&root, &Destination::Local { path }, true, false).unwrap();
        assert_eq!(report.converted.len(), 1);
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
