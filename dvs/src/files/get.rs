//! `get`: restore tracked files from storage without losing local work.
//!
//! Every requested file is classified first, locally and without writing
//! anything (the plan). A working file that differs from its target is only
//! replaced without asking when it equals the baseline, the bytes dvs itself
//! last wrote or added there; anything else is a conflict, handled by the
//! `on_conflict` policy. Remote and runtime problems are per file and never stop
//! the rest of the batch. See work/plans/2026-10-17-get-spec.md in the workspace.

use std::io;
use std::path::{Component, Path, PathBuf};
use std::sync::Mutex;

use crate::backends::server::Unreachable;
use crate::cache::{FileStat, HashCache, baseline_for, record_baseline, try_open_cache};
use crate::config::{OnConflict, SettingSource};
use crate::files::metadata::FileMetadata;
use crate::globbing::GetSelection;
use crate::paths::ProjectPath;
use crate::progress::OnFileStart;
use crate::utils::get_threadpool;
use crate::{Backend, Compression, DvsPaths, RetrieveRequest, cache};
use fs_err as fs;
use rayon::prelude::*;
use serde::Serialize;
use uuid::Uuid;

/// The state of a requested file before `get` touches it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Before {
    /// No working file
    Absent,
    /// The working file is the target
    Current,
    /// Differs from the target but is exactly what dvs last wrote or added here
    Clean,
    /// Differs from both the target and what dvs last wrote or added here
    Modified,
    /// Differs from the target, and dvs has no record of writing it
    Unknown,
    /// No `.dvs` file for the requested path
    NotTracked,
    /// The destination is a directory, a symlink or another non-regular file
    NotAFile,
    /// The destination resolves outside the project
    UnsafePath,
}

impl Before {
    pub fn is_conflict(self) -> bool {
        matches!(self, Before::Modified | Before::Unknown)
    }

    pub fn as_str(&self) -> &'static str {
        match self {
            Before::Absent => "absent",
            Before::Current => "current",
            Before::Clean => "clean",
            Before::Modified => "modified",
            Before::Unknown => "unknown",
            Before::NotTracked => "not_tracked",
            Before::NotAFile => "not_a_file",
            Before::UnsafePath => "unsafe_path",
        }
    }
}

/// What `get` did (or, in a dry run, would do) with a requested file.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Action {
    Restored,
    Replaced,
    Unchanged,
    Skipped,
    NotRun,
    Failed,
    WouldRestore,
    WouldReplace,
}

impl Action {
    pub fn as_str(&self) -> &'static str {
        match self {
            Action::Restored => "restored",
            Action::Replaced => "replaced",
            Action::Unchanged => "unchanged",
            Action::Skipped => "skipped",
            Action::NotRun => "not_run",
            Action::Failed => "failed",
            Action::WouldRestore => "would_restore",
            Action::WouldReplace => "would_replace",
        }
    }
}

/// Whether the configured storage appears to hold the working file's bytes.
/// Existence only: it does not prove the stored copy is intact.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum StoredCopy {
    Present,
    Absent,
    Unknown,
}

impl StoredCopy {
    pub fn as_str(&self) -> &'static str {
        match self {
            StoredCopy::Present => "present",
            StoredCopy::Absent => "absent",
            StoredCopy::Unknown => "unknown",
        }
    }
}

/// Why a file did not end up at its target.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum GetCode {
    LocalChanges,
    UnknownLocal,
    BatchAborted,
    NotTracked,
    NotAFile,
    UnsafePath,
    SourceMissing,
    HashMismatch,
    AccessDenied,
    ServerUnavailable,
    DestinationChanged,
    PermissionDenied,
    FileInUse,
    IoError,
    BadMetadata,
}

impl GetCode {
    /// Conflicts, and files held back because of them: local, nothing broken.
    pub fn is_conflict(self) -> bool {
        matches!(
            self,
            GetCode::LocalChanges | GetCode::UnknownLocal | GetCode::BatchAborted
        )
    }

    pub fn as_str(&self) -> &'static str {
        match self {
            GetCode::LocalChanges => "local_changes",
            GetCode::UnknownLocal => "unknown_local",
            GetCode::BatchAborted => "batch_aborted",
            GetCode::NotTracked => "not_tracked",
            GetCode::NotAFile => "not_a_file",
            GetCode::UnsafePath => "unsafe_path",
            GetCode::SourceMissing => "source_missing",
            GetCode::HashMismatch => "hash_mismatch",
            GetCode::AccessDenied => "access_denied",
            GetCode::ServerUnavailable => "server_unavailable",
            GetCode::DestinationChanged => "destination_changed",
            GetCode::PermissionDenied => "permission_denied",
            GetCode::FileInUse => "file_in_use",
            GetCode::IoError => "io_error",
            GetCode::BadMetadata => "bad_metadata",
        }
    }
}

