use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use anyhow::{Result, anyhow, bail};
use fs_err as fs;
use rayon::iter::Either;
use rayon::prelude::*;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::backends::Backend;
use crate::backends::server::{UploadManifest, UploadManifestEntry};
use crate::cache::try_open_cache;
use crate::config::Compression;
use crate::error::{ErrorCode, backend_error_code};
use crate::files::metadata::FileMetadata;
use crate::gitignore::add_to_gitignore;
use crate::operation::{
    ByteTotals, FileRow, LazyPhase, OperationContext, OperationPlan, OperationRow, Phase,
    PhaseGuard, RowState, RunSummary, duration_ms, millis,
};
use crate::paths::{DvsPaths, ProjectPath};
use crate::utils::get_threadpool;
use crate::{Outcome, cache};

/// Result of adding a single file.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AddResult {
    pub path: PathBuf,
    #[serde(flatten)]
    pub detail: AddDetail,
    /// Time spent on this file (hash + transfer + record); `None` when it did
    /// no work (including rows cancelled before hashing; a row cancelled after
    /// hashing carries its work so far). Not serialized by the library, so
    /// the R package's output is unchanged; the CLI adds it to its rows.
    #[serde(skip)]
    pub duration_ms: Option<u64>,
}

impl OperationRow for AddResult {
    fn state(&self) -> RowState {
        match self.detail {
            AddDetail::Success { .. } => RowState::Succeeded,
            AddDetail::Error { .. } => RowState::Failed,
            AddDetail::Cancelled { .. } => RowState::Cancelled,
        }
    }
}

/// The rows of an `add`, sorted by path, plus byte totals.
#[derive(Debug, Clone)]
pub struct AddReport {
    pub files: Vec<AddResult>,
    pub bytes: ByteTotals,
    /// The cancel flag was set when the add finished
    pub cancelled: bool,
}

impl AddReport {
    pub fn summary(&self) -> RunSummary {
        RunSummary::of(&self.files, self.cancelled)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum AddDetail {
    Success {
        outcome: Outcome,
        hash: String,
        size: u64,
        stored_size: u64,
    },
    Error {
        code: AddErrorCode,
        error: String,
    },
    /// Not started because the operation was cancelled: no upload, no record
    Cancelled {
        message: String,
    },
}

/// Stable machine-readable category for a per-file add failure.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AddErrorCode {
    AccessDenied,
    ServerUnavailable,
    IoError,
    OperationFailed,
    AuthRequired,
    AuthExpired,
    ServerTooOld,
    ProjectNotFound,
    BadMetadata,
    CredentialsUnreadable,
}

impl From<ErrorCode> for AddErrorCode {
    fn from(code: ErrorCode) -> Self {
        match code {
            ErrorCode::AuthRequired => AddErrorCode::AuthRequired,
            ErrorCode::AuthExpired => AddErrorCode::AuthExpired,
            ErrorCode::AccessDenied => AddErrorCode::AccessDenied,
            ErrorCode::ServerUnavailable => AddErrorCode::ServerUnavailable,
            ErrorCode::ServerTooOld => AddErrorCode::ServerTooOld,
            ErrorCode::ProjectNotFound => AddErrorCode::ProjectNotFound,
            ErrorCode::BadMetadata => AddErrorCode::BadMetadata,
            ErrorCode::CredentialsUnreadable => AddErrorCode::CredentialsUnreadable,
        }
    }
}

fn add_error_code(error: &anyhow::Error) -> AddErrorCode {
    if let Some(code) = backend_error_code(error) {
        code.into()
    } else if error.downcast_ref::<std::io::Error>().is_some() {
        AddErrorCode::IoError
    } else {
        AddErrorCode::OperationFailed
    }
}

fn error_row(path: PathBuf, error: &anyhow::Error, duration_ms: Option<u64>) -> AddResult {
    log::warn!("Failed to add {}: {error}", path.display());
    AddResult {
        path,
        detail: AddDetail::Error {
            code: add_error_code(error),
            error: error.to_string(),
        },
        duration_ms,
    }
}

/// A file that was not added because the operation was cancelled: no upload,
/// no record. `work` is the time it already spent (its hashing), `None` when
/// it was cancelled before it was hashed.
fn cancelled_row(path: PathBuf, work: Option<Duration>) -> AddResult {
    AddResult {
        path,
        detail: AddDetail::Cancelled {
            message: "cancelled before this file was added".to_string(),
        },
        duration_ms: work.map(millis),
    }
}

/// A file to add: the metadata it will get, waiting on the backend.
struct Pending {
    relative_path: PathBuf,
    metadata: FileMetadata,
    stored: bool,
    /// Time spent on this file so far: hashing, then storage and record
    work: Duration,
}

impl Pending {
    /// Whether the metadata file on disk already says exactly this
    fn in_sync(&self, paths: &DvsPaths) -> bool {
        let Ok(file) = fs::File::open(paths.metadata_path(&self.relative_path)) else {
            return false;
        };
        serde_json::from_reader::<_, FileMetadata>(file)
            .is_ok_and(|existing| existing == self.metadata)
    }

