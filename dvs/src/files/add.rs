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
use crate::files::types::OutputOptions;
use crate::gitignore::add_to_gitignore;
use crate::paths::{AddPathStatus, DvsPaths};
use crate::utils::get_threadpool;
use crate::utils::format_size;
use crate::{Outcome, cache};

/// Options specific to the add command.
#[derive(Debug, Clone)]
pub struct AddOptions {
    pub message: Option<String>,
    pub compression: Compression,
    #[allow(dead_code)]
    pub output: OutputOptions,
}

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
    },
    Error {
        error: String,
    },
}

fn add_file(
    relative_path: &Path,
    paths: &DvsPaths,
    backend: &dyn Backend,
    cache: Option<&Mutex<HashCache>>,
    operation_id: Uuid,
    opts: &AddOptions,
) -> Result<(Outcome, FileMetadata)> {
    let verbose = opts.output.verbose;
    let full_path = paths.file_path(relative_path);
    let rel_str = relative_path.to_string_lossy();
    let (hashes, size) = cache::hashes_for_file(&full_path, &rel_str, cache, verbose)?;
    if verbose {
        eprintln!(
            "  [{}] File size: {}",
            rel_str,
            format_size(size)
        );
    }
    let metadata = FileMetadata::from_hashes(hashes, size, opts.compression, opts.message.clone());
    if opts.output.dry_run {
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
        if verbose {
            eprintln!("  [{rel_str}] Dry run: would be {outcome:?}");
        }
        Ok((outcome, metadata))
    } else {
        let outcome = metadata.save(operation_id, &full_path, backend, paths, relative_path, verbose)?;
        Ok((outcome, metadata))
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
    opts: &AddOptions,
) -> Result<Vec<AddResult>> {
    let verbose = opts.output.verbose;
    if verbose {
        eprintln!(
            "Adding {} file{} (hash: blake3, compression: {:?})...",
            files.len(),
            if files.len() == 1 { "" } else { "s" },
            opts.compression,
        );
    }
    let matched_paths = paths.validate_for_add(&files);
    let pool = get_threadpool(matched_paths.len())?;
    let cache = try_open_cache(paths);
    let operation_id = Uuid::new_v4();

    let total_start = verbose.then(std::time::Instant::now);
    let mut results: Vec<AddResult> = pool.install(|| {
        matched_paths
            .into_par_iter()
            .map(|(relative_path, status)| {
                let rel_display = relative_path.display();
                match status {
                    AddPathStatus::NotFound => {
                        if verbose {
                            eprintln!("  [{rel_display}] Skipped: file not found");
                        }
                        return AddResult {
                            path: relative_path,
                            detail: AddDetail::Error {
                                error: "file not found".to_string(),
                            },
                        };
                    }
                    AddPathStatus::OutsideProject => {
                        if verbose {
                            eprintln!("  [{rel_display}] Skipped: path is outside project");
                        }
                        return AddResult {
                            path: relative_path,
                            detail: AddDetail::Error {
                                error: "path is outside project".to_string(),
                            },
                        };
                    }
                    AddPathStatus::IsDirectory => {
                        if verbose {
                            eprintln!("  [{rel_display}] Skipped: path is a directory");
                        }
                        return AddResult {
                            path: relative_path,
                            detail: AddDetail::Error {
                                error: "path is a directory".to_string(),
                            },
                        };
                    }
                    AddPathStatus::Valid => {}
                }

                let full_path = paths.file_path(&relative_path);
                match full_path.canonicalize() {
                    Ok(canonical) if !canonical.starts_with(paths.repo_root()) => {
                        if verbose {
                            eprintln!("  [{rel_display}] Skipped: path is outside the dvs repository");
                        }
                        return AddResult {
                            path: relative_path,
                            detail: AddDetail::Error {
                                error: "path is outside the dvs repository".to_string(),
                            },
                        };
                    }
                    Err(e) => {
                        if verbose {
                            eprintln!("  [{rel_display}] Skipped: failed to resolve path: {e}");
                        }
                        return AddResult {
                            path: relative_path,
                            detail: AddDetail::Error {
                                error: format!("failed to resolve path: {e}"),
                            },
                        };
                    }
                    _ => {} // ok
                }

                let file_start = verbose.then(std::time::Instant::now);
                match add_file(
                    &relative_path,
                    paths,
                    backend,
                    cache.as_ref(),
                    operation_id,
                    opts,
                ) {
                    Ok((outcome, metadata)) => {
                        if let Some(file_start) = file_start {
                            eprintln!(
                                "  [{rel_display}] Completed in {:.2?}",
                                file_start.elapsed()
                            );
                        }
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
                            },
                        }
                    }
                    Err(e) => {
                        if let Some(file_start) = file_start {
                            eprintln!(
                                "  [{rel_display}] Failed in {:.2?}: {e}",
                                file_start.elapsed()
                            );
                        }
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
    if !opts.output.dry_run && !successful_paths.is_empty() {
        if verbose {
            eprintln!("Updating .gitignore...");
        }
        if let Err(e) = add_to_gitignore(paths.repo_root(), &successful_paths) {
            log::warn!("Failed to update .gitignore: {e}");
        }
    }

    if let Some(total_start) = total_start {
        let total_elapsed = total_start.elapsed();
        let n_ok = successful_paths.len();
        let n_err = results.len() - n_ok;
        eprintln!("Done in {total_elapsed:.2?}: {n_ok} succeeded, {n_err} failed");
    }

    Ok(results)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testutil::{create_file, create_temp_git_repo, init_dvs_repo};
    use std::path::Path;

    fn make_paths(root: &Path, config: &crate::config::Config) -> DvsPaths {
        DvsPaths::new(
            root.to_path_buf(),
            root.to_path_buf(),
            config.metadata_folder_name(),
        )
    }

    fn default_add_opts() -> AddOptions {
        AddOptions {
            message: None,
            compression: Compression::Zstd,
            output: OutputOptions::default(),
        }
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
            &default_add_opts(),
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
        let outside_file = outside_tmp.path().join("outside.txt");
        std::fs::write(&outside_file, b"outside").unwrap();
        let outside_relative =
            PathBuf::from("..").join(outside_file.strip_prefix(root.parent().unwrap()).unwrap());

        // Directory inside the repo
        std::fs::create_dir(root.join("subdir")).unwrap();

        let results = add_files(
            vec![
                "a.txt".into(),
                "missing.csv".into(),
                outside_relative,
                "subdir".into(),
            ],
            &paths,
            backend,
            &default_add_opts(),
        )
        .unwrap();
        assert_eq!(results.len(), 4);

        let valid = results
            .iter()
            .find(|r| r.path == Path::new("a.txt"))
            .unwrap();
        assert!(matches!(
            &valid.detail,
            AddDetail::Success { outcome: Outcome::Copied, hash, size }
            if !hash.is_empty() && *size > 0
        ));

        let missing = results
            .iter()
            .find(|r| r.path == Path::new("missing.csv"))
            .unwrap();
        assert!(
            matches!(&missing.detail, AddDetail::Error { error } if error.contains("not found"))
        );

        let outside = results.iter().find(|r| {
            matches!(&r.detail, AddDetail::Error { error } if error.contains("outside project"))
        });
        assert!(outside.is_some());

        let dir = results
            .iter()
            .find(|r| r.path == Path::new("subdir"))
            .unwrap();
        assert!(matches!(&dir.detail, AddDetail::Error { error } if error.contains("directory")));
    }
}