/// One requested file. Every field is always present (null when not
/// applicable) so JSON and R consumers get a fixed schema.
#[derive(Debug, Clone, Serialize)]
pub struct GetRow {
    /// Relative to the project root
    pub path: PathBuf,
    /// Relative to the current directory
    pub local_path: PathBuf,
    pub before: Before,
    pub action: Action,
    /// The file is (or after a dry run would be) at its target
    pub satisfied: bool,
    /// Size of the target in bytes
    pub size: Option<u64>,
    pub target_hash: Option<String>,
    pub local_hash: Option<String>,
    /// Only for conflicts
    pub stored_copy: Option<StoredCopy>,
    pub code: Option<GetCode>,
    pub message: Option<String>,
    /// The next command to run. Left empty by core: each frontend phrases it
    /// in its own syntax.
    pub hint: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum GetOutcome {
    /// Every requested file is at its target
    Complete,
    /// The policy held the batch back: no file was changed
    Aborted,
    /// Some files are at their target, some are not
    Partial,
    /// Nothing is at its target, or the operation could not run
    Failed,
}

impl GetOutcome {
    pub fn as_str(&self) -> &'static str {
        match self {
            GetOutcome::Complete => "complete",
            GetOutcome::Aborted => "aborted",
            GetOutcome::Partial => "partial",
            GetOutcome::Failed => "failed",
        }
    }
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct GetCounts {
    pub restored: usize,
    pub replaced: usize,
    pub unchanged: usize,
    pub skipped: usize,
    pub not_run: usize,
    pub failed: usize,
    pub would_restore: usize,
    pub would_replace: usize,
}

/// The whole result of a `get`: operation information plus one row per file.
#[derive(Debug, Clone, Serialize)]
pub struct GetReport {
    pub outcome: GetOutcome,
    pub dry_run: bool,
    pub on_conflict: OnConflict,
    pub source: SettingSource,
    pub counts: GetCounts,
    /// An operation-level problem, such as nothing matching the request
    pub problem: Option<String>,
    pub files: Vec<GetRow>,
}

impl GetReport {
    /// A report for a request that selected nothing or could not run at all.
    pub fn problem(
        problem: impl Into<String>,
        dry_run: bool,
        on_conflict: OnConflict,
        source: SettingSource,
    ) -> Self {
        GetReport {
            outcome: GetOutcome::Failed,
            dry_run,
            on_conflict,
            source,
            counts: GetCounts::default(),
            problem: Some(problem.into()),
            files: Vec::new(),
        }
    }

    /// Some file failed for a reason other than a local conflict, or the
    /// operation itself had a problem.
    pub fn has_failures(&self) -> bool {
        self.problem.is_some()
            || self
                .files
                .iter()
                .any(|r| !r.satisfied && r.code.is_some_and(|c| !c.is_conflict()))
    }

    /// Some file was held back or skipped because of a local conflict.
    pub fn has_conflicts(&self) -> bool {
        self.files
            .iter()
            .any(|r| !r.satisfied && r.code.is_some_and(GetCode::is_conflict))
    }
}

#[derive(Debug, Clone, Copy)]
pub struct GetOptions {
    pub dry_run: bool,
    pub on_conflict: OnConflict,
    pub source: SettingSource,
}

impl Default for GetOptions {
    fn default() -> Self {
        GetOptions {
            dry_run: false,
            on_conflict: OnConflict::Abort,
            source: SettingSource::Default,
        }
    }
}

/// A row being worked on, with what the plan learned about the working file.
struct Planned {
    row: GetRow,
    metadata: Option<FileMetadata>,
    /// The working file's stat when it was classified, to detect it changing
    /// while its replacement downloads. None when it was absent.
    stat: Option<FileStat>,
}

impl Planned {
    /// Classified as a conflict (rows that already failed in the plan are not).
    fn is_conflict(&self) -> bool {
        self.row.before.is_conflict() && self.row.action != Action::Failed
    }

    fn needs_transfer(&self, on_conflict: OnConflict) -> bool {
        self.row.action == Action::NotRun
            && (matches!(self.row.before, Before::Absent | Before::Clean)
                || (self.is_conflict() && on_conflict == OnConflict::Overwrite))
    }

    /// Detected before anything is written: under abort/overwrite, any of these
    /// holds the whole batch back. That includes every file the plan could not
    /// classify (unreadable or malformed metadata, e.g. a merge conflict in a
    /// `.dvs` file).
    fn is_known_problem(&self, on_conflict: OnConflict) -> bool {
        self.row.action == Action::Failed
            || (self.is_conflict() && on_conflict != OnConflict::Overwrite)
    }

    fn set(&mut self, action: Action, code: Option<GetCode>, message: Option<String>) {
        self.row.action = action;
        self.row.satisfied = matches!(
            action,
            Action::Restored
                | Action::Replaced
                | Action::Unchanged
                | Action::WouldRestore
                | Action::WouldReplace
        );
        self.row.code = code;
        self.row.message = message;
    }

    fn conflict_code(&self) -> GetCode {
        if self.row.before == Before::Modified {
            GetCode::LocalChanges
        } else {
            GetCode::UnknownLocal
        }
    }