    fn manifest_entry(&self) -> Result<UploadManifestEntry> {
        Ok(UploadManifestEntry {
            path: ProjectPath::from_path(&self.relative_path).map_err(|e| anyhow!("{e}"))?,
            size: self.metadata.size,
            hash: self.metadata.hashes.blake3.clone(),
        })
    }

    /// Once hashed the size is known, so a later failure or cancel still counts it.
    fn row_bytes(&self, transferred: bool) -> RowBytes {
        RowBytes {
            size: self.metadata.size,
            stored: self.stored,
            transferred,
        }
    }
}

struct RowBytes {
    size: u64,
    stored: bool,
    transferred: bool,
}

/// What step 3 does with a hashed file.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Finish {
    /// Never starts: a `cancelled` row, no upload, no record
    Cancel,
    /// Upload if storage lacks the content, then write the record
    Add,
    /// This add already uploaded the content: write the record only
    RecordOnly,
}

/// Decides what step 3 does with a file as it would start. `manifest_missing`
/// is the server's manifest answer, `None` when none was sent. The server
/// records every manifest entry whose content it already has, and completes
/// every same-hash entry once content arrives, so those files finish even
/// after a cancel to keep the local records in agreement.
fn finish_for(
    cancelled: bool,
    hash: &str,
    manifest_missing: Option<&HashSet<String>>,
    uploaded_in_this_add: bool,
) -> Finish {
    let recorded_by_server = manifest_missing.is_some_and(|missing| !missing.contains(hash));
    if uploaded_in_this_add {
        Finish::RecordOnly
    } else if cancelled && !recorded_by_server {
        Finish::Cancel
    } else {
        Finish::Add
    }
}

/// What finishing one file did.
struct Added {
    outcome: Outcome,
    stored_size: Option<u64>,
    /// The backend created the blob, i.e. this file's bytes moved
    created: bool,
}

/// `missing` is the server's answer to the manifest: the hashes to upload. The local
/// backend has no manifest and checks for itself. `uploaded` is the stored size
/// of this file's content when this add already uploaded it.
#[allow(clippy::too_many_arguments)]
fn add_file(
    pending: &mut Pending,
    paths: &DvsPaths,
    backend: &Backend,
    operation_id: Uuid,
    missing: Option<&HashSet<String>>,
    uploaded: Option<u64>,
    dry_run: bool,
    on_bytes: Option<&(dyn Fn(u64) + Send + Sync)>,
) -> Result<Added> {
    let full_path = paths.file_path(&pending.relative_path);
    if dry_run {
        let outcome = if pending.stored && pending.in_sync(paths) {
            Outcome::Present
        } else {
            Outcome::Copied
        };
        return Ok(Added {
            outcome,
            stored_size: None,
            created: false,
        });
    }
    let stored = |res: crate::StoreResult| (Some(res.stored_size), res.created);
    let (outcome, (stored_size, created)) = match (missing, uploaded) {
        // The local backend checks for itself whether it has the content (and
        // dedups a copy this add already stored, recording its path)
        (None, _) => {
            let (outcome, res) = pending.metadata.save_reporting(
                operation_id,
                &full_path,
                backend,
                paths,
                &pending.relative_path,
                on_bytes,
            )?;
            (outcome, res.map_or((None, false), stored))
        }
        // This add uploaded the content already, and the server completes
        // this entry with it: uploading again would move the bytes twice
        (Some(_), Some(size)) => {
            let outcome = if pending.in_sync(paths) {
                Outcome::Present
            } else {
                pending.metadata.write(paths, &pending.relative_path)?;
                Outcome::Copied
            };
            (outcome, (Some(size), false))
        }
        (Some(missing), None) if missing.contains(&pending.metadata.hashes.blake3) => {
            let res = pending.metadata.store_and_write_reporting(
                operation_id,
                &full_path,
                backend,
                paths,
                &pending.relative_path,
                on_bytes,
            )?;
            (Outcome::Copied, stored(res))
        }
        // The server has the content and recorded the add from the manifest
        (Some(_), None) if pending.in_sync(paths) => (Outcome::Present, (None, false)),
        (Some(_), None) => {
            pending.metadata.write(paths, &pending.relative_path)?;
            (Outcome::Copied, (None, false))
        }
    };
    Ok(Added {
        outcome,
        stored_size,
        created,
    })
}

/// Whether [`add_file`] (not in a dry run) will make a store call, i.e. may
/// move bytes. Mirrors its branches: the local backend stores unless the file
/// is in storage and its record already says exactly this (as
/// `save_reporting` checks); the server only gets the content it said it is
/// missing; content this add already uploaded is never stored again.
fn may_move_bytes(
    pending: &Pending,
    paths: &DvsPaths,
    missing: Option<&HashSet<String>>,
    uploaded: Option<u64>,
) -> bool {
    if uploaded.is_some() {
        return false;
    }
    match missing {
        None => !(pending.stored && pending.in_sync(paths)),
        Some(missing) => missing.contains(&pending.metadata.hashes.blake3),
    }
}

