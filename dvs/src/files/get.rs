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
use std::time::{Duration, Instant};

use crate::cache::{FileStat, HashCache, baseline_for, record_baseline, try_open_cache};
use crate::config::{OnConflict, SettingSource};
use crate::error::{ErrorCode, backend_error_code};
use crate::files::metadata::FileMetadata;
use crate::globbing::GetSelection;
use crate::operation::{
    ByteTotals, FileRow, LazyPhase, OperationContext, OperationPlan, Phase, millis,
};
use crate::paths::ProjectPath;
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
    AuthRequired,
    AuthExpired,
    ServerTooOld,
    ProjectNotFound,
    CredentialsUnreadable,
}

impl From<ErrorCode> for GetCode {
    fn from(code: ErrorCode) -> Self {
        match code {
            ErrorCode::AuthRequired => GetCode::AuthRequired,
            ErrorCode::AuthExpired => GetCode::AuthExpired,
            ErrorCode::AccessDenied => GetCode::AccessDenied,
            ErrorCode::ServerUnavailable => GetCode::ServerUnavailable,
            ErrorCode::ServerTooOld => GetCode::ServerTooOld,
            ErrorCode::ProjectNotFound => GetCode::ProjectNotFound,
            ErrorCode::BadMetadata => GetCode::BadMetadata,
            ErrorCode::CredentialsUnreadable => GetCode::CredentialsUnreadable,
        }
    }
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
            GetCode::AuthRequired => "auth_required",
            GetCode::AuthExpired => "auth_expired",
            GetCode::ServerTooOld => "server_too_old",
            GetCode::ProjectNotFound => "project_not_found",
            GetCode::CredentialsUnreadable => "credentials_unreadable",
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
    /// Time spent on this file (hash + transfer + install). Any row that was
    /// hashed has at least its hash time, whatever happened after (K11);
    /// `None` for rows never hashed and not transferred (untracked, unreadable
    /// metadata, not a file, absent and held back). Not serialized here, so R output is unchanged (K8); the CLI
    /// adds it.
    #[serde(skip)]
    pub duration_ms: Option<u64>,
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
    /// Byte totals (C8, K5). Not serialized here, so R output is unchanged
    /// (K8); the CLI adds it.
    #[serde(skip)]
    pub bytes: ByteTotals,
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
            bytes: ByteTotals::default(),
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
    /// Time spent classifying it (mostly hashing), part of its `duration_ms`.
    hash_time: Duration,
    /// Hashing the working file was attempted, whether or not it succeeded:
    /// an unreadable file did that work too (K11).
    hashed: bool,
    /// Already reported through `file_finished`: every row is, exactly once.
    emitted: bool,
}

impl Planned {
    fn new(row: GetRow) -> Self {
        Planned {
            row,
            metadata: None,
            stat: None,
            hash_time: Duration::ZERO,
            hashed: false,
            emitted: false,
        }
    }

    /// Report the row as final, unless it already was.
    fn emit(&mut self, ctx: &OperationContext) {
        if !self.emitted {
            self.emitted = true;
            self.fill_hash_duration();
            ctx.observer().file_finished(FileRow::Get(&self.row));
        }
    }

    /// K11: a row that was hashed carries at least its hash time, whatever
    /// happened to it after (hash failure, skipped, held back, access
    /// failure). Rows never hashed keep `None`.
    fn fill_hash_duration(&mut self) {
        if self.row.duration_ms.is_none() && self.hashed {
            self.row.duration_ms = Some(millis(self.hash_time));
        }
    }

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
        set_row(&mut self.row, action, code, message);
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

fn set_row(row: &mut GetRow, action: Action, code: Option<GetCode>, message: Option<String>) {
    row.action = action;
    row.satisfied = matches!(
        action,
        Action::Restored
            | Action::Replaced
            | Action::Unchanged
            | Action::WouldRestore
            | Action::WouldReplace
    );
    row.code = code;
    row.message = message;
}

/// An absolute path as a path from the project root, if it is inside it
/// (also when one side has symlinks resolved, e.g. macOS /var -> /private/var).
fn project_relative(root: &Path, absolute: &Path) -> Option<PathBuf> {
    let absolute = normalize(absolute);
    if let Ok(rel) = absolute.strip_prefix(root) {
        return Some(rel.to_path_buf());
    }
    let root = root.canonicalize().ok()?;
    let parent = absolute.parent()?.canonicalize().ok()?;
    let rel = parent.join(absolute.file_name()?);
    rel.strip_prefix(&root).ok().map(Path::to_path_buf)
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
        duration_ms: None,
    }
}

