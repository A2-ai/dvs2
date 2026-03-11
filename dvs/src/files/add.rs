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
use crate::files::types::TimingRecord;
use crate::gitignore::add_to_gitignore;
use crate::paths::{AddPathStatus, DvsPaths};
use crate::utils::format_size;
use crate::utils::get_threadpool;
use crate::{Outcome, OutputOptions, cache};

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

#[allow(clippy::too_many_arguments)]
fn add_file(
    relative_path: &Path,
    paths: &DvsPaths,
    backend: &dyn Backend,
    cache: Option<&Mutex<HashCache>>,
    operation_id: Uuid,
    message: Option<String>,
    compression: Compression,
    output: &OutputOptions,
) -> Result<(Outcome, FileMetadata)> {
    let v2 = output.verbosity >= 2;
    let full_path = paths.file_path(relative_path);
    let rel_str = relative_path.to_string_lossy();
    let (hashes, size) = cache::hashes_for_file(&full_path, &rel_str, cache, output)?;
    if v2 {
        eprintln!("  [{}] File size: {}", rel_str, format_size(size));
    }
    let metadata = FileMetadata::from_hashes(hashes, size, compression, message);
    if output.dry_run {
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
        if v2 {
            eprintln!("  [{rel_str}] Dry run: would be {outcome:?}");
        }
        Ok((outcome, metadata))
    } else {
        let outcome = metadata.save(
            operation_id,
            &full_path,
            backend,
            paths,
            relative_path,
            output,
        )?;
        Ok((outcome, metadata))
    }
}

/// Adds files matching a glob pattern to DVS.
///
/// The pattern is matched against files relative to cwd.
/// Files are stored with paths relative to repo_root.
#[allow(clippy::too_many_arguments)]
pub fn add_files(
    files: Vec<PathBuf>,
    paths: &DvsPaths,
    backend: &dyn Backend,
    message: Option<String>,
    compression: Compression,
    output: &OutputOptions,
) -> Result<Vec<AddResult>> {
    let v1 = output.verbosity >= 1;
    let v2 = output.verbosity >= 2;
    if v1 {
        eprintln!(
            "Adding {} file{} (hash: blake3, compression: {:?})...",
            files.len(),
            if files.len() == 1 { "" } else { "s" },
            compression,
        );
    }
    let matched_paths = paths.validate_for_add(&files);
    let pool = get_threadpool(matched_paths.len())?;
    let cache = try_open_cache(paths);
    let operation_id = Uuid::new_v4();
    let canon_root = paths.repo_root().canonicalize().unwrap_or_else(|_| paths.repo_root().to_path_buf());

    let total_start = v1.then(std::time::Instant::now);
    let mut results: Vec<AddResult> = pool.install(|| {
        matched_paths
            .into_par_iter()
            .map(|(relative_path, status)| {
                let rel_display = relative_path.display();
                match status {
                    AddPathStatus::NotFound => {
                        if v2 {
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
                        if v2 {
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
                        if v2 {
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
                    Ok(canonical) if !canonical.starts_with(&canon_root) => {
                        if v2 {
                            eprintln!(
                                "  [{rel_display}] Skipped: path is outside the dvs repository"
                            );
                        }
                        return AddResult {
                            path: relative_path,
                            detail: AddDetail::Error {
                                error: "path is outside the dvs repository".to_string(),
                            },
                        };
                    }
                    Err(e) => {
                        if v2 {
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

                let file_start = v1.then(std::time::Instant::now);
                match add_file(
                    &relative_path,
                    paths,
                    backend,
                    cache.as_ref(),
                    operation_id,
                    message.clone(),
                    compression,
                    output,
                ) {
                    Ok((outcome, metadata)) => {
                        if let Some(file_start) = file_start {
                            let elapsed = file_start.elapsed();
                            eprintln!("  [{rel_display}] Completed in {elapsed:.2?}",);
                            output.send_timing(TimingRecord {
                                file: relative_path.display().to_string(),
                                step: "add_file_total".into(),
                                duration_ms: elapsed.as_secs_f64() * 1000.0,
                                file_size_bytes: Some(metadata.size),
                                compression: format!("{:?}", compression),
                                ..output.timing_template("add")
                            });
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
    if !output.dry_run && !successful_paths.is_empty() {
        if v2 {
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
        output.send_timing(TimingRecord {
            file: String::new(),
            step: "add_total".into(),
            duration_ms: total_elapsed.as_secs_f64() * 1000.0,
            file_size_bytes: None,
            num_files: Some(results.len()),
            compression: format!("{:?}", compression),
            ..output.timing_template("add")
        });
    }

    Ok(results)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testutil::{create_file, create_temp_git_repo, init_dvs_repo};
    use std::fs;
    use std::path::Path;

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
            &OutputOptions::default(),
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
            None,
            Compression::Zstd,
            &OutputOptions::default(),
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