    fn conflict_message(&self) -> String {
        if self.row.before == Before::Modified {
            "local changes since dvs last wrote this file".to_string()
        } else {
            "differs from the tracked version, and dvs has no record of writing it".to_string()
        }
    }
}

/// `target` lexically relative to `base`; both absolute.
fn relative_to(base: &Path, target: &Path) -> PathBuf {
    let base: Vec<_> = base.components().collect();
    let target: Vec<_> = target.components().collect();
    let common = base
        .iter()
        .zip(target.iter())
        .take_while(|(a, b)| a == b)
        .count();
    let mut out = PathBuf::new();
    for _ in common..base.len() {
        out.push("..");
    }
    for c in &target[common..] {
        out.push(c.as_os_str());
    }
    if out.as_os_str().is_empty() {
        out.push(".");
    }
    out
}

/// Resolve `.` and `..` without touching the filesystem.
fn normalize(path: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for c in path.components() {
        match c {
            Component::CurDir => {}
            Component::ParentDir => {
                if !out.pop() {
                    out.push("..");
                }
            }
            other => out.push(other.as_os_str()),
        }
    }
    out
}

/// True when the nearest existing ancestor of `target` resolves inside `root`,
/// so writing `target` cannot escape the project through a symlinked directory.
fn destination_is_contained(root: &Path, target: &Path) -> bool {
    let mut dir = target.parent();
    while let Some(d) = dir {
        match std::fs::symlink_metadata(d) {
            Ok(_) => {
                return std::fs::canonicalize(d)
                    .map(|real| real.starts_with(root))
                    .unwrap_or(false);
            }
            Err(_) => dir = d.parent(),
        }
    }
    false
}

fn blank_row(paths: &DvsPaths, relative_path: &Path) -> GetRow {
    GetRow {
        path: relative_path.to_path_buf(),
        local_path: relative_to(paths.cwd(), &paths.file_path(relative_path)),
        before: Before::Absent,
        action: Action::NotRun,
        satisfied: false,
        size: None,
        target_hash: None,
        local_hash: None,
        stored_copy: None,
        code: None,
        message: None,
        hint: None,
    }
}

/// Classify one tracked file. Reads only local state.
fn plan_file(paths: &DvsPaths, relative_path: &Path, cache: Option<&Mutex<HashCache>>) -> Planned {
    let mut planned = Planned {
        row: blank_row(paths, relative_path),
        metadata: None,
        stat: None,
    };
    let fail = |mut planned: Planned, before: Before, code: GetCode, message: String| {
        planned.row.before = before;
        planned.set(Action::Failed, Some(code), Some(message));
        planned
    };

    let metadata_path = paths.metadata_path(relative_path);
    let metadata: FileMetadata = match fs::File::open(&metadata_path)
        .map_err(anyhow::Error::from)
        .and_then(|f| serde_json::from_reader(f).map_err(anyhow::Error::from))
    {
        Ok(m) => m,
        Err(e) => {
            return fail(
                planned,
                Before::Unknown,
                GetCode::BadMetadata,
                format!("cannot read {}: {e}", metadata_path.display()),
            );
        }
    };
    planned.row.size = Some(metadata.size);
    planned.row.target_hash = Some(metadata.hashes.blake3.clone());

    let target = paths.file_path(relative_path);
    if !destination_is_contained(paths.repo_root(), &target) {
        return fail(
            planned,
            Before::UnsafePath,
            GetCode::UnsafePath,
            "a parent directory leads outside the project (symlink?); dvs will not write there"
                .to_string(),
        );
    }

    match std::fs::symlink_metadata(&target) {
        Err(e) if e.kind() == io::ErrorKind::NotFound => {
            planned.row.before = Before::Absent;
            planned.metadata = Some(metadata);
            return planned;
        }
        Err(e) => {
            return fail(planned, Before::Unknown, io_code(&e), e.to_string());
        }
        Ok(meta) if meta.file_type().is_symlink() => {
            return fail(
                planned,
                Before::NotAFile,
                GetCode::NotAFile,
                "is a symbolic link; dvs will not replace it or write through it".to_string(),
            );
        }
        Ok(meta) if !meta.is_file() => {
            let what = if meta.is_dir() {
                "is a directory"
            } else {
                "is not a regular file"
            };
            return fail(
                planned,
                Before::NotAFile,
                GetCode::NotAFile,
                format!("{what}; dvs will not replace it"),
            );
        }
        Ok(_) => {}
    }

    let rel_str = relative_path.to_string_lossy();
    let stat = FileStat::from_path(&target).ok();
    let (hashes, size) = match cache::hashes_for_file(&target, &rel_str, cache) {
        Ok(h) => h,
        Err(e) => {
            let code = e
                .downcast_ref::<io::Error>()
                .map(io_code)
                .unwrap_or(GetCode::IoError);
            return fail(
                planned,
                Before::Unknown,
                code,
                format!("cannot read the working file: {e}"),
            );
        }
    };
    planned.row.local_hash = Some(hashes.blake3.clone());
    planned.stat = stat;

    planned.row.before = if hashes == metadata.hashes && size == metadata.size {
        Before::Current
    } else {
        match baseline_for(cache, &rel_str) {
            Some((blake3, base_size)) if blake3 == hashes.blake3 && base_size == size => {
                Before::Clean
            }
            Some(_) => Before::Modified,
            None => Before::Unknown,
        }
    };
    planned.metadata = Some(metadata);
    planned
}

fn io_code(e: &io::Error) -> GetCode {
    // ERROR_SHARING_VIOLATION / ERROR_LOCK_VIOLATION: open in another program
    if cfg!(windows) && matches!(e.raw_os_error(), Some(32) | Some(33)) {
        return GetCode::FileInUse;
    }
    match e.kind() {
        io::ErrorKind::PermissionDenied => GetCode::PermissionDenied,
        _ => GetCode::IoError,
    }
}

fn code_for(e: &anyhow::Error) -> GetCode {
    e.chain()
        .find_map(|cause| cause.downcast_ref::<io::Error>())
        .map(io_code)
        .unwrap_or(GetCode::IoError)
}

/// Whether the destination is still exactly as the plan saw it.
fn destination_unchanged(target: &Path, planned: &Option<FileStat>) -> bool {
    match planned {
        None => std::fs::symlink_metadata(target).is_err(),
        Some(stat) => FileStat::from_path(target).ok().as_ref() == Some(stat),
    }
}

/// A per-file failure: its code and message.
type FileError = (GetCode, String);

/// Download, verify and install one file. The existing working file is only
/// replaced after the new bytes are verified and the destination is unchanged.
fn install(
    backend: &Backend,
    paths: &DvsPaths,
    relative_path: &Path,
    metadata: &FileMetadata,
    planned_stat: &Option<FileStat>,
    operation_id: Uuid,
    on_bytes: Option<&(dyn Fn(u64) + Send + Sync)>,
) -> Result<(), FileError> {
    let target = paths.file_path(relative_path);
    if let Some(parent) = target.parent() {
        fs::create_dir_all(parent)
            .map_err(|e| (io_code(&e), format!("cannot create its directory: {e}")))?;
    }
    let tmp_path = target.with_extension(format!("dvs-tmp.{}", Uuid::new_v4()));
    let path =
        ProjectPath::from_path(relative_path).map_err(|e| (GetCode::IoError, e.to_string()))?;

    let result = (|| {
        let retrieved = backend
            .retrieve(RetrieveRequest {
                hashes: &metadata.hashes,
                target: &tmp_path,
                compression: metadata.compression,
                path,
                operation_id,
                on_bytes,
            })
            .map_err(|e| (code_for(&e), format!("retrieval failed: {e:#}")))?;
        if !retrieved {
            return Err((
                GetCode::SourceMissing,
                "this version is not in storage".to_string(),
            ));
        }
        let actual = FileMetadata::from_file(&tmp_path, Compression::None, None)
            .map_err(|e| (code_for(&e), e.to_string()))?;
        if actual.hashes != metadata.hashes {
            return Err((
                GetCode::HashMismatch,
                "the stored content does not match the tracked hash".to_string(),
            ));
        }
        if !destination_unchanged(&target, planned_stat) {
            return Err((
                GetCode::DestinationChanged,
                "the file changed while dvs was downloading it; your version was kept".to_string(),
            ));
        }
        fs::rename(&tmp_path, &target)
            .map_err(|e| (io_code(&e), format!("cannot replace the file: {e}")))?;
        Ok(())
    })();
    if result.is_err() {
        let _ = fs::remove_file(&tmp_path);
    }
    result
}

/// Gets the selected files from storage, following `options.on_conflict`.
///
/// Never errors: every problem is reported in the returned rows or as the
/// report's `problem`.
pub fn get_files(
    selection: GetSelection,
    paths: &DvsPaths,
    backend: &Backend,
    options: GetOptions,
    on_file_start: Option<&OnFileStart>,
) -> GetReport {
    let GetOptions {
        dry_run,
        on_conflict,
        source,
    } = options;
    if selection.tracked.is_empty() && selection.unmatched.is_empty() {
        return GetReport::problem("no tracked files matched", dry_run, on_conflict, source);
    }

    let pool = match get_threadpool(selection.tracked.len().max(1)) {
        Ok(p) => p,
        Err(e) => return GetReport::problem(e.to_string(), dry_run, on_conflict, source),
    };
    let cache = try_open_cache(paths);
    let cache = cache.as_ref();

    // 1. Plan: classify every file, locally, writing nothing.
    let mut planned: Vec<Planned> = pool.install(|| {
        selection
            .tracked
            .into_par_iter()
            .map(|p| plan_file(paths, &p, cache))
            .collect()
    });
    let cwd_rel = paths
        .cwd_relative_to_root()
        .map(Path::to_path_buf)
        .unwrap_or_default();
    for user_path in selection.unmatched {
        let mut p = Planned {
            row: blank_row(paths, &normalize(&cwd_rel.join(&user_path))),
            metadata: None,
            stat: None,
        };
        let is_dir = paths.file_path(&p.row.path).is_dir();
        p.row.local_path = user_path;
        p.row.before = Before::NotTracked;
        p.set(
            Action::Failed,
            Some(GetCode::NotTracked),
            Some(if is_dir {
                "no tracked files selected in this directory".to_string()
            } else {
                "not tracked by dvs".to_string()
            }),
        );
        planned.push(p);
    }
    planned.sort_by(|a, b| a.row.path.cmp(&b.row.path));

    for p in planned
        .iter_mut()
        .filter(|p| p.row.before == Before::Current)
    {
        p.set(Action::Unchanged, None, None);
        if !dry_run {
            if let Some(m) = &p.metadata {
                record_baseline(
                    cache,
                    &p.row.path.to_string_lossy(),
                    &m.hashes.blake3,
                    m.size,
                );
            }
        }
    }

    // One access check for everything that needs the remote: transfers, and
    // the stored-copy hints on conflicts. Files already current need neither.
    let needs_remote = planned
        .iter()
        .any(|p| p.needs_transfer(on_conflict) || p.is_conflict());
    let access = if needs_remote {
        backend.check_access().map_err(|e| {
            let code = if e.is::<Unreachable>() {
                GetCode::ServerUnavailable
            } else {
                GetCode::AccessDenied
            };
            (code, format!("{e:#}"))
        })
    } else {
        Ok(())
    };
    for p in planned.iter_mut().filter(|p| p.is_conflict()) {
        p.row.stored_copy = Some(match (&access, &p.row.local_hash) {
            (Ok(()), Some(blake3)) => {
                let hashes = crate::Hashes {
                    blake3: blake3.clone(),
                    md5: None,
                };
                match backend.exists(&hashes) {
                    Ok(true) => StoredCopy::Present,
                    Ok(false) => StoredCopy::Absent,
                    Err(_) => StoredCopy::Unknown,
                }
            }
            _ => StoredCopy::Unknown,
        });
    }

    // 2. Decide: only `skip` continues past a known problem.
    let blocked =
        on_conflict != OnConflict::Skip && planned.iter().any(|p| p.is_known_problem(on_conflict));
    if blocked {
        for p in planned.iter_mut() {
            if p.row.before == Before::Current || p.row.action == Action::Failed {
                continue;
            }
            if p.is_conflict() && on_conflict != OnConflict::Overwrite {
                let (code, message) = (p.conflict_code(), p.conflict_message());
                p.set(Action::NotRun, Some(code), Some(message));
            } else {
                p.set(
                    Action::NotRun,
                    Some(GetCode::BatchAborted),
                    Some("held back: another requested file has a problem".to_string()),
                );
            }
        }
        return finish(planned, dry_run, on_conflict, source, true);
    }
    for p in planned
        .iter_mut()
        .filter(|p| p.is_conflict() && on_conflict == OnConflict::Skip)
    {
        let (code, message) = (p.conflict_code(), p.conflict_message());
        p.set(Action::Skipped, Some(code), Some(message));
    }

    // 3. Execute the transfers. Each file succeeds or fails on its own.
    let transfer_idx: Vec<usize> = planned
        .iter()
        .enumerate()
        .filter(|(_, p)| p.needs_transfer(on_conflict))
        .map(|(i, _)| i)
        .collect();
    if let Err((code, message)) = &access {
        for &i in &transfer_idx {
            planned[i].set(Action::Failed, Some(*code), Some(message.clone()));
        }
        return finish(planned, dry_run, on_conflict, source, false);
    }

    let operation_id = Uuid::new_v4();
    let outcomes: Vec<(usize, Result<(), FileError>)> = pool.install(|| {
        transfer_idx
            .par_iter()
            .map(|&i| {
                let p = &planned[i];
                let metadata = p
                    .metadata
                    .as_ref()
                    .expect("planned transfers have metadata");
                if dry_run {
                    let found = backend.exists(&metadata.hashes);
                    return (
                        i,
                        match found {
                            Ok(true) => Ok(()),
                            Ok(false) => Err((
                                GetCode::SourceMissing,
                                "this version is not in storage".to_string(),
                            )),
                            Err(e) => Err((GetCode::IoError, format!("{e:#}"))),
                        },
                    );
                }
                let file_progress = on_file_start.map(|f| f(&p.row.path, metadata.size));
                let on_bytes = file_progress.as_ref().map(|fp| &*fp.on_bytes);
                let result = install(
                    backend,
                    paths,
                    &p.row.path,
                    metadata,
                    &p.stat,
                    operation_id,
                    on_bytes,
                );
                if let Some(fp) = &file_progress {
                    (fp.on_done)(result.is_ok());
                }
                (i, result)
            })
            .collect()
    });

    for (i, result) in outcomes {
        let p = &mut planned[i];
        let was_absent = p.row.before == Before::Absent;
        let was_conflict = p.is_conflict();
        match result {
            Ok(()) => {
                let action = match (dry_run, was_absent) {
                    (true, true) => Action::WouldRestore,
                    (true, false) => Action::WouldReplace,
                    (false, true) => Action::Restored,
                    (false, false) => Action::Replaced,
                };
                let message = was_conflict.then(|| {
                    match (dry_run, p.row.before == Before::Modified) {
                        (true, true) => "local changes would be discarded",
                        (false, true) => "local changes were discarded",
                        (true, false) => "the local version would be replaced",
                        (false, false) => "the local version was replaced",
                    }
                    .to_string()
                });
                p.set(action, None, message);
                if !dry_run {
                    let metadata = p
                        .metadata
                        .as_ref()
                        .expect("planned transfers have metadata");
                    let rel_str = p.row.path.to_string_lossy().into_owned();
                    record_baseline(cache, &rel_str, &metadata.hashes.blake3, metadata.size);
                    if let Some(mtx) = cache {
                        if let Ok(stat) = FileStat::from_path(&paths.file_path(&p.row.path)) {
                            if let Err(e) =
                                mtx.lock()
                                    .unwrap()
                                    .insert(&rel_str, &stat, &metadata.hashes)
                            {
                                log::warn!("Cache store failed after get for {rel_str}: {e}");
                            }
                        }
                    }
                    log::info!("Retrieved {} ({})", p.row.path.display(), action.as_str());
                }
            }
            Err((code, message)) => {
                log::warn!("Failed to get {}: {message}", p.row.path.display());
                p.set(Action::Failed, Some(code), Some(message));
            }
        }
    }

    finish(planned, dry_run, on_conflict, source, false)
}

fn finish(
    planned: Vec<Planned>,
    dry_run: bool,
    on_conflict: OnConflict,
    source: SettingSource,
    aborted: bool,
) -> GetReport {
    let files: Vec<GetRow> = planned.into_iter().map(|p| p.row).collect();
    let mut counts = GetCounts::default();
    for r in &files {
        let slot = match r.action {
            Action::Restored => &mut counts.restored,
            Action::Replaced => &mut counts.replaced,
            Action::Unchanged => &mut counts.unchanged,
            Action::Skipped => &mut counts.skipped,
            Action::NotRun => &mut counts.not_run,
            Action::Failed => &mut counts.failed,
            Action::WouldRestore => &mut counts.would_restore,
            Action::WouldReplace => &mut counts.would_replace,
        };
        *slot += 1;
    }
    let satisfied = files.iter().filter(|r| r.satisfied).count();
    let outcome = if aborted {
        GetOutcome::Aborted
    } else if satisfied == files.len() {
        GetOutcome::Complete
    } else if satisfied == 0 {
        GetOutcome::Failed
    } else {
        GetOutcome::Partial
    };
    GetReport {
        outcome,
        dry_run,
        on_conflict,
        source,
        counts,
        problem: None,
        files,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::add_files;
    use crate::config::Config;
    use crate::testutil::{create_file, create_temp_git_repo, init_dvs_repo};
    use std::collections::HashSet;
    use tempfile::TempDir;

    struct TestRepo {
        _tmp: TempDir,
        root: PathBuf,
        config: Config,
        paths: DvsPaths,
    }

    impl TestRepo {
        fn new() -> Self {
            let (_tmp, root) = create_temp_git_repo();
            let (config, _dvs_dir) = init_dvs_repo(&root);
            let paths =
                DvsPaths::new(root.clone(), root.clone(), config.metadata_folder_name()).unwrap();
            Self {
                _tmp,
                root,
                config,
                paths,
            }
        }

        fn backend(&self) -> &Backend {
            self.config.backend()
        }

        /// Write `content` at `name` and `dvs add` it.
        fn add(&self, name: &str, content: &[u8]) {
            create_file(&self.root, name, content);
            let results = add_files(
                vec![name.into()],
                &self.paths,
                self.backend(),
                None,
                Compression::Zstd,
                false,
                None,
            )
            .unwrap();
            assert!(matches!(
                results[0].detail,
                crate::AddDetail::Success { .. }
            ));
        }

        fn metadata_bytes(&self, name: &str) -> Vec<u8> {
            fs::read(self.paths.metadata_path(Path::new(name))).unwrap()
        }

        /// Stand-in for `git checkout`: put back an earlier `.dvs` file.
        fn set_metadata_bytes(&self, name: &str, bytes: &[u8]) {
            fs::write(self.paths.metadata_path(Path::new(name)), bytes).unwrap();
        }

        /// The state of a clone where `name` is tracked at `v1` but the working
        /// file holds `v2`, which dvs itself put there (a branch switch).
        fn switched_branch(&self, name: &str, v1: &[u8], v2: &[u8]) {
            self.add(name, v1);
            let old = self.metadata_bytes(name);
            self.add(name, v2);
            self.set_metadata_bytes(name, &old);
        }

        fn read(&self, name: &str) -> Vec<u8> {
            fs::read(self.root.join(name)).unwrap()
        }

        fn get(&self, names: &[&str], on_conflict: OnConflict) -> GetReport {
            self.get_with(names, &[], on_conflict, false)
        }

        fn get_with(
            &self,
            names: &[&str],
            unmatched: &[&str],
            on_conflict: OnConflict,
            dry_run: bool,
        ) -> GetReport {
            let selection = GetSelection {
                tracked: names.iter().map(PathBuf::from).collect::<HashSet<_>>(),
                unmatched: unmatched.iter().map(PathBuf::from).collect(),
            };
            get_files(
                selection,
                &self.paths,
                self.backend(),
                GetOptions {
                    dry_run,
                    on_conflict,
                    source: SettingSource::Argument,
                },
                None,
            )
        }

        fn corrupt_blob(&self, name: &str) {
            let meta: FileMetadata = serde_json::from_slice(&self.metadata_bytes(name)).unwrap();
            let hash = meta.hashes.blake3;
            let storage = match &self.config.backend {
                Backend::Local(b) => b.path.clone(),
                Backend::Server(_) => unreachable!(),
            };
            let blob = storage.join(&hash[..2]).join(&hash[2..]);
            let mut perms = fs::metadata(&blob).unwrap().permissions();
            #[allow(clippy::permissions_set_readonly_false)]
            perms.set_readonly(false);
            fs::set_permissions(&blob, perms).unwrap();
            fs::write(&blob, b"CORRUPTED BLOB BYTES").unwrap();
        }
    }

    fn row<'a>(report: &'a GetReport, name: &str) -> &'a GetRow {
        report
            .files
            .iter()
            .find(|r| r.path == Path::new(name))
            .unwrap_or_else(|| panic!("no row for {name}: {report:#?}"))
    }

