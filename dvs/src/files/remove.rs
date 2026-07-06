use std::path::PathBuf;

use anyhow::{Result, bail};
use fs_err as fs;
use serde::{Deserialize, Serialize};

use crate::gitignore::remove_from_gitignore;
use crate::paths::DvsPaths;

/// Result of removing (untracking) a single file.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RemoveResult {
    pub path: PathBuf,
    #[serde(flatten)]
    pub detail: RemoveDetail,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum RemoveDetail {
    // `Error` is listed before the fieldless `Success` so untagged
    // deserialization tries it first. `Success {}` matches any map, so it would
    // otherwise swallow an `{"error": ...}` result.
    Error { error: String },
    Success {},
}

/// Stops tracking the given files without touching their stored data or their
/// local copies.
///
/// For each file it deletes the metadata sidecar
/// (`<metadata folder>/<path>.dvs`) and drops the file's `.gitignore` entry. It
/// never reads or removes storage blobs and never touches the local data file,
/// and it is not recorded in the audit log.
///
/// Every input is validated first: a path is removable only if it is tracked
/// (its sidecar exists). If any path is not tracked the whole batch is refused
/// and nothing is removed, mirroring the fail-fast validation of `add` and
/// `get`. Once every input resolves the removal is best-effort: a file whose
/// sidecar cannot be deleted (for example a permission error) is reported and
/// does not stop the others.
pub fn remove_files(paths: Vec<PathBuf>, dvs_paths: &DvsPaths) -> Result<Vec<RemoveResult>> {
    let matched_paths = dvs_paths.validate_for_get(&paths);
    if matched_paths.is_empty() {
        return Ok(Vec::new());
    }

    // Validate every path up front, we bail if any is not tracked.
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
            "Refusing to remove, the following paths are not tracked:\n{}",
            invalid.join("\n")
        );
    }

    let tracked_paths = matched_paths
        .into_iter()
        .map(|(path, _)| path)
        .collect::<Vec<_>>();

    // Best-effort per file: delete the metadata sidecar. Storage blobs and the
    // local data file are never touched.
    let mut results: Vec<RemoveResult> = tracked_paths
        .into_iter()
        .map(|relative_path| {
            let sidecar = dvs_paths.metadata_path(&relative_path);
            match fs::remove_file(&sidecar) {
                Ok(()) => {
                    log::info!("Stopped tracking {}", relative_path.display());
                    RemoveResult {
                        path: relative_path,
                        detail: RemoveDetail::Success {},
                    }
                }
                Err(e) => {
                    log::warn!("Failed to remove {}: {e}", relative_path.display());
                    RemoveResult {
                        path: relative_path,
                        detail: RemoveDetail::Error {
                            error: e.to_string(),
                        },
                    }
                }
            }
        })
        .collect();
    results.sort_by(|a, b| a.path.cmp(&b.path));

    // Drop the gitignore entries for the files we untracked. Like `add`, a
    // gitignore failure is logged and does not fail the operation.
    let removed_paths: Vec<_> = results
        .iter()
        .filter(|r| matches!(r.detail, RemoveDetail::Success {}))
        .map(|r| r.path.clone())
        .collect();
    if !removed_paths.is_empty() {
        if let Err(e) = remove_from_gitignore(dvs_paths.repo_root(), &removed_paths) {
            log::warn!("Failed to update .gitignore: {e}");
        }
    }

    Ok(results)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::add_files;
    use crate::config::Compression;
    use crate::files::add::AddDetail;
    use crate::testutil::{create_file, create_temp_git_repo, init_dvs_repo};
    use std::path::Path;

    fn make_paths(root: &Path, config: &crate::config::Config) -> DvsPaths {
        DvsPaths::new(
            root.to_path_buf(),
            root.to_path_buf(),
            config.metadata_folder_name(),
        )
        .unwrap()
    }

    /// Adds `name` and returns its blake3 hash, so tests can assert the storage
    /// blob outlives a remove.
    fn add_one(
        paths: &DvsPaths,
        backend: &dyn crate::Backend,
        name: &str,
        content: &[u8],
    ) -> String {
        create_file(paths.repo_root(), name, content);
        let results = add_files(
            vec![name.into()],
            paths,
            backend,
            None,
            Compression::Zstd,
            false,
            None,
        )
        .unwrap();
        match &results[0].detail {
            AddDetail::Success { hash, .. } => hash.clone(),
            AddDetail::Error { error } => panic!("add failed: {error}"),
        }
    }

    fn blob_path(root: &Path, hash: &str) -> PathBuf {
        root.parent()
            .unwrap()
            .join(".storage")
            .join(&hash[..2])
            .join(&hash[2..])
    }

    #[test]
    fn remove_untracks_but_keeps_data_and_blob() {
        let (_tmp, root) = create_temp_git_repo();
        let (config, _dvs_dir) = init_dvs_repo(&root);
        let backend = config.backend();
        let paths = make_paths(&root, &config);

        let hash = add_one(&paths, backend, "data.csv", b"payload");
        let sidecar = paths.metadata_path(Path::new("data.csv"));
        assert!(sidecar.is_file(), "sidecar should exist after add");
        assert!(blob_path(&root, &hash).is_file(), "blob should exist");
        let gitignore = fs::read_to_string(root.join(".gitignore")).unwrap();
        assert!(
            gitignore.contains("/data.csv"),
            "add writes gitignore entry"
        );

        let results = remove_files(vec!["data.csv".into()], &paths).unwrap();
        assert_eq!(results.len(), 1);
        assert!(matches!(results[0].detail, RemoveDetail::Success {}));

        // Sidecar gone.
        assert!(!sidecar.exists(), "sidecar should be deleted");
        // Local data file untouched.
        assert!(root.join("data.csv").is_file(), "local file must be kept");
        assert_eq!(fs::read(root.join("data.csv")).unwrap(), b"payload");
        // Storage blob untouched.
        assert!(
            blob_path(&root, &hash).is_file(),
            "storage blob must be kept"
        );
        // Gitignore entry gone (the root .gitignore held only /data.csv, so it
        // is removed entirely).
        assert!(
            !root.join(".gitignore").exists(),
            "empty gitignore should be removed"
        );
    }

    #[test]
    fn removed_file_can_be_readded() {
        let (_tmp, root) = create_temp_git_repo();
        let (config, _dvs_dir) = init_dvs_repo(&root);
        let backend = config.backend();
        let paths = make_paths(&root, &config);

        add_one(&paths, backend, "data.csv", b"payload");
        remove_files(vec!["data.csv".into()], &paths).unwrap();
        let sidecar = paths.metadata_path(Path::new("data.csv"));
        assert!(!sidecar.exists());

        // The local file is still there, so a re-add tracks it again.
        let results = add_files(
            vec!["data.csv".into()],
            &paths,
            backend,
            None,
            Compression::Zstd,
            false,
            None,
        )
        .unwrap();
        assert!(matches!(results[0].detail, AddDetail::Success { .. }));
        assert!(sidecar.is_file(), "sidecar should be recreated");
        let gitignore = fs::read_to_string(root.join(".gitignore")).unwrap();
        assert!(gitignore.contains("/data.csv"), "gitignore entry restored");
    }

    #[test]
    fn remove_refuses_batch_when_any_path_not_tracked() {
        let (_tmp, root) = create_temp_git_repo();
        let (config, _dvs_dir) = init_dvs_repo(&root);
        let backend = config.backend();
        let paths = make_paths(&root, &config);

        add_one(&paths, backend, "tracked.csv", b"a");
        let tracked_sidecar = paths.metadata_path(Path::new("tracked.csv"));

        // A file on disk that was never added.
        create_file(&root, "untracked.csv", b"b");

        let err = remove_files(vec!["tracked.csv".into(), "untracked.csv".into()], &paths)
            .unwrap_err()
            .to_string();
        assert!(err.contains("untracked.csv"), "unexpected error: {err}");
        assert!(err.contains("not tracked"), "unexpected error: {err}");

        // Nothing removed: the tracked sidecar must still be present.
        assert!(
            tracked_sidecar.is_file(),
            "no files should be removed when one is not tracked"
        );
    }

    #[test]
    fn remove_resolved_directory_paths() {
        use crate::globbing::resolve_paths_for_get;

        let (_tmp, root) = create_temp_git_repo();
        let (config, _dvs_dir) = init_dvs_repo(&root);
        let backend = config.backend();
        let paths = make_paths(&root, &config);

        add_one(&paths, backend, "dir/a.csv", b"a");
        add_one(&paths, backend, "dir/b.csv", b"b");

        // The CLI resolver expands a directory to its tracked files.
        let resolved: Vec<_> = resolve_paths_for_get(vec!["dir".into()], None, &paths, false)
            .unwrap()
            .into_iter()
            .collect();
        assert_eq!(resolved.len(), 2);

        let results = remove_files(resolved, &paths).unwrap();
        assert_eq!(results.len(), 2);
        assert!(
            results
                .iter()
                .all(|r| matches!(r.detail, RemoveDetail::Success {}))
        );
        assert!(!paths.metadata_path(Path::new("dir/a.csv")).exists());
        assert!(!paths.metadata_path(Path::new("dir/b.csv")).exists());
        // The nested .gitignore held only these two entries, so it is removed.
        assert!(!root.join("dir/.gitignore").exists());
    }

    #[test]
    fn remove_result_json_round_trips() {
        // The CLI serializes results for `--json`; lock the flat shape and the
        // untagged Success/Error discrimination.
        let ok = RemoveResult {
            path: PathBuf::from("data.csv"),
            detail: RemoveDetail::Success {},
        };
        let json = serde_json::to_string(&ok).unwrap();
        assert_eq!(json, r#"{"path":"data.csv"}"#);
        let back: RemoveResult = serde_json::from_str(&json).unwrap();
        assert!(matches!(back.detail, RemoveDetail::Success {}));

        let err = RemoveResult {
            path: PathBuf::from("bad.csv"),
            detail: RemoveDetail::Error {
                error: "boom".into(),
            },
        };
        let json = serde_json::to_string(&err).unwrap();
        assert_eq!(json, r#"{"path":"bad.csv","error":"boom"}"#);
        let back: RemoveResult = serde_json::from_str(&json).unwrap();
        assert!(matches!(back.detail, RemoveDetail::Error { .. }));
    }
}
