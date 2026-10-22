use std::collections::HashSet;
use std::path::PathBuf;

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
use crate::operation::ByteTotals;
use crate::paths::{DvsPaths, ProjectPath};
use crate::progress::OnFileStart;
use crate::utils::get_threadpool;
use crate::{Outcome, cache};

/// Result of adding a single file.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AddResult {
    pub path: PathBuf,
    #[serde(flatten)]
    pub detail: AddDetail,
}

/// The whole result of an `add`: one row per file, sorted by path, plus byte
/// totals.
#[derive(Debug, Clone)]
pub struct AddReport {
    pub files: Vec<AddResult>,
    /// Sums over the rows whose size is known (every file that was hashed,
    /// failed ones included); see [`ByteTotals`]. Not serialized by the
    /// library.
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

/// A file to add: the metadata it will get, waiting on the backend.
struct Pending {
    relative_path: PathBuf,
    metadata: FileMetadata,
    /// Whether storage already has this content, so nothing needs uploading.
    /// Known once storage has been asked (step 2); false until then.
    stored: bool,
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

    /// What this file contributes to the byte totals however it ends: once
    /// hashed its size is known, so a later failure still counts it (in
    /// `to_transfer` unless storage said it has the content).
    fn row_bytes(&self, transferred: bool) -> RowBytes {
        RowBytes {
            size: self.metadata.size,
            stored: self.stored,
            transferred,
        }
    }
}

/// What a finished file contributes to the result's [`ByteTotals`].
struct RowBytes {
    size: u64,
    /// The content was already in storage when the add was planned
    stored: bool,
    /// This file's bytes were uploaded by this add: the backend created the
    /// blob. A second copy of content this add already uploaded moves nothing.
    transferred: bool,
}

/// What finishing one file did.
struct Added {
    outcome: Outcome,
    /// Size of the stored blob, when a store call reported it
    stored_size: Option<u64>,
    /// Whether the backend created the blob, i.e. this file's bytes moved
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

/// Adds files matching a glob pattern to DVS.
///
/// The pattern is matched against files relative to cwd.
/// Files are stored with paths relative to repo_root.
pub fn add_files(
    files: Vec<PathBuf>,
    paths: &DvsPaths,
    backend: &Backend,
    message: Option<String>,
    compression: Compression,
    dry_run: bool,
    on_file_start: Option<&OnFileStart>,
) -> Result<AddReport> {
    let matched_paths = paths.validate_for_add(&files);
    if matched_paths.is_empty() {
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

    // Fail fast on auth/permission problems before doing any work
    backend.check_access()?;

    let pool = get_threadpool(valid_paths.len())?;
    let cache = try_open_cache(paths);
    let operation_id = Uuid::new_v4();

    // Rows with their byte contribution (`None`: size unknown)
    let mut results: Vec<(AddResult, Option<RowBytes>)>;

    // 1. Hash everything first: the server wants the whole list before any bytes move
    let mut pending: Vec<Pending>;
    (pending, results) = pool.install(|| {
        valid_paths.into_par_iter().partition_map(|relative_path| {
            let full_path = paths.file_path(&relative_path);
            let rel_str = relative_path.to_string_lossy();
            match cache::hashes_for_file(&full_path, &rel_str, cache.as_ref()) {
                Ok((hashes, size)) => Either::Left(Pending {
                    metadata: FileMetadata::from_hashes(hashes, size, compression, message.clone()),
                    relative_path,
                    stored: false,
                }),
                Err(e) => {
                    log::warn!("Failed to add {}: {e}", relative_path.display());
                    let row = AddResult {
                        path: relative_path,
                        detail: AddDetail::Error {
                            code: add_error_code(&e),
                            error: e.to_string(),
                        },
                    };
                    Either::Right((row, None))
                }
            }
        })
    });

    // 2. Ask storage what it already has. The server is told what we are about
    // to add (minus files already in sync) and answers with what it is missing.
    // Other backends, and dry runs, are asked file by file.
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
            if !entries.is_empty() {
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
            let (checked, unchecked): (Vec<Pending>, Vec<_>) = pool.install(|| {
                pending.into_par_iter().partition_map(|mut p| {
                    match backend.exists(&p.metadata.hashes) {
                        Ok(stored) => {
                            p.stored = stored;
                            Either::Left(p)
                        }
                        Err(e) => {
                            log::warn!("Failed to add {}: {e}", p.relative_path.display());
                            let bytes = p.row_bytes(false);
                            let row = AddResult {
                                path: p.relative_path,
                                detail: AddDetail::Error {
                                    code: add_error_code(&e),
                                    error: e.to_string(),
                                },
                            };
                            Either::Right((row, Some(bytes)))
                        }
                    }
                })
            });
            pending = checked;
            results.extend(unchecked);
        }
    }

    // 3. Store what is missing and write the metadata files
    results.extend(pool.install(|| {
        pending
            .into_par_iter()
            .map(|mut pending| {
                let file_progress =
                    on_file_start.map(|f| f(&pending.relative_path, pending.metadata.size));
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
                        };
                        (result, bytes)
                    }
                    Err(e) => {
                        log::warn!("Failed to add {}: {e}", pending.relative_path.display());
                        let bytes = pending.row_bytes(false);
                        let result = AddResult {
                            path: pending.relative_path,
                            detail: AddDetail::Error {
                                code: add_error_code(&e),
                                error: e.to_string(),
                            },
                        };
                        (result, bytes)
                    }
                };
                if let Some(fp) = &file_progress {
                    (fp.on_done)(matches!(result.detail, AddDetail::Success { .. }));
                }
                (result, Some(bytes))
            })
            .collect::<Vec<_>>()
    }));

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
    if !dry_run {
        // Successfully added bytes are what dvs last placed at these paths, so a
        // later `get` may replace them without asking (see files/get.rs).
        for r in &results {
            if let AddDetail::Success { hash, size, .. } = &r.detail {
                cache::record_baseline(cache.as_ref(), &r.path.to_string_lossy(), hash, *size);
            }
        }
    }
    if !dry_run && !successful_paths.is_empty() {
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
    use crate::progress::FileProgress;
    use crate::testutil::{create_file, create_temp_git_repo, init_dvs_repo};
    use fs_err as fs;
    use std::path::Path;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};

    fn make_paths(root: &Path, config: &crate::config::Config) -> DvsPaths {
        DvsPaths::new(
            root.to_path_buf(),
            root.to_path_buf(),
            config.metadata_folder_name(),
        )
        .unwrap()
    }

    #[test]
    fn add_files_aborts_batch_when_any_file_missing() {
        let (_tmp, root) = create_temp_git_repo();
        let (config, _dvs_dir) = init_dvs_repo(&root);
        let backend = config.backend();
        let paths = make_paths(&root, &config);

        // One valid file alongside a missing one.
        create_file(&root, "a.txt", b"a");

        let err = add_files(
            vec!["a.txt".into(), "missing.csv".into()],
            &paths,
            backend,
            None,
            Compression::Zstd,
            false,
            None,
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
        let (_tmp, root) = create_temp_git_repo();
        let (config, _dvs_dir) = init_dvs_repo(&root);
        let backend = config.backend();
        let paths = make_paths(&root, &config);

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
            None,
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
        let (_tmp, root) = create_temp_git_repo();
        let (config, _dvs_dir) = init_dvs_repo(&root);
        let backend = config.backend();
        let paths = make_paths(&root, &config);

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
            Some(&on_file_start),
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

    /// A local-storage project: its temp dir guard, root, config and paths.
    fn project() -> (tempfile::TempDir, PathBuf, crate::config::Config, DvsPaths) {
        let (tmp, root) = create_temp_git_repo();
        let (config, _dvs_dir) = init_dvs_repo(&root);
        let paths = make_paths(&root, &config);
        (tmp, root, config, paths)
    }

    fn add(names: &[&str], paths: &DvsPaths, backend: &Backend, dry_run: bool) -> AddReport {
        add_files(
            names.iter().map(PathBuf::from).collect(),
            paths,
            backend,
            None,
            Compression::Zstd,
            dry_run,
            None,
        )
        .unwrap()
    }

    fn row_size_sum(report: &AddReport) -> u64 {
        report
            .files
            .iter()
            .filter_map(|r| match r.detail {
                AddDetail::Success { size, .. } => Some(size),
                AddDetail::Error { .. } => None,
            })
            .sum()
    }

    // C8: the totals are the per-row sums in a dry run and a real one; only
    // the real one moves bytes, and only the content storage lacked.
    #[test]
    fn byte_totals_match_rows_in_dry_and_real_runs() {
        let (_tmp, root, config, paths) = project();
        let backend = config.backend();
        create_file(&root, "old.txt", b"already stored");
        add(&["old.txt"], &paths, backend, false);
        // Same content at a new path: stored, so nothing to upload
        create_file(&root, "copy.txt", b"already stored");
        create_file(&root, "new.txt", b"new content!");
        let names = ["copy.txt", "new.txt", "old.txt"];

        let dry = add(&names, &paths, backend, true);
        assert_eq!(dry.bytes.total, row_size_sum(&dry));
        assert_eq!(
            dry.bytes,
            ByteTotals {
                total: 14 + 12 + 14,
                to_transfer: 12,
                already_stored: 28,
                transferred: 0,
            }
        );

        let real = add(&names, &paths, backend, false);
        assert_eq!(real.bytes.total, row_size_sum(&real));
        assert_eq!(
            real.bytes,
            ByteTotals {
                transferred: 12,
                ..dry.bytes
            }
        );
        assert_eq!(
            real.bytes.to_transfer + real.bytes.already_stored,
            real.bytes.total
        );
    }

    // C8 (K20): a file that fails after it was hashed still counts its size.
    #[test]
    #[cfg(unix)]
    fn a_file_that_fails_after_hashing_still_counts_its_size() {
        use std::os::unix::fs::PermissionsExt;
        let (_tmp, root, config, paths) = project();
        let backend = config.backend();
        create_file(&root, "a.txt", b"aaaa");
        // Storage refuses the blob
        let storage = root.parent().unwrap().join(".storage");
        fs::set_permissions(&storage, std::fs::Permissions::from_mode(0o555)).unwrap();
        if fs::write(storage.join("probe"), b"").is_ok() {
            // Running as root: permissions do not stop writes
            fs::set_permissions(&storage, std::fs::Permissions::from_mode(0o755)).unwrap();
            return;
        }
        let report = add(&["a.txt"], &paths, backend, false);
        fs::set_permissions(&storage, std::fs::Permissions::from_mode(0o755)).unwrap();

        assert!(
            matches!(report.files[0].detail, AddDetail::Error { .. }),
            "{:?}",
            report.files
        );
        assert_eq!(
            report.bytes,
            ByteTotals {
                total: 4,
                to_transfer: 4,
                already_stored: 0,
                transferred: 0,
            }
        );
    }

    // C8: a file that could not be hashed has no known size and counts in no
    // total.
    #[test]
    #[cfg(unix)]
    fn an_unreadable_file_counts_in_no_byte_total() {
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
        let report = add(&["locked.txt", "ok.txt"], &paths, backend, false);
        fs::set_permissions(&locked, std::fs::Permissions::from_mode(0o644)).unwrap();

        assert!(matches!(
            report.files[0].detail,
            AddDetail::Error {
                code: AddErrorCode::IoError,
                ..
            }
        ));
        assert!(matches!(report.files[1].detail, AddDetail::Success { .. }));
        assert_eq!(report.bytes.total, 2);
    }
}
