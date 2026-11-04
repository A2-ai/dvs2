use std::collections::{BTreeMap, HashSet};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::Instant;

use anyhow::{Result, anyhow};
use fs_err as fs;
use rayon::prelude::*;
use serde::Serialize;
use url::Url;
use uuid::Uuid;

use crate::backends::local::LocalBackend;
use crate::backends::server::{ServerBackend, UploadManifest, UploadManifestEntry};
use crate::config::Config;
use crate::dvs1::{self, Dvs1Config, Dvs1Metadata};
use crate::error::{Error, ErrorCode, backend_error_code};
use crate::gitignore::remove_dvs1_exceptions;
use crate::operation::{
    ByteTotals, Counts, FileRow, OperationContext, OperationPlan, OperationRow, RowState,
    RunOutcome, millis,
};
use crate::paths::{CONFIG_FILE_NAME, DEFAULT_FOLDER_NAME, normalize_path};
use crate::utils::get_threadpool;
use crate::{
    Backend, Compression, DvsPaths, FileMetadata, GetCode, Hashes, Outcome, ProjectPath,
    StoreRequest, StoreResult,
};

/// In the metadata folder: the old storage each migration copied from, one path per line.
pub const MIGRATED_FROM_FILE_NAME: &str = "migrated-from";

/// The sources listed in the project's `migrated-from`, in order; none without one.
pub fn migrated_from(paths: &DvsPaths) -> Result<Vec<PathBuf>> {
    let path = paths.metadata_folder().join(MIGRATED_FROM_FILE_NAME);
    match fs::read_to_string(&path) {
        Ok(content) => Ok(content
            .lines()
            .filter(|line| !line.is_empty())
            .map(PathBuf::from)
            .collect()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Vec::new()),
        Err(e) => Err(e.into()),
    }
}

/// The hint for content with BLAKE3 `hash` not in the project's storage,
/// naming `migrated_from`'s sources; none when no source holds it.
pub fn migrate_hint(hash: &str, migrated_from: &[PathBuf]) -> Option<String> {
    let hashes = Hashes {
        blake3: hash.to_string(),
        md5: None,
    };
    if find_blobs(migrated_from, &hashes).ok()?.is_empty() {
        return None;
    }
    let sources: Vec<_> = migrated_from
        .iter()
        .map(|s| s.display().to_string())
        .collect();
    Some(format!(
        "not in this project's storage yet; run dvs migrate to copy it from {}",
        sources.join(" or ")
    ))
}

/// Appends `source` to the project's `migrated-from`, unless it is already listed.
pub fn record_migrated_from(paths: &DvsPaths, source: &Path) -> Result<()> {
    if migrated_from(paths)?.iter().any(|listed| listed == source) {
        return Ok(());
    }
    let line = source
        .to_str()
        .ok_or_else(|| anyhow!("{} is not UTF-8", source.display()))?;
    let path = paths.metadata_folder().join(MIGRATED_FROM_FILE_NAME);
    let existing = fs::read(&path).unwrap_or_default();
    fs::create_dir_all(paths.metadata_folder())?;
    let mut file = fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)?;
    if !existing.is_empty() && !existing.ends_with(b"\n") {
        writeln!(file)?;
    }
    writeln!(file, "{line}")?;
    Ok(())
}

/// Where `migrate` moves the checkout's data.
#[derive(Debug, Clone)]
pub enum Target {
    /// A dvs1 checkout to dvs2 local storage: its own folder in place, or a
    /// copy into a new folder at `path`. `apply_local_modes` gives the folder
    /// in place dvs2's modes and group.
    Local {
        path: Option<PathBuf>,
        apply_local_modes: bool,
    },
    /// A dvs1 or dvs2 local checkout to the server project `name`, created
    /// with `group` if it does not exist yet
    Server {
        name: String,
        url: Url,
        group: Option<String>,
    },
    /// The checkout's own `dvs.toml`: copies what it lacks from the
    /// `migrated-from` sources and converts dvs1 files a merge brought in
    Configured,
}

/// What a migration to a server does to its project.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum ProjectAction {
    Created,
    Added,
}

/// One file the checkout lists.
#[derive(Debug, Clone, Serialize)]
pub struct MigrateRow {
    /// Relative to the project root
    pub path: PathBuf,
    #[serde(flatten)]
    pub detail: MigrateDetail,
    /// Time spent copying it; `None` when nothing was copied
    pub duration_ms: Option<u64>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(untagged)]
pub enum MigrateDetail {
    /// `copied`: stored in the target (or would be, in a dry run);
    /// `present`: the target already had it
    Success {
        outcome: Outcome,
        hash: String,
        size: u64,
    },
    /// In neither the target nor any source, so it was already unfetchable.
    /// Its metadata is converted all the same.
    Missing {
        code: GetCode,
        hash: String,
        size: u64,
        message: String,
    },
    Error {
        code: ErrorCode,
        error: String,
    },
    Cancelled {
        message: String,
    },
}

