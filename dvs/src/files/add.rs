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
) -> Result<crate::BatchOutcome<AddSuccess, AddError>> {
    let matched_paths = paths.validate_for_add(&files);
    let pool = get_threadpool(matched_paths.len())?;
    let cache = try_open_cache(paths);
    let operation_id = Uuid::new_v4();

    let mut rows: Vec<Row> = pool.install(|| {
        matched_paths
            .into_par_iter()
            .map(|(relative_path, status)| match status {
                AddPathStatus::NotFound => Row::Err(AddError::NotFound {
                    path: relative_path,
                }),
                AddPathStatus::OutsideProject => Row::Err(AddError::OutsideProject {
                    path: relative_path,
                }),
                AddPathStatus::IsDirectory => Row::Err(AddError::IsDirectory {
                    path: relative_path,
                }),
                AddPathStatus::Valid => {
                    let full_path = paths.file_path(&relative_path);
                    match full_path.canonicalize() {
                        Ok(canonical) if !canonical.starts_with(paths.repo_root()) => {
                            return Row::Err(AddError::OutsideProject {
                                path: relative_path,
                            });
                        }
                        Err(e) => {
                            return Row::Err(AddError::PathResolution {
                                path: relative_path,
                                reason: e.to_string(),
                            });
                        }
                        _ => {}
                    }
                    let file_size = std::fs::metadata(&full_path).map(|m| m.len()).unwrap_or(0);
                    let file_progress = on_file_start.map(|f| f(&relative_path, file_size));
                    let on_bytes = file_progress.as_ref().map(|fp| &*fp.on_bytes);
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
                            log::info!(
                                "Successfully added {} ({:?})",
                                relative_path.display(),
                                outcome
                            );
                            Row::Ok(AddSuccess {
                                path: relative_path,
                                outcome,
                                hash: metadata.hashes.blake3,
                                size: metadata.size,
                                stored_size,
                            })
                        }
                        Err(e) => {
                            log::warn!("Failed to add {}: {e}", relative_path.display());
                            classify_add_runtime_err(relative_path, e)
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

    let mut outcome = crate::BatchOutcome::<AddSuccess, AddError>::new();
    for row in rows {
        match row {
            Row::Ok(success) => outcome.ok.push(success),
            Row::Err(err) => outcome.err.push(err),
        }
    }

    let successful_paths: Vec<_> = outcome.ok.iter().map(|s| s.path.clone()).collect();
    if !dry_run && !successful_paths.is_empty() {
        if let Err(e) = add_to_gitignore(paths.repo_root(), &successful_paths) {
            log::warn!("Failed to update .gitignore: {e}");
        }
    }

    Ok(outcome)
}

/// Successful add of a single file.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AddSuccess {
    pub path: PathBuf,
    pub outcome: Outcome,
    pub hash: String,
    pub size: u64,
    pub stored_size: Option<u64>,
}

/// Structured failure for a single file in `add_files`.
/// Serializes with a `"kind"` tag so R / JSON consumers can group on it.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum AddError {
    /// Pre-flight: path does not exist on disk.
    NotFound { path: PathBuf },
    /// Pre-flight: path escapes the repo root.
    OutsideProject { path: PathBuf },
    /// Pre-flight: path refers to a directory; `dvs` versions files only.
    IsDirectory { path: PathBuf },
    /// Pre-flight: canonicalize / resolve failed.
    PathResolution { path: PathBuf, reason: String },
    /// Pre-flight: glob pattern could not be compiled or walked.
    GlobFailure { pattern: String, reason: String },
    /// Runtime: hashing the local file failed.
    HashFailure { path: PathBuf, reason: String },
    /// Runtime: writing the stored object / metadata failed.
    StorageWrite { path: PathBuf, reason: String },
}

impl AddError {
    /// The path this error refers to, if any. `None` for `GlobFailure`.
    pub fn path(&self) -> Option<&Path> {
        match self {
            AddError::NotFound { path }
            | AddError::OutsideProject { path }
            | AddError::IsDirectory { path }
            | AddError::PathResolution { path, .. }
            | AddError::HashFailure { path, .. }
            | AddError::StorageWrite { path, .. } => Some(path.as_path()),
            AddError::GlobFailure { .. } => None,
        }
    }

    /// The serde `kind` string. Stable identifier, safe for scripting.
    pub fn kind(&self) -> &'static str {
        match self {
            AddError::NotFound { .. } => "not_found",
            AddError::OutsideProject { .. } => "outside_project",
            AddError::IsDirectory { .. } => "is_directory",
            AddError::PathResolution { .. } => "path_resolution",
            AddError::GlobFailure { .. } => "glob_failure",
            AddError::HashFailure { .. } => "hash_failure",
            AddError::StorageWrite { .. } => "storage_write",
        }
    }
}

#[derive(Debug)]
enum Row {
    Ok(AddSuccess),
    Err(AddError),
}

/// Bucket a runtime add error into the closest structured AddError variant.
fn classify_add_runtime_err(path: PathBuf, err: anyhow::Error) -> Row {
    let reason = err.to_string();
    if reason.contains("hash") || reason.contains("blake3") {
        Row::Err(AddError::HashFailure { path, reason })
    } else {
        Row::Err(AddError::StorageWrite { path, reason })
    }
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

        let outcome = add_files(
            vec!["nonexistent.csv".into()],
            &paths,
            backend,
            None,
            Compression::Zstd,
            false,
            None,
        )
        .unwrap();
        assert!(outcome.ok.is_empty());
        assert_eq!(outcome.err.len(), 1);
        assert!(matches!(outcome.err[0], AddError::NotFound { .. }));
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

        let outcome = add_files(
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
        assert_eq!(outcome.ok.len(), 1);
        assert_eq!(outcome.ok[0].path, PathBuf::from("a.txt"));
        assert_eq!(outcome.ok[0].outcome, Outcome::Copied);

        assert_eq!(outcome.err.len(), 3);
        assert!(
            outcome
                .err
                .iter()
                .any(|e| matches!(e, AddError::NotFound { .. }))
        );
        assert!(
            outcome
                .err
                .iter()
                .any(|e| matches!(e, AddError::OutsideProject { .. }))
        );
        assert!(
            outcome
                .err
                .iter()
                .any(|e| matches!(e, AddError::IsDirectory { .. }))
        );
    }

    #[test]
    fn add_error_serializes_with_snake_case_kind() {
        let cases: Vec<(AddError, &str)> = vec![
            (AddError::NotFound { path: "a.txt".into() }, "not_found"),
            (
                AddError::OutsideProject {
                    path: "../x".into(),
                },
                "outside_project",
            ),
            (
                AddError::IsDirectory {
                    path: "data".into(),
                },
                "is_directory",
            ),
            (
                AddError::PathResolution {
                    path: "a".into(),
                    reason: "x".into(),
                },
                "path_resolution",
            ),
            (
                AddError::GlobFailure {
                    pattern: "*.csv".into(),
                    reason: "x".into(),
                },
                "glob_failure",
            ),
            (
                AddError::HashFailure {
                    path: "a".into(),
                    reason: "x".into(),
                },
                "hash_failure",
            ),
            (
                AddError::StorageWrite {
                    path: "a".into(),
                    reason: "x".into(),
                },
                "storage_write",
            ),
        ];
        for (err, expected_kind) in cases {
            assert_eq!(err.kind(), expected_kind);
            let json = serde_json::to_value(&err).unwrap();
            assert_eq!(json["kind"], expected_kind, "for variant {err:?}");
        }
    }

    #[test]
    fn add_error_path_accessor() {
        let e = AddError::NotFound {
            path: "a.txt".into(),
        };
        assert_eq!(e.path().unwrap(), Path::new("a.txt"));

        let e = AddError::GlobFailure {
            pattern: "*".into(),
            reason: "x".into(),
        };
        assert!(e.path().is_none());
    }

    #[test]
    fn add_success_roundtrips_json() {
        let s = AddSuccess {
            path: "a.txt".into(),
            outcome: Outcome::Copied,
            hash: "deadbeef".into(),
            size: 42,
            stored_size: Some(30),
        };
        let json = serde_json::to_string(&s).unwrap();
        let back: AddSuccess = serde_json::from_str(&json).unwrap();
        assert_eq!(back.hash, "deadbeef");
        assert_eq!(back.size, 42);
    }
}
