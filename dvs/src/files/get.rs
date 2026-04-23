use std::path::{Path, PathBuf};
use std::sync::Mutex;

use crate::cache::{HashCache, try_open_cache};
use crate::files::metadata::FileMetadata;
use crate::paths::GetPathStatus;
use crate::progress::OnFileStart;
use crate::utils::get_threadpool;
use crate::{Backend, Compression, DvsPaths, Outcome, cache};
use anyhow::{Context, Result, bail};
use fs_err as fs;
use rayon::prelude::*;
use serde::{Deserialize, Serialize};

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

    // Retrieve from backend to target path
    log::debug!(
        "Copying {} from storage to {}",
        metadata.hashes,
        target_path.display()
    );

    backend
        .retrieve(
            &metadata.hashes,
            &target_path,
            metadata.compression,
            on_bytes,
        )
        .with_context(|| format!("Failed to retrieve {}", relative_path.as_ref().display()))?;
    let actual = FileMetadata::from_file(&target_path, Compression::None, None)?;
    if actual.hashes != metadata.hashes {
        fs::remove_file(&target_path)?;
        bail!("Retrieved file does not match expected hash");
    }

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
) -> Result<crate::BatchOutcome<GetSuccess, GetError>> {
    let matched_paths = paths.validate_for_get(&files);
    let pool = get_threadpool(matched_paths.len())?;
    let cache = try_open_cache(paths);

    let mut rows: Vec<Row> = pool.install(|| {
        matched_paths
            .into_par_iter()
            .map(|(relative_path, validation)| match validation {
                GetPathStatus::NotFound => Row::Err(GetError::NotFound {
                    path: relative_path,
                }),
                GetPathStatus::NotTracked => Row::Err(GetError::NotTracked {
                    path: relative_path,
                }),
                GetPathStatus::Tracked => {
                    let file_size = {
                        let meta_path = paths.metadata_path(&relative_path);
                        std::fs::File::open(&meta_path)
                            .ok()
                            .and_then(|f| serde_json::from_reader::<_, FileMetadata>(f).ok())
                            .map(|m| m.size)
                            .unwrap_or(0)
                    };
                    let file_progress = on_file_start.map(|f| f(&relative_path, file_size));
                    let on_bytes = file_progress.as_ref().map(|fp| &*fp.on_bytes);

                    match get_file(
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
                            Row::Ok(GetSuccess {
                                path: relative_path,
                                outcome,
                                size,
                            })
                        }
                        Err(e) => {
                            log::warn!("Failed to get {}: {e}", relative_path.display());
                            classify_get_runtime_err(relative_path, e)
                        }
                    }
                }
            })
            .collect()
    });
    rows.sort_by(|a, b| {
        let a_path = match a {
            Row::Ok(success) => success.path.as_path(),
            Row::Err(err) => err.path().unwrap_or(Path::new("")),
        };
        let b_path = match b {
            Row::Ok(success) => success.path.as_path(),
            Row::Err(err) => err.path().unwrap_or(Path::new("")),
        };
        a_path.cmp(b_path)
    });

    let mut outcome = crate::BatchOutcome::<GetSuccess, GetError>::new();
    for row in rows {
        match row {
            Row::Ok(success) => outcome.ok.push(success),
            Row::Err(err) => outcome.err.push(err),
        }
    }

    Ok(outcome)
}

/// Successful get of a single file.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GetSuccess {
    pub path: PathBuf,
    pub outcome: Outcome,
    pub size: u64,
}

/// Structured failure for a single file in `get_files`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum GetError {
    /// Pre-flight: user path does not resolve to any tracked file.
    NotFound { path: PathBuf },
    /// Pre-flight: the file exists on disk but is not tracked by DVS.
    NotTracked { path: PathBuf },
    /// Pre-flight: glob pattern could not be compiled.
    GlobFailure { pattern: String, reason: String },
    /// Runtime: reading the `.dvs` metadata file failed.
    MetadataRead { path: PathBuf, reason: String },
    /// Runtime: stored object is missing in the backend for the tracked hash.
    StorageMissing { path: PathBuf, hash: String },
    /// Runtime: backend retrieval failed.
    StorageRead { path: PathBuf, reason: String },
    /// Runtime: retrieved object did not hash to the expected value.
    HashMismatch {
        path: PathBuf,
        expected: String,
        got: String,
    },
}