/// Classify one tracked file. Reads only local state. `hashing` begins
/// just before the working file is hashed.
fn plan_file(
    paths: &DvsPaths,
    relative_path: &Path,
    cache: Option<&Mutex<HashCache>>,
    hashing: &LazyPhase<'_>,
) -> Planned {
    let mut planned = Planned::new(blank_row(paths, relative_path));
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
    hashing.begin();
    planned.hashed = true;
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

/// The code for a failure: server unreachable or refusing, else by I/O kind.
fn code_for(e: &anyhow::Error) -> GetCode {
    if let Some(code) = backend_error_code(e) {
        return code.into();
    }
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
///
/// Reports to `ctx`'s observer as it goes: the `hash` phase (the plan, only
/// when it hashes some working file), the
/// `check_storage` phase when anything needs the remote, `planned` once the
/// plan and storage checks are done, then every row through `file_finished`
/// exactly once, as soon as it is final (rows decided before the transfers
/// first, then each transfer as it completes). In a dry run the existence
/// checks that stand in for transfers belong to `check_storage`, so there is
/// no `transfer` phase.
pub fn get_files(
    selection: GetSelection,
    paths: &DvsPaths,
    backend: &Backend,
    options: GetOptions,
    ctx: &OperationContext,
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
    let observer = ctx.observer();
    let ending = |planned, aborted| {
        finish(
            planned,
            Ending {
                dry_run,
                on_conflict,
                source,
                aborted,
            },
            ctx,
        )
    };

    // 1. Plan: classify every file, locally, writing nothing. The hash phase
    // starts with the first file actually hashed, so a plan that hashes
    // nothing (every file absent) reports none (K13).
    let hashing = LazyPhase::new(observer, Phase::Hash);
    let mut planned: Vec<Planned> = pool.install(|| {
        selection
            .tracked
            .into_par_iter()
            .map(|p| {
                let started = Instant::now();
                let mut planned = plan_file(paths, &p, cache, &hashing);
                planned.hash_time = started.elapsed();
                planned
            })
            .collect()
    });
    hashing.finish();
    let cwd_rel = paths
        .cwd_relative_to_root()
        .map(Path::to_path_buf)
        .unwrap_or_default();
    for user_path in selection.unmatched {
        // Report it like a tracked file: `path` from the project root, and
        // `local_path` from the cwd, however the user wrote it.
        let inside = if user_path.is_absolute() {
            project_relative(paths.repo_root(), &user_path)
        } else {
            Some(normalize(&cwd_rel.join(&user_path)))
        };
        let mut p = Planned::new(blank_row(paths, inside.as_deref().unwrap_or(&user_path)));
        let is_dir = paths.file_path(&p.row.path).is_dir();
        if inside.is_none() {
            p.row.local_path = user_path;
        }
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
        // Hashing it was the work: nothing to transfer.
        p.row.duration_ms = Some(millis(p.hash_time));
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
    let end_check_storage = || {
        if needs_remote {
            observer.phase_finished(Phase::CheckStorage);
        }
    };
    let access = if needs_remote {
        observer.phase_started(Phase::CheckStorage);
        backend.check_access().map_err(access_failure)
    } else {
        Ok(())
    };
    for p in planned.iter_mut().filter(|p| p.is_conflict()) {
        p.row.stored_copy = Some(match (&access, &p.row.local_hash) {
            (Ok(()), Some(blake3)) if needs_remote => {
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
        end_check_storage();
        let plan = operation_plan(&planned, &[]);
        observer.planned(&plan);
        return ending(planned, true);
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
    let plan = operation_plan(&planned, &transfer_idx);
    if let Err((code, message)) = &access {
        for &i in &transfer_idx {
            planned[i].set(Action::Failed, Some(*code), Some(message.clone()));
        }
        end_check_storage();
        observer.planned(&plan);
        return ending(planned, false);
    }
    // A dry run's existence checks are storage checks: check_storage stays
    // open across them, and there is no transfer phase.
    if !dry_run {
        end_check_storage();
    }
    observer.planned(&plan);
    // Everything that will not be transferred is final now.
    for p in planned.iter_mut() {
        if !p.needs_transfer(on_conflict) {
            p.emit(ctx);
        }
    }

    let transfers = !dry_run && !transfer_idx.is_empty();
    if transfers {
        observer.phase_started(Phase::Transfer);
    }
    let operation_id = Uuid::new_v4();
    let finished: Vec<(usize, GetRow)> = pool.install(|| {
        transfer_idx
            .par_iter()
            .map(|&i| {
                let p = &planned[i];
                let mut row = p.row.clone();
                let started = Instant::now();
                let result = transfer(p, backend, paths, dry_run, operation_id, ctx);
                settle(&mut row, p, result, dry_run, paths, cache);
                row.duration_ms = Some(millis(p.hash_time + started.elapsed()));
                // Final the moment its own transfer is done, not when the batch is.
                observer.file_finished(FileRow::Get(&row));
                (i, row)
            })
            .collect()
    });
    for (i, row) in finished {
        planned[i].row = row;
        planned[i].emitted = true;
    }
    if transfers {
        observer.phase_finished(Phase::Transfer);
    } else if dry_run {
        end_check_storage();
    }

    ending(planned, false)
}

/// Fetch one planned file (a dry run only asks storage whether it is there).
fn transfer(
    p: &Planned,
    backend: &Backend,
    paths: &DvsPaths,
    dry_run: bool,
    operation_id: Uuid,
    ctx: &OperationContext,
) -> Result<(), FileError> {
    let metadata = p
        .metadata
        .as_ref()
        .expect("planned transfers have metadata");
    if dry_run {
        return match backend.exists(&metadata.hashes) {
            Ok(true) => Ok(()),
            Ok(false) => Err((
                GetCode::SourceMissing,
                "this version is not in storage".to_string(),
            )),
            Err(e) => Err((code_for(&e), format!("{e:#}"))),
        };
    }
    let file_progress = ctx.observer().transfer_started(&p.row.path, metadata.size);
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
    result
}

/// Make `row` (a copy of `p.row`) final for its transfer's `result`, and
/// record what a real install wrote.
fn settle(
    row: &mut GetRow,
    p: &Planned,
    result: Result<(), FileError>,
    dry_run: bool,
    paths: &DvsPaths,
    cache: Option<&Mutex<HashCache>>,
) {
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
            set_row(row, action, None, message);
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
            set_row(row, Action::Failed, Some(code), Some(message));
        }
    }
}

/// What every file needing the remote gets when the access check fails: the
/// shared code (auth_required, auth_expired, access_denied,
/// server_unavailable, server_too_old, project_not_found, ...), not one
/// catch-all.
///
/// A failure that is not a classified backend error is still the remote
/// being unusable, so `server_unavailable` (K14), never an I/O code.
fn access_failure(e: anyhow::Error) -> FileError {
    let code = backend_error_code(&e)
        .map(GetCode::from)
        .unwrap_or(GetCode::ServerUnavailable);
    (code, format!("{e:#}"))
}

/// The plan as reported to the observer: every row, and the ones to download.
/// Rows of unknown size count in no byte total.
fn operation_plan(planned: &[Planned], transfer_idx: &[usize]) -> OperationPlan {
    let size = |p: &Planned| p.row.size.unwrap_or(0);
    OperationPlan {
        files_total: planned.len() as u64,
        bytes_total: planned.iter().map(size).sum(),
        files_to_transfer: transfer_idx.len() as u64,
        bytes_to_transfer: transfer_idx.iter().map(|&i| size(&planned[i])).sum(),
    }
}

/// How a `get` ended, for [`finish`].
struct Ending {
    dry_run: bool,
    on_conflict: OnConflict,
    source: SettingSource,
    aborted: bool,
}

fn finish(planned: Vec<Planned>, ending: Ending, ctx: &OperationContext) -> GetReport {
    let Ending {
        dry_run,
        on_conflict,
        source,
        aborted,
    } = ending;
    let files: Vec<GetRow> = planned
        .into_iter()
        .map(|mut p| {
            p.emit(ctx);
            p.row
        })
        .collect();
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
    // K5, K12: from the rows, the same in every ending (aborted included).
    // `to_transfer` is every row whose local copy does not match its target,
    // whatever then happened to it (held back, skipped, failed);
    // `already_stored` the ones that match. Unlike the plan's
    // `bytes_to_transfer` (what get attempts), it does not depend on the
    // policy. `transferred` is the size of what was installed, so 0 in a dry
    // run. Rows of unknown size count in none.
    let mut bytes = ByteTotals::default();
    for r in &files {
        if let Some(size) = r.size {
            let installed = matches!(r.action, Action::Restored | Action::Replaced);
            bytes.count(size, r.before == Before::Current, installed);
        }
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
        bytes,
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
                &OperationContext::new(),
            )
            .unwrap()
            .files;
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
                &OperationContext::new(),
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
            // Valid zstd of other bytes: decompression works, the hash check must catch it.
            fs::write(&blob, zstd::encode_all(&b"other bytes"[..], 0).unwrap()).unwrap();
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
        let r = row(&report, "a.csv");
        assert_eq!(
            (r.action, r.code),
            (Action::Failed, Some(GetCode::HashMismatch))
        );
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

    // Spec 4s and 21
    #[test]
    fn skip_keeps_local_changes_and_gets_the_rest() {
        let repo = TestRepo::new();
        repo.add("a.csv", b"a");
        fs::remove_file(repo.root.join("a.csv")).unwrap();
        repo.switched_branch("b.csv", b"b1", b"b2");
        fs::write(repo.root.join("b.csv"), b"edited").unwrap();
        repo.add("c.csv", b"c");
        repo.switched_branch("d.csv", b"d1", b"d2");

        let report = repo.get(&["a.csv", "b.csv", "c.csv", "d.csv"], OnConflict::Skip);
        assert_eq!(row(&report, "a.csv").action, Action::Restored);
        assert_eq!(row(&report, "b.csv").action, Action::Skipped);
        assert_eq!(row(&report, "c.csv").action, Action::Unchanged);
        assert_eq!(row(&report, "d.csv").action, Action::Replaced);
        assert_eq!(report.outcome, GetOutcome::Partial);
        assert!(report.has_conflicts() && !report.has_failures());
        assert_eq!(repo.read("b.csv"), b"edited");
        assert_eq!(repo.read("d.csv"), b"d1");
    }

    // Run from a subdirectory: paths in the report are from the project root
    // and from the cwd, also for requests that match nothing, absolute or not.
    #[test]
    fn paths_from_a_subdirectory() {
        let repo = TestRepo::new();
        repo.add("data/x.csv", b"x");
        fs::remove_file(repo.root.join("data/x.csv")).unwrap();
        let in_data = DvsPaths::new(
            repo.root.join("data"),
            repo.root.clone(),
            repo.config.metadata_folder_name(),
        )
        .unwrap();
        let selection = GetSelection {
            tracked: HashSet::from([PathBuf::from("data/x.csv")]),
            unmatched: vec!["typo.csv".into(), repo.root.join("data/abs-typo.csv")],
        };
        let options = GetOptions {
            dry_run: false,
            on_conflict: OnConflict::Skip,
            source: SettingSource::Argument,
        };
        let report = get_files(
            selection,
            &in_data,
            repo.backend(),
            options,
            &OperationContext::new(),
        );
        let local = |name: &str| row(&report, name).local_path.clone();
        assert_eq!(local("data/x.csv"), PathBuf::from("x.csv"));
        assert_eq!(local("data/typo.csv"), PathBuf::from("typo.csv"));
        assert_eq!(local("data/abs-typo.csv"), PathBuf::from("abs-typo.csv"));
        assert!(repo.root.join("data/x.csv").exists());
    }

    // Upgrading: an existing cache db without baseline rows. Changed files are
    // unknown until a get sees them current; dry runs never record anything.
    #[cfg(feature = "cache")]
    #[test]
    fn an_upgraded_clone_learns_the_baseline_from_a_real_get() {
        let repo = TestRepo::new();
        repo.add("a.csv", b"one");
        let one = repo.metadata_bytes("a.csv");
        repo.add("a.csv", b"two");
        let two = repo.metadata_bytes("a.csv");
        rusqlite::Connection::open(repo.paths.cache_folder().join("dvs.db"))
            .unwrap()
            .execute_batch("DROP TABLE baseline;")
            .unwrap();

        let report = repo.get_with(&["a.csv"], &[], OnConflict::Abort, true);
        assert_eq!(row(&report, "a.csv").action, Action::Unchanged);
        repo.set_metadata_bytes("a.csv", &one);
        let report = repo.get(&["a.csv"], OnConflict::Abort);
        assert_eq!(
            row(&report, "a.csv").before,
            Before::Unknown,
            "dry run recorded"
        );

        repo.set_metadata_bytes("a.csv", &two);
        repo.get(&["a.csv"], OnConflict::Abort);
        repo.set_metadata_bytes("a.csv", &one);
        let report = repo.get(&["a.csv"], OnConflict::Abort);
        let r = row(&report, "a.csv");
        assert_eq!((r.before, r.action), (Before::Clean, Action::Replaced));
        assert_eq!(repo.read("a.csv"), b"one");
    }

    // K11: a working file that could not be hashed (unreadable) still did
    // the hashing work: its failed row carries the time spent.
    #[cfg(unix)]
    #[test]
    fn a_failed_hash_keeps_its_hash_time() {
        use std::os::unix::fs::PermissionsExt;
        let repo = TestRepo::new();
        repo.add("a.csv", b"a");
        // New content, so the hash cache cannot answer for it.
        let path = repo.root.join("a.csv");
        fs::write(&path, b"changed").unwrap();
        fs::set_permissions(&path, std::fs::Permissions::from_mode(0o000)).unwrap();
        if fs::read(&path).is_ok() {
            fs::set_permissions(&path, std::fs::Permissions::from_mode(0o644)).unwrap();
            return; // running as root: permissions are not enforced
        }
        let report = repo.get(&["a.csv"], OnConflict::Abort);
        fs::set_permissions(&path, std::fs::Permissions::from_mode(0o644)).unwrap();
        let r = row(&report, "a.csv");
        assert_eq!(
            (r.action, r.code, r.local_hash.as_deref()),
            (Action::Failed, Some(GetCode::PermissionDenied), None)
        );
        assert!(r.duration_ms.is_some(), "hashing was attempted");
    }

    // Spec 16: a destination that can't be written fails alone, keeps what
    // was there, and a rerun finishes once it is fixed.
    #[cfg(unix)]
    #[test]
    fn an_unwritable_destination_fails_alone_and_a_rerun_finishes() {
        use std::os::unix::fs::PermissionsExt;
        let repo = TestRepo::new();
        repo.add("locked/a.csv", b"a");
        repo.add("b.csv", b"b");
        fs::remove_file(repo.root.join("locked/a.csv")).unwrap();
        fs::remove_file(repo.root.join("b.csv")).unwrap();
        let locked = repo.root.join("locked");
        fs::set_permissions(&locked, std::fs::Permissions::from_mode(0o555)).unwrap();
        if fs::write(locked.join("probe"), b"").is_ok() {
            return; // running as root: permissions are not enforced
        }

        let report = repo.get(&["locked/a.csv", "b.csv"], OnConflict::Abort);
        fs::set_permissions(&locked, std::fs::Permissions::from_mode(0o755)).unwrap();
        assert_eq!(report.outcome, GetOutcome::Partial);
        assert_eq!(
            row(&report, "locked/a.csv").code,
            Some(GetCode::PermissionDenied)
        );
        assert_eq!(row(&report, "b.csv").action, Action::Restored);

        let report = repo.get(&["locked/a.csv", "b.csv"], OnConflict::Abort);
        assert_eq!(report.outcome, GetOutcome::Complete);
        assert_eq!(repo.read("locked/a.csv"), b"a");
    }

    /// What an observer heard, in order.
    #[derive(Debug, Clone)]
    enum Heard {
        Planned(OperationPlan),
        Started(Phase),
        Finished(Phase),
        Transfer,
        Row(GetRow),
    }

    /// Records everything; optionally holds the second transfer open until
    /// another row is reported.
    #[derive(Default)]
    struct Recorder {
        heard: Mutex<Vec<Heard>>,
        rows_seen: std::sync::Condvar,
        hold_second_transfer: bool,
        /// Whether the held transfer saw another file's row before it finished
        second_saw_a_row: Mutex<Option<bool>>,
    }

    impl Recorder {
        fn phases(&self) -> Vec<String> {
            self.heard()
                .into_iter()
                .filter_map(|h| match h {
                    Heard::Started(p) => Some(format!("+{}", p.as_str())),
                    Heard::Finished(p) => Some(format!("-{}", p.as_str())),
                    _ => None,
                })
                .collect()
        }

        fn heard(&self) -> Vec<Heard> {
            self.heard.lock().unwrap().clone()
        }

        fn rows(&self) -> Vec<GetRow> {
            self.heard()
                .into_iter()
                .filter_map(|h| match h {
                    Heard::Row(r) => Some(r),
                    _ => None,
                })
                .collect()
        }
    }

    impl crate::OperationObserver for std::sync::Arc<Recorder> {
        fn planned(&self, plan: &OperationPlan) {
            self.heard.lock().unwrap().push(Heard::Planned(*plan));
        }
        fn phase_started(&self, phase: Phase) {
            self.heard.lock().unwrap().push(Heard::Started(phase));
        }
        fn phase_finished(&self, phase: Phase) {
            self.heard.lock().unwrap().push(Heard::Finished(phase));
        }
        fn transfer_started(
            &self,
            path: &Path,
            _total_bytes: u64,
        ) -> Option<crate::progress::FileProgress> {
            let nth = {
                let mut heard = self.heard.lock().unwrap();
                heard.push(Heard::Transfer);
                heard
                    .iter()
                    .filter(|h| matches!(h, Heard::Transfer))
                    .count()
            };
            if !(self.hold_second_transfer && nth == 2) {
                return None;
            }
            let me = self.clone();
            let path = path.to_path_buf();
            Some(crate::progress::FileProgress {
                on_bytes: Box::new(|_| {}),
                on_done: Box::new(move |_| {
                    // The slow transfer: it does not finish until another
                    // file's row is reported (or 5 s pass).
                    let other_row = |heard: &Vec<Heard>| {
                        heard
                            .iter()
                            .any(|h| matches!(h, Heard::Row(r) if r.path != path))
                    };
                    let heard = me.heard.lock().unwrap();
                    let (heard, _) = me
                        .rows_seen
                        .wait_timeout_while(heard, std::time::Duration::from_secs(5), |h| {
                            !other_row(h)
                        })
                        .unwrap();
                    *me.second_saw_a_row.lock().unwrap() = Some(other_row(&heard));
                }),
            })
        }
        fn file_finished(&self, row: crate::FileRow<'_>) {
            let crate::FileRow::Get(row) = row else {
                panic!("get reported a non-get row");
            };
            self.heard.lock().unwrap().push(Heard::Row(row.clone()));
            self.rows_seen.notify_all();
        }
    }

    impl TestRepo {
        fn get_observed(
            &self,
            names: &[&str],
            unmatched: &[&str],
            on_conflict: OnConflict,
            dry_run: bool,
            recorder: &std::sync::Arc<Recorder>,
        ) -> GetReport {
            let backend = self.backend();
            self.get_observed_on(backend, names, unmatched, on_conflict, dry_run, recorder)
        }

        fn get_observed_on(
            &self,
            backend: &Backend,
            names: &[&str],
            unmatched: &[&str],
            on_conflict: OnConflict,
            dry_run: bool,
            recorder: &std::sync::Arc<Recorder>,
        ) -> GetReport {
            let selection = GetSelection {
                tracked: names.iter().map(PathBuf::from).collect::<HashSet<_>>(),
                unmatched: unmatched.iter().map(PathBuf::from).collect(),
            };
            let ctx = OperationContext::new().with_observer(std::sync::Arc::new(recorder.clone()));
            get_files(
                selection,
                &self.paths,
                backend,
                GetOptions {
                    dry_run,
                    on_conflict,
                    source: SettingSource::Argument,
                },
                &ctx,
            )
        }
    }

    /// The JSON shape of a row plus the fields JSON skips.
    fn shape(r: &GetRow) -> (serde_json::Value, Option<u64>) {
        (serde_json::to_value(r).unwrap(), r.duration_ms)
    }

    /// A mixed batch: restored, unchanged, skipped and untracked rows.
    fn mixed_batch(repo: &TestRepo) {
        repo.add("a.csv", b"aaa");
        fs::remove_file(repo.root.join("a.csv")).unwrap();
        repo.add("b.csv", b"bb");
        repo.switched_branch("c.csv", b"c1", b"c2");
        fs::write(repo.root.join("c.csv"), b"edited").unwrap();
        repo.add("d.csv", b"dddd");
        fs::remove_file(repo.root.join("d.csv")).unwrap();
    }

    // C3: every row is reported once, in the same shape as the result.
    #[test]
    fn every_row_is_reported_once_in_its_final_shape() {
        for dry_run in [false, true] {
            let repo = TestRepo::new();
            mixed_batch(&repo);
            let recorder = std::sync::Arc::new(Recorder::default());
            let report = repo.get_observed(
                &["a.csv", "b.csv", "c.csv", "d.csv"],
                &["typo.csv"],
                OnConflict::Skip,
                dry_run,
                &recorder,
            );
            let rows = recorder.rows();
            assert_eq!(rows.len(), report.files.len(), "{rows:#?}");
            for final_row in &report.files {
                let reported: Vec<_> = rows.iter().filter(|r| r.path == final_row.path).collect();
                assert_eq!(
                    reported.len(),
                    1,
                    "{} reported once",
                    final_row.path.display()
                );
                assert_eq!(shape(reported[0]), shape(final_row));
            }
            // files[] keeps its order: sorted by path
            let order: Vec<_> = report.files.iter().map(|r| r.path.clone()).collect();
            let mut sorted = order.clone();
            sorted.sort();
            assert_eq!(order, sorted);
        }
    }

    // C3: the first file's row arrives before the second transfer finishes.
    #[test]
    fn a_transfer_row_is_reported_before_a_slower_transfer_finishes() {
        let repo = TestRepo::new();
        repo.add("a.csv", b"a");
        repo.add("b.csv", b"b");
        fs::remove_file(repo.root.join("a.csv")).unwrap();
        fs::remove_file(repo.root.join("b.csv")).unwrap();
        let recorder = std::sync::Arc::new(Recorder {
            hold_second_transfer: true,
            ..Default::default()
        });
        let report = repo.get_observed(
            &["a.csv", "b.csv"],
            &[],
            OnConflict::Abort,
            false,
            &recorder,
        );
        assert_eq!(report.outcome, GetOutcome::Complete);
        assert_eq!(*recorder.second_saw_a_row.lock().unwrap(), Some(true));
    }

    // O1: phases, then the plan, then rows; dry runs have no transfer phase.
    #[test]
    fn phases_and_the_plan_come_in_order() {
        for dry_run in [false, true] {
            let repo = TestRepo::new();
            mixed_batch(&repo);
            let recorder = std::sync::Arc::new(Recorder::default());
            let report = repo.get_observed(
                &["a.csv", "b.csv", "c.csv", "d.csv"],
                &[],
                OnConflict::Skip,
                dry_run,
                &recorder,
            );
            let heard = recorder.heard();
            let marks: Vec<String> = heard
                .iter()
                .filter_map(|h| match h {
                    Heard::Started(p) => Some(format!("+{}", p.as_str())),
                    Heard::Finished(p) => Some(format!("-{}", p.as_str())),
                    Heard::Planned(_) => Some("planned".to_string()),
                    _ => None,
                })
                .collect();
            let expected: &[&str] = if dry_run {
                &[
                    "+hash",
                    "-hash",
                    "+check_storage",
                    "planned",
                    "-check_storage",
                ]
            } else {
                &[
                    "+hash",
                    "-hash",
                    "+check_storage",
                    "-check_storage",
                    "planned",
                    "+transfer",
                    "-transfer",
                ]
            };
            assert_eq!(marks, expected, "dry_run {dry_run}");
            // No row before the plan.
            let planned_at = heard
                .iter()
                .position(|h| matches!(h, Heard::Planned(_)))
                .unwrap();
            assert!(
                heard[..planned_at]
                    .iter()
                    .all(|h| !matches!(h, Heard::Row(_)))
            );
            let Heard::Planned(plan) = heard[planned_at] else {
                unreachable!()
            };
            // a (3) and d (4) need downloading; b (2) is current, c (2) skipped.
            assert_eq!(
                plan,
                OperationPlan {
                    files_total: 4,
                    bytes_total: 11,
                    files_to_transfer: 2,
                    bytes_to_transfer: 7,
                }
            );
            assert_eq!(report.bytes.total, plan.bytes_total);
            // K12: the plan counts what get attempts; the result every file
            // not at its target, so c (skipped) too.
            assert_eq!(report.bytes.to_transfer, 9);
        }
    }

    // O1: nothing needs the remote: no check_storage phase, nothing to transfer.
    #[test]
    fn a_batch_already_current_skips_the_storage_phases() {
        let repo = TestRepo::new();
        repo.add("a.csv", b"a");
        let recorder = std::sync::Arc::new(Recorder::default());
        repo.get_observed(&["a.csv"], &[], OnConflict::Abort, false, &recorder);
        let phases: Vec<_> = recorder
            .heard()
            .into_iter()
            .filter_map(|h| match h {
                Heard::Started(p) => Some(p),
                _ => None,
            })
            .collect();
        assert_eq!(phases, [Phase::Hash]);
    }

    // C8 (K5): totals are the per-row sums, dry run and real.
    #[test]
    fn byte_totals_match_the_rows() {
        for dry_run in [true, false] {
            let repo = TestRepo::new();
            mixed_batch(&repo);
            let report = repo.get_with(
                &["a.csv", "b.csv", "c.csv", "d.csv"],
                &["typo.csv"],
                OnConflict::Skip,
                dry_run,
            );
            let sum = |f: &dyn Fn(&GetRow) -> bool| -> u64 {
                report
                    .files
                    .iter()
                    .filter(|r| f(r))
                    .filter_map(|r| r.size)
                    .sum()
            };
            // K12: every file whose local copy is not its target, the
            // skipped c (2) included.
            let to_transfer = sum(&|r| r.before != Before::Current);
            let bytes = report.bytes;
            assert_eq!(bytes.total, sum(&|_| true));
            assert_eq!(bytes.total, 11, "the untracked row has no size");
            assert_eq!(bytes.to_transfer, to_transfer);
            assert_eq!(bytes.to_transfer, 9);
            assert_eq!(bytes.already_stored, sum(&|r| r.before == Before::Current));
            assert_eq!(bytes.already_stored, bytes.total - bytes.to_transfer);
            let moved = if dry_run { 0 } else { 7 };
            assert_eq!(bytes.transferred, moved, "dry_run {dry_run}");
        }
    }

    // C2: rows that did work carry a duration; the rest none.
    #[test]
    fn rows_that_did_work_have_a_duration() {
        let repo = TestRepo::new();
        mixed_batch(&repo);
        let report = repo.get_with(
            &["a.csv", "b.csv", "c.csv", "d.csv"],
            &["typo.csv"],
            OnConflict::Skip,
            false,
        );
        let duration = |name| row(&report, name).duration_ms;
        assert!(duration("a.csv").is_some(), "restored");
        assert!(duration("b.csv").is_some(), "unchanged: hashed");
        assert!(duration("c.csv").is_some(), "skipped, but hashed (K11)");
        assert_eq!(duration("typo.csv"), None, "not tracked");
        // Held back: the hashed conflict keeps its hash time, the absent
        // file (never hashed) has none.
        let repo = TestRepo::new();
        mixed_batch(&repo);
        let report = repo.get(&["a.csv", "c.csv"], OnConflict::Abort);
        assert_eq!(report.outcome, GetOutcome::Aborted);
        assert_eq!(duration_of(&report, "a.csv"), None);
        assert!(duration_of(&report, "c.csv").is_some(), "held back, hashed");
    }

    fn duration_of(report: &GetReport, name: &str) -> Option<u64> {
        row(report, name).duration_ms
    }

    // K13: a plan that hashes nothing (every file absent) has no hash phase.
    #[test]
    fn a_batch_with_nothing_to_hash_has_no_hash_phase() {
        let repo = TestRepo::new();
        repo.add("a.csv", b"a");
        fs::remove_file(repo.root.join("a.csv")).unwrap();
        let recorder = std::sync::Arc::new(Recorder::default());
        let report = repo.get_observed(&["a.csv"], &[], OnConflict::Abort, false, &recorder);
        assert_eq!(report.outcome, GetOutcome::Complete);
        assert_eq!(
            recorder.phases(),
            ["+check_storage", "-check_storage", "+transfer", "-transfer"]
        );
    }

    // C4: an access failure keeps its shared code per row instead of
    // collapsing to access_denied.
    #[test]
    fn access_failures_keep_their_shared_code() {
        use crate::{BackendError, ErrorCode};
        for code in [
            ErrorCode::AuthRequired,
            ErrorCode::AuthExpired,
            ErrorCode::AccessDenied,
            ErrorCode::ServerUnavailable,
            ErrorCode::ServerTooOld,
            ErrorCode::ProjectNotFound,
            ErrorCode::CredentialsUnreadable,
        ] {
            let e: anyhow::Error = BackendError::new(code, "nope").into();
            let (got, message) = access_failure(e.context("checking access"));
            assert_eq!(got.as_str(), code.as_str());
            assert_eq!(message, "checking access: nope");
        }
    }

    // K14: an access failure that is no classified backend error is the
    // remote being unusable, not a local I/O problem.
    #[test]
    fn an_unclassified_access_failure_is_server_unavailable() {
        let plain = anyhow::anyhow!("something went wrong");
        assert_eq!(access_failure(plain).0, GetCode::ServerUnavailable);
        let io: anyhow::Error = io::Error::new(io::ErrorKind::PermissionDenied, "no").into();
        assert_eq!(access_failure(io).0, GetCode::ServerUnavailable);
    }
}
