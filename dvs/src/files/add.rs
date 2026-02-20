use std::path::{Path, PathBuf};

use anyhow::Result;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::backends::Backend;
use crate::cache::HashCache;
use crate::config::Compression;
use crate::files::metadata::FileMetadata;
use crate::gitignore::add_to_gitignore;
use crate::paths::DvsPaths;
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
    },
    Error {
        error: String,
    },
}

fn add_file(
    relative_path: &Path,
    paths: &DvsPaths,
    backend: &dyn Backend,
    cache: Option<&HashCache>,
    operation_id: Uuid,
    message: Option<String>,
    compression: Compression,
) -> Result<(Outcome, FileMetadata)> {
    let full_path = paths.file_path(relative_path);
    let rel_str = relative_path.to_string_lossy();
    let (hashes, size) = cache::hashes_for_file(&full_path, &rel_str, cache)?;
    let metadata = FileMetadata::from_hashes(hashes, size, compression, message);
    let outcome = metadata.save(operation_id, &full_path, backend, paths, relative_path)?;
    Ok((outcome, metadata))
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
) -> Result<Vec<AddResult>> {
    let matched_paths = paths.validate_for_add(&files);
    let cache = match cache::open_cache(paths) {
        Ok(c) => Some(c),
        Err(e) => {
            log::warn!("Failed to open hash cache: {e}");
            None
        }
    };
    let mut results = Vec::new();
    let operation_id = Uuid::new_v4();

    for (relative_path, exists) in matched_paths {
        if !exists {
            results.push(AddResult {
                path: relative_path,
                detail: AddDetail::Error {
                    error: "file not found".to_string(),
                },
            });
            continue;
        }

        match add_file(
            &relative_path,
            paths,
            backend,
            cache.as_ref(),
            operation_id,
            message.clone(),
            compression,
        ) {
            Ok((outcome, metadata)) => {
                log::info!(
                    "Successfully added {} ({:?})",
                    relative_path.display(),
                    outcome
                );
                results.push(AddResult {
                    path: relative_path,
                    detail: AddDetail::Success {
                        outcome,
                        hash: metadata.hashes.blake3,
                        size: metadata.size,
                    },
                });
            }
            Err(e) => {
                log::warn!("Failed to add {}: {e}", relative_path.display());
                results.push(AddResult {
                    path: relative_path,
                    detail: AddDetail::Error {
                        error: e.to_string(),
                    },
                });
            }
        }
    }

    let successful_paths: Vec<_> = results
        .iter()
        .filter(|r| matches!(r.detail, AddDetail::Success { .. }))
        .map(|r| r.path.clone())
        .collect();
    if !successful_paths.is_empty() {
        if let Err(e) = add_to_gitignore(paths.repo_root(), &successful_paths) {
            log::warn!("Failed to update .gitignore: {e}");
        }
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
        )
        .unwrap();
        assert_eq!(results.len(), 1);
        assert!(
            matches!(&results[0].detail, AddDetail::Error { error } if error.contains("not found"))
        );
    }

    #[test]
    fn add_files_mixed_valid_and_missing() {
        let (_tmp, root) = create_temp_git_repo();
        let (config, _dvs_dir) = init_dvs_repo(&root);
        let backend = config.backend();
        let paths = make_paths(&root, &config);

        create_file(&root, "a.txt", b"a");

        let results = add_files(
            vec!["a.txt".into(), "missing.csv".into()],
            &paths,
            backend,
            None,
            Compression::Zstd,
        )
        .unwrap();
        assert_eq!(results.len(), 2);

        // First file succeeded
        assert!(matches!(
            &results[0].detail,
            AddDetail::Success { outcome: Outcome::Copied, hash, size }
            if !hash.is_empty() && *size > 0
        ));

        // Second file failed
        assert!(
            matches!(&results[1].detail, AddDetail::Error { error } if error.contains("not found"))
        );
    }
}