    // Spec 1
    #[test]
    fn absent_files_are_restored() {
        let repo = TestRepo::new();
        repo.add("a.csv", b"a");
        fs::remove_file(repo.root.join("a.csv")).unwrap();

        let report = repo.get(&["a.csv"], OnConflict::Abort);
        assert_eq!(report.outcome, GetOutcome::Complete);
        let r = row(&report, "a.csv");
        assert_eq!(
            (r.before, r.action, r.satisfied),
            (Before::Absent, Action::Restored, true)
        );
        assert_eq!(repo.read("a.csv"), b"a");
    }

    // Spec 2
    #[test]
    fn current_files_are_unchanged() {
        let repo = TestRepo::new();
        repo.add("a.csv", b"a");
        let report = repo.get(&["a.csv"], OnConflict::Abort);
        assert_eq!(report.outcome, GetOutcome::Complete);
        assert_eq!(row(&report, "a.csv").action, Action::Unchanged);
    }

    // Spec 3 and 20: the working file is what dvs (here: add) last put there
    #[test]
    fn clean_files_after_a_branch_switch_are_replaced() {
        let repo = TestRepo::new();
        repo.switched_branch("a.csv", b"version one", b"version two");

        let report = repo.get(&["a.csv"], OnConflict::Abort);
        let r = row(&report, "a.csv");
        assert_eq!((r.before, r.action), (Before::Clean, Action::Replaced));
        assert_eq!(report.outcome, GetOutcome::Complete);
        assert_eq!(repo.read("a.csv"), b"version one");
    }

