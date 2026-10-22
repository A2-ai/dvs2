use std::collections::HashSet;
use std::path::PathBuf;
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
    ByteTotals, FileRow, LazyPhase, OperationContext, OperationPlan, Phase, PhaseGuard,
    duration_ms, millis,
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
    /// no work. Not serialized by the library, so the R package's output is
    /// unchanged; the CLI adds it to its rows.
    #[serde(skip)]
    pub duration_ms: Option<u64>,
}

/// The rows of an `add`, sorted by path, plus byte totals.
#[derive(Debug, Clone)]
pub struct AddReport {
    pub files: Vec<AddResult>,
    pub bytes: ByteTotals,
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

    /// Once hashed the size is known, so a later failure still counts it.
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

struct Added {
    outcome: Outcome,
    stored_size: Option<u64>,
    /// The backend created the blob, i.e. this file's bytes moved
    created: bool,
}

/// `missing` is the server's answer to the manifest: the hashes to upload. The local
/// backend has no manifest and checks for itself.
fn add_file(
    pending: &mut Pending,
    paths: &DvsPaths,
    backend: &Backend,
    operation_id: Uuid,
    missing: Option<&HashSet<String>>,
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
    let (outcome, (stored_size, created)) = match missing {
        // The local backend checks for itself whether it has the content
        None => {
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
        Some(missing) if missing.contains(&pending.metadata.hashes.blake3) => {
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
        Some(_) if pending.in_sync(paths) => (Outcome::Present, (None, false)),
        Some(_) => {
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
/// missing.
fn may_move_bytes(pending: &Pending, paths: &DvsPaths, missing: Option<&HashSet<String>>) -> bool {
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
/// each row as soon as it is final.
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
    // files already in sync), other backends and dry runs file by file
    let mut missing = None;
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
                p.stored = !answer_missing.contains(&p.metadata.hashes.blake3);
            }
            missing = Some(answer_missing);
        }
        _ => {
            let check = LazyPhase::new(observer, Phase::CheckStorage);
            let (checked, unchecked): (Vec<Pending>, Vec<_>) = pool.install(|| {
                pending.into_par_iter().partition_map(|mut p| {
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

    let to_transfer = pending.iter().filter(|p| !p.stored);
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
        // Only a store call can move bytes: a file already in sync and a
        // record the server already has are no transfer work
        if !dry_run && may_move_bytes(&pending, paths, missing.as_ref()) {
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
        if let Some(fp) = &file_progress {
            (fp.on_done)(matches!(result.detail, AddDetail::Success { .. }));
        }
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

    Ok(AddReport {
        files: results,
        bytes,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::operation::TransferObserver;
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
            AddDetail::Success { .. } => None,
            AddDetail::Error { code, .. } => Some(code),
        }
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
