use std::collections::HashSet;
use std::path::PathBuf;
use std::time::{Duration, Instant};

use anyhow::{Result, anyhow};
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
    ByteTotals, Counts, FileRow, LazyPhase, OperationContext, OperationObserver, OperationPlan,
    OperationRow, Phase, PhaseGuard, RowState, RunOutcome, duration_ms, millis,
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
    /// hashing carries its work so far).
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

/// The rows of an `add`, sorted by path, plus counts, outcome and byte totals.
#[derive(Debug, Clone)]
pub struct AddReport {
    pub files: Vec<AddResult>,
    pub counts: Counts,
    pub outcome: RunOutcome,
    pub bytes: ByteTotals,
}

impl AddReport {
    fn new(files: Vec<AddResult>, bytes: ByteTotals, cancelled: bool) -> Self {
        let counts = Counts::of(&files);
        AddReport {
            outcome: RunOutcome::of(&counts, cancelled),
            files,
            counts,
            bytes,
        }
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
        code: ErrorCode,
        error: String,
    },
    Cancelled {
        message: String,
    },
}

fn add_error_code(error: &anyhow::Error) -> ErrorCode {
    if let Some(code) = backend_error_code(error) {
        code
    } else if error.downcast_ref::<std::io::Error>().is_some() {
        ErrorCode::IoError
    } else {
        ErrorCode::OperationFailed
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

/// Whether a file that has not started yet is cancelled. The server records
/// every manifest entry whose content it already has, so after a cancel those
/// files still write their local record, keeping the two in agreement.
fn cancelled_before_start(
    cancelled: bool,
    hash: &str,
    manifest_missing: Option<&HashSet<String>>,
) -> bool {
    cancelled && !manifest_missing.is_some_and(|missing| !missing.contains(hash))
}

struct Added {
    outcome: Outcome,
    stored_size: Option<u64>,
    /// The backend created the blob, i.e. this file's bytes moved
    created: bool,
}

/// `missing` is the server's answer to the manifest: the hashes to upload. The local
/// backend has no manifest and checks for itself. Only a store moves bytes, so
/// only it is reported as a transfer.
#[allow(clippy::too_many_arguments)]
fn add_file(
    pending: &mut Pending,
    paths: &DvsPaths,
    backend: &Backend,
    operation_id: Uuid,
    missing: Option<&HashSet<String>>,
    dry_run: bool,
    observer: &dyn OperationObserver,
    transfer: &LazyPhase<'_>,
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
    let store = match missing {
        None => !pending
            .metadata
            .is_saved(backend, paths, &pending.relative_path)?,
        Some(missing) => missing.contains(&pending.metadata.hashes.blake3),
    };
    if !store {
        // In sync, or the server has the content and recorded the add from the manifest
        let outcome = if missing.is_none() || pending.in_sync(paths) {
            Outcome::Present
        } else {
            pending.metadata.write(paths, &pending.relative_path)?;
            Outcome::Copied
        };
        return Ok(Added {
            outcome,
            stored_size: None,
            created: false,
        });
    }
    transfer.begin();
    let file_progress = observer.transfer_started(&pending.relative_path, pending.metadata.size);
    let res = pending.metadata.store_and_write(
        operation_id,
        &full_path,
        backend,
        paths,
        &pending.relative_path,
        file_progress.as_ref().map(|fp| &*fp.on_bytes),
    );
    if let Some(fp) = &file_progress {
        (fp.on_done)(res.is_ok());
    }
    let res = res?;
    Ok(Added {
        outcome: Outcome::Copied,
        stored_size: Some(res.stored_size),
        created: res.created,
    })
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
/// recorded (see [`cancelled_before_start`]). A run that fails after a cancel
/// still returns its (empty) cancelled report.
pub fn add_files(
    files: Vec<PathBuf>,
    paths: &DvsPaths,
    backend: &Backend,
    message: Option<String>,
    compression: Compression,
    dry_run: bool,
    ctx: &OperationContext,
) -> Result<AddReport> {
    match add_all(files, paths, backend, message, compression, dry_run, ctx) {
        Err(_) if ctx.is_cancelled() => Ok(AddReport::new(Vec::new(), ByteTotals::default(), true)),
        result => result,
    }
}

fn add_all(
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
        return Ok(AddReport::new(
            Vec::new(),
            ByteTotals::default(),
            ctx.is_cancelled(),
        ));
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
        return Err(crate::Error::new(
            ErrorCode::InvalidArguments,
            format!(
                "Refusing to add, the following paths are invalid:\n{}",
                invalid.join("\n")
            ),
        )
        .into());
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

    // 3. Store what is missing and write the metadata files
    let transfer = LazyPhase::new(observer, Phase::Transfer);
    let finish_one = |mut pending: Pending| -> (AddResult, Option<RowBytes>) {
        let hash = &pending.metadata.hashes.blake3;
        if cancelled_before_start(ctx.is_cancelled(), hash, missing.as_ref()) {
            let bytes = pending.row_bytes(false);
            let row = cancelled_row(pending.relative_path, Some(pending.work));
            observer.file_finished(FileRow::Add(&row));
            return (row, Some(bytes));
        }
        let start = Instant::now();
        let added = add_file(
            &mut pending,
            paths,
            backend,
            operation_id,
            missing.as_ref(),
            dry_run,
            observer,
            &transfer,
        );
        pending.work += start.elapsed();
        let (result, bytes) = match added {
            Ok(added) => {
                log::info!(
                    "Successfully added {} ({:?})",
                    pending.relative_path.display(),
                    added.outcome
                );
                // Storing content that was already there moves no bytes
                let bytes = pending.row_bytes(added.created);
                let result = AddResult {
                    path: pending.relative_path,
                    detail: AddDetail::Success {
                        outcome: added.outcome,
                        hash: pending.metadata.hashes.blake3,
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
        observer.file_finished(FileRow::Add(&result));
        (result, Some(bytes))
    };
    results.extend(pool.install(|| pending.into_par_iter().map(finish_one).collect::<Vec<_>>()));
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

    Ok(AddReport::new(results, bytes, ctx.is_cancelled()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::operation::{CancelToken, TransferObserver};
    use crate::progress::FileProgress;
    use crate::testutil::{Recorder, create_file, create_temp_git_repo, init_dvs_repo};
    use fs_err as fs;
    use std::path::Path;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};

    #[test]
    fn row_errors_carry_a_code() {
        let backend = anyhow::Error::from(crate::Error::new(ErrorCode::AccessDenied, "no"));
        let io = anyhow::Error::from(std::io::Error::other("disk"));
        for (error, expected) in [
            (backend, ErrorCode::AccessDenied),
            (io, ErrorCode::IoError),
            (anyhow::anyhow!("other"), ErrorCode::OperationFailed),
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
        .unwrap_err();
        assert_eq!(backend_error_code(&err), Some(ErrorCode::InvalidArguments));
        let err = err.to_string();
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
        ctx: &OperationContext,
    ) -> AddReport {
        add_files(
            names.iter().map(PathBuf::from).collect(),
            paths,
            backend,
            None,
            Compression::Zstd,
            false,
            ctx,
        )
        .unwrap()
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
        let report = add(&names, &paths, backend, &ctx);

        let seen = recorder.seen();
        let transfers: Vec<_> = seen
            .iter()
            .filter_map(|s| s.strip_prefix("transfer "))
            .map(PathBuf::from)
            .collect();
        let rows: HashSet<_> = seen.iter().filter(|s| s.starts_with("row ")).collect();
        let (added, cancelled): (Vec<_>, Vec<_>) = report
            .files
            .iter()
            .partition(|r| r.state() == RowState::Succeeded);
        assert_eq!(report.outcome, RunOutcome::Cancelled);
        assert_eq!(report.counts.cancelled, cancelled.len());
        assert!(cancelled.iter().all(|r| r.state() == RowState::Cancelled));
        // One row per file, reported once each
        assert_eq!(report.files.len(), 48);
        assert_eq!((rows.len(), recorder.rows.lock().unwrap().len()), (48, 48));
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
        // Files that never started have neither a record nor a blob, but were
        // hashed before the cancel came, so their hashing time is reported
        for row in &cancelled {
            assert!(!transfers.contains(&row.path));
            assert!(!paths.metadata_path(&row.path).exists());
            let content = fs::read(paths.file_path(&row.path)).unwrap();
            assert!(!backend.exists(&content_hashes(&content)).unwrap());
            assert!(row.duration_ms.is_some());
        }
        // Every file was hashed before the cancel, so every row counts in the
        // plan's total; only what was added was transferred
        let all_bytes: u64 = names.iter().map(|n| n.len() as u64).sum();
        let added_bytes: u64 = added.iter().map(|r| r.path.as_os_str().len() as u64).sum();
        assert!(seen.contains(&format!("plan 48 {all_bytes} 48 {all_bytes}")));
        assert_eq!(report.bytes.total, all_bytes);
        assert_eq!(report.bytes.to_transfer, all_bytes);
        assert_eq!(report.bytes.transferred, added_bytes);
    }

    #[test]
    fn a_run_that_fails_after_a_cancel_is_reported_cancelled() {
        let (_tmp, _root, config, paths) = project();
        let token = CancelToken::new();
        token.cancel();
        // A missing path refuses the batch
        let report = add_files(
            vec!["missing.csv".into()],
            &paths,
            config.backend(),
            None,
            Compression::Zstd,
            false,
            &OperationContext::new().with_cancel(token),
        )
        .unwrap();
        assert_eq!(report.outcome, RunOutcome::Cancelled);
        assert!(report.files.is_empty());
    }

    #[test]
    fn after_a_cancel_only_files_the_server_already_recorded_start() {
        let missing: HashSet<String> = ["new".to_string()].into();
        // The manifest recorded "old" (the server has it), but not "new"
        assert!(!cancelled_before_start(true, "old", Some(&missing)));
        assert!(cancelled_before_start(true, "new", Some(&missing)));
        // No manifest (local storage): every file not yet started is cancelled
        assert!(cancelled_before_start(true, "old", None));
        assert!(!cancelled_before_start(false, "new", Some(&missing)));
    }

    #[test]
    fn observer_hears_the_plan_then_transfers_of_stored_files_only_then_each_row() {
        let (_tmp, root, config, paths) = project();
        let backend = config.backend();
        create_file(&root, "a.txt", b"aaaa");
        add(&["a.txt"], &paths, backend, &OperationContext::new());
        create_file(&root, "c.txt", b"c");

        let recorder = Arc::new(Recorder::default());
        let ctx = OperationContext::new().with_observer(recorder.clone());
        let report = add(&["a.txt", "c.txt"], &paths, backend, &ctx);
        let seen = recorder.seen();
        let at = |line: &str| seen.iter().position(|s| s == line).unwrap();
        // a.txt is in sync: only c.txt is stored, so it alone is a transfer
        let plan = at("plan 2 5 1 1");
        let transfers: Vec<_> = seen.iter().filter(|s| s.starts_with("transfer")).collect();
        assert_eq!(transfers, ["transfer c.txt"]);
        assert!(plan < at("transfer c.txt") && at("transfer c.txt") < at("row c.txt"));
        assert!(plan < at("row a.txt"));
        assert_eq!(seen.iter().filter(|s| s.starts_with("row")).count(), 2);
        // The library serializes each row's time
        assert!(serde_json::to_value(&report.files[0]).unwrap()["duration_ms"].is_u64());
    }

    #[cfg(unix)]
    #[test]
    fn counts_include_failed_rows() {
        use std::os::unix::fs::PermissionsExt;
        let (_tmp, root, config, paths) = project();
        create_file(&root, "a.txt", b"a");
        let b = create_file(&root, "b.txt", b"b");
        fs::set_permissions(&b, std::fs::Permissions::from_mode(0o000)).unwrap();
        if fs::read(&b).is_ok() {
            return; // running as root: permissions are not enforced
        }

        let report = add(
            &["a.txt", "b.txt"],
            &paths,
            config.backend(),
            &OperationContext::new(),
        );
        let c = report.counts;
        assert_eq!((c.total, c.succeeded, c.failed), (2, 1, 1));
    }
}