    #[test]
    fn a_get_install_records_the_baseline() {
        let repo = TestRepo::new();
        repo.switched_branch("a.csv", b"one", b"two");
        repo.get(&["a.csv"], OnConflict::Abort);
        // Switch "back": .dvs says two again, the working file is the one get wrote.
        repo.add("tmp.csv", b"two");
        let two = repo.metadata_bytes("tmp.csv");
        repo.set_metadata_bytes("a.csv", &two);
        let report = repo.get(&["a.csv"], OnConflict::Abort);
        assert_eq!(row(&report, "a.csv").before, Before::Clean, "{report:#?}");
        assert_eq!(repo.read("a.csv"), b"two");
    }

    // Spec 4: abort changes nothing
    #[test]
    fn local_changes_abort_the_whole_batch() {
        let repo = TestRepo::new();
        repo.switched_branch("a.csv", b"a1", b"a2");
        repo.switched_branch("b.csv", b"b1", b"b2");
        fs::write(repo.root.join("b.csv"), b"my edits").unwrap();

        let report = repo.get(&["a.csv", "b.csv"], OnConflict::Abort);
        assert_eq!(report.outcome, GetOutcome::Aborted);
        let b = row(&report, "b.csv");
        assert_eq!(
            (b.before, b.action, b.code),
            (
                Before::Modified,
                Action::NotRun,
                Some(GetCode::LocalChanges)
            )
        );
        assert_eq!(b.stored_copy, Some(StoredCopy::Absent));
        let a = row(&report, "a.csv");
        assert_eq!(
            (a.action, a.code),
            (Action::NotRun, Some(GetCode::BatchAborted))
        );
        assert_eq!(repo.read("a.csv"), b"a2", "nothing may change on abort");
        assert_eq!(repo.read("b.csv"), b"my edits");
        assert!(report.has_conflicts() && !report.has_failures());
    }

