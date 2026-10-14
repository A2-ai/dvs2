use std::path::{Path, PathBuf};
use std::sync::Mutex;

use crate::api::v1::{BatchKind, CompleteBatch};
use crate::backends::Batch;
use crate::cache::{HashCache, try_open_cache};
use crate::errors::{DvsError, ErrorCode};
use crate::files::metadata::FileMetadata;
use crate::git::current_git_context;
use crate::paths::ProjectPath;
use crate::progress::OnFileStart;
use crate::utils::get_threadpool;
use crate::{Backend, Compression, DvsPaths, Outcome, RetrieveRequest, cache};
use anyhow::{Context, Result, anyhow, bail};
use fs_err as fs;
use rayon::prelude::*;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// A tracked file whose local copy is missing or different.
struct Needed {
    metadata: FileMetadata,
    path: ProjectPath,
}

/// Read the file's metadata: `None` when the local copy already matches.
fn check_file(
    paths: &DvsPaths,
    relative_path: &Path,
    cache: Option<&Mutex<HashCache>>,
) -> Result<Option<Needed>> {
    let dvs_file_path = paths.metadata_path(relative_path);
    if !dvs_file_path.is_file() {
        return Err(DvsError::new(
            ErrorCode::NotTracked,
            format!("File {} is not tracked by DVS", relative_path.display()),
        )
        .into());
    }
    let metadata = FileMetadata::read(&dvs_file_path)?;
    log::debug!(
        "Read metadata for {}: {}",
        relative_path.display(),
        metadata.hashes
    );

    let target_path = paths.file_path(relative_path);
    if target_path.is_file() {
        let rel_str = relative_path.to_string_lossy();
        let (hashes, size) = cache::hashes_for_file(&target_path, &rel_str, cache)?;
        if hashes == metadata.hashes && size == metadata.size {
            log::debug!(
                "File {} already present locally and matches",
                relative_path.display()
            );
            return Ok(None);
        }
    }
    let path = ProjectPath::from_path(relative_path).map_err(|e| anyhow!("{e}"))?;
    Ok(Some(Needed { metadata, path }))
}

/// Download one file into place: into a temporary file, verified, then renamed over
/// the target, so a failure leaves the local copy as it was.
fn fetch_file(
    backend: &Backend,
    batch: &Batch,
    paths: &DvsPaths,
    relative_path: &Path,
    needed: Needed,
    cache: Option<&Mutex<HashCache>>,
    on_bytes: Option<&(dyn Fn(u64) + Send + Sync)>,
) -> Result<u64> {
    let Needed { metadata, path } = needed;
    let target_path = paths.file_path(relative_path);
    let tmp_path = target_path.with_extension(format!("dvs-tmp.{}", Uuid::new_v4()));
    log::debug!(
        "Copying {} from storage to temp file {}",
        metadata.hashes,
        tmp_path.display()
    );

    let result = (|| {
        let retrieved = backend
            .fetch(
                batch,
                RetrieveRequest {
                    hashes: &metadata.hashes,
                    target: &tmp_path,
                    compression: metadata.compression,
                    path,
                    on_bytes,
                },
            )
            .with_context(|| format!("Failed to retrieve {}", relative_path.display()))?;
        if !retrieved {
            bail!("Storage file missing for hash: {}", metadata.hashes);
        }
        let actual = FileMetadata::from_file(&tmp_path, Compression::None, None)?;
        if actual.hashes != metadata.hashes {
            bail!("Retrieved file does not match expected hash");
        }
        fs::rename(&tmp_path, &target_path)?;
        Ok(actual)
    })();
    if result.is_err() {
        let _ = fs::remove_file(&tmp_path);
    }
    let actual = result?;

    // Store retrieved file's hashes in cache
    let rel_str = relative_path.to_string_lossy();
    if let Some(mtx) = cache {
        if let Ok(stat) = cache::FileStat::from_path(&target_path) {
            if let Err(e) = mtx.lock().unwrap().insert(&rel_str, &stat, &actual.hashes) {
                log::warn!("Cache store failed after get for {rel_str}: {e}");
            }
        }
    }

    Ok(metadata.size)
}

