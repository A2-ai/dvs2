use std::path::{Path, PathBuf};
use std::sync::Mutex;

use anyhow::Result;
use rayon::prelude::*;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::backends::Backend;
use crate::cache::{HashCache, try_open_cache};
use crate::config::Compression;
use crate::files::metadata::FileMetadata;
use crate::gitignore::add_to_gitignore;
use crate::paths::{AddPathStatus, DvsPaths};
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
        stored_size: Option<u64>,
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
    let pool = get_threadpool(matched_paths.len())?;
    let cache = try_open_cache(paths);
    let operation_id = Uuid::new_v4();

    let mut results: Vec<AddResult> = pool.install(|| {
        matched_paths
            .into_par_iter()
            .map(|(relative_path, status)| {
                let full_path = paths.file_path(&relative_path);
                let file_size = std::fs::metadata(&full_path).map(|m| m.len()).unwrap_or(0);
                let file_progress = on_file_start.map(|f| f(&relative_path, file_size));
                let on_bytes = file_progress.as_ref().map(|fp| &*fp.on_bytes);
                let on_done = file_progress.as_ref().map(|fp| &*fp.on_done);
                let finish = |ok| {
                    if let Some(done) = on_done {
                        done(ok);
                    }
                };

                match status {
                    AddPathStatus::NotFound => {
                        finish(false);
                        return AddResult {
                            path: relative_path,
                            detail: AddDetail::Error {
                                error: "file not found".to_string(),
                            },
                        };
                    }
                    AddPathStatus::OutsideProject => {
                        finish(false);
                        return AddResult {
                            path: relative_path,
                            detail: AddDetail::Error {
                                error: "path is outside project".to_string(),
                            },
                        };
                    }
                    AddPathStatus::IsDirectory => {
                        finish(false);
                        return AddResult {
                            path: relative_path,
                            detail: AddDetail::Error {
                                error: "path is a directory".to_string(),
                            },
                        };
                    }
                    AddPathStatus::Valid => {}
                }

                match full_path.canonicalize() {
                    Ok(canonical) if !canonical.starts_with(paths.repo_root()) => {
                        finish(false);
                        return AddResult {
                            path: relative_path,
                            detail: AddDetail::Error {
                                error: "path is outside the dvs repository".to_string(),
                            },
                        };
                    }
                    Err(e) => {
                        finish(false);
                        return AddResult {
                            path: relative_path,
                            detail: AddDetail::Error {
                                error: format!("failed to resolve path: {e}"),
                            },
                        };
                    }
                    _ => {} // ok
                }
                match add_file(
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
                        finish(true);
                        log::info!(
                            "Successfully added {} ({:?})",
                            relative_path.display(),
                            outcome
                        );
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
                        finish(false);
                        log::warn!("Failed to add {}: {e}", relative_path.display());
                        AddResult {
                            path: relative_path,
                            detail: AddDetail::Error {
                                error: e.to_string(),
                            },
                        }
                    }
                }
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
    use crate::FileProgress;
    use crate::testutil::{create_file, create_temp_git_repo, init_dvs_repo};
    use std::fs;
    use std::path::Path;
    use std::sync::{Arc, Mutex};

    fn make_paths(root: &Path, config: &crate::config::Config) -> DvsPaths {
        DvsPaths::new(
            root.to_path_buf(),
            root.to_path_buf(),
            config.metadata_folder_name(),
        )
        .unwrap()
    }

    #[test]
    fn add_files_reports_not_found_per_file() {
        let (_tmp, root) = create_temp_git_repo();
        let (config, _dvs_dir) = init_dvs_repo(&root);
        let backend = config.backend();
        let paths = make_paths(&root, &config);

        create_file(&root, "a.txt", b"a");

        let results = add_files(
            vec!["nonexistent.csv".into()],
            &paths,
            backend,
            None,
            Compression::Zstd,
            false,
            None,
        )
        .unwrap();
        assert_eq!(results.len(), 1);
        assert!(
            matches!(&results[0].detail, AddDetail::Error { error } if error.contains("not found"))
        );
    }

    #[test]
    fn add_files_mixed_statuses() {
        let (_tmp, root) = create_temp_git_repo();
        let (config, _dvs_dir) = init_dvs_repo(&root);
        let backend = config.backend();
        let paths = make_paths(&root, &config);

        // Valid file
        create_file(&root, "a.txt", b"a");

        // Outside-project file
        let outside_tmp = tempfile::tempdir().unwrap();
        let outside_file = fs::canonicalize(outside_tmp.path())
            .unwrap()
            .join("outside.txt");
        fs::write(&outside_file, b"outside").unwrap();
        let outside_relative =
            PathBuf::from("..").join(outside_file.strip_prefix(root.parent().unwrap()).unwrap());

        // Directory inside the repo
        fs::create_dir(root.join("subdir")).unwrap();

        let results = add_files(
            vec![
                "a.txt".into(),
                "missing.csv".into(),
                outside_relative,
                "subdir".into(),
            ],
            &paths,
            backend,
            None,
            Compression::Zstd,
            false,
            None,
        )
        .unwrap();
        assert_eq!(results.len(), 4);

        let valid = results.iter().find(|r| *r.path == *"a.txt").unwrap();
        assert!(matches!(
            &valid.detail,
            AddDetail::Success { outcome: Outcome::Copied, hash, size, .. }
            if !hash.is_empty() && *size > 0
        ));

        let missing = results.iter().find(|r| *r.path == *"missing.csv").unwrap();
        assert!(
            matches!(&missing.detail, AddDetail::Error { error } if error.contains("not found"))
        );

        let outside = results.iter().find(|r| {
            matches!(&r.detail, AddDetail::Error { error } if error.contains("outside project"))
        });
        assert!(outside.is_some());

        let dir = results.iter().find(|r| *r.path == *"subdir").unwrap();
        assert!(matches!(&dir.detail, AddDetail::Error { error } if error.contains("directory")));
    }

    #[test]
    fn add_files_calls_on_done_for_each_attempt() {
        let (_tmp, root) = create_temp_git_repo();
        let (config, _dvs_dir) = init_dvs_repo(&root);
        let backend = config.backend();
        let paths = make_paths(&root, &config);

        create_file(&root, "a.txt", b"a");

        let done_events = Arc::new(Mutex::new(Vec::new()));
        let on_file_start = {
            let done_events = Arc::clone(&done_events);
            move |path: &Path, _size: u64| {
                let path = path.to_path_buf();
                let done_events = Arc::clone(&done_events);
                FileProgress {
                    on_bytes: Box::new(|_| {}),
                    on_done: Box::new(move |ok| {
                        done_events
                            .lock()
                            .unwrap()
                            .push((path.to_string_lossy().into_owned(), ok));
                    }),
                }
            }
        };

        let results = add_files(
            vec!["a.txt".into(), "missing.csv".into()],
            &paths,
            backend,
            None,
            Compression::Zstd,
            false,
            Some(&on_file_start),
        )
        .unwrap();

        assert_eq!(results.len(), 2);

        let mut events = done_events.lock().unwrap().clone();
        events.sort();
        assert_eq!(
            events,
            vec![
                ("a.txt".to_string(), true),
                ("missing.csv".to_string(), false),
            ]
        );
    }
}