    // Spec 4s
    #[test]
    fn skip_keeps_local_changes_and_gets_the_rest() {
        let repo = TestRepo::new();
        repo.switched_branch("a.csv", b"a1", b"a2");
        repo.switched_branch("b.csv", b"b1", b"b2");
        fs::write(repo.root.join("b.csv"), b"my edits").unwrap();

        let report = repo.get(&["a.csv", "b.csv"], OnConflict::Skip);
        assert_eq!(report.outcome, GetOutcome::Partial);
        assert_eq!(row(&report, "a.csv").action, Action::Replaced);
        assert_eq!(row(&report, "b.csv").action, Action::Skipped);
        assert_eq!(repo.read("a.csv"), b"a1");
        assert_eq!(repo.read("b.csv"), b"my edits");
    }

    // Spec 4o
    #[test]
    fn overwrite_replaces_local_changes() {
        let repo = TestRepo::new();
        repo.switched_branch("b.csv", b"b1", b"b2");
        fs::write(repo.root.join("b.csv"), b"my edits").unwrap();

        let report = repo.get(&["b.csv"], OnConflict::Overwrite);
        assert_eq!(report.outcome, GetOutcome::Complete);
        let b = row(&report, "b.csv");
        assert_eq!((b.before, b.action), (Before::Modified, Action::Replaced));
        assert_eq!(repo.read("b.csv"), b"b1");
    }