/// Result of getting a single file.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GetResult {
    pub path: PathBuf,
    #[serde(flatten)]
    pub detail: GetDetail,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum GetDetail {
    Success {
        outcome: Outcome,
        size: u64,
    },
    Error {
        error: String,
        /// The server's error code, when it gave one
        #[serde(default, skip_serializing_if = "Option::is_none")]
        code: Option<String>,
    },
}

impl GetDetail {
    fn error(e: &anyhow::Error) -> Self {
        GetDetail::Error {
            error: crate::errors::error_message(e),
            code: crate::errors::error_code(e).map(str::to_string),
        }
    }
}

/// Gets files matching a glob pattern from DVS storage.
///
/// The pattern is matched against tracked files (paths in metadata folder).
/// The pattern is adjusted based on cwd relative to repo root.
///
/// Files whose local copy already matches are not fetched. A download batch is
/// opened only when some file needs fetching, and closed when they are done.
pub fn get_files(
    files: Vec<PathBuf>,
    paths: &DvsPaths,
    backend: &Backend,
    dry_run: bool,
    on_file_start: Option<&OnFileStart>,
) -> Result<Vec<GetResult>> {
    let matched_paths = paths.validate_for_get(&files);
    if matched_paths.is_empty() {
        return Ok(Vec::new());
    }

    // Validate every path up front, we bail if any is invalid
    let invalid = matched_paths
        .iter()
        .filter_map(|(path, status)| {
            status
                .reason()
                .map(|reason| format!("  {}: {reason}", path.display()))
        })
        .collect::<Vec<_>>();
    if !invalid.is_empty() {
        return Err(DvsError::new(
            ErrorCode::NotTracked,
            format!(
                "Refusing to get, the following paths cannot be retrieved:\n{}",
                invalid.join("\n")
            ),
        )
        .into());
    }

    let tracked_paths = matched_paths
        .into_iter()
        .map(|(path, _)| path)
        .collect::<Vec<_>>();

    // Fail fast on auth/permission problems before retrieving anything,
    // so the user gets a single error instead of one per file.
    backend.check_access()?;

    let pool = get_threadpool(tracked_paths.len())?;
    let cache = try_open_cache(paths);

    let checked: Vec<_> = pool.install(|| {
        tracked_paths
            .into_par_iter()
            .map(|path| {
                let checked = check_file(paths, &path, cache.as_ref());
                (path, checked)
            })
            .collect()
    });

    let mut results = Vec::with_capacity(checked.len());
    let mut needed = Vec::new();
    for (path, checked) in checked {
        match checked {
            Ok(None) => results.push((
                path,
                GetDetail::Success {
                    outcome: Outcome::Present,
                    size: 0,
                },
            )),
            Ok(Some(n)) => needed.push((path, n)),
            Err(e) => {
                log::warn!("Failed to get {}: {e:#}", path.display());
                results.push((path, GetDetail::error(&e)));
            }
        }
    }
    // A present file reports the size its metadata records
    for (path, detail) in &mut results {
        if let GetDetail::Success { size, .. } = detail {
            if let Ok(m) = FileMetadata::read(&paths.metadata_path(path)) {
                *size = m.size;
            }
        }
    }
    let report = |results: &[(PathBuf, GetDetail)]| {
        if let Some(on_file_start) = on_file_start {
            for (path, detail) in results {
                let progress = on_file_start(path, 0);
                (progress.on_done)(matches!(detail, GetDetail::Success { .. }));
            }
        }
    };
    report(&results);

    if dry_run {
        let checked: Vec<_> = pool.install(|| {
            needed
                .into_par_iter()
                .map(|(path, n)| {
                    let detail = match backend.exists(&n.metadata.hashes) {
                        Ok(true) => GetDetail::Success {
                            outcome: Outcome::Copied,
                            size: n.metadata.size,
                        },
                        Ok(false) => GetDetail::error(&anyhow!(
                            "Storage file missing for hash: {}",
                            n.metadata.hashes
                        )),
                        Err(e) => GetDetail::error(&e),
                    };
                    (path, detail)
                })
                .collect()
        });
        report(&checked);
        results.extend(checked);
    } else if !needed.is_empty() {
        let git = current_git_context(paths.repo_root());
        let batch = backend.open_batch(BatchKind::Download, None, &git, Compression::default())?;
        let fetched: Vec<_> = pool.install(|| {
            needed
                .into_par_iter()
                .map(|(path, n)| {
                    let progress = on_file_start.map(|f| f(&path, n.metadata.size));
                    let on_bytes = progress.as_ref().map(|p| &*p.on_bytes);
                    let detail = match fetch_file(
                        backend,
                        &batch,
                        paths,
                        &path,
                        n,
                        cache.as_ref(),
                        on_bytes,
                    ) {
                        Ok(size) => {
                            log::info!("Successfully retrieved {}", path.display());
                            GetDetail::Success {
                                outcome: Outcome::Copied,
                                size,
                            }
                        }
                        Err(e) => {
                            log::warn!("Failed to get {}: {e:#}", path.display());
                            GetDetail::error(&e)
                        }
                    };
                    if let Some(p) = &progress {
                        (p.on_done)(matches!(detail, GetDetail::Success { .. }));
                    }
                    (path, detail)
                })
                .collect()
        });
        let mut counts = CompleteBatch::default();
        for (_, detail) in results.iter().chain(&fetched) {
            match detail {
                GetDetail::Success {
                    outcome: Outcome::Copied,
                    ..
                } => counts.ok += 1,
                GetDetail::Success { .. } => counts.skipped += 1,
                GetDetail::Error { .. } => counts.failed += 1,
            }
        }
        if let Err(e) = backend.complete(&batch, &counts) {
            log::warn!("The files were retrieved, but closing the download failed: {e:#}");
        }
        results.extend(fetched);
    }

    let mut results: Vec<_> = results
        .into_iter()
        .map(|(path, detail)| GetResult { path, detail })
        .collect();
    results.sort_by(|a, b| a.path.cmp(&b.path));

    Ok(results)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Hashes;
    use crate::add_files;
    use crate::config::Config;
    use crate::files::add::AddOutcome;
    use crate::files::status::get_status;
    use crate::progress::FileProgress;
    use crate::testutil::{create_file, create_temp_git_repo, init_dvs_repo};
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use tempfile::TempDir;

    /// An initialized DVS repo with a temporary backend, plus helpers to track and
    /// retrieve files. Collapses the per-test setup boilerplate to `TestRepo::new()`.
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

        /// Create `name` with `content` and track it, returning its path and metadata.
        fn track(
            &self,
            name: &str,
            content: &[u8],
            compression: Compression,
        ) -> (PathBuf, FileMetadata) {
            let file_path = create_file(&self.root, name, content);
            let results = add_files(
                vec![name.into()],
                &self.paths,
                self.backend(),
                None,
                compression,
                false,
                None,
            )
            .unwrap();
            assert!(!results[0].is_failure(), "{results:?}");
            let metadata = FileMetadata::read(&self.paths.metadata_path(Path::new(name))).unwrap();
            (file_path, metadata)
        }

        /// Run a real (non-dry-run) single-file `get`.
        fn get(&self, name: &str) -> Result<(Outcome, u64)> {
            let results = get_files(vec![name.into()], &self.paths, self.backend(), false, None)?;
            match &results[0].detail {
                GetDetail::Success { outcome, size } => Ok((*outcome, *size)),
                GetDetail::Error { error, .. } => Err(anyhow!("{error}")),
            }
        }

        /// Overwrite the stored blob for `hashes` with `content`, defeating verification.
        fn corrupt_blob(&self, hashes: &Hashes, content: &[u8]) {
            let hash = hashes.get_blake3();
            let storage = match &self.config.backend {
                Backend::Local(b) => b.path.clone(),
                Backend::Server(_) => unreachable!(),
            };
            let blob = storage.join(&hash[..2]).join(&hash[2..]);
            // Blobs are stored read-only; make it writable before overwriting.
            let mut perms = fs::metadata(&blob).unwrap().permissions();
            #[allow(clippy::permissions_set_readonly_false)]
            perms.set_readonly(false);
            fs::set_permissions(&blob, perms).unwrap();
            fs::write(&blob, content).unwrap();
        }
    }

    #[test]
    fn get_file_retrieves_from_storage() {
        let repo = TestRepo::new();
        let (file_path, _) = repo.track("retrieve.txt", b"stored content", Compression::None);

        fs::remove_file(&file_path).unwrap();
        assert!(!file_path.exists());

        let (outcome, _) = repo.get("retrieve.txt").unwrap();
        assert_eq!(outcome, Outcome::Copied);
        assert_eq!(fs::read(&file_path).unwrap(), b"stored content");
        assert!(
            !fs::metadata(&file_path).unwrap().permissions().readonly(),
            "restored file inherited the blob's read-only mode"
        );
    }

    #[test]
    fn get_file_returns_present_when_already_current() {
        let repo = TestRepo::new();
        repo.track("present.txt", b"content", Compression::Zstd);

        // File still exists and matches - should return Present
        let (outcome, _) = repo.get("present.txt").unwrap();
        assert_eq!(outcome, Outcome::Present);
    }

    #[test]
    fn get_file_fails_for_untracked_file() {
        let repo = TestRepo::new();
        let err = repo.get("untracked.txt").unwrap_err().to_string();
        assert!(
            err.contains("cannot be retrieved"),
            "unexpected error: {err}"
        );
    }

    #[test]
    fn get_file_preserves_target_when_stored_blob_is_corrupt() {
        let repo = TestRepo::new();
        // No compression so the corrupt blob still "decompresses" (copies) cleanly
        // and reaches the hash-mismatch check.
        let (file_path, metadata) =
            repo.track("precious.txt", b"original content", Compression::None);
        repo.corrupt_blob(&metadata.hashes, b"CORRUPTED BLOB BYTES");

        // Stand in for the user's uncommitted local edits, then get must fail on the
        // hash mismatch WITHOUT destroying the local file or leaking a tmp file.
        fs::write(&file_path, b"my uncommitted edits").unwrap();
        let err = repo.get("precious.txt").unwrap_err().to_string();
        assert!(
            err.contains("does not match expected hash"),
            "unexpected error: {err}"
        );
        assert_eq!(fs::read(&file_path).unwrap(), b"my uncommitted edits");
        let tmp_left = fs::read_dir(&repo.root)
            .unwrap()
            .flatten()
            .any(|e| e.file_name().to_string_lossy().contains("dvs-tmp"));
        assert!(!tmp_left);
    }

    #[test]
    fn get_files_aborts_batch_when_any_path_not_tracked() {
        let repo = TestRepo::new();

        // Track a.txt, then drop its local copy so a successful get would restore it.
        repo.track("a.txt", b"a", Compression::Zstd);
        fs::remove_file(repo.root.join("a.txt")).unwrap();

        // A file that exists on disk but is not tracked by DVS.
        create_file(&repo.root, "untracked.txt", b"hello");

        let err = get_files(
            vec!["a.txt".into(), "untracked.txt".into()],
            &repo.paths,
            repo.backend(),
            false,
            None,
        )
        .unwrap_err()
        .to_string();
        assert!(err.contains("not tracked"), "unexpected error: {err}");
        assert!(err.contains("untracked.txt"), "unexpected error: {err}");

        // No partial success: the tracked file must not have been restored.
        assert!(
            !repo.root.join("a.txt").exists(),
            "no files should be retrieved when one is not tracked"
        );
    }

    fn run_add_get_roundtrip(file_paths: Vec<PathBuf>, expected_files: &[&str]) {
        let repo = TestRepo::new();

        create_file(&repo.root, "a.txt", b"a");
        create_file(&repo.root, "b.txt", b"b");
        create_file(&repo.root, "c.csv", b"c");

        // Add files
        let results = add_files(
            file_paths.clone(),
            &repo.paths,
            repo.backend(),
            None,
            Compression::Zstd,
            false,
            None,
        )
        .unwrap();
        assert_eq!(results.len(), expected_files.len());
        for result in &results {
            assert_eq!(result.outcome, AddOutcome::Added);
        }

        // Verify correct files are tracked
        let statuses = get_status(&repo.paths, None, None).unwrap();
        assert_eq!(statuses.len(), expected_files.len());
        let tracked_names: Vec<_> = statuses.iter().map(|s| s.path.to_str().unwrap()).collect();
        for expected in expected_files {
            assert!(
                tracked_names.contains(expected),
                "Expected {expected} to be tracked"
            );
        }

        // Delete tracked files
        for name in expected_files {
            fs::remove_file(repo.root.join(name)).unwrap();
        }

        // Get files back
        let results = get_files(file_paths, &repo.paths, repo.backend(), false, None).unwrap();
        assert_eq!(results.len(), expected_files.len());
        for result in &results {
            assert!(matches!(
                result.detail,
                GetDetail::Success {
                    outcome: Outcome::Copied,
                    size: _,
                }
            ));
        }

        // Verify files restored
        for name in expected_files {
            assert!(
                repo.root.join(name).exists(),
                "Expected {name} to be restored"
            );
        }
    }

    #[test]
    fn add_get_roundtrip_with_explicit_paths() {
        let paths: Vec<PathBuf> = vec!["a.txt".into(), "c.csv".into()];
        run_add_get_roundtrip(paths, &["a.txt", "c.csv"]);
    }

    #[test]
    fn get_file_errors_on_corrupted_storage() {
        let repo = TestRepo::new();
        let (file_path, metadata) = repo.track("data.txt", b"original content", Compression::Zstd);
        fs::remove_file(&file_path).unwrap();
        repo.corrupt_blob(&metadata.hashes, b"corrupted content");

        // get_file should error on decompression or hash mismatch.
        assert!(repo.get("data.txt").is_err());
    }

    #[test]
    fn get_files_invokes_on_done_for_each_file() {
        let repo = TestRepo::new();

        // Track two files, one of which will have its storage blob corrupted.
        let (bad_path, bad_meta) = repo.track("bad.txt", b"bad content", Compression::Zstd);
        let (good_path, _) = repo.track("good.txt", b"good content", Compression::Zstd);
        fs::remove_file(&bad_path).unwrap();
        fs::remove_file(&good_path).unwrap();

        // Corrupt bad.txt's stored blob so its get fails while good.txt succeeds.
        repo.corrupt_blob(&bad_meta.hashes, b"corrupted content");

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

        let results = get_files(
            vec!["bad.txt".into(), "good.txt".into()],
            &repo.paths,
            repo.backend(),
            false,
            Some(&on_file_start),
        )
        .unwrap();

        // Results are sorted by path: bad.txt fails, good.txt succeeds.
        assert_eq!(results.len(), 2);
        assert!(matches!(results[0].detail, GetDetail::Error { .. }));
        assert!(matches!(results[1].detail, GetDetail::Success { .. }));

        // on_done fired once per file, with `true` only for the success.
        assert_eq!(done_calls.load(Ordering::SeqCst), 2);
        assert_eq!(success_calls.load(Ordering::SeqCst), 1);
    }
}
