use std::path::{Path, PathBuf};
use std::sync::Mutex;

use anyhow::{Result, bail};
use fs_err as fs;
use rayon::prelude::*;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::backends::Backend;
use crate::cache::{HashCache, try_open_cache};
use crate::config::Compression;
use crate::files::metadata::FileMetadata;
use crate::gitignore::add_to_gitignore;
use crate::paths::DvsPaths;
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
        error: String,
    },
}

#[allow(clippy::too_many_arguments)]
fn add_file(
    relative_path: &Path,
    paths: &DvsPaths,
    backend: &dyn Backend,
    cache: Option<&Mutex<HashCache>>,
    operation_id: Uuid,
    message: Option<String>,
    compression: Compression,
    dry_run: bool,
    on_bytes: Option<&(dyn Fn(u64) + Send + Sync)>,
) -> Result<(Outcome, FileMetadata, Option<u64>)> {
    let full_path = paths.file_path(relative_path);
    let rel_str = relative_path.to_string_lossy();
    let (hashes, size) = cache::hashes_for_file(&full_path, &rel_str, cache)?;
    let metadata = FileMetadata::from_hashes(hashes, size, compression, message);
    if dry_run {
        let dvs_file_path = paths.metadata_path(relative_path);
        let dvs_file_exists = dvs_file_path.is_file();
        let storage_exists = backend.exists(&metadata.hashes)?;
        let outcome = if dvs_file_exists && storage_exists {
            let existing: FileMetadata =
                serde_json::from_reader(fs_err::File::open(&dvs_file_path)?)?;
            if existing == metadata {
                Outcome::Present
            } else {
                Outcome::Copied
            }
        } else {
            Outcome::Copied
        };
        Ok((outcome, metadata, None))
    } else {
        let (outcome, stored_size) = metadata.save(
            operation_id,
            &full_path,
            backend,
            paths,
            relative_path,
            on_bytes,
        )?;
        Ok((outcome, metadata, stored_size))
    }
}

/// Adds files matching a glob pattern to DVS.
///
/// The pattern is matched against files relative to cwd.
/// Files are stored with paths relative to repo_root.
pub fn add_files(
    files: Vec<PathBuf>,
    paths: &DvsPaths,
    backend: &dyn Backend,
    message: Option<String>,
    compression: Compression,
    dry_run: bool,
    on_file_start: Option<&OnFileStart>,
) -> Result<Vec<AddResult>> {
    let matched_paths = paths.validate_for_add(&files);
    if matched_paths.is_empty() {
        return Ok(Vec::new());
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

    let mut results: Vec<AddResult> = pool.install(|| {
        valid_paths
            .into_par_iter()
            .map(|relative_path| {
                let full_path = paths.file_path(&relative_path);
                let file_size = fs::metadata(&full_path).map(|m| m.len()).unwrap_or(0);
                let file_progress = on_file_start.map(|f| f(&relative_path, file_size));
                let on_bytes = file_progress.as_ref().map(|fp| &*fp.on_bytes);
                let result = match add_file(
                    &relative_path,
                    paths,
                    backend,
                    cache.as_ref(),
                    operation_id,
                    message.clone(),
                    compression,
                    dry_run,
                    on_bytes,
                ) {
                    Ok((outcome, metadata, stored_size)) => {
                        log::info!(
                            "Successfully added {} ({:?})",
                            relative_path.display(),
                            outcome
                        );
                        let stored_size = stored_size.unwrap_or(metadata.size);
                        AddResult {
                            path: relative_path,
                            detail: AddDetail::Success {
                                outcome,
                                hash: metadata.hashes.blake3,
                                size: metadata.size,
                                stored_size,
                            },
                        }
                    }
                    Err(e) => {
                        log::warn!("Failed to add {}: {e}", relative_path.display());
                        AddResult {
                            path: relative_path,
                            detail: AddDetail::Error {
                                error: e.to_string(),
                            },
                        }
                    }
                };
                if let Some(fp) = &file_progress {
                    (fp.on_done)(matches!(result.detail, AddDetail::Success { .. }));
                }
                result
            })
            .collect()
    });
    results.sort_by(|a, b| a.path.cmp(&b.path));

    let successful_paths: Vec<_> = results
        .iter()
        .filter(|r| matches!(r.detail, AddDetail::Success { .. }))
        .map(|r| r.path.clone())
        .collect();
    if !dry_run && !successful_paths.is_empty() {
        if let Err(e) = add_to_gitignore(paths.repo_root(), &successful_paths) {
            log::warn!("Failed to update .gitignore: {e}");
        }
    }

    Ok(results)
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
        .unwrap();

        assert_eq!(results.len(), 3);
        assert!(
            results
                .iter()
                .all(|r| matches!(r.detail, AddDetail::Success { .. }))
        );
        assert_eq!(done_calls.load(Ordering::SeqCst), 3);
        assert_eq!(success_calls.load(Ordering::SeqCst), 3);
    }
}