    // Spec 5
    #[test]
    fn a_differing_file_without_baseline_is_unknown() {
        let repo = TestRepo::new();
        repo.add("a.csv", b"tracked");
        fs::remove_dir_all(repo.paths.cache_folder()).unwrap();
        fs::write(repo.root.join("a.csv"), b"copied in by hand").unwrap();

        let report = repo.get(&["a.csv"], OnConflict::Abort);
        let r = row(&report, "a.csv");
        assert_eq!(
            (r.before, r.action, r.code),
            (Before::Unknown, Action::NotRun, Some(GetCode::UnknownLocal))
        );
        assert_eq!(repo.read("a.csv"), b"copied in by hand");
    }

    // Spec 6
    #[test]
    fn an_edit_undone_exactly_is_clean() {
        let repo = TestRepo::new();
        repo.switched_branch("a.csv", b"one", b"two");
        fs::write(repo.root.join("a.csv"), b"temporary edit").unwrap();
        fs::write(repo.root.join("a.csv"), b"two").unwrap();
        let report = repo.get(&["a.csv"], OnConflict::Abort);
        assert_eq!(row(&report, "a.csv").before, Before::Clean);
    }

    // Spec 8 and 8s
    #[test]
    fn untracked_requests_are_rows_and_block_unless_skip() {
        let repo = TestRepo::new();
        repo.add("a.csv", b"a");
        fs::remove_file(repo.root.join("a.csv")).unwrap();

        let report = repo.get_with(&["a.csv"], &["typo.csv"], OnConflict::Abort, false);
        assert_eq!(report.outcome, GetOutcome::Aborted);
        let t = row(&report, "typo.csv");
        assert_eq!(
            (t.before, t.code),
            (Before::NotTracked, Some(GetCode::NotTracked))
        );
        assert_eq!(row(&report, "a.csv").code, Some(GetCode::BatchAborted));
        assert!(!repo.root.join("a.csv").exists());
        assert!(report.has_failures());

        let report = repo.get_with(&["a.csv"], &["typo.csv"], OnConflict::Skip, false);
        assert_eq!(report.outcome, GetOutcome::Partial);
        assert_eq!(row(&report, "a.csv").action, Action::Restored);
    }

    // Spec 8b: a merge conflict in a .dvs file is detected before anything is written
    #[test]
    fn malformed_metadata_blocks_the_batch_unless_skip() {
        let repo = TestRepo::new();
        repo.add("a.csv", b"a");
        repo.add("b.csv", b"b");
        fs::remove_file(repo.root.join("a.csv")).unwrap();
        repo.set_metadata_bytes("b.csv", b"<<<<<<< HEAD\n{}\n=======\n{}\n>>>>>>> other\n");

        let report = repo.get(&["a.csv", "b.csv"], OnConflict::Abort);
        assert_eq!(report.outcome, GetOutcome::Aborted);
        assert_eq!(row(&report, "b.csv").code, Some(GetCode::BadMetadata));
        assert_eq!(row(&report, "a.csv").code, Some(GetCode::BatchAborted));
        assert!(!repo.root.join("a.csv").exists());

        let report = repo.get(&["a.csv", "b.csv"], OnConflict::Skip);
        assert_eq!(row(&report, "a.csv").action, Action::Restored);
    }

    #[test]
    fn a_directory_without_selected_files_says_so() {
        let repo = TestRepo::new();
        repo.add("a.csv", b"a");
        fs::create_dir_all(repo.root.join("empty")).unwrap();
        let report = repo.get_with(&["a.csv"], &["empty"], OnConflict::Skip, false);
        let r = row(&report, "empty");
        assert_eq!(r.code, Some(GetCode::NotTracked));
        assert_eq!(
            r.message.as_deref(),
            Some("no tracked files selected in this directory")
        );
    }

    // Spec 9
    #[test]
    fn an_empty_selection_is_an_operation_problem() {
        let repo = TestRepo::new();
        let report = repo.get(&[], OnConflict::Abort);
        assert_eq!(report.outcome, GetOutcome::Failed);
        assert!(report.problem.is_some() && report.files.is_empty());
    }

    // Spec 10
    #[test]
    fn missing_content_fails_only_that_file() {
        let repo = TestRepo::new();
        repo.add("a.csv", b"a");
        repo.add("b.csv", b"b");
        let storage = match &repo.config.backend {
            Backend::Local(b) => b.path.clone(),
            Backend::Server(_) => unreachable!(),
        };
        let meta: FileMetadata = serde_json::from_slice(&repo.metadata_bytes("b.csv")).unwrap();
        let h = meta.hashes.blake3;
        fs::remove_file(storage.join(&h[..2]).join(&h[2..])).unwrap();
        fs::remove_file(repo.root.join("a.csv")).unwrap();
        fs::remove_file(repo.root.join("b.csv")).unwrap();

        let report = repo.get(&["a.csv", "b.csv"], OnConflict::Abort);
        assert_eq!(report.outcome, GetOutcome::Partial);
        assert_eq!(row(&report, "a.csv").action, Action::Restored);
        assert_eq!(row(&report, "b.csv").code, Some(GetCode::SourceMissing));
    }