impl OperationRow for MigrateRow {
    fn state(&self) -> RowState {
        match self.detail {
            MigrateDetail::Success { .. } | MigrateDetail::Missing { .. } => RowState::Succeeded,
            MigrateDetail::Error { .. } => RowState::Failed,
            MigrateDetail::Cancelled { .. } => RowState::Cancelled,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum ChangeKind {
    Written,
    Removed,
}

/// A working-tree file the migration changes, for the user to commit.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Change {
    /// Relative to the project root
    pub path: PathBuf,
    pub change: ChangeKind,
}

/// The rows of a migration, sorted by path, plus what it did around them.
#[derive(Debug, Clone, Default)]
pub struct MigrateReport {
    pub dry_run: bool,
    /// The old storage read from, as recorded in `migrated-from`
    pub sources: Vec<PathBuf>,
    /// For a server target, whether the project is (or would be) created or added to
    pub project: Option<ProjectAction>,
    pub files: Vec<MigrateRow>,
    pub counts: Counts,
    pub outcome: RunOutcome,
    pub bytes: ByteTotals,
    /// Sorted by path; a dry run lists what it would change
    pub changes: Vec<Change>,
    pub warnings: Vec<String>,
}

impl MigrateReport {
    pub fn already_stored(&self) -> usize {
        self.files
            .iter()
            .filter(|r| {
                matches!(
                    r.detail,
                    MigrateDetail::Success {
                        outcome: Outcome::Present,
                        ..
                    }
                )
            })
            .count()
    }
}

/// A form of migrate, resolved against the checkout.
struct Setup {
    backend: Backend,
    /// What the target stores with; for a server, known once it answers the manifest
    compression: Option<Compression>,
    /// The `dvs.toml` to write
    config: Option<Config>,
    sources: Vec<PathBuf>,
    project: Option<ProjectAction>,
    /// Only files the target lacks and dvs1 files are migrated
    rerun: bool,
    /// The local folder is new
    init: bool,
    /// The folder in place to give dvs2's modes
    modes: Option<LocalBackend>,
    warnings: Vec<String>,
}

struct Entry {
    path: PathBuf,
    /// As the source has it: its compression is how the source stored the blob
    metadata: FileMetadata,
    /// The dvs1 metadata file it came from, relative to the root
    sidecar: Option<PathBuf>,
}

/// A file's row, its bytes (size, stored, transferred) and whether the target
/// has its blob after the run.
type Done = (MigrateRow, Option<(u64, bool, bool)>, bool);

enum Step {
    Present,
    Copy(Vec<PathBuf>),
    Missing,
}

fn refusal(code: ErrorCode, message: impl Into<String>) -> anyhow::Error {
    Error::new(code, message).into()
}

/// Canonical through the deepest part that exists, so a path not made yet is
/// still compared through symlinks and `..`.
fn absolute(path: &Path) -> Result<PathBuf> {
    let path = std::path::absolute(path)?;
    let existing = path
        .ancestors()
        .find(|a| a.exists())
        .ok_or_else(|| anyhow!("{} has no existing ancestor", path.display()))?;
    let joined = fs::canonicalize(existing)?.join(path.strip_prefix(existing)?);
    normalize_path(joined).ok_or_else(|| anyhow!("{} escapes the root", path.display()))
}

fn local_setup(
    root: &Path,
    config: Option<&Config>,
    dvs1: Option<&Dvs1Config>,
    path: Option<PathBuf>,
    apply_local_modes: bool,
) -> Result<Setup> {
    let (None, Some(dvs1)) = (config, dvs1) else {
        return Err(refusal(
            ErrorCode::InvalidConfiguration,
            "dvs migrate local converts a dvs1 project, and this one has a dvs.toml; \
             run dvs migrate to finish an earlier migration",
        ));
    };
    let storage = dvs1.storage_dir.clone();
    let mut warnings = Vec::new();
    let (local, compression, init) = match path {
        None if apply_local_modes => (
            LocalBackend::new(&storage, dvs1.group.clone())?,
            Compression::None,
            false,
        ),
        None => {
            if let Some(permissions) = dvs1.permissions {
                warnings.push(format!(
                    "dvs.yaml permissions = {permissions:o} was dropped: the folder keeps its \
                     modes (--apply-local-modes sets dvs2's)"
                ));
            }
            (
                LocalBackend::without_group(&storage),
                Compression::None,
                false,
            )
        }
        Some(_) if apply_local_modes => {
            return Err(refusal(
                ErrorCode::InvalidArguments,
                "local modes only apply to the dvs1 folder in place",
            ));
        }
        Some(path) => {
            let path = absolute(&path)?;
            if path.starts_with(root) {
                return Err(refusal(
                    ErrorCode::InvalidArguments,
                    "The given storage path is within the repository.",
                ));
            }
            if path.starts_with(absolute(&storage)?) {
                return Err(refusal(
                    ErrorCode::InvalidArguments,
                    "that is in the dvs1 folder; leave out the path to use it in place",
                ));
            }
            let local = LocalBackend::new(&path, dvs1.group.clone())
                .or_else(|_| LocalBackend::new(&path, None))?;
            (local, Compression::Zstd, true)
        }
    };
    let mut new_config = Config::from_dvs1(root)?;
    new_config.set_backend(Backend::Local(local.clone()), Some(compression));
    Ok(Setup {
        modes: apply_local_modes.then(|| local.clone()),
        backend: Backend::Local(local),
        compression: Some(compression),
        config: Some(new_config),
        sources: vec![storage],
        project: None,
        rerun: false,
        init,
        warnings,
    })
}

/// The current storage, then the leftover `dvs.yaml`'s, then `migrated-from`'s, each once.
fn server_sources(
    current: PathBuf,
    leftover: Option<PathBuf>,
    migrated_from: Vec<PathBuf>,
) -> Vec<PathBuf> {
    let mut sources = vec![current];
    for earlier in leftover.into_iter().chain(migrated_from) {
        if !sources.contains(&earlier) {
            sources.push(earlier);
        }
    }
    sources
}

/// The checkout's storage and the config it gets, or why it cannot move to a server.
fn server_source(
    root: &Path,
    config: Option<&Config>,
    dvs1: Option<&Dvs1Config>,
) -> Result<(PathBuf, Config)> {
    match (config, dvs1) {
        (Some(config), _) => match config.backend() {
            Backend::Local(local) => Ok((local.path.clone(), config.clone())),
            Backend::Server(_) => Err(refusal(
                ErrorCode::InvalidConfiguration,
                "this project already uses a server; run dvs migrate to copy what it \
                 lacks from its earlier storage",
            )),
        },
        (None, Some(dvs1)) => Ok((dvs1.storage_dir.clone(), Config::from_dvs1(root)?)),
        (None, None) => unreachable!("migrate refuses a checkout with neither"),
    }
}

#[allow(clippy::too_many_arguments)]
fn server_setup(
    root: &Path,
    config: Option<&Config>,
    dvs1: Option<&Dvs1Config>,
    leftover: Option<PathBuf>,
    paths: &DvsPaths,
    name: String,
    url: Url,
    group: Option<String>,
    dry_run: bool,
) -> Result<Setup> {
    let (source, mut new_config) = server_source(root, config, dvs1)?;
    let sources = server_sources(source, leftover, migrated_from(paths)?);

    let server = ServerBackend::new(name, group, url);
    let project = if server.project_exists()? {
        server.check_access()?;
        ProjectAction::Added
    } else if server.group.is_none() {
        return Err(refusal(
            ErrorCode::InvalidArguments,
            format!(
                "the project {} does not exist on {} yet; give a group to create it",
                server.name, server.url
            ),
        ));
    } else if dry_run || !server.init(Compression::Zstd)? {
        ProjectAction::Created
    } else {
        ProjectAction::Added
    };
    // The server decides the compression
    new_config.set_backend(Backend::Server(server.clone()), None);
    Ok(Setup {
        backend: Backend::Server(server),
        compression: (dry_run && project == ProjectAction::Created).then_some(Compression::Zstd),
        config: Some(new_config),
        sources,
        project: Some(project),
        rerun: false,
        init: false,
        modes: None,
        warnings: Vec::new(),
    })
}

/// `None` when there is nowhere to copy from.
fn rerun_setup(
    config: Option<&Config>,
    leftover: Option<PathBuf>,
    paths: &DvsPaths,
) -> Result<Option<Setup>> {
    let Some(config) = config else {
        return Err(refusal(
            ErrorCode::InvalidConfiguration,
            "this dvs1 project has not been migrated yet; run dvs migrate local or dvs migrate server",
        ));
    };
    let mut sources = migrated_from(paths)?;
    sources.extend(leftover.filter(|l| !sources.contains(l)));
    if sources.is_empty() {
        return Ok(None);
    }
    let backend = config.backend().clone();
    if let Backend::Server(server) = &backend {
        server.check_access()?;
    }
    Ok(Some(Setup {
        compression: matches!(backend, Backend::Local(_)).then(|| config.compression()),
        backend,
        config: None,
        sources,
        project: None,
        rerun: true,
        init: false,
        modes: None,
        warnings: Vec::new(),
    }))
}

fn row_error_code(error: &anyhow::Error) -> ErrorCode {
    if let Some(code) = backend_error_code(error) {
        code
    } else if error.downcast_ref::<serde_json::Error>().is_some() {
        ErrorCode::BadMetadata
    } else if error.downcast_ref::<std::io::Error>().is_some() {
        ErrorCode::IoError
    } else {
        ErrorCode::OperationFailed
    }
}

fn error_row(path: PathBuf, error: &anyhow::Error, duration_ms: Option<u64>) -> MigrateRow {
    log::warn!("Failed to migrate {}: {error}", path.display());
    MigrateRow {
        path,
        detail: MigrateDetail::Error {
            code: row_error_code(error),
            error: error.to_string(),
        },
        duration_ms,
    }
}

/// The files the checkout lists, from its metadata folder `folder` (`dvs2`)
/// and dvs1 files next to the data, plus a row for each that cannot be read.
/// Without a dvs1 dvs.yaml (`dvs1`), a `.dvs` file beside a dvs.toml is only
/// dvs1's if it reads as such.
fn read_entries(
    paths: &DvsPaths,
    folder: &str,
    dvs2: bool,
    dvs1: bool,
) -> (Vec<Entry>, Vec<MigrateRow>) {
    let mut found: BTreeMap<PathBuf, Result<Entry>> = BTreeMap::new();
    if dvs2 {
        for path in paths.tracked_paths() {
            let read = FileMetadata::read(paths, &path);
            let entry = read.map(|metadata| Entry {
                path: path.clone(),
                metadata,
                sidecar: None,
            });
            found.insert(path, entry);
        }
    }
    let root = paths.repo_root();
    let dvs1_paths = if dvs2 {
        dvs1::leftover_paths(paths, folder, dvs1)
    } else {
        dvs1::tracked_paths(root, folder)
    };
    for path in dvs1_paths {
        let sidecar = dvs1::metadata_path(&path);
        let read = Dvs1Metadata::read(&root.join(&sidecar)).and_then(|m| m.to_metadata());
        match (found.get_mut(&path), read) {
            (None, read) => {
                let entry = read.map(|metadata| Entry {
                    path: path.clone(),
                    metadata,
                    sidecar: Some(sidecar),
                });
                found.insert(path, entry);
            }
            (Some(Err(_)), _) => {}
            (Some(Ok(entry)), Ok(metadata))
                if entry.metadata.hashes.blake3 == metadata.hashes.blake3 =>
            {
                entry.sidecar = Some(sidecar);
            }
            (Some(slot), read) => {
                *slot = Err(read.err().unwrap_or_else(|| {
                    let dvs2 = paths.metadata_path(&path);
                    let dvs2 = dvs2.strip_prefix(root).unwrap_or(&dvs2);
                    refusal(
                        ErrorCode::BadMetadata,
                        format!(
                            "{} and {} list different content; delete the one to drop",
                            sidecar.display(),
                            dvs2.display()
                        ),
                    )
                }));
            }
        }
    }
    let mut entries = Vec::new();
    let mut rows = Vec::new();
    for (path, entry) in found {
        match entry {
            Ok(entry) => entries.push(entry),
            Err(e) => rows.push(error_row(path, &e, None)),
        }
    }
    (entries, rows)
}

/// The blob for `hashes` in each source holding it, in order. Old storage of
/// either generation lays blobs out like local storage.
fn find_blobs(sources: &[PathBuf], hashes: &Hashes) -> Result<Vec<PathBuf>> {
    let mut found = Vec::new();
    for source in sources {
        let blob = LocalBackend::without_group(source).hash_to_path(hashes)?;
        if blob.is_file() {
            found.push(blob);
        }
    }
    Ok(found)
}

/// Stores the first of `blobs` that, decoded with the compression the source
/// stored it with or else the other one, has the entry's hash.
fn copy(
    entry: &Entry,
    blobs: &[PathBuf],
    backend: &Backend,
    compression: Compression,
    operation_id: Uuid,
    message: &str,
    on_bytes: Option<&(dyn Fn(u64) + Send + Sync)>,
) -> Result<StoreResult> {
    let decoded = tempfile::NamedTempFile::new()?.into_temp_path();
    let recorded = entry.metadata.compression;
    let other = match recorded {
        Compression::None => Compression::Zstd,
        Compression::Zstd => Compression::None,
    };
    for blob in blobs {
        for stored in [recorded, other] {
            let source = match stored {
                Compression::None => blob.as_path(),
                stored => {
                    if stored.decompress(blob, &decoded, None).is_err() {
                        continue;
                    }
                    &*decoded
                }
            };
            if Hashes::compute_from_path(source, &[])?.0.blake3 != entry.metadata.hashes.blake3 {
                continue;
            }
            return backend.store(StoreRequest {
                hashes: &entry.metadata.hashes,
                source,
                compression,
                path: ProjectPath::from_path(&entry.path).map_err(|e| anyhow!("{e}"))?,
                operation_id,
                message: Some(message),
                on_bytes,
            });
        }
    }
    Err(refusal(
        ErrorCode::BadMetadata,
        "no old storage has a blob that decodes to its hash",
    ))
}

fn manifest_entry(entry: &Entry) -> Result<UploadManifestEntry> {
    Ok(UploadManifestEntry {
        path: ProjectPath::from_path(&entry.path).map_err(|e| anyhow!("{e}"))?,
        size: entry.metadata.size,
        hash: entry.metadata.hashes.blake3.clone(),
    })
}

fn relative(root: &Path, path: &Path) -> PathBuf {
    path.strip_prefix(root).unwrap_or(path).to_path_buf()
}

/// The checkout's root, `dvs.toml` and dvs1 `dvs.yaml`; at least one of them.
fn checkout(repo_root: &Path) -> Result<(PathBuf, Option<Config>, Option<Dvs1Config>)> {
    let root = fs::canonicalize(repo_root)?;
    let config = if root.join(CONFIG_FILE_NAME).is_file() {
        Config::find(&root).transpose()?
    } else {
        None
    };
    let dvs1 = dvs1::is_dvs1(&root)
        .then(|| Dvs1Config::read(&root))
        .transpose()?;
    if config.is_none() && dvs1.is_none() {
        return Err(refusal(
            ErrorCode::NotInProject,
            format!("{} has no dvs.toml or dvs1 dvs.yaml", root.display()),
        ));
    }
    Ok((root, config, dvs1))
}

/// The refusals `migrate` gives a server `target` before contacting the server.
pub fn check(repo_root: &Path, target: &Target) -> Result<()> {
    let (root, config, dvs1) = checkout(repo_root)?;
    if let Target::Server { .. } = target {
        server_source(&root, config.as_ref(), dvs1.as_ref())?;
    }
    Ok(())
}

/// Migrates the checkout at `repo_root` to `target`: copies the blob of every
/// file it lists that the target lacks from old storage, which it only reads,
/// rewrites the metadata for the target and writes `dvs.toml`, leaving the
/// changes for the user to commit. Re-running only copies what is still
/// missing. A dry run plans the same and changes nothing.
///
/// Reports the plan and each copy to `ctx`'s observer. Once `ctx` is
/// cancelled no new copy starts; the checkout is still converted, dvs1
/// sidecars included, and the `.dvs` of each file not copied keeps its source
/// compression so that a bare `dvs migrate` finishes them.
pub fn migrate(
    repo_root: &Path,
    target: Target,
    dry_run: bool,
    ctx: &OperationContext,
) -> Result<MigrateReport> {
    let (root, config, dvs1) = checkout(repo_root)?;
    let folder = config
        .as_ref()
        .map_or(DEFAULT_FOLDER_NAME, Config::metadata_folder_name)
        .to_string();
    let paths = DvsPaths::new(root.clone(), root.clone(), folder.clone())?;
    // dvs1 files beside a dvs.toml came in with a merge
    let leftover = dvs1
        .as_ref()
        .filter(|_| config.is_some())
        .map(|d| d.storage_dir.clone());

    let mut setup = match target {
        Target::Local {
            path,
            apply_local_modes,
        } => local_setup(
            &root,
            config.as_ref(),
            dvs1.as_ref(),
            path,
            apply_local_modes,
        )?,
        Target::Server { name, url, group } => server_setup(
            &root,
            config.as_ref(),
            dvs1.as_ref(),
            leftover,
            &paths,
            name,
            url,
            group,
            dry_run,
        )?,
        Target::Configured => match rerun_setup(config.as_ref(), leftover, &paths)? {
            Some(setup) => setup,
            None => {
                return Ok(MigrateReport {
                    dry_run,
                    ..Default::default()
                });
            }
        },
    };

    let (entries, mut files) = read_entries(&paths, &folder, config.is_some(), dvs1.is_some());
    // While a dvs1 file is left unconverted, a later run needs dvs.yaml to find it
    let keep_dvs_yaml = files
        .iter()
        .any(|r| root.join(dvs1::metadata_path(&r.path)).is_file());
    let pool = get_threadpool(entries.len().max(1))?;

    // 1. What the target already has: the server answers the manifest when
    // every file goes in it, and a project yet to be created has nothing
    let server = match &setup.backend {
        Backend::Server(server) => Some(server),
        Backend::Local(_) => None,
    };
    let manifest_all = server.is_some() && !dry_run && !setup.rerun;
    let target_empty = dry_run && setup.project == Some(ProjectAction::Created);
    let mut has: Vec<Result<bool>> = if manifest_all || target_empty {
        entries.iter().map(|_| Ok(false)).collect()
    } else {
        pool.install(|| {
            entries
                .par_iter()
                .map(|e| setup.backend.exists(&e.metadata.hashes))
                .collect()
        })
    };
    let in_scope: Vec<bool> = entries
        .iter()
        .zip(&has)
        .map(|(e, has)| {
            !setup.rerun
                || e.sidecar.is_some()
                || !matches!(has, Ok(true))
                || setup
                    .compression
                    .is_some_and(|c| c != e.metadata.compression)
        })
        .collect();

    let operation_id = Uuid::new_v4();
    let sources: Vec<_> = setup
        .sources
        .iter()
        .map(|s| s.display().to_string())
        .collect();
    let message = format!("migrated from {}", sources.join(", "));
    if let Some(server) = server
        && !dry_run
    {
        let manifest_entries = entries
            .iter()
            .zip(&in_scope)
            .filter(|(_, in_scope)| **in_scope)
            .map(|(e, _)| manifest_entry(e))
            .collect::<Result<Vec<_>>>()?;
        if !manifest_entries.is_empty() {
            let checkout = crate::git::current_checkout(&root);
            let answer = server.manifest(&UploadManifest {
                operation_id,
                message: Some(message.clone()),
                entries: manifest_entries,
                git_branch: checkout.branch,
                git_commit: checkout.commit,
            })?;
            setup.compression = Some(answer.compression);
            let missing: HashSet<String> = answer.missing.into_iter().collect();
            for ((e, has), in_scope) in entries.iter().zip(&mut has).zip(&in_scope) {
                if *in_scope {
                    *has = Ok(!missing.contains(&e.metadata.hashes.blake3));
                }
            }
        }
    }

    // 2. Where to copy each missing blob from
    let steps: Vec<Result<Step>> = entries
        .iter()
        .zip(has)
        .map(|(e, has)| match has? {
            true => Ok(Step::Present),
            false => {
                let blobs = find_blobs(&setup.sources, &e.metadata.hashes)?;
                Ok(if blobs.is_empty() {
                    Step::Missing
                } else {
                    Step::Copy(blobs)
                })
            }
        })
        .collect();
    let observer = ctx.observer();
    let copies = entries
        .iter()
        .zip(&steps)
        .filter(|(_, step)| matches!(step, Ok(Step::Copy(_))));
    observer.planned(&OperationPlan {
        files_total: (entries.len() + files.len()) as u64,
        bytes_total: entries.iter().map(|e| e.metadata.size).sum(),
        files_to_transfer: copies.clone().count() as u64,
        bytes_to_transfer: copies.map(|(e, _)| e.metadata.size).sum(),
    });
    for row in &files {
        observer.file_finished(FileRow::Migrate(row));
    }

    if !dry_run {
        if setup.init {
            setup.backend.init(Compression::Zstd)?;
        }
        if let Some(local) = &setup.modes {
            local.apply_modes()?;
        }
    }

    // 3. Copy
    let store_compression = setup.compression.unwrap_or_default();
    let done: Vec<Done> = pool.install(|| {
        entries
            .par_iter()
            .zip(steps.into_par_iter())
            .map(|(entry, step)| {
                let path = entry.path.clone();
                let size = entry.metadata.size;
                let success = |outcome, duration_ms| MigrateRow {
                    path: path.clone(),
                    detail: MigrateDetail::Success {
                        outcome,
                        hash: entry.metadata.hashes.blake3.clone(),
                        size,
                    },
                    duration_ms,
                };
                let blobs = match step {
                    Err(e) => {
                        return (error_row(path, &e, None), Some((size, false, false)), false);
                    }
                    Ok(Step::Present) => {
                        return (
                            success(Outcome::Present, None),
                            Some((size, true, false)),
                            true,
                        );
                    }
                    Ok(Step::Missing) => {
                        let row = MigrateRow {
                            detail: MigrateDetail::Missing {
                                code: GetCode::SourceMissing,
                                hash: entry.metadata.hashes.blake3.clone(),
                                size,
                                message: "in neither the target nor the old storage".to_string(),
                            },
                            path,
                            duration_ms: None,
                        };
                        return (row, None, false);
                    }
                    Ok(Step::Copy(blobs)) => blobs,
                };
                if dry_run {
                    return (
                        success(Outcome::Copied, None),
                        Some((size, false, false)),
                        true,
                    );
                }
                if ctx.is_cancelled() {
                    let row = MigrateRow {
                        path,
                        detail: MigrateDetail::Cancelled {
                            message: "cancelled before this file was copied".to_string(),
                        },
                        duration_ms: None,
                    };
                    return (row, Some((size, false, false)), false);
                }
                let start = Instant::now();
                let progress = observer.transfer_started(&entry.path, size);
                let stored = copy(
                    entry,
                    &blobs,
                    &setup.backend,
                    store_compression,
                    operation_id,
                    &message,
                    progress.as_ref().map(|p| &*p.on_bytes),
                );
                if let Some(p) = &progress {
                    (p.on_done)(stored.is_ok());
                }
                let duration_ms = Some(millis(start.elapsed()));
                match stored {
                    Ok(stored) => (
                        success(Outcome::Copied, duration_ms),
                        Some((size, false, stored.created)),
                        true,
                    ),
                    Err(e) => (
                        error_row(path, &e, duration_ms),
                        Some((size, false, false)),
                        false,
                    ),
                }
            })
            .inspect(|(row, _, _)| observer.file_finished(FileRow::Migrate(row)))
            .collect()
    });

    // 4. The working tree. dvs.toml comes after the metadata so that a run
    // stopped before it is re-run as the same migration, and the dvs1 files go
    // last so that one stopped before that is finished by a bare migrate
    let mut changes = Vec::new();
    let written = |path: PathBuf| Change {
        path,
        change: ChangeKind::Written,
    };
    let mut sidecars = Vec::new();
    for ((entry, (_, _, target_has)), in_scope) in entries.iter().zip(&done).zip(&in_scope) {
        if let Some(sidecar) = &entry.sidecar {
            sidecars.push(sidecar.clone());
        }
        if !in_scope {
            continue;
        }
        let metadata_path = paths.metadata_path(&entry.path);
        let mut metadata = entry.metadata.clone();
        if *target_has {
            match setup.compression {
                Some(compression) => metadata.compression = compression,
                // Only a dry run to a server does not know what it stores with
                None => {
                    changes.push(written(relative(&root, &metadata_path)));
                    continue;
                }
            }
        }
        let content = serde_json::to_string_pretty(&metadata)?;
        if fs::read_to_string(&metadata_path).ok().as_deref() != Some(content.as_str()) {
            changes.push(written(relative(&root, &metadata_path)));
            if !dry_run {
                metadata.write(&paths, &entry.path)?;
            }
        }
    }
    let listed = migrated_from(&paths)?;
    if setup.sources.iter().any(|s| !listed.contains(s)) {
        changes.push(written(Path::new(&folder).join(MIGRATED_FROM_FILE_NAME)));
        if !dry_run {
            for source in &setup.sources {
                record_migrated_from(&paths, source)?;
            }
        }
    }
    if let Some(config) = &setup.config {
        changes.push(written(PathBuf::from(CONFIG_FILE_NAME)));
        if !dry_run {
            config.save(&root)?;
        }
    }
    let gitignores = remove_dvs1_exceptions(&root, &sidecars, dry_run)?;
    changes.extend(gitignores.into_iter().map(written));
    for sidecar in &sidecars {
        changes.push(Change {
            path: sidecar.clone(),
            change: ChangeKind::Removed,
        });
        if !dry_run {
            fs::remove_file(root.join(sidecar))?;
        }
    }
    if dvs1.is_some() && !keep_dvs_yaml {
        changes.push(Change {
            path: PathBuf::from(dvs1::CONFIG_FILE_NAME),
            change: ChangeKind::Removed,
        });
        if !dry_run {
            fs::remove_file(root.join(dvs1::CONFIG_FILE_NAME))?;
        }
    }
    changes.sort_by(|a, b| a.path.cmp(&b.path));

    let mut bytes = ByteTotals::default();
    for (row, row_bytes, _) in done {
        if let Some((size, stored, transferred)) = row_bytes {
            bytes.count(size, stored, transferred);
        }
        files.push(row);
    }
    files.sort_by(|a, b| a.path.cmp(&b.path));
    let counts = Counts::of(&files);
    Ok(MigrateReport {
        dry_run,
        sources: setup.sources,
        project: setup.project,
        outcome: RunOutcome::of(&counts, ctx.is_cancelled()),
        files,
        counts,
        bytes,
        changes,
        warnings: setup.warnings,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testutil::{
        Recorder, create_file, create_temp_git_repo, init_dvs_repo, init_dvs1_repo,
    };
    use crate::{CancelToken, RetrieveRequest};
    use std::sync::Arc;

    fn run(root: &Path, target: Target) -> MigrateReport {
        migrate(root, target, false, &OperationContext::new()).unwrap()
    }

    fn to_path(path: &Path) -> Target {
        Target::Local {
            path: Some(path.to_path_buf()),
            apply_local_modes: false,
        }
    }

    const IN_PLACE: Target = Target::Local {
        path: None,
        apply_local_modes: false,
    };

    /// Every file under `dir` with its content and mode.
    fn snapshot(dir: &Path) -> Vec<(PathBuf, Vec<u8>, u32)> {
        #[cfg(unix)]
        let mode = |p: &Path| {
            use std::os::unix::fs::PermissionsExt;
            fs::metadata(p).unwrap().permissions().mode()
        };
        #[cfg(not(unix))]
        let mode = |_: &Path| 0;
        walkdir::WalkDir::new(dir)
            .sort_by_file_name()
            .into_iter()
            .map(|e| e.unwrap())
            .map(|e| {
                let content = if e.file_type().is_file() {
                    fs::read(e.path()).unwrap()
                } else {
                    Vec::new()
                };
                (e.path().to_path_buf(), content, mode(e.path()))
            })
            .collect()
    }

    /// What `get` would restore for `rel` from the checkout's target.
    fn fetched(root: &Path, rel: &str) -> Vec<u8> {
        let config = Config::find(root).unwrap().unwrap();
        let folder = config.metadata_folder_name();
        let paths = DvsPaths::new(root.to_path_buf(), root.to_path_buf(), folder).unwrap();
        let metadata = FileMetadata::read(&paths, Path::new(rel)).unwrap();
        let target = root.parent().unwrap().join("fetched");
        let retrieved = config
            .backend()
            .retrieve(RetrieveRequest::new_local(
                &metadata.hashes,
                &target,
                ProjectPath::from_canonical(rel).unwrap(),
                metadata.compression,
            ))
            .unwrap();
        assert!(retrieved, "{rel} is not in the target");
        fs::read(&target).unwrap()
    }

    fn metadata(root: &Path, rel: &str) -> FileMetadata {
        let path = root.join(".dvs").join(format!("{rel}.dvs"));
        serde_json::from_str(&fs::read_to_string(path).unwrap()).unwrap()
    }

    /// A dvs2 local checkout whose `x.csv` is stored zstd in its folder, set up
    /// as migrated from there to a new raw folder.
    fn zstd_migrated_to_raw(root: &Path, content: &[u8]) -> (FileMetadata, DvsPaths, Config) {
        let (old, _) = init_dvs_repo(root);
        let file = create_file(root, "x.csv", content);
        let mut stored = FileMetadata::from_file(&file, Compression::Zstd, None).unwrap();
        let paths = DvsPaths::new(root.to_path_buf(), root.to_path_buf(), ".dvs").unwrap();
        stored
            .save(Uuid::nil(), &file, old.backend(), &paths, "x.csv", None)
            .unwrap();
        let Backend::Local(old_local) = old.backend() else {
            unreachable!()
        };
        record_migrated_from(&paths, &old_local.path).unwrap();
        let mut config = Config::new_local(root.parent().unwrap().join("raw"), None).unwrap();
        config.set_compression(Some(Compression::None));
        config.save(root).unwrap();
        (stored, paths, config)
    }

    fn outcomes(report: &MigrateReport) -> Vec<(String, &'static str)> {
        let outcome = |row: &MigrateRow| match &row.detail {
            MigrateDetail::Success {
                outcome: Outcome::Copied,
                ..
            } => "copied",
            MigrateDetail::Success { .. } => "present",
            MigrateDetail::Missing { .. } => "missing",
            MigrateDetail::Error { .. } => "error",
            MigrateDetail::Cancelled { .. } => "cancelled",
        };
        let rows = report.files.iter();
        rows.map(|r| (r.path.display().to_string(), outcome(r)))
            .collect()
    }

    #[test]
    fn in_place_converts_the_checkout_and_leaves_the_dvs1_folder_alone() {
        let (_tmp, root) = create_temp_git_repo();
        let storage = init_dvs1_repo(&root, &[("x.csv", b"x"), ("sub/y.csv", b"y")]);
        create_file(&root, ".gitignore", b"/x.csv\n!/x.csv.dvs\n");
        // Left by a get or status at the dvs1 checkout
        create_file(&root, ".dvs/.gitignore", b"/.cache\n");
        create_file(&root, ".dvs/.cache/hashes.db", b"db");
        let before = snapshot(&storage);

        let report = run(&root, IN_PLACE);

        assert_eq!(snapshot(&storage), before);
        assert_eq!(
            outcomes(&report),
            [("sub/y.csv".into(), "present"), ("x.csv".into(), "present")]
        );
        let toml = fs::read_to_string(root.join(CONFIG_FILE_NAME)).unwrap();
        assert!(!toml.contains("group"), "{toml}");
        assert!(toml.contains("compression = \"none\""), "{toml}");
        assert_eq!(fetched(&root, "sub/y.csv"), b"y");
        let converted = metadata(&root, "x.csv");
        assert_eq!(converted.created_by, "tester");
        assert_eq!(converted.compression, Compression::None);
        assert!(!root.join("x.csv.dvs").exists());
        assert!(!root.join(dvs1::CONFIG_FILE_NAME).exists());
        let gitignore = fs::read_to_string(root.join(".gitignore")).unwrap();
        assert_eq!(gitignore, "/x.csv\n");
        let paths = DvsPaths::new(root.clone(), root.clone(), ".dvs").unwrap();
        assert_eq!(migrated_from(&paths).unwrap(), [storage]);
        assert!(report.warnings.is_empty());
    }

    #[test]
    fn in_place_warns_that_dvs1_permissions_are_dropped() {
        let (_tmp, root) = create_temp_git_repo();
        let storage = init_dvs1_repo(&root, &[]);
        let yaml = format!("storage_dir: {}\npermissions: 664\n", storage.display());
        create_file(&root, dvs1::CONFIG_FILE_NAME, yaml.as_bytes());

        let report = run(&root, IN_PLACE);

        assert_eq!(report.warnings.len(), 1);
        assert!(report.warnings[0].contains("permissions = 664"));
    }

    #[cfg(unix)]
    #[test]
    fn applying_local_modes_gives_the_dvs1_folder_dvs2_modes() {
        use std::os::unix::fs::PermissionsExt;
        let (_tmp, root) = create_temp_git_repo();
        let content: &[u8] = b"x";
        let storage = init_dvs1_repo(&root, &[("x.csv", content)]);
        let hash = blake3::hash(content).to_string();

        let target = Target::Local {
            path: None,
            apply_local_modes: true,
        };
        run(&root, target);

        let mode = |p: PathBuf| fs::metadata(p).unwrap().permissions().mode() & 0o7777;
        assert_eq!(mode(storage.clone()), 0o2770);
        assert_eq!(mode(storage.join(&hash[..2])), 0o2770);
        assert_eq!(mode(storage.join(&hash[..2]).join(&hash[2..])), 0o440);
        let toml = fs::read_to_string(root.join(CONFIG_FILE_NAME)).unwrap();
        assert!(toml.contains("group"), "{toml}");
    }

    #[test]
    fn a_path_gets_zstd_copies_and_the_dvs1_folder_is_only_read() {
        let (tmp, root) = create_temp_git_repo();
        let storage = init_dvs1_repo(&root, &[("x.csv", b"x content")]);
        let before = snapshot(&storage);
        let new_storage = tmp.path().join("dvs2-storage");

        let report = run(&root, to_path(&new_storage));

        assert_eq!(snapshot(&storage), before);
        assert_eq!(outcomes(&report), [("x.csv".into(), "copied")]);
        assert_eq!(report.bytes.transferred, 9);
        assert_eq!(metadata(&root, "x.csv").compression, Compression::Zstd);
        assert_eq!(fetched(&root, "x.csv"), b"x content");
    }

    #[test]
    fn a_path_on_or_in_the_dvs1_folder_is_refused() {
        let (_tmp, root) = create_temp_git_repo();
        let storage = init_dvs1_repo(&root, &[]);

        for path in [storage.clone(), storage.join("dvs2")] {
            let error = migrate(&root, to_path(&path), false, &OperationContext::new());

            assert_eq!(
                backend_error_code(&error.unwrap_err()),
                Some(ErrorCode::InvalidArguments)
            );
            assert!(root.join(dvs1::CONFIG_FILE_NAME).exists());
            assert!(!path.join("dvs2").exists());
        }
    }

    #[cfg(unix)]
    #[test]
    fn a_new_path_is_compared_through_symlinks_and_dot_dot() {
        let (tmp, root) = create_temp_git_repo();
        let storage = init_dvs1_repo(&root, &[("x.csv", b"x")]);
        let alias = tmp.path().join("alias");
        std::os::unix::fs::symlink(&storage, &alias).unwrap();

        let error = migrate(
            &root,
            to_path(&alias.join("new")),
            false,
            &OperationContext::new(),
        );
        assert_eq!(
            backend_error_code(&error.unwrap_err()),
            Some(ErrorCode::InvalidArguments)
        );
        assert!(!storage.join("new").exists());

        let report = run(&root, to_path(&root.join("..").join("dvs2-copy")));
        assert_eq!(outcomes(&report), [("x.csv".into(), "copied")]);
        assert!(tmp.path().join("dvs2-copy").exists());
    }

    #[test]
    fn server_sources_keep_order_and_drop_duplicates() {
        let p = PathBuf::from;
        assert_eq!(
            server_sources(p("a"), Some(p("b")), vec![p("a"), p("c"), p("b")]),
            [p("a"), p("b"), p("c")]
        );
        assert_eq!(server_sources(p("a"), Some(p("a")), vec![]), [p("a")]);
    }

    #[cfg(unix)]
    #[test]
    fn a_path_keeps_the_dvs1_group_when_it_resolves() {
        let id = |flags| {
            let out = std::process::Command::new("id")
                .arg(flags)
                .output()
                .unwrap();
            String::from_utf8(out.stdout).unwrap()
        };
        let primary = id("-gn").trim().to_string();
        let groups = id("-Gn");
        let other = groups.split_whitespace().find(|g| *g != primary);
        let other = other.unwrap_or(&primary).to_string();

        for (group, expected) in [
            (other.clone(), other),
            ("no-such-dvs-group".into(), primary),
        ] {
            let (tmp, root) = create_temp_git_repo();
            let storage = init_dvs1_repo(&root, &[("x.csv", b"x")]);
            let yaml = format!("storage_dir: {}\ngroup: {group}\n", storage.display());
            create_file(&root, dvs1::CONFIG_FILE_NAME, yaml.as_bytes());

            run(&root, to_path(&tmp.path().join("dvs2-storage")));

            let toml = fs::read_to_string(root.join(CONFIG_FILE_NAME)).unwrap();
            assert!(toml.contains(&format!("\"{expected}\"")), "{toml}");
        }
    }

    #[test]
    fn a_blob_missing_from_old_storage_is_a_row_and_its_metadata_is_converted() {
        let (tmp, root) = create_temp_git_repo();
        let content: &[u8] = b"gone";
        let storage = init_dvs1_repo(&root, &[("x.csv", content)]);
        let hash = blake3::hash(content).to_string();
        fs::remove_file(storage.join(&hash[..2]).join(&hash[2..])).unwrap();

        let report = run(&root, to_path(&tmp.path().join("dvs2-storage")));

        assert_eq!(outcomes(&report), [("x.csv".into(), "missing")]);
        let json = serde_json::to_value(&report.files[0]).unwrap();
        assert_eq!(json["code"], "source_missing");
        assert_eq!(report.outcome, RunOutcome::Complete);
        assert_eq!(metadata(&root, "x.csv").hashes.blake3, hash);
        assert!(!root.join("x.csv.dvs").exists());
    }

    #[test]
    fn a_dry_run_plans_the_same_and_changes_nothing() {
        let (tmp, root) = create_temp_git_repo();
        let storage = init_dvs1_repo(&root, &[("x.csv", b"xx"), ("y.csv", b"yyy")]);
        let hash = blake3::hash(b"yyy").to_string();
        fs::remove_file(storage.join(&hash[..2]).join(&hash[2..])).unwrap();
        create_file(&root, ".gitignore", b"/x.csv\n!/x.csv.dvs\n");
        let new_storage = tmp.path().join("dvs2-storage");
        let target = to_path(&new_storage);
        let before = snapshot(tmp.path());

        let dry = migrate(&root, target.clone(), true, &OperationContext::new()).unwrap();

        assert_eq!(snapshot(tmp.path()), before);
        assert_eq!(
            outcomes(&dry),
            [("x.csv".into(), "copied"), ("y.csv".into(), "missing")]
        );
        assert_eq!((dry.counts.total, dry.already_stored()), (2, 0));
        assert_eq!((dry.bytes.to_transfer, dry.bytes.transferred), (2, 0));
        let written = |p: &str| Change {
            path: p.into(),
            change: ChangeKind::Written,
        };
        let removed = |p: &str| Change {
            path: p.into(),
            change: ChangeKind::Removed,
        };
        assert_eq!(
            dry.changes,
            [
                written(".dvs/migrated-from"),
                written(".dvs/x.csv.dvs"),
                written(".dvs/y.csv.dvs"),
                written(".gitignore"),
                written("dvs.toml"),
                removed("dvs.yaml"),
                removed("x.csv.dvs"),
                removed("y.csv.dvs"),
            ]
        );
        assert_eq!(run(&root, target).changes, dry.changes);
    }

    #[test]
    fn a_rerun_after_a_merge_copies_what_the_target_lacks_and_converts_dvs1_files() {
        let (tmp, root) = create_temp_git_repo();
        let storage = init_dvs1_repo(&root, &[("x.csv", b"x")]);
        let new_storage = tmp.path().join("dvs2-storage");
        run(&root, to_path(&new_storage));
        // An unmigrated branch merged in brings dvs.yaml and its dvs1 files back
        init_dvs1_repo(&root, &[("x.csv", b"x"), ("z.csv", b"z")]);
        let before = snapshot(&storage);

        let report = run(&root, Target::Configured);

        assert_eq!(snapshot(&storage), before);
        assert_eq!(
            outcomes(&report),
            [("x.csv".into(), "present"), ("z.csv".into(), "copied")]
        );
        assert_eq!(fetched(&root, "z.csv"), b"z");
        assert_eq!(metadata(&root, "z.csv").compression, Compression::Zstd);
        assert!(!root.join("z.csv.dvs").exists());
        assert!(!root.join(dvs1::CONFIG_FILE_NAME).exists());
    }

    #[test]
    fn a_rerun_after_a_clean_merge_converts_dvs1_files_without_dvs_yaml() {
        let (tmp, root) = create_temp_git_repo();
        let storage = init_dvs1_repo(&root, &[("x.csv", b"x")]);
        run(&root, to_path(&tmp.path().join("dvs2-storage")));
        // A dvs1 branch that never changed dvs.yaml merges cleanly: only its
        // new dvs1 file comes in
        init_dvs1_repo(&root, &[("z.csv", b"z")]);
        fs::remove_file(root.join(dvs1::CONFIG_FILE_NAME)).unwrap();
        // Not dvs1's: a dvs2 data file that reads as dvs1 metadata, and other text
        let config = Config::find(&root).unwrap().unwrap();
        let paths = DvsPaths::new(root.clone(), root.clone(), ".dvs").unwrap();
        let notes = create_file(
            &root,
            "notes.dvs",
            &fs::read(root.join("z.csv.dvs")).unwrap(),
        );
        FileMetadata::from_file(&notes, Compression::Zstd, None)
            .unwrap()
            .save(
                Uuid::nil(),
                &notes,
                config.backend(),
                &paths,
                "notes.dvs",
                None,
            )
            .unwrap();
        create_file(&root, "readme.dvs", b"not dvs1 metadata");
        let before = snapshot(&storage);

        let report = run(&root, Target::Configured);

        assert_eq!(snapshot(&storage), before);
        assert_eq!(
            outcomes(&report),
            [
                ("notes.dvs".into(), "present"),
                ("x.csv".into(), "present"),
                ("z.csv".into(), "copied")
            ]
        );
        assert_eq!(fetched(&root, "z.csv"), b"z");
        assert!(!root.join("z.csv.dvs").exists());
        assert!(root.join("notes.dvs").exists() && root.join("readme.dvs").exists());
    }

    #[test]
    fn a_rerun_copies_a_raw_dvs1_blob_whose_metadata_says_zstd() {
        let (tmp, root) = create_temp_git_repo();
        init_dvs1_repo(&root, &[("x.csv", b"x content")]);
        let new_storage = tmp.path().join("dvs2-storage");
        run(&root, to_path(&new_storage));
        let hashes = metadata(&root, "x.csv").hashes;
        let blob = LocalBackend::without_group(&new_storage).hash_to_path(&hashes);
        fs::remove_file(blob.unwrap()).unwrap();

        let report = run(&root, Target::Configured);

        assert_eq!(outcomes(&report), [("x.csv".into(), "copied")]);
        assert_eq!(metadata(&root, "x.csv").compression, Compression::Zstd);
        assert_eq!(fetched(&root, "x.csv"), b"x content");
    }

    #[test]
    fn a_rerun_decodes_each_blob_with_the_compression_its_source_used() {
        let (_tmp, root) = create_temp_git_repo();
        zstd_migrated_to_raw(&root, b"zstd in the old folder");

        let report = run(&root, Target::Configured);

        assert_eq!(outcomes(&report), [("x.csv".into(), "copied")]);
        assert_eq!(metadata(&root, "x.csv").compression, Compression::None);
        assert_eq!(fetched(&root, "x.csv"), b"zstd in the old folder");
    }

    #[test]
    fn a_rerun_rewrites_merged_metadata_whose_compression_the_target_does_not_use() {
        let (_tmp, root) = create_temp_git_repo();
        let (stored, paths, _) = zstd_migrated_to_raw(&root, b"zstd in the old folder");
        run(&root, Target::Configured);
        // A merge brings back the old metadata for content the target has
        stored.write(&paths, "x.csv").unwrap();

        let report = run(&root, Target::Configured);

        assert_eq!(outcomes(&report), [("x.csv".into(), "present")]);
        assert_eq!(metadata(&root, "x.csv").compression, Compression::None);
        assert_eq!(fetched(&root, "x.csv"), b"zstd in the old folder");
    }

    #[test]
    fn a_blob_that_does_not_decode_to_its_hash_is_not_stored() {
        let (tmp, root) = create_temp_git_repo();
        let content: &[u8] = b"not what the old folder holds";
        let (stored, paths, config) = zstd_migrated_to_raw(&root, content);
        let old = migrated_from(&paths).unwrap();
        let blob = LocalBackend::without_group(&old[0]).hash_to_path(&stored.hashes);
        let blob = blob.unwrap();
        fs::remove_file(&blob).unwrap();
        fs::write(&blob, b"garbage").unwrap();

        let report = run(&root, Target::Configured);

        assert_eq!(outcomes(&report), [("x.csv".into(), "error")]);
        let json = serde_json::to_value(&report.files[0]).unwrap();
        assert_eq!(json["code"], "bad_metadata");
        assert!(!config.backend().exists(&stored.hashes).unwrap());
        assert_eq!(metadata(&root, "x.csv"), stored);

        // A later source with the right bytes is used instead
        let hash = &stored.hashes.blake3;
        let later = tmp.path().join("later");
        create_file(&later, &format!("{}/{}", &hash[..2], &hash[2..]), content);
        record_migrated_from(&paths, &later).unwrap();
        let report = run(&root, Target::Configured);
        assert_eq!(outcomes(&report), [("x.csv".into(), "copied")]);
        assert_eq!(fetched(&root, "x.csv"), content);
    }

    #[test]
    fn a_rerun_with_no_sources_has_nothing_to_do() {
        let (_tmp, root) = create_temp_git_repo();
        init_dvs_repo(&root);

        let report = run(&root, Target::Configured);

        assert!(report.files.is_empty() && report.changes.is_empty());
    }

    #[test]
    fn a_stopped_migration_is_finished_by_a_rerun() {
        let (tmp, root) = create_temp_git_repo();
        // More files than the pool can have in flight, so some never start
        let names: Vec<String> = (0..48).map(|i| format!("f{i:02}.csv")).collect();
        let files: Vec<(&str, &[u8])> = names.iter().map(|n| (&**n, n.as_bytes())).collect();
        init_dvs1_repo(&root, &files);
        let cancel = CancelToken::new();
        let recorder = Arc::new(Recorder::cancelling_on_transfer(cancel.clone(), 1));
        let ctx = OperationContext::new()
            .with_cancel(cancel)
            .with_observer(recorder);
        let target = to_path(&tmp.path().join("dvs2-storage"));

        let stopped = migrate(&root, target, false, &ctx).unwrap();
        assert_eq!(stopped.outcome, RunOutcome::Cancelled);
        assert!(stopped.counts.cancelled > 0);
        run(&root, Target::Configured);

        for (name, content) in files {
            assert_eq!(fetched(&root, name), content);
        }
    }

    #[cfg(unix)]
    #[test]
    fn the_gitignore_is_fixed_before_the_dvs1_files_go() {
        use std::os::unix::fs::PermissionsExt;
        let (_tmp, root) = create_temp_git_repo();
        init_dvs1_repo(&root, &[("sub/y.csv", b"y")]);
        create_file(&root, "sub/.gitignore", b"/y.csv\n!/y.csv.dvs\n");
        let sub = root.join("sub");
        let set_mode =
            |mode| fs::set_permissions(&sub, std::fs::Permissions::from_mode(mode)).unwrap();
        // The dvs1 file cannot be removed, as if the run stopped there
        set_mode(0o555);

        let stopped = migrate(&root, IN_PLACE, false, &OperationContext::new());
        set_mode(0o755);

        assert!(stopped.is_err());
        assert!(root.join("sub/y.csv.dvs").exists());
        assert_eq!(
            fs::read_to_string(sub.join(".gitignore")).unwrap(),
            "/y.csv\n"
        );
        run(&root, Target::Configured);
        assert!(!root.join("sub/y.csv.dvs").exists());
        assert!(!root.join(dvs1::CONFIG_FILE_NAME).exists());
        assert_eq!(fetched(&root, "sub/y.csv"), b"y");
    }

    #[test]
    fn differing_dvs1_and_dvs2_metadata_for_a_file_is_a_row_and_both_are_kept() {
        let (_tmp, root) = create_temp_git_repo();
        init_dvs_repo(&root);
        let file = create_file(&root, "x.csv", b"dvs2");
        let paths = DvsPaths::new(root.clone(), root.clone(), ".dvs").unwrap();
        let ours = FileMetadata::from_file(&file, Compression::Zstd, None).unwrap();
        ours.write(&paths, "x.csv").unwrap();
        init_dvs1_repo(&root, &[("x.csv", b"dvs1")]);

        let report = run(&root, Target::Configured);

        assert_eq!(outcomes(&report), [("x.csv".into(), "error")]);
        assert!(root.join("x.csv.dvs").exists());
        assert!(root.join(dvs1::CONFIG_FILE_NAME).exists());
        assert_eq!(metadata(&root, "x.csv").hashes, ours.hashes);
    }

    #[test]
    fn each_form_refuses_a_checkout_it_does_not_convert() {
        let ctx = OperationContext::new();
        let (_tmp, dvs2) = create_temp_git_repo();
        init_dvs_repo(&dvs2);
        let (_tmp1, dvs1) = create_temp_git_repo();
        init_dvs1_repo(&dvs1, &[]);
        let (_tmp2, server) = create_temp_git_repo();
        let url = Url::parse("https://dvs.example.com").unwrap();
        Config::new_server("p".into(), url.clone(), None)
            .save(&server)
            .unwrap();
        let (_tmp3, neither) = create_temp_git_repo();
        let to_server = Target::Server {
            name: "p".into(),
            url,
            group: None,
        };

        for (root, target, code) in [
            (&dvs2, IN_PLACE, ErrorCode::InvalidConfiguration),
            (&dvs1, Target::Configured, ErrorCode::InvalidConfiguration),
            (&server, to_server, ErrorCode::InvalidConfiguration),
            (&neither, IN_PLACE, ErrorCode::NotInProject),
        ] {
            let error = migrate(root, target, false, &ctx).unwrap_err();
            assert_eq!(backend_error_code(&error), Some(code), "{}", root.display());
        }
    }

    #[test]
    fn check_refuses_a_server_checkout_without_the_server() {
        // Nothing listens here, so any server access would fail differently
        let url = Url::parse("http://127.0.0.1:9").unwrap();
        let to_server = Target::Server {
            name: "p".into(),
            url: url.clone(),
            group: None,
        };
        let (_tmp, server) = create_temp_git_repo();
        Config::new_server("p".into(), url, None)
            .save(&server)
            .unwrap();
        let (_tmp1, dvs2) = create_temp_git_repo();
        init_dvs_repo(&dvs2);

        let error = check(&server, &to_server).unwrap_err();
        assert_eq!(
            backend_error_code(&error),
            Some(ErrorCode::InvalidConfiguration)
        );
        check(&dvs2, &to_server).unwrap();
    }

    #[test]
    fn each_row_is_reported_after_the_plan() {
        let (tmp, root) = create_temp_git_repo();
        init_dvs1_repo(&root, &[("x.csv", b"x"), ("y.csv", b"y")]);
        // Unreadable, so a row before any copy
        create_file(&root, ".dvs/z.csv.dvs", b"not json");
        let config = Config::new_local(tmp.path().join("new"), None).unwrap();
        config.save(&root).unwrap();
        let recorder = Arc::new(Recorder::default());
        let ctx = OperationContext::new().with_observer(recorder.clone());

        let report = migrate(&root, Target::Configured, true, &ctx).unwrap();

        let seen = recorder.seen();
        assert!(seen[0].starts_with("plan 3 "), "{seen:?}");
        let mut rows = seen[1..].to_vec();
        rows.sort();
        assert_eq!(rows, ["row x.csv", "row y.csv", "row z.csv"]);
        assert_eq!(report.files.len(), 3);
    }

    #[test]
    fn record_appends_each_source_once() {
        let (_tmp, root) = create_temp_git_repo();
        let (config, dvs_dir) = init_dvs_repo(&root);
        let paths = DvsPaths::new(root.clone(), root, config.metadata_folder_name()).unwrap();
        assert!(migrated_from(&paths).unwrap().is_empty());
        create_file(&dvs_dir, MIGRATED_FROM_FILE_NAME, b"/data/old");

        record_migrated_from(&paths, Path::new("/data/new")).unwrap();
        record_migrated_from(&paths, Path::new("/data/old")).unwrap();
        record_migrated_from(&paths, Path::new("/data/new")).unwrap();

        assert_eq!(
            fs::read_to_string(dvs_dir.join(MIGRATED_FROM_FILE_NAME)).unwrap(),
            "/data/old\n/data/new\n"
        );
        assert_eq!(
            migrated_from(&paths).unwrap(),
            vec![PathBuf::from("/data/old"), PathBuf::from("/data/new")]
        );
    }
}
