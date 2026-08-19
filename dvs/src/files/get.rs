use std::path::{Path, PathBuf};
use std::sync::Mutex;

use crate::cache::{HashCache, try_open_cache};
use crate::files::metadata::FileMetadata;
use crate::progress::OnFileStart;
use crate::utils::get_threadpool;
use crate::{Backend, Compression, DvsPaths, Outcome, RetrieveRequest, cache};
use anyhow::{Context, Result, bail};
use fs_err as fs;
use rayon::prelude::*;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

fn get_file(
    backend: &dyn Backend,
    paths: &DvsPaths,
    relative_path: impl AsRef<Path>,
    cache: Option<&Mutex<HashCache>>,
    dry_run: bool,
    on_bytes: Option<&(dyn Fn(u64) + Send + Sync)>,
) -> Result<(Outcome, u64)> {
    log::debug!("Retrieving file: {}", relative_path.as_ref().display());
    let dvs_file_path = paths.metadata_path(relative_path.as_ref());
    if !dvs_file_path.is_file() {
        bail!(
            "File {} is not tracked by DVS",
            relative_path.as_ref().display()
        );
    }

    let metadata: FileMetadata = serde_json::from_reader(fs::File::open(&dvs_file_path)?)?;
    log::debug!(
        "Read metadata for {}: {}",
        relative_path.as_ref().display(),
        metadata.hashes
    );

    if !backend.exists(&metadata.hashes)? {
        bail!("Storage file missing for hash: {}", metadata.hashes);
    }

    let target_path = paths.file_path(relative_path.as_ref());
    let rel_str = relative_path.as_ref().to_string_lossy();

    // Check if target already exists and matches
    if target_path.is_file() {
        let (hashes, size) = cache::hashes_for_file(&target_path, &rel_str, cache)?;

        if hashes == metadata.hashes && size == metadata.size {
            log::debug!(
                "File {} already present locally and matches",
                relative_path.as_ref().display()
            );
            return Ok((Outcome::Present, metadata.size));
        }
    }

    if dry_run {
        return Ok((Outcome::Copied, metadata.size));
    }

    let tmp_path = target_path.with_extension(format!("dvs-tmp.{}", Uuid::new_v4()));
    log::debug!(
        "Copying {} from storage to temp file {}",
        metadata.hashes,
        tmp_path.display()
    );

    let result = (|| {
        let retrieved = backend
            .retrieve(RetrieveRequest {
                hashes: &metadata.hashes,
                target: &tmp_path,
                compression: metadata.compression,
                path: relative_path.as_ref(),
                on_bytes,
            })
            .with_context(|| format!("Failed to retrieve {}", relative_path.as_ref().display()))?;
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
    if let Some(mtx) = cache {
        if let Ok(stat) = cache::FileStat::from_path(&target_path) {
            if let Err(e) = mtx.lock().unwrap().insert(&rel_str, &stat, &actual.hashes) {
                log::warn!("Cache store failed after get for {rel_str}: {e}");
            }
        }
    }

    Ok((Outcome::Copied, metadata.size))
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
    Success { outcome: Outcome, size: u64 },
    Error { error: String },
}

/// Gets files matching a glob pattern from DVS storage.
///
/// The pattern is matched against tracked files (paths in metadata folder).
/// The pattern is adjusted based on cwd relative to repo root.
pub fn get_files(
    files: Vec<PathBuf>,
    paths: &DvsPaths,
    backend: &dyn Backend,
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
        bail!(
            "Refusing to get, the following paths cannot be retrieved:\n{}",
            invalid.join("\n")
        );
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

    let mut results: Vec<GetResult> = pool.install(|| {
        tracked_paths
            .into_par_iter()
            .map(|relative_path| {
                let file_size = {
                    let meta_path = paths.metadata_path(&relative_path);
                    fs::File::open(&meta_path)
                        .ok()
                        .and_then(|f| serde_json::from_reader::<_, FileMetadata>(f).ok())
                        .map(|m| m.size)
                        .unwrap_or(0)
                };
                let file_progress = on_file_start.map(|f| f(&relative_path, file_size));
                let on_bytes = file_progress.as_ref().map(|fp| &*fp.on_bytes);

                let result = match get_file(
                    backend,
                    paths,
                    &relative_path,
                    cache.as_ref(),
                    dry_run,
                    on_bytes,
                ) {
                    Ok((outcome, size)) => {
                        log::info!(
                            "Successfully retrieved {} ({:?})",
                            relative_path.display(),
                            outcome
                        );
                        GetResult {
                            path: relative_path,
                            detail: GetDetail::Success { outcome, size },
                        }
                    }
                    Err(e) => {
                        log::warn!("Failed to get {}: {e}", relative_path.display());
                        GetResult {
                            path: relative_path,
                            detail: GetDetail::Error {
                                error: e.to_string(),
                            },
                        }
                    }
                };
                if let Some(fp) = &file_progress {
                    (fp.on_done)(matches!(result.detail, GetDetail::Success { .. }));
                }
                result
            })
            .collect()
    });
    results.sort_by(|a, b| a.path.cmp(&b.path));

    Ok(results)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Hashes;
    use crate::add_files;
    use crate::config::Config;
    use crate::files::add::AddDetail;
    use crate::files::status::get_status;
    use crate::progress::FileProgress;
    use crate::testutil::{create_file, create_temp_git_repo, init_dvs_repo};
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use tempfile::TempDir;
    use uuid::Uuid;

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

        fn backend(&self) -> &dyn Backend {
            self.config.backend()
        }

        fn cache(&self) -> Mutex<HashCache> {
            Mutex::new(HashCache::open(&self.paths.cache_folder().join("dvs.db")).unwrap())
        }

        /// Create `name` with `content` and track it, returning its path and metadata.
        fn track(
            &self,
            name: &str,
            content: &[u8],
            compression: Compression,
        ) -> (PathBuf, FileMetadata) {
            let file_path = create_file(&self.root, name, content);
            let metadata = FileMetadata::from_file(&file_path, compression, None).unwrap();
            metadata
                .save(
                    Uuid::new_v4(),
                    &file_path,
                    self.backend(),
                    &self.paths,
                    name,
                    None,
                )
                .unwrap();
            (file_path, metadata)
        }

        /// Run a real (non-dry-run) single-file `get`.
        fn get(&self, name: &str) -> Result<(Outcome, u64)> {
            get_file(
                self.backend(),
                &self.paths,
                name,
                Some(&self.cache()),
                false,
                None,
            )
        }

        /// Overwrite the stored blob for `hashes` with `content`, defeating verification.
        fn corrupt_blob(&self, hashes: &Hashes, content: &[u8]) {
            let hash = hashes.get_blake3();
            let storage = match &self.config.backend {
                crate::config::Backend::Local(b) => b.path.clone(),
                crate::config::Backend::Server(_) => unreachable!(),
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
        assert!(err.contains("not tracked"), "unexpected error: {err}");
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
            assert!(matches!(
                result.detail,
                AddDetail::Success {
                    outcome: Outcome::Copied,
                    ..
                }
            ));
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