    // Spec 11
    #[test]
    fn corrupt_content_keeps_the_existing_file() {
        let repo = TestRepo::new();
        repo.switched_branch("a.csv", b"original content", b"newer content");
        // switched_branch's first add stored "original content": corrupt that one.
        repo.corrupt_blob("a.csv");
        let report = repo.get(&["a.csv"], OnConflict::Abort);
        assert_eq!(row(&report, "a.csv").action, Action::Failed);
        assert_eq!(repo.read("a.csv"), b"newer content");
        let tmp_left = fs::read_dir(&repo.root)
            .unwrap()
            .flatten()
            .any(|e| e.file_name().to_string_lossy().contains("dvs-tmp"));
        assert!(!tmp_left);
    }

    // Spec 13
    #[test]
    fn a_directory_in_the_way_is_never_replaced() {
        let repo = TestRepo::new();
        repo.add("a.csv", b"a");
        fs::remove_file(repo.root.join("a.csv")).unwrap();
        fs::create_dir(repo.root.join("a.csv")).unwrap();
        fs::write(repo.root.join("a.csv/inside"), b"keep").unwrap();

        let report = repo.get(&["a.csv"], OnConflict::Overwrite);
        assert_eq!(row(&report, "a.csv").code, Some(GetCode::NotAFile));
        assert!(repo.root.join("a.csv/inside").exists());
    }

    // Spec 13, symlink at the destination
    #[cfg(unix)]
    #[test]
    fn a_symlinked_destination_is_never_written_through() {
        let repo = TestRepo::new();
        repo.add("a.csv", b"a");
        let elsewhere = repo.root.parent().unwrap().join("elsewhere.csv");
        fs::write(&elsewhere, b"precious").unwrap();
        fs::remove_file(repo.root.join("a.csv")).unwrap();
        std::os::unix::fs::symlink(&elsewhere, repo.root.join("a.csv")).unwrap();

        let report = repo.get(&["a.csv"], OnConflict::Overwrite);
        assert_eq!(row(&report, "a.csv").code, Some(GetCode::NotAFile));
        assert_eq!(fs::read(&elsewhere).unwrap(), b"precious");
    }

    // Spec 14
    #[cfg(unix)]
    #[test]
    fn a_symlinked_parent_leading_outside_is_refused() {
        let repo = TestRepo::new();
        repo.add("data/x.txt", b"tracked");
        let outside = repo.root.parent().unwrap().join("outside");
        fs::create_dir(&outside).unwrap();
        fs::write(outside.join("x.txt"), b"precious").unwrap();
        fs::remove_dir_all(repo.root.join("data")).unwrap();
        std::os::unix::fs::symlink(&outside, repo.root.join("data")).unwrap();

        for policy in [OnConflict::Abort, OnConflict::Skip, OnConflict::Overwrite] {
            let report = repo.get(&["data/x.txt"], policy);
            let r = row(&report, "data/x.txt");
            assert_eq!(
                (r.before, r.code),
                (Before::UnsafePath, Some(GetCode::UnsafePath))
            );
            assert_eq!(fs::read(outside.join("x.txt")).unwrap(), b"precious");
        }
    }

    // Spec 15: the recheck itself
    #[test]
    fn a_destination_that_changed_after_planning_is_detected() {
        let repo = TestRepo::new();
        let path = create_file(&repo.root, "a.csv", b"before");
        let stat = Some(FileStat::from_path(&path).unwrap());
        assert!(destination_unchanged(&path, &stat));
        std::thread::sleep(std::time::Duration::from_millis(5));
        fs::write(&path, b"edited meanwhile").unwrap();
        assert!(!destination_unchanged(&path, &stat));
        let absent = repo.root.join("new.csv");
        assert!(destination_unchanged(&absent, &None));
        fs::write(&absent, b"appeared").unwrap();
        assert!(!destination_unchanged(&absent, &None));
    }

    // Spec 17
    #[test]
    fn a_dry_run_writes_nothing() {
        let repo = TestRepo::new();
        repo.switched_branch("a.csv", b"a1", b"a2");
        repo.add("b.csv", b"b");
        fs::remove_file(repo.root.join("b.csv")).unwrap();

        let report = repo.get_with(&["a.csv", "b.csv"], &[], OnConflict::Abort, true);
        assert_eq!(report.outcome, GetOutcome::Complete);
        assert_eq!(row(&report, "a.csv").action, Action::WouldReplace);
        assert_eq!(row(&report, "b.csv").action, Action::WouldRestore);
        assert_eq!(repo.read("a.csv"), b"a2");
        assert!(!repo.root.join("b.csv").exists());
    }

    // Spec 21
    #[test]
    fn a_mixed_batch_under_skip() {
        let repo = TestRepo::new();
        repo.add("a.csv", b"a");
        fs::remove_file(repo.root.join("a.csv")).unwrap();
        repo.switched_branch("b.csv", b"b1", b"b2");
        fs::write(repo.root.join("b.csv"), b"edited").unwrap();
        repo.add("c.csv", b"c");

        let report = repo.get(&["a.csv", "b.csv", "c.csv"], OnConflict::Skip);
        assert_eq!(row(&report, "a.csv").action, Action::Restored);
        assert_eq!(row(&report, "b.csv").action, Action::Skipped);
        assert_eq!(row(&report, "c.csv").action, Action::Unchanged);
        assert_eq!(report.outcome, GetOutcome::Partial);
        assert!(report.has_conflicts() && !report.has_failures());
    }

    #[test]
    fn local_paths_are_relative_to_the_cwd() {
        assert_eq!(
            relative_to(Path::new("/r/data"), Path::new("/r/data/x.csv")),
            PathBuf::from("x.csv")
        );
        assert_eq!(
            relative_to(Path::new("/r/data"), Path::new("/r/other/y.csv")),
            PathBuf::from("../other/y.csv")
        );
        assert_eq!(
            normalize(Path::new("data/../x.csv")),
            PathBuf::from("x.csv")
        );
    }
}