/// Adds files matching a glob pattern to DVS.
///
/// The pattern is matched against files relative to cwd.
/// Files are stored with paths relative to repo_root.
///
/// Reports to `ctx`'s observer as it goes: phases (only those in which work
/// happens, each finished also on an error exit), the plan (once storage has
/// been asked what it has, and always before any row), each transfer, and
/// each row as soon as it is final. Once `ctx` is cancelled no new file
/// starts: files in flight finish (upload and record together), the rest get
/// a `cancelled` row and no record, except files whose add the server already
/// recorded or whose content this add already uploaded (see [`finish_for`]).
pub fn add_files(
    files: Vec<PathBuf>,
    paths: &DvsPaths,
    backend: &Backend,
    message: Option<String>,
    compression: Compression,
    dry_run: bool,
    ctx: &OperationContext,
) -> Result<AddReport> {
    let observer = ctx.observer();
    let scan = PhaseGuard::start(observer, Phase::Scan);
    let matched_paths = paths.validate_for_add(&files);
    if matched_paths.is_empty() {
        scan.finish();
        // Still planned, once: nothing to add, and no rows follow
        observer.planned(&OperationPlan::default());
        return Ok(AddReport {
            files: Vec::new(),
            bytes: ByteTotals::default(),
            cancelled: ctx.is_cancelled(),
        });
    }

    // If any input is invalid we just bail instead of doing partial execution
    let invalid = matched_paths
        .iter()
        .filter_map(|(path, status)| {
            status
                .reason()
                .map(|reason| format!("  {}: {reason}", path.display()))
        })
        .collect::<Vec<_>>();
    if !invalid.is_empty() {
        bail!(
            "Refusing to add, the following paths are invalid:\n{}",
            invalid.join("\n")
        );
    }

    let valid_paths = matched_paths
        .into_iter()
        .map(|(path, _)| path)
        .collect::<Vec<_>>();

    // Fail fast on auth/permission problems before doing any work. Part of the
    // scan so that it runs before hashing, which can take long
    backend.check_access()?;
    scan.finish();

    let files_total = valid_paths.len() as u64;
    let pool = get_threadpool(valid_paths.len())?;
    let cache = try_open_cache(paths);
    let operation_id = Uuid::new_v4();

    // Rows final before the plan, reported only once it is, so that `planned`
    // precedes every row. `None`: size unknown.
    let mut results: Vec<(AddResult, Option<RowBytes>)>;

    // 1. Hash everything first: the server wants the whole list before any bytes move
    let hashing = LazyPhase::new(observer, Phase::Hash);
    let mut pending: Vec<Pending>;
    (pending, results) = pool.install(|| {
        valid_paths.into_par_iter().partition_map(|relative_path| {
            if ctx.is_cancelled() {
                return Either::Right((cancelled_row(relative_path, None), None));
            }
            hashing.begin();
            let start = Instant::now();
            let full_path = paths.file_path(&relative_path);
            let rel_str = relative_path.to_string_lossy();
            match cache::hashes_for_file(&full_path, &rel_str, cache.as_ref()) {
                Ok((hashes, size)) => Either::Left(Pending {
                    metadata: FileMetadata::from_hashes(hashes, size, compression, message.clone()),
                    relative_path,
                    stored: false,
                    work: start.elapsed(),
                }),
                Err(e) => {
                    Either::Right((error_row(relative_path, &e, Some(duration_ms(start))), None))
                }
            }
        })
    });
    hashing.finish();

    // 2. Ask storage what it already has: the server through the manifest (minus
    // files already in sync), other backends and dry runs file by file. The
    // server records every manifest entry, so a cancel before this sends none
    let mut missing = None;
    let mut storage_asked = false;
    if !ctx.is_cancelled() {
        match backend {
            Backend::Server(server) if !dry_run => {
                let entries = pending
                    .iter()
                    .filter(|p| !p.in_sync(paths))
                    .map(Pending::manifest_entry)
                    .collect::<Result<Vec<_>>>()?;
                let checkout = crate::git::current_checkout(paths.repo_root());
                let mut answer_missing = HashSet::new();
                // Everything in sync: no manifest, so no storage check either
                if !entries.is_empty() {
                    let _check = PhaseGuard::start(observer, Phase::CheckStorage);
                    let answer = server.manifest(&UploadManifest {
                        operation_id,
                        message: message.clone(),
                        git_branch: checkout.branch.clone(),
                        git_commit: checkout.commit.clone(),
                        entries,
                    })?;
                    // The metadata files record the project's compression, not the one asked for
                    for p in &mut pending {
                        p.metadata.compression = answer.compression;
                    }
                    answer_missing = answer.missing.into_iter().collect();
                }
                for p in &mut pending {
                    // In sync files were not in the manifest, but there is
                    // nothing left to do for them either
                    p.stored = !answer_missing.contains(&p.metadata.hashes.blake3);
                }
                missing = Some(answer_missing);
            }
            _ => {
                let check = LazyPhase::new(observer, Phase::CheckStorage);
                let (checked, unchecked): (Vec<Pending>, Vec<_>) = pool.install(|| {
                    pending.into_par_iter().partition_map(|mut p| {
                        if ctx.is_cancelled() {
                            let bytes = p.row_bytes(false);
                            let row = cancelled_row(p.relative_path, Some(p.work));
                            return Either::Right((row, Some(bytes)));
                        }
                        check.begin();
                        let start = Instant::now();
                        match backend.exists(&p.metadata.hashes) {
                            Ok(stored) => {
                                p.stored = stored;
                                p.work += start.elapsed();
                                Either::Left(p)
                            }
                            Err(e) => {
                                let bytes = p.row_bytes(false);
                                let work = p.work + start.elapsed();
                                let row = error_row(p.relative_path, &e, Some(millis(work)));
                                Either::Right((row, Some(bytes)))
                            }
                        }
                    })
                });
                check.finish();
                pending = checked;
                results.extend(unchecked);
            }
        }
        storage_asked = true;
    }

    // Nothing is known to need uploading until storage has been asked
    let to_transfer = pending.iter().filter(|p| storage_asked && !p.stored);
    let known_sizes = results.iter().filter_map(|(_, bytes)| bytes.as_ref());
    observer.planned(&OperationPlan {
        files_total,
        bytes_total: pending.iter().map(|p| p.metadata.size).sum::<u64>()
            + known_sizes.map(|b| b.size).sum::<u64>(),
        files_to_transfer: to_transfer.clone().count() as u64,
        bytes_to_transfer: to_transfer.map(|p| p.metadata.size).sum(),
    });
    for (row, _) in &results {
        observer.file_finished(FileRow::Add(row));
    }

    // 3. Store what is missing and write the metadata files. A second file
    // with content this add already uploads waits until that upload is done,
    // so it can finish from it (see `finish_for`) rather than upload the same
    // bytes again or, after a cancel, leave the server listing an add the
    // client never recorded.
    let transfer = LazyPhase::new(observer, Phase::Transfer);
    // Content uploaded by this add: hash -> stored size
    let uploaded: Mutex<HashMap<String, u64>> = Mutex::new(HashMap::new());
    let finish_one = |mut pending: Pending| -> (AddResult, Option<RowBytes>) {
        let hash = pending.metadata.hashes.blake3.clone();
        let uploaded_size = uploaded.lock().unwrap().get(&hash).copied();
        let finish = finish_for(
            ctx.is_cancelled(),
            &hash,
            missing.as_ref(),
            uploaded_size.is_some(),
        );
        if finish == Finish::Cancel {
            let bytes = pending.row_bytes(false);
            let row = cancelled_row(pending.relative_path, Some(pending.work));
            observer.file_finished(FileRow::Add(&row));
            return (row, Some(bytes));
        }
        // Only a store call can move bytes: a file already in sync, a record
        // the server already has and a repeat of content this add uploaded
        // are no transfer work
        if !dry_run && may_move_bytes(&pending, paths, missing.as_ref(), uploaded_size) {
            transfer.begin();
        }
        let start = Instant::now();
        let file_progress =
            observer.transfer_started(&pending.relative_path, pending.metadata.size);
        let on_bytes = file_progress.as_ref().map(|fp| &*fp.on_bytes);
        let added = add_file(
            &mut pending,
            paths,
            backend,
            operation_id,
            missing.as_ref(),
            uploaded_size,
            dry_run,
            on_bytes,
        );
        pending.work += start.elapsed();
        let (result, bytes) = match added {
            Ok(added) => {
                log::info!(
                    "Successfully added {} ({:?})",
                    pending.relative_path.display(),
                    added.outcome
                );
                if let Some(stored_size) = added.stored_size {
                    uploaded
                        .lock()
                        .unwrap()
                        .entry(hash.clone())
                        .or_insert(stored_size);
                }
                // Storing content that was already there moves no bytes
                let bytes = pending.row_bytes(added.created);
                let result = AddResult {
                    path: pending.relative_path,
                    detail: AddDetail::Success {
                        outcome: added.outcome,
                        hash,
                        size: pending.metadata.size,
                        stored_size: added.stored_size.unwrap_or(pending.metadata.size),
                    },
                    duration_ms: Some(millis(pending.work)),
                };
                (result, bytes)
            }
            Err(e) => {
                let bytes = pending.row_bytes(false);
                let row = error_row(pending.relative_path, &e, Some(millis(pending.work)));
                (row, bytes)
            }
        };
        if let Some(fp) = &file_progress {
            (fp.on_done)(matches!(result.detail, AddDetail::Success { .. }));
        }
        observer.file_finished(FileRow::Add(&result));
        (result, Some(bytes))
    };
    let mut hashes_seen = HashSet::new();
    let (firsts, repeats): (Vec<Pending>, Vec<Pending>) = pending
        .into_iter()
        .partition(|p| hashes_seen.insert(p.metadata.hashes.blake3.clone()));
    for batch in [firsts, repeats] {
        let finished = pool.install(|| batch.into_par_iter().map(&finish_one).collect::<Vec<_>>());
        results.extend(finished);
    }
    transfer.finish();

    let mut bytes = ByteTotals::default();
    let mut results: Vec<AddResult> = results
        .into_iter()
        .map(|(result, row_bytes)| {
            if let Some(row) = &row_bytes {
                bytes.count(row.size, row.stored, row.transferred);
            }
            result
        })
        .collect();
    results.sort_by(|a, b| a.path.cmp(&b.path));

    let successful_paths: Vec<_> = results
        .iter()
        .filter(|r| matches!(r.detail, AddDetail::Success { .. }))
        .map(|r| r.path.clone())
        .collect();
    if !dry_run && !successful_paths.is_empty() {
        let _record = PhaseGuard::start(observer, Phase::Record);
        // Successfully added bytes are what dvs last placed at these paths, so a
        // later `get` may replace them without asking (see files/get.rs).
        for r in &results {
            if let AddDetail::Success { hash, size, .. } = &r.detail {
                cache::record_baseline(cache.as_ref(), &r.path.to_string_lossy(), hash, *size);
            }
        }
        if let Err(e) = add_to_gitignore(paths.repo_root(), &successful_paths) {
            log::warn!("Failed to update .gitignore: {e}");
        }
    }

    Ok(AddReport {
        files: results,
        bytes,
        cancelled: ctx.is_cancelled(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::operation::{CancelToken, TransferObserver};
    use crate::progress::FileProgress;
    use crate::testutil::{Recorder, Seen, create_file, create_temp_git_repo, init_dvs_repo};
    use fs_err as fs;
    use std::path::Path;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};

    #[test]
    fn row_errors_carry_a_code() {
        let backend = anyhow::Error::from(crate::BackendError::new(ErrorCode::AccessDenied, "no"));
        let io = anyhow::Error::from(std::io::Error::other("disk"));
        for (error, expected) in [
            (backend, AddErrorCode::AccessDenied),
            (io, AddErrorCode::IoError),
            (anyhow::anyhow!("other"), AddErrorCode::OperationFailed),
        ] {
            assert_eq!(add_error_code(&error), expected);
        }
    }

    /// A local-storage project: its temp dir guard, root, config and paths.
    fn project() -> (tempfile::TempDir, PathBuf, crate::config::Config, DvsPaths) {
        let (tmp, root) = create_temp_git_repo();
        let (config, _dvs_dir) = init_dvs_repo(&root);
        let paths = DvsPaths::new(root.clone(), root.clone(), config.metadata_folder_name());
        (tmp, root, config, paths.unwrap())
    }

    #[test]
    fn add_files_aborts_batch_when_any_file_missing() {
        let (_tmp, root, config, paths) = project();
        let backend = config.backend();

        // One valid file alongside a missing one.
        create_file(&root, "a.txt", b"a");

        let err = add_files(
            vec!["a.txt".into(), "missing.csv".into()],
            &paths,
            backend,
            None,
            Compression::Zstd,
            false,
            &OperationContext::new(),
        )
        .unwrap_err()
        .to_string();
        assert!(err.contains("missing.csv"), "unexpected error: {err}");

        // No partial success: the valid file must not have been added.
        assert!(
            !paths.metadata_path(Path::new("a.txt")).exists(),
            "no files should be added when one is missing"
        );
    }

    #[test]
    fn add_files_aborts_when_path_outside_or_directory() {
        let (_tmp, root, config, paths) = project();
        let backend = config.backend();

        // Valid file
        create_file(&root, "a.txt", b"a");

        // Outside-project file: a sibling of the repo, outside its root.
        let outside_file = root.parent().unwrap().join("outside.txt");
        fs::write(&outside_file, b"outside").unwrap();
        let outside_relative = PathBuf::from("..").join("outside.txt");

        // Directory inside the repo
        fs::create_dir(root.join("subdir")).unwrap();

        // An outside-project path or a directory is invalid up front, so the
        // whole batch is refused rather than partially added.
        let err = add_files(
            vec!["a.txt".into(), outside_relative, "subdir".into()],
            &paths,
            backend,
            None,
            Compression::Zstd,
            false,
            &OperationContext::new(),
        )
        .unwrap_err()
        .to_string();
        assert!(err.contains("outside project"), "unexpected error: {err}");
        assert!(err.contains("directory"), "unexpected error: {err}");

        // No partial success: the valid file must not have been added.
        assert!(
            !paths.metadata_path(Path::new("a.txt")).exists(),
            "no files should be added when any path is invalid"
        );
    }

    #[test]
    fn add_files_invokes_on_done_for_each_file() {
        let (_tmp, root, config, paths) = project();
        let backend = config.backend();

        create_file(&root, "a.txt", b"a");
        create_file(&root, "b.txt", b"b");
        create_file(&root, "c.txt", b"c");

        let done_calls = Arc::new(AtomicUsize::new(0));
        let success_calls = Arc::new(AtomicUsize::new(0));
        let on_file_start = {
            let done_calls = Arc::clone(&done_calls);
            let success_calls = Arc::clone(&success_calls);
            move |_path: &Path, _size: u64| {
                let done_calls = Arc::clone(&done_calls);
                let success_calls = Arc::clone(&success_calls);
                FileProgress {
                    on_bytes: Box::new(|_| {}),
                    on_done: Box::new(move |success| {
                        done_calls.fetch_add(1, Ordering::SeqCst);
                        if success {
                            success_calls.fetch_add(1, Ordering::SeqCst);
                        }
                    }),
                }
            }
        };

        let results = add_files(
            vec!["a.txt".into(), "b.txt".into(), "c.txt".into()],
            &paths,
            backend,
            None,
            Compression::Zstd,
            false,
            &OperationContext::new().with_observer(Arc::new(TransferObserver(on_file_start))),
        )
        .unwrap()
        .files;

        assert_eq!(results.len(), 3);
        assert!(
            results
                .iter()
                .all(|r| matches!(r.detail, AddDetail::Success { .. }))
        );
        assert_eq!(done_calls.load(Ordering::SeqCst), 3);
        assert_eq!(success_calls.load(Ordering::SeqCst), 3);
    }

    fn add(
        names: &[&str],
        paths: &DvsPaths,
        backend: &Backend,
        dry_run: bool,
        ctx: &OperationContext,
    ) -> AddReport {
        add_files(
            names.iter().map(PathBuf::from).collect(),
            paths,
            backend,
            None,
            Compression::Zstd,
            dry_run,
            ctx,
        )
        .unwrap()
    }

    fn code(row: &AddResult) -> Option<AddErrorCode> {
        match row.detail {
            AddDetail::Error { code, .. } => Some(code),
            AddDetail::Success { .. } | AddDetail::Cancelled { .. } => None,
        }
    }

    fn content_hashes(content: &[u8]) -> crate::Hashes {
        let tmp = tempfile::NamedTempFile::new().unwrap();
        fs::write(tmp.path(), content).unwrap();
        crate::Hashes::compute_from_path(tmp.path(), &[]).unwrap().0
    }

    #[test]
    fn cancel_mid_add_finishes_in_flight_files_and_records_nothing_else() {
        let (_tmp, root, config, paths) = project();
        let backend = config.backend();
        // More files than the pool can have in flight, so some never start
        let names: Vec<String> = (0..48).map(|i| format!("f{i:02}.txt")).collect();
        for name in &names {
            create_file(&root, name, name.as_bytes());
        }
        let names: Vec<&str> = names.iter().map(String::as_str).collect();

        let token = CancelToken::new();
        let recorder = Arc::new(Recorder::cancelling_on_transfer(token.clone(), 1));
        let ctx = OperationContext::new()
            .with_cancel(token)
            .with_observer(recorder.clone());
        let report = add(&names, &paths, backend, false, &ctx);

        let transfers = recorder.transfers();
        let added: Vec<_> = report
            .files
            .iter()
            .filter(|r| r.state() == RowState::Succeeded)
            .collect();
        let cancelled: Vec<_> = report
            .files
            .iter()
            .filter(|r| r.state() == RowState::Cancelled)
            .collect();
        assert_eq!(report.summary().counts.cancelled, cancelled.len());
        assert_eq!(report.summary().outcome, crate::RunOutcome::Cancelled);
        assert_eq!(report.files.len(), 48);
        assert_eq!(added.len() + cancelled.len(), 48, "{:?}", report.files);
        // Exactly the files in flight when the cancel came finished
        assert_eq!(added.len(), transfers.len());
        assert!(!cancelled.is_empty());
        for row in &added {
            assert!(transfers.contains(&row.path));
            assert!(paths.metadata_path(&row.path).is_file());
            let content = fs::read(paths.file_path(&row.path)).unwrap();
            assert!(backend.exists(&content_hashes(&content)).unwrap());
            assert!(row.duration_ms.is_some());
        }
        // Files that never started have neither a record nor a blob
        for row in &cancelled {
            assert!(!transfers.contains(&row.path));
            assert!(!paths.metadata_path(&row.path).exists());
            let content = fs::read(paths.file_path(&row.path)).unwrap();
            assert!(!backend.exists(&content_hashes(&content)).unwrap());
            // Hashed before the cancel came, so its hashing time is reported
            assert!(row.duration_ms.is_some());
        }
        // One file_finished per row, with the row's code
        let finished = recorder.finished();
        assert_eq!(finished.len(), 48);
        let mut finished_paths: Vec<_> = finished.iter().map(|(p, _, _)| p.clone()).collect();
        finished_paths.sort();
        finished_paths.dedup();
        assert_eq!(finished_paths.len(), 48);
        // Every file was hashed before the cancel, so every row counts in the
        // total, which is the plan's; only what was added was transferred
        let added_bytes: u64 = added
            .iter()
            .map(|r| match r.detail {
                AddDetail::Success { size, .. } => size,
                _ => unreachable!(),
            })
            .sum();
        let all_bytes: u64 = names.iter().map(|n| n.len() as u64).sum();
        assert_eq!(recorder.plan().bytes_total, all_bytes);
        assert_eq!(report.bytes.total, all_bytes);
        assert_eq!(
            report.bytes.to_transfer + report.bytes.already_stored,
            report.bytes.total
        );
        assert_eq!(report.bytes.to_transfer, all_bytes);
        assert_eq!(report.bytes.transferred, added_bytes);
    }

    #[test]
    fn identical_new_files_are_transferred_once() {
        let (_tmp, root, config, paths) = project();
        let backend = config.backend();
        create_file(&root, "a.txt", b"same content");
        create_file(&root, "b.txt", b"same content");

        let report = add(
            &["a.txt", "b.txt"],
            &paths,
            backend,
            false,
            &OperationContext::new(),
        );
        assert!(
            report
                .files
                .iter()
                .all(|r| r.state() == RowState::Succeeded)
        );
        assert!(paths.metadata_path(Path::new("b.txt")).is_file());
        // Both needed uploading, but the bytes moved once
        assert_eq!(
            report.bytes,
            ByteTotals {
                total: 24,
                to_transfer: 24,
                already_stored: 0,
                transferred: 12,
            }
        );
    }

    #[test]
    fn a_copy_of_content_uploaded_before_a_cancel_is_still_recorded() {
        let (_tmp, root, config, paths) = project();
        let backend = config.backend();
        create_file(&root, "a.txt", b"same content");
        create_file(&root, "b.txt", b"same content");

        // Cancelled as the first upload starts, before the copy has started
        let token = CancelToken::new();
        let recorder = Arc::new(Recorder::cancelling_on_transfer(token.clone(), 1));
        let ctx = OperationContext::new()
            .with_cancel(token)
            .with_observer(recorder.clone());
        let report = add(&["a.txt", "b.txt"], &paths, backend, false, &ctx);

        assert!(ctx.is_cancelled());
        assert!(
            report
                .files
                .iter()
                .all(|r| r.state() == RowState::Succeeded),
            "{:?}",
            report.files
        );
        for name in ["a.txt", "b.txt"] {
            assert!(paths.metadata_path(Path::new(name)).is_file());
        }
        assert_eq!(report.bytes.transferred, 12);
    }

    #[test]
    #[cfg(unix)]
    fn nothing_hashed_means_no_storage_check() {
        use std::os::unix::fs::PermissionsExt;
        let (_tmp, root, config, paths) = project();
        let backend = config.backend();
        let locked = create_file(&root, "locked.txt", b"secret");
        fs::set_permissions(&locked, std::fs::Permissions::from_mode(0o000)).unwrap();
        if fs::read(&locked).is_ok() {
            // Running as root: permissions do not stop reads
            return;
        }
        let recorder = Arc::new(Recorder::default());
        add(
            &["locked.txt"],
            &paths,
            backend,
            false,
            &OperationContext::new().with_observer(recorder.clone()),
        );
        fs::set_permissions(&locked, std::fs::Permissions::from_mode(0o644)).unwrap();
        assert_eq!(
            recorder.outline(),
            vec![
                Seen::PhaseStarted(Phase::Scan),
                Seen::PhaseFinished(Phase::Scan),
                Seen::PhaseStarted(Phase::Hash),
                Seen::PhaseFinished(Phase::Hash),
                Seen::Planned(OperationPlan {
                    files_total: 1,
                    ..OperationPlan::default()
                }),
            ]
        );
    }

    /// The plan is reported once, before every one of `rows` rows.
    fn assert_planned_before_rows(recorder: &Recorder, rows: usize) {
        let seen = recorder.seen();
        let planned: Vec<_> = seen
            .iter()
            .enumerate()
            .filter(|(_, s)| matches!(s, Seen::Planned(_)))
            .map(|(i, _)| i)
            .collect();
        assert_eq!(planned.len(), 1, "{seen:?}");
        let finished: Vec<_> = seen
            .iter()
            .enumerate()
            .filter(|(_, s)| matches!(s, Seen::Finished { .. }))
            .map(|(i, _)| i)
            .collect();
        assert_eq!(finished.len(), rows, "{seen:?}");
        assert!(finished.iter().all(|&i| i > planned[0]), "{seen:?}");
    }

    #[test]
    fn add_reports_phases_and_plan_in_order() {
        let (_tmp, root, config, paths) = project();
        let backend = config.backend();
        create_file(&root, "a.txt", b"aaaa");
        create_file(&root, "b.txt", b"bb");

        let plan = OperationPlan {
            files_total: 2,
            bytes_total: 6,
            files_to_transfer: 2,
            bytes_to_transfer: 6,
        };
        let dry = Arc::new(Recorder::default());
        add(
            &["a.txt", "b.txt"],
            &paths,
            backend,
            true,
            &OperationContext::new().with_observer(dry.clone()),
        );
        // A dry run moves nothing and records nothing
        assert_eq!(
            dry.outline(),
            vec![
                Seen::PhaseStarted(Phase::Scan),
                Seen::PhaseFinished(Phase::Scan),
                Seen::PhaseStarted(Phase::Hash),
                Seen::PhaseFinished(Phase::Hash),
                Seen::PhaseStarted(Phase::CheckStorage),
                Seen::PhaseFinished(Phase::CheckStorage),
                Seen::Planned(plan),
            ]
        );

        let real = Arc::new(Recorder::default());
        add(
            &["a.txt", "b.txt"],
            &paths,
            backend,
            false,
            &OperationContext::new().with_observer(real.clone()),
        );
        assert_eq!(
            real.outline(),
            vec![
                Seen::PhaseStarted(Phase::Scan),
                Seen::PhaseFinished(Phase::Scan),
                Seen::PhaseStarted(Phase::Hash),
                Seen::PhaseFinished(Phase::Hash),
                Seen::PhaseStarted(Phase::CheckStorage),
                Seen::PhaseFinished(Phase::CheckStorage),
                Seen::Planned(plan),
                Seen::PhaseStarted(Phase::Transfer),
                Seen::PhaseFinished(Phase::Transfer),
                Seen::PhaseStarted(Phase::Record),
                Seen::PhaseFinished(Phase::Record),
            ]
        );
        // Every row finished after the plan, once, with its time
        let seen = real.seen();
        let planned_at = seen.iter().position(|s| *s == Seen::Planned(plan)).unwrap();
        let finished = real.finished();
        assert_eq!(finished.len(), 2);
        assert!(
            finished
                .iter()
                .all(|(_, code, ms)| code.is_none() && ms.is_some())
        );
        assert!(
            seen.iter()
                .position(|s| matches!(s, Seen::Finished { .. }))
                .unwrap()
                > planned_at
        );

        // Nothing left to transfer the second time
        let again = Arc::new(Recorder::default());
        add(
            &["a.txt", "b.txt"],
            &paths,
            backend,
            true,
            &OperationContext::new().with_observer(again.clone()),
        );
        assert!(again.outline().contains(&Seen::Planned(OperationPlan {
            files_to_transfer: 0,
            bytes_to_transfer: 0,
            ..plan
        })));
    }

    #[test]
    fn transfer_phase_only_when_bytes_may_move() {
        let (_tmp, root, config, paths) = project();
        let backend = config.backend();
        create_file(&root, "a.txt", b"aaaa");
        create_file(&root, "b.txt", b"bb");
        add(
            &["a.txt", "b.txt"],
            &paths,
            backend,
            false,
            &OperationContext::new(),
        );

        // Everything in sync: no store call, so no transfer phase
        let in_sync = Arc::new(Recorder::default());
        let report = add(
            &["a.txt", "b.txt"],
            &paths,
            backend,
            false,
            &OperationContext::new().with_observer(in_sync.clone()),
        );
        assert!(report.files.iter().all(|r| matches!(
            r.detail,
            AddDetail::Success {
                outcome: Outcome::Present,
                ..
            }
        )));
        assert!(
            !in_sync
                .outline()
                .contains(&Seen::PhaseStarted(Phase::Transfer)),
            "{:?}",
            in_sync.outline()
        );

        // A new file is stored, so the phase happens
        create_file(&root, "c.txt", b"c");
        let with_new = Arc::new(Recorder::default());
        add(
            &["a.txt", "c.txt"],
            &paths,
            backend,
            false,
            &OperationContext::new().with_observer(with_new.clone()),
        );
        let outline = with_new.outline();
        assert!(
            outline.contains(&Seen::PhaseStarted(Phase::Transfer)),
            "{outline:?}"
        );
        assert!(
            outline.contains(&Seen::PhaseFinished(Phase::Transfer)),
            "{outline:?}"
        );
    }

    #[test]
    #[cfg(unix)]
    fn unreadable_file_gets_a_timed_error_row() {
        use std::os::unix::fs::PermissionsExt;
        let (_tmp, root, config, paths) = project();
        let backend = config.backend();
        create_file(&root, "ok.txt", b"ok");
        let locked = create_file(&root, "locked.txt", b"secret");
        fs::set_permissions(&locked, std::fs::Permissions::from_mode(0o000)).unwrap();
        if fs::read(&locked).is_ok() {
            // Running as root: permissions do not stop reads
            return;
        }

        let recorder = Arc::new(Recorder::default());
        let report = add(
            &["locked.txt", "ok.txt"],
            &paths,
            backend,
            false,
            &OperationContext::new().with_observer(recorder.clone()),
        );
        fs::set_permissions(&locked, std::fs::Permissions::from_mode(0o644)).unwrap();

        assert_eq!(code(&report.files[0]), Some(AddErrorCode::IoError));
        assert!(report.files[0].duration_ms.is_some());
        assert_eq!(code(&report.files[1]), None);
        // The hash error is reported after the plan, like every row
        assert_planned_before_rows(&recorder, 2);
        // The failed row has no known size, so it counts in no total
        assert_eq!(report.bytes.total, 2);
        assert_eq!(recorder.plan().bytes_total, 2);
    }
}
