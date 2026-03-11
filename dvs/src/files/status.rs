use std::path::{Path, PathBuf};
use std::sync::Mutex;

use anyhow::Result;
use fs_err as fs;
use rayon::prelude::*;
use serde::{Deserialize, Serialize};
use walkdir::WalkDir;

use crate::cache::{HashCache, try_open_cache};
use crate::files::metadata::FileMetadata;
use crate::files::types::OutputOptions;
use crate::utils::get_threadpool;
use crate::{DvsPaths, Status, cache};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FileStatus {
    pub path: PathBuf,
    #[serde(flatten)]
    pub detail: StatusDetail,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum StatusDetail {
    Success { status: Status },
    Error { error: String },
}

fn get_file_status(
    paths: &DvsPaths,
    relative_path: impl AsRef<Path>,
    cache: Option<&Mutex<HashCache>>,
    output: &OutputOptions,
) -> Result<Status> {
    let verbose = output.verbose;
    let rel_display = relative_path.as_ref().display();
    let dvs_file_path = paths.metadata_path(relative_path.as_ref());
    if !dvs_file_path.is_file() {
        if verbose {
            eprintln!("  [{rel_display}] No metadata found: Untracked");
        }
        return Ok(Status::Untracked);
    }
    let existing_metadata: FileMetadata = serde_json::from_reader(fs::File::open(dvs_file_path)?)?;
    // If we have read the metadata, but we can't find the original file
    let file_path = paths.file_path(relative_path.as_ref());
    if !file_path.is_file() {
        if verbose {
            eprintln!("  [{rel_display}] Local file missing: Absent");
        }
        return Ok(Status::Absent);
    }
    let rel_str = relative_path.as_ref().to_string_lossy();
    let (hashes, size) = cache::hashes_for_file(&file_path, &rel_str, cache, verbose)?;

    if existing_metadata.hashes == hashes && existing_metadata.size == size {
        if verbose {
            eprintln!("  [{rel_display}] Hash matches: Current");
        }
        Ok(Status::Current)
    } else {
        if verbose {
            eprintln!("  [{rel_display}] Hash mismatch: Unsynced");
        }
        Ok(Status::Unsynced)
    }
}

pub fn get_status(paths: &DvsPaths, output: &OutputOptions) -> Result<Vec<FileStatus>> {
    let verbose = output.verbose;
    let dvs_directory = paths.metadata_folder();
    log::debug!("Scanning metadata folder: {}", dvs_directory.display());
    if verbose {
        eprintln!("Scanning metadata folder...");
    }
    let cache = try_open_cache(paths);

    // Collect entries first so we can process in parallel
    let entries: Vec<PathBuf> = WalkDir::new(&dvs_directory)
        .into_iter()
        .filter_map(|e| e.ok())
        .filter(|e| e.file_type().is_file())
        .filter(|e| {
            e.path()
                .extension()
                .map(|ext| ext == "dvs")
                .unwrap_or(false)
        })
        .map(|e| e.into_path())
        .collect();

    if verbose {
        eprintln!(
            "Found {} tracked file{}, checking status...",
            entries.len(),
            if entries.len() == 1 { "" } else { "s" }
        );
    }

    let pool = get_threadpool(entries.len())?;

    let total_start = verbose.then(std::time::Instant::now);
    let mut results: Vec<FileStatus> = pool.install(|| {
        entries
            .into_par_iter()
            .map(|dvs_path| {
                let relative = match dvs_path.strip_prefix(&dvs_directory) {
                    Ok(r) => r.with_extension(""),
                    Err(e) => {
                        return FileStatus {
                            path: dvs_path,
                            detail: StatusDetail::Error {
                                error: format!("failed to determine relative path: {e}"),
                            },
                        };
                    }
                };
                let detail = match get_file_status(paths, &relative, cache.as_ref(), output) {
                    Ok(status) => StatusDetail::Success { status },
                    Err(e) => {
                        if verbose {
                            eprintln!("  [{}] Error: {e}", relative.display());
                        }
                        StatusDetail::Error {
                            error: e.to_string(),
                        }
                    }
                };
                FileStatus {
                    path: relative.to_path_buf(),
                    detail,
                }
            })
            .collect()
    });
    results.sort_by(|a, b| a.path.cmp(&b.path));

    if let Some(total_start) = total_start {
        eprintln!("Done in {:.2?}: {} file{} checked", total_start.elapsed(), results.len(), if results.len() == 1 { "" } else { "s" });
    }

    log::debug!("Found {} tracked files", results.len());
    Ok(results)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Compression;
    use crate::testutil::{create_file, create_temp_git_repo, init_dvs_repo};
    use uuid::Uuid;

    fn make_paths(root: &Path, config: &crate::config::Config) -> DvsPaths {
        DvsPaths::new(
            root.to_path_buf(),
            root.to_path_buf(),
            config.metadata_folder_name(),
        )
    }

    fn make_cache(paths: &DvsPaths) -> Mutex<HashCache> {
        Mutex::new(HashCache::open(&paths.cache_folder().join("dvs.db")).unwrap())
    }

    #[test]
    fn get_file_status_returns_untracked_for_new_file() {
        let (_tmp, root) = create_temp_git_repo();
        let (config, _dvs_dir) = init_dvs_repo(&root);
        let paths = make_paths(&root, &config);
        create_file(&root, "new.txt", b"content");

        let cache = make_cache(&paths);
        let status = get_file_status(&paths, "new.txt", Some(&cache), &OutputOptions::default()).unwrap();
        assert_eq!(status, Status::Untracked);
    }

    #[test]
    fn get_file_status_returns_current_for_synced_file() {
        let (_tmp, root) = create_temp_git_repo();
        let (config, _dvs_dir) = init_dvs_repo(&root);
        let backend = config.backend();
        let paths = make_paths(&root, &config);
        let file_path = create_file(&root, "synced.txt", b"content");

        let metadata = FileMetadata::from_file(&file_path, Compression::Zstd, None).unwrap();
        metadata
            .save(Uuid::new_v4(), &file_path, backend, &paths, "synced.txt", false)
            .unwrap();

        let cache = make_cache(&paths);
        let status = get_file_status(&paths, "synced.txt", Some(&cache), &OutputOptions::default()).unwrap();
        assert_eq!(status, Status::Current);
    }

    #[test]
    fn get_file_status_returns_absent_when_file_deleted() {
        let (_tmp, root) = create_temp_git_repo();
        let (config, _dvs_dir) = init_dvs_repo(&root);
        let backend = config.backend();
        let paths = make_paths(&root, &config);
        let file_path = create_file(&root, "deleted.txt", b"content");

        let metadata = FileMetadata::from_file(&file_path, Compression::Zstd, None).unwrap();
        metadata
            .save(Uuid::new_v4(), &file_path, backend, &paths, "deleted.txt", false)
            .unwrap();

        // Delete the original file
        fs::remove_file(&file_path).unwrap();

        let cache = make_cache(&paths);
        let status = get_file_status(&paths, "deleted.txt", Some(&cache), &OutputOptions::default()).unwrap();
        assert_eq!(status, Status::Absent);
    }

    #[test]
    fn get_file_status_returns_unsynced_when_file_modified() {
        let (_tmp, root) = create_temp_git_repo();
        let (config, _dvs_dir) = init_dvs_repo(&root);
        let backend = config.backend();
        let paths = make_paths(&root, &config);
        let file_path = create_file(&root, "modified.txt", b"original");

        let metadata = FileMetadata::from_file(&file_path, Compression::Zstd, None).unwrap();
        metadata
            .save(Uuid::new_v4(), &file_path, backend, &paths, "modified.txt", false)
            .unwrap();

        // Modify the file
        fs::write(&file_path, b"changed content").unwrap();

        let cache = make_cache(&paths);
        let status = get_file_status(&paths, "modified.txt", Some(&cache), &OutputOptions::default()).unwrap();
        assert_eq!(status, Status::Unsynced);
    }

    #[test]
    fn get_status_returns_all_tracked_files() {
        let (_tmp, root) = create_temp_git_repo();
        let (config, _dvs_dir) = init_dvs_repo(&root);
        let backend = config.backend();
        let paths = make_paths(&root, &config);

        // Add multiple files
        for name in ["a.txt", "b.txt", "subdir/c.txt"] {
            let file_path = create_file(&root, name, name.as_bytes());
            let metadata = FileMetadata::from_file(&file_path, Compression::Zstd, None).unwrap();
            metadata
                .save(Uuid::new_v4(), &file_path, backend, &paths, name, false)
                .unwrap();
        }

        let statuses = get_status(&paths, &OutputOptions::default()).unwrap();
        assert_eq!(statuses.len(), 3);

        // All should be Current
        for status in &statuses {
            match &status.detail {
                StatusDetail::Success { status } => assert_eq!(*status, Status::Current),
                StatusDetail::Error { error } => panic!("unexpected error: {error}"),
            }
        }
    }

    #[test]
    fn save_local_updates_metadata_when_content_matches_different_file() {
        // - Add file A with content "foo" (hash H1)
        // - Add file B with content "bar" (hash H2)
        // - Change file B's content to "foo" (now hash H1)
        // - Run `add` on B
        // => B's metadata is updated to hash H1
        let (_tmp, root) = create_temp_git_repo();
        let (config, dvs_dir) = init_dvs_repo(&root);
        let backend = config.backend();
        let paths = make_paths(&root, &config);

        // Add file A with content "foo" (hash H1)
        let file_a = create_file(&root, "a.txt", b"foo");
        let metadata_a = FileMetadata::from_file(&file_a, Compression::Zstd, None).unwrap();
        metadata_a
            .save(Uuid::new_v4(), &file_a, backend, &paths, "a.txt", false)
            .unwrap();
        let hash_h1 = metadata_a.hashes.blake3.clone();

        // Add file B with content "bar" (hash H2)
        let file_b = create_file(&root, "b.txt", b"bar");
        let metadata_b = FileMetadata::from_file(&file_b, Compression::Zstd, None).unwrap();
        metadata_b
            .save(Uuid::new_v4(), &file_b, backend, &paths, "b.txt", false)
            .unwrap();
        let hash_h2 = metadata_b.hashes.blake3.clone();
        assert_ne!(hash_h1, hash_h2);

        // Change file B's content to "foo" (now hash H1)
        fs::write(&file_b, b"foo").unwrap();

        // Run add on B with new content
        let metadata_b_new = FileMetadata::from_file(&file_b, Compression::Zstd, None).unwrap();
        assert_eq!(metadata_b_new.hashes.blake3, hash_h1);

        metadata_b_new
            .save(Uuid::new_v4(), &file_b, backend, &paths, "b.txt", false)
            .unwrap();

        // Verify metadata was updated
        let dvs_file = dvs_dir.join("b.txt.dvs");
        let stored: FileMetadata =
            serde_json::from_reader(fs::File::open(&dvs_file).unwrap()).unwrap();

        assert_eq!(
            stored.hashes.blake3, hash_h1,
            "Metadata should be updated to new hash"
        );

        let cache = make_cache(&paths);
        let status = get_file_status(&paths, "b.txt", Some(&cache), &OutputOptions::default()).unwrap();
        assert_eq!(status, Status::Current);
    }
}
