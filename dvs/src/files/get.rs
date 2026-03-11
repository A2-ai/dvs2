use std::path::{Path, PathBuf};
use std::sync::Mutex;

use crate::cache::{HashCache, try_open_cache};
use crate::files::metadata::FileMetadata;
use crate::files::types::{OutputOptions, TimingRecord};
use crate::paths::GetPathStatus;
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
    output: &OutputOptions,
) -> Result<(Outcome, u64)> {
    let v2 = output.verbosity >= 2;
    let rel_display = relative_path.as_ref().display();
    log::debug!("Retrieving file: {}", rel_display);
    let dvs_file_path = paths.metadata_path(relative_path.as_ref());
    if !dvs_file_path.is_file() {
        bail!("File {} is not tracked by DVS", rel_display);
    }

    if v2 {
        eprintln!("  [{rel_display}] Reading metadata...");
    }
    let metadata: FileMetadata = serde_json::from_reader(fs::File::open(&dvs_file_path)?)?;
    log::debug!("Read metadata for {}: {}", rel_display, metadata.hashes);

    if !backend.exists(&metadata.hashes)? {
        bail!("Storage file missing for hash: {}", metadata.hashes);
    }

    let target_path = paths.file_path(relative_path.as_ref());
    let rel_str = relative_path.as_ref().to_string_lossy();

    // Check if target already exists and matches
    if target_path.is_file() {
        if v2 {
            eprintln!("  [{rel_display}] Local file exists, verifying hash...");
        }
        let (hashes, size) = cache::hashes_for_file(&target_path, &rel_str, cache, output)?;

        if hashes == metadata.hashes && size == metadata.size {
            log::debug!("File {} already present locally and matches", rel_display);
            if v2 {
                eprintln!("  [{rel_display}] Already up to date, skipping");
            }
            return Ok((Outcome::Present, metadata.size));
        }
    }

    if output.dry_run {
        if v2 {
            eprintln!(
                "  [{rel_display}] Dry run: would retrieve ({} bytes)",
                metadata.size
            );
        }
        return Ok((Outcome::Copied, metadata.size));
    }

    // Retrieve from backend to target path
    log::debug!(
        "Copying {} from storage to {}",
        metadata.hashes,
        target_path.display()
    );
    if v2 {
        let decompress_label = match metadata.compression {
            Compression::Zstd => "retrieving + decompressing",
            Compression::None => "retrieving",
        };
        eprintln!("  [{rel_display}] {decompress_label} from backend...");
    }
    let retrieve_start = v2.then(std::time::Instant::now);
    backend
        .retrieve(&metadata.hashes, &target_path, metadata.compression)
        .with_context(|| format!("Failed to retrieve {}", rel_display))?;
    if let Some(retrieve_start) = retrieve_start {
        let elapsed = retrieve_start.elapsed();
        eprintln!("  [{rel_display}] Retrieved from backend in {elapsed:.2?}");
        output.send_timing(TimingRecord {
            file: relative_path.as_ref().display().to_string(),
            step: "backend_retrieve".into(),
            duration_ms: elapsed.as_secs_f64() * 1000.0,
            file_size_bytes: Some(metadata.size),
            compression: format!("{:?}", metadata.compression),
            ..output.timing_template("get")
        });
    }

    if v2 {
        eprintln!("  [{rel_display}] Verifying retrieved file hash...");
    }
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
    output: &OutputOptions,
) -> Result<Vec<GetResult>> {
    let v1 = output.verbosity >= 1;
    let v2 = output.verbosity >= 2;
    if v1 {
        eprintln!(
            "Getting {} file{}...",
            files.len(),
            if files.len() == 1 { "" } else { "s" }
        );
    }
    let matched_paths = paths.validate_for_get(&files);
    let pool = get_threadpool(matched_paths.len())?;
    let cache = try_open_cache(paths);

    let total_start = v1.then(std::time::Instant::now);
    let mut results: Vec<GetResult> = pool.install(|| {
        matched_paths
            .into_par_iter()
            .map(|(relative_path, validation)| {
                let rel_display = relative_path.display();
                match validation {
                    GetPathStatus::NotFound => {
                        if v2 {
                            eprintln!("  [{rel_display}] Skipped: file not found");
                        }
                        return GetResult {
                            path: relative_path,
                            detail: GetDetail::Error {
                                error: "file not found".to_string(),
                            },
                        };
                    }
                    GetPathStatus::NotTracked => {
                        if v2 {
                            eprintln!("  [{rel_display}] Skipped: not tracked by DVS");
                        }
                        return GetResult {
                            path: relative_path,
                            detail: GetDetail::Error {
                                error: "not tracked by DVS".to_string(),
                            },
                        };
                    }
                    GetPathStatus::Tracked => {}
                }

                let file_start = v1.then(std::time::Instant::now);
                match get_file(backend, paths, &relative_path, cache.as_ref(), output) {
                    Ok((outcome, size)) => {
                        if let Some(file_start) = file_start {
                            let elapsed = file_start.elapsed();
                            eprintln!("  [{rel_display}] Completed in {elapsed:.2?}",);
                            output.send_timing(TimingRecord {
                                file: relative_path.display().to_string(),
                                step: "get_file_total".into(),
                                duration_ms: elapsed.as_secs_f64() * 1000.0,
                                file_size_bytes: Some(size),
                                ..output.timing_template("get")
                            });
                        }
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
                        if let Some(file_start) = file_start {
                            eprintln!(
                                "  [{rel_display}] Failed in {:.2?}: {e}",
                                file_start.elapsed()
                            );
                        }
                        log::warn!("Failed to get {}: {e}", relative_path.display());
                        GetResult {
                            path: relative_path,
                            detail: GetDetail::Error {
                                error: e.to_string(),
                            },
                        }
                    }
                }
            })
            .collect()
    });
    results.sort_by(|a, b| a.path.cmp(&b.path));

    if let Some(total_start) = total_start {
        let total_elapsed = total_start.elapsed();
        let n_ok = results
            .iter()
            .filter(|r| matches!(r.detail, GetDetail::Success { .. }))
            .count();
        let n_err = results.len() - n_ok;
        eprintln!("Done in {total_elapsed:.2?}: {n_ok} succeeded, {n_err} failed");
        output.send_timing(TimingRecord {
            file: String::new(),
            step: "get_total".into(),
            duration_ms: total_elapsed.as_secs_f64() * 1000.0,
            file_size_bytes: None,
            num_files: Some(results.len()),
            ..output.timing_template("get")
        });
    }

    Ok(results)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::add_files;
    use crate::files::add::AddDetail;
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
                &OutputOptions::default(),
            )
            .unwrap();

        // Delete the original file
        fs::remove_file(&file_path).unwrap();
        assert!(!file_path.exists());

        // Retrieve it
        let cache = make_cache(&paths);
        let (outcome, _size) = get_file(
            backend,
            &paths,
            "retrieve.txt",
            Some(&cache),
            &OutputOptions::default(),
        )
        .unwrap();
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
                &OutputOptions::default(),
            )
            .unwrap();

        // File still exists and matches - should return Present
        let cache = make_cache(&paths);
        let (outcome, _size) = get_file(
            backend,
            &paths,
            "present.txt",
            Some(&cache),
            &OutputOptions::default(),
        )
        .unwrap();
        assert_eq!(outcome, Outcome::Present);
    }

    #[test]
    fn get_file_fails_for_untracked_file() {
        let (_tmp, root) = create_temp_git_repo();
        let (config, _dvs_dir) = init_dvs_repo(&root);
        let backend = config.backend();
        let paths = make_paths(&root, &config);

        let cache = make_cache(&paths);
        let result = get_file(
            backend,
            &paths,
            "untracked.txt",
            Some(&cache),
            &OutputOptions::default(),
        );
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
            &OutputOptions::default(),
        )
        .unwrap();

        let results = get_files(
            vec!["nonexistent.csv".into()],
            &paths,
            backend,
            &OutputOptions::default(),
        )
        .unwrap();
        assert_eq!(results.len(), 1);
        assert!(
            matches!(&results[0].detail, GetDetail::Error { error } if error.contains("not found"))
        );
    }

    #[test]
    fn get_files_reports_not_tracked_for_untracked_file() {
        let (_tmp, root) = create_temp_git_repo();
        let (config, _dvs_dir) = init_dvs_repo(&root);
        let backend = config.backend();
        let paths = make_paths(&root, &config);

        // Create a file on disk but don't dvs add it
        create_file(&root, "untracked.txt", b"hello");

        let results = get_files(
            vec!["untracked.txt".into()],
            &paths,
            backend,
            &OutputOptions::default(),
        )
        .unwrap();
        assert_eq!(results.len(), 1);
        assert!(
            matches!(&results[0].detail, GetDetail::Error { error } if error.contains("not tracked"))
        );
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
        let results = add_files(
            file_paths.clone(),
            &paths,
            backend,
            None,
            Compression::Zstd,
            &OutputOptions::default(),
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
        let statuses = get_status(&paths, &OutputOptions::default()).unwrap();
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
            fs::remove_file(root.join(name)).unwrap();
        }

        // Get files back
        let results = get_files(file_paths, &paths, backend, &OutputOptions::default()).unwrap();
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
                &OutputOptions::default(),
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
            #[allow(clippy::permissions_set_readonly_false)]
            perms.set_readonly(false);
            fs::set_permissions(&storage_path, perms).unwrap();
        }
        fs::write(&storage_path, b"corrupted content").unwrap();

        // get_file should error on decompression or hash mismatch
        let cache = make_cache(&paths);
        let result = get_file(
            backend,
            &paths,
            "data.txt",
            Some(&cache),
            &OutputOptions::default(),
        );
        assert!(result.is_err());
    }
}