impl GetError {
    pub fn path(&self) -> Option<&Path> {
        match self {
            GetError::NotFound { path }
            | GetError::NotTracked { path }
            | GetError::MetadataRead { path, .. }
            | GetError::StorageMissing { path, .. }
            | GetError::StorageRead { path, .. }
            | GetError::HashMismatch { path, .. } => Some(path.as_path()),
            GetError::GlobFailure { .. } => None,
        }
    }

    pub fn kind(&self) -> &'static str {
        match self {
            GetError::NotFound { .. } => "not_found",
            GetError::NotTracked { .. } => "not_tracked",
            GetError::GlobFailure { .. } => "glob_failure",
            GetError::MetadataRead { .. } => "metadata_read",
            GetError::StorageMissing { .. } => "storage_missing",
            GetError::StorageRead { .. } => "storage_read",
            GetError::HashMismatch { .. } => "hash_mismatch",
        }
    }
}

#[derive(Debug)]
enum Row {
    Ok(GetSuccess),
    Err(GetError),
}

fn classify_get_runtime_err(path: PathBuf, err: anyhow::Error) -> Row {
    let reason = err.to_string();
    if reason.contains("is not tracked") {
        Row::Err(GetError::NotTracked { path })
    } else if reason.starts_with("Storage file missing") {
        let hash = reason.rsplit(':').next().unwrap_or("").trim().to_string();
        Row::Err(GetError::StorageMissing { path, hash })
    } else if reason.contains("does not match expected hash") {
        Row::Err(GetError::HashMismatch {
            path,
            expected: String::new(),
            got: String::new(),
        })
    } else if reason.contains("metadata") || reason.contains("serde_json") {
        Row::Err(GetError::MetadataRead { path, reason })
    } else {
        Row::Err(GetError::StorageRead { path, reason })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::add_files;
    use crate::files::add::AddSuccess;
    use crate::files::status::get_status;
    use crate::testutil::{create_file, create_temp_git_repo, init_dvs_repo};
    use uuid::Uuid;

    fn make_paths(root: &Path, config: &crate::config::Config) -> DvsPaths {
        DvsPaths::new(
            root.to_path_buf(),
            root.to_path_buf(),
            config.metadata_folder_name(),
        )
        .unwrap()
    }

    fn make_cache(paths: &DvsPaths) -> Mutex<cache::HashCache> {
        Mutex::new(cache::HashCache::open(&paths.cache_folder().join("dvs.db")).unwrap())
    }

    #[test]
    fn get_file_retrieves_from_storage() {
        let (_tmp, root) = create_temp_git_repo();
        let (config, _dvs_dir) = init_dvs_repo(&root);
        let backend = config.backend();
        let paths = make_paths(&root, &config);
        let file_path = create_file(&root, "retrieve.txt", b"stored content");

        let metadata = FileMetadata::from_file(&file_path, Compression::Zstd, None).unwrap();
        metadata
            .save(
                Uuid::new_v4(),
                &file_path,
                backend,
                &paths,
                "retrieve.txt",
                None,
            )
            .unwrap();

        // Delete the original file
        fs::remove_file(&file_path).unwrap();
        assert!(!file_path.exists());

        // Retrieve it
        let cache = make_cache(&paths);
        let (outcome, _size) =
            get_file(backend, &paths, "retrieve.txt", Some(&cache), false, None).unwrap();
        assert_eq!(outcome, Outcome::Copied);
        assert!(file_path.exists());
        assert_eq!(fs::read(&file_path).unwrap(), b"stored content");
    }

    #[test]
    fn get_file_returns_present_when_already_current() {
        let (_tmp, root) = create_temp_git_repo();
        let (config, _dvs_dir) = init_dvs_repo(&root);
        let backend = config.backend();
        let paths = make_paths(&root, &config);
        let file_path = create_file(&root, "present.txt", b"content");

        let metadata = FileMetadata::from_file(&file_path, Compression::Zstd, None).unwrap();
        metadata
            .save(
                Uuid::new_v4(),
                &file_path,
                backend,
                &paths,
                "present.txt",
                None,
            )
            .unwrap();

        // File still exists and matches - should return Present
        let cache = make_cache(&paths);
        let (outcome, _size) =
            get_file(backend, &paths, "present.txt", Some(&cache), false, None).unwrap();
        assert_eq!(outcome, Outcome::Present);
    }

    #[test]
    fn get_file_fails_for_untracked_file() {
        let (_tmp, root) = create_temp_git_repo();
        let (config, _dvs_dir) = init_dvs_repo(&root);
        let backend = config.backend();
        let paths = make_paths(&root, &config);

        let cache = make_cache(&paths);
        let result = get_file(backend, &paths, "untracked.txt", Some(&cache), false, None);
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("not tracked"));
    }

    #[test]
    fn get_files_reports_not_found_per_file() {
        let (_tmp, root) = create_temp_git_repo();
        let (config, _dvs_dir) = init_dvs_repo(&root);
        let backend = config.backend();
        let paths = make_paths(&root, &config);

        create_file(&root, "a.txt", b"a");
        add_files(
            vec!["a.txt".into()],
            &paths,
            backend,
            None,
            Compression::Zstd,
            false,
            None,
        )
        .unwrap();

        let outcome =
            get_files(vec!["nonexistent.csv".into()], &paths, backend, false, None).unwrap();
        assert!(outcome.ok.is_empty());
        assert_eq!(outcome.err.len(), 1);
        assert!(matches!(outcome.err[0], GetError::NotFound { .. }));
    }

    #[test]
    fn get_error_serializes_with_snake_case_kind() {
        let cases: Vec<(GetError, &str)> = vec![
            (GetError::NotFound { path: "a".into() }, "not_found"),
            (GetError::NotTracked { path: "a".into() }, "not_tracked"),
            (
                GetError::GlobFailure {
                    pattern: "*".into(),
                    reason: "x".into(),
                },
                "glob_failure",
            ),
            (
                GetError::MetadataRead {
                    path: "a".into(),
                    reason: "x".into(),
                },
                "metadata_read",
            ),
            (
                GetError::StorageMissing {
                    path: "a".into(),
                    hash: "h".into(),
                },
                "storage_missing",
            ),
            (
                GetError::StorageRead {
                    path: "a".into(),
                    reason: "x".into(),
                },
                "storage_read",
            ),
            (
                GetError::HashMismatch {
                    path: "a".into(),
                    expected: "x".into(),
                    got: "y".into(),
                },
                "hash_mismatch",
            ),
        ];
        for (err, expected_kind) in cases {
            assert_eq!(err.kind(), expected_kind);
            let json = serde_json::to_value(&err).unwrap();
            assert_eq!(json["kind"], expected_kind);
        }
    }

    #[test]
    fn get_success_roundtrips_json() {
        let s = GetSuccess {
            path: "a".into(),
            outcome: Outcome::Copied,
            size: 10,
        };
        let back: GetSuccess = serde_json::from_str(&serde_json::to_string(&s).unwrap()).unwrap();
        assert_eq!(back.size, 10);
    }

    #[test]
    fn get_files_reports_not_tracked_for_untracked_file() {
        let (_tmp, root) = create_temp_git_repo();
        let (config, _dvs_dir) = init_dvs_repo(&root);
        let backend = config.backend();
        let paths = make_paths(&root, &config);

        // Create a file on disk but don't dvs add it
        create_file(&root, "untracked.txt", b"hello");

        let outcome =
            get_files(vec!["untracked.txt".into()], &paths, backend, false, None).unwrap();
        assert!(outcome.ok.is_empty());
        assert_eq!(outcome.err.len(), 1);
        assert!(matches!(outcome.err[0], GetError::NotTracked { .. }));
    }

    fn run_add_get_roundtrip(file_paths: Vec<PathBuf>, expected_files: &[&str]) {
        let (_tmp, root) = create_temp_git_repo();
        let (config, _dvs_dir) = init_dvs_repo(&root);
        let backend = config.backend();
        let paths = make_paths(&root, &config);

        create_file(&root, "a.txt", b"a");
        create_file(&root, "b.txt", b"b");
        create_file(&root, "c.csv", b"c");

        // Add files
        let outcome = add_files(
            file_paths.clone(),
            &paths,
            backend,
            None,
            Compression::Zstd,
            false,
            None,
        )
        .unwrap();
        assert!(outcome.err.is_empty());
        assert_eq!(outcome.ok.len(), expected_files.len());
        for result in &outcome.ok {
            assert!(matches!(
                result,
                AddSuccess {
                    outcome: Outcome::Copied,
                    ..
                }
            ));
        }

        // Verify correct files are tracked
        let statuses = get_status(&paths, None).unwrap();
        assert!(statuses.err.is_empty());
        assert_eq!(statuses.ok.len(), expected_files.len());
        let tracked_names: Vec<_> = statuses
            .ok
            .iter()
            .map(|status| status.path.to_str().unwrap())
            .collect();
        for expected in expected_files {
            assert!(
                tracked_names.contains(expected),
                "Expected {expected} to be tracked"
            );
        }

        // Delete tracked files
        for name in expected_files {
            fs::remove_file(root.join(name)).unwrap();
        }

        // Get files back
        let outcome = get_files(file_paths, &paths, backend, false, None).unwrap();
        assert!(outcome.err.is_empty());
        assert_eq!(outcome.ok.len(), expected_files.len());
        for result in &outcome.ok {
            assert_eq!(result.outcome, Outcome::Copied);
        }

        // Verify files restored
        for name in expected_files {
            assert!(root.join(name).exists(), "Expected {name} to be restored");
        }
    }

    #[test]
    fn add_get_roundtrip_with_explicit_paths() {
        let paths: Vec<PathBuf> = vec!["a.txt".into(), "c.csv".into()];
        run_add_get_roundtrip(paths, &["a.txt", "c.csv"]);
    }

    #[test]
    fn get_file_errors_on_corrupted_storage() {
        let (_tmp, root) = create_temp_git_repo();
        let (config, _dvs_dir) = init_dvs_repo(&root);
        let backend = config.backend();
        let paths = make_paths(&root, &config);

        // Add a file
        let file_path = create_file(&root, "data.txt", b"original content");
        let metadata = FileMetadata::from_file(&file_path, Compression::Zstd, None).unwrap();
        metadata
            .save(
                Uuid::new_v4(),
                &file_path,
                backend,
                &paths,
                "data.txt",
                None,
            )
            .unwrap();

        // Delete the local file
        fs::remove_file(&file_path).unwrap();

        // Corrupt the storage file (must remove read-only first)
        let storage_path = root
            .join(".storage")
            .join(&metadata.hashes.blake3[..2])
            .join(&metadata.hashes.blake3[2..]);
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let perms = std::fs::Permissions::from_mode(0o644);
            fs::set_permissions(&storage_path, perms).unwrap();
        }
        #[cfg(not(unix))]
        {
            let mut perms = fs::metadata(&storage_path).unwrap().permissions();
            perms.set_readonly(false);
            fs::set_permissions(&storage_path, perms).unwrap();
        }
        fs::write(&storage_path, b"corrupted content").unwrap();

        // get_file should error on decompression or hash mismatch
        let cache = make_cache(&paths);
        let result = get_file(backend, &paths, "data.txt", Some(&cache), false, None);
        assert!(result.is_err());
    }
}
