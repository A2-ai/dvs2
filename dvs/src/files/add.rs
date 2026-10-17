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
use crate::files::metadata::FileMetadata;
use crate::gitignore::add_to_gitignore;
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

/// A file to add: the metadata it will get, waiting on the backend.
struct Pending {
    relative_path: PathBuf,
    metadata: FileMetadata,
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
) -> Result<(Outcome, Option<u64>)> {
    let full_path = paths.file_path(&pending.relative_path);
    if dry_run {
        let outcome = if pending.in_sync(paths) && backend.exists(&pending.metadata.hashes)? {
            Outcome::Present
        } else {
            Outcome::Copied
        };
        return Ok((outcome, None));
    }
    match missing {
        // The local backend checks for itself whether it has the content
        None => pending.metadata.save(
            operation_id,
            &full_path,
            backend,
            paths,
            &pending.relative_path,
            on_bytes,
        ),
        Some(missing) if missing.contains(&pending.metadata.hashes.blake3) => {
            let stored_size = pending.metadata.store_and_write(
                operation_id,
                &full_path,
                backend,
                paths,
                &pending.relative_path,
                on_bytes,
            )?;
            Ok((Outcome::Copied, Some(stored_size)))
        }
        // The server has the content and recorded the add from the manifest
        Some(_) if pending.in_sync(paths) => Ok((Outcome::Present, None)),
        Some(_) => {
            pending.metadata.write(paths, &pending.relative_path)?;
            Ok((Outcome::Copied, None))
        }
    }
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

    // 1. Hash everything first: the server wants the whole list before any bytes move
    let (mut pending, mut results): (Vec<Pending>, Vec<AddResult>) = pool.install(|| {
        valid_paths.into_par_iter().partition_map(|relative_path| {
            let full_path = paths.file_path(&relative_path);
            let rel_str = relative_path.to_string_lossy();
            match cache::hashes_for_file(&full_path, &rel_str, cache.as_ref()) {
                Ok((hashes, size)) => Either::Left(Pending {
                    metadata: FileMetadata::from_hashes(hashes, size, compression, message.clone()),
                    relative_path,
                }),
                Err(e) => {
                    log::warn!("Failed to add {}: {e}", relative_path.display());
                    Either::Right(AddResult {
                        path: relative_path,
                        detail: AddDetail::Error {
                            error: e.to_string(),
                        },
                    })
                }
            }
        })
    });

    // 2. Tell the server what we are about to add, minus files already in sync
    let missing = match backend {
        Backend::Server(server) if !dry_run => {
            let entries = pending
                .iter()
                .filter(|p| !p.in_sync(paths))
                .map(Pending::manifest_entry)
                .collect::<Result<Vec<_>>>()?;
            let checkout = crate::git::current_checkout(paths.repo_root());
            let mut missing = HashSet::new();
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
                missing = answer.missing.into_iter().collect();
            }
            Some(missing)
        }
        _ => None,
    };

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
                let result = match added {
                    Ok((outcome, stored_size)) => {
                        log::info!(
                            "Successfully added {} ({:?})",
                            pending.relative_path.display(),
                            outcome
                        );
                        AddResult {
                            path: pending.relative_path,
                            detail: AddDetail::Success {
                                outcome,
                                hash: pending.metadata.hashes.blake3,
                                size: pending.metadata.size,
                                stored_size: stored_size.unwrap_or(pending.metadata.size),
                            },
                        }
                    }
                    Err(e) => {
                        log::warn!("Failed to add {}: {e}", pending.relative_path.display());
                        AddResult {
                            path: pending.relative_path,
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
            .collect::<Vec<_>>()
    }));
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
