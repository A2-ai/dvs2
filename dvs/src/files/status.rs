use std::path::{Component, Path, PathBuf};
use std::sync::Mutex;

use anyhow::Result;
use fs_err as fs;
use rayon::prelude::*;
use serde::{Deserialize, Serialize};
use walkdir::WalkDir;

use crate::cache::{HashCache, try_open_cache};
use crate::files::metadata::FileMetadata;
use crate::paths::DvsPaths;
use crate::utils::get_threadpool;
use crate::{Status, cache};

/// Which paths to get status for
/// eg you can pass dir1/ dir2/ and it will expand to dir1/* dir2/*
/// If `recursive` is `true`, then it will expand to dir1/**/* dir2/**/*
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct StatusFilter {
    paths: Vec<PathBuf>,
    recursive: bool,
}

/// We need to handle `.`, `./` etc but we can't canonicalize because
/// the path might not exist and we want the path relative to the directory so no symlink resolution
fn normalize_path(p: PathBuf) -> Option<PathBuf> {
    let mut out = PathBuf::new();
    for c in p.components() {
        match c {
            Component::CurDir => {}
            Component::ParentDir => {
                if !out.pop() {
                    return None;
                }
            }
            _ => out.push(c),
        }
    }
    Some(out)
}

impl StatusFilter {
    /// Create a filter from user-provided paths (relative to cwd) and a recursive flag.
    /// Translates cwd-relative paths to repo-root-relative using `dvs_paths.cwd_relative_to_root()`.
    pub fn from_user_paths(
        user_paths: Vec<PathBuf>,
        recursive: bool,
        dvs_paths: &DvsPaths,
    ) -> Self {
        let cwd_prefix = dvs_paths.cwd_relative_to_root();
        let repo_root = dvs_paths.repo_root();
        let paths = user_paths
            .into_iter()
            .filter_map(|p| {
                if p.is_absolute() {
                    p.strip_prefix(repo_root)
                        .ok()
                        .map(|r| r.to_path_buf())
                        .and_then(normalize_path)
                } else {
                    let joined = if let Some(prefix) = cwd_prefix {
                        prefix.join(&p)
                    } else {
                        p
                    };
                    normalize_path(joined)
                }
            })
            .collect();
        StatusFilter { paths, recursive }
    }

    fn matches(&self, tracked_path: &Path) -> bool {
        self.paths.iter().any(|filter_path| {
            // Exact match (user passed a file path)
            tracked_path == filter_path
                // Recursive: any descendant
                || (self.recursive && tracked_path.starts_with(filter_path))
                // Non-recursive: direct child
                || (!self.recursive && tracked_path.parent() == Some(filter_path.as_path()))
        })
    }
}

fn get_file_status(
    paths: &DvsPaths,
    relative_path: impl AsRef<Path>,
    cache: Option<&Mutex<HashCache>>,
) -> Result<(Status, Option<FileMetadata>)> {
    let dvs_file_path = paths.metadata_path(relative_path.as_ref());
    if !dvs_file_path.is_file() {
        return Ok((Status::Untracked, None));
    }
    let existing_metadata: FileMetadata = serde_json::from_reader(fs::File::open(dvs_file_path)?)?;
    // If we have read the metadata, but we can't find the original file
    let file_path = paths.file_path(relative_path.as_ref());
    if !file_path.is_file() {
        return Ok((Status::Absent, Some(existing_metadata)));
    }
    let rel_str = relative_path.as_ref().to_string_lossy();
    let (hashes, size) = cache::hashes_for_file(&file_path, &rel_str, cache)?;

    if existing_metadata.hashes == hashes && existing_metadata.size == size {
        Ok((Status::Current, Some(existing_metadata)))
    } else {
        Ok((Status::Unsynced, Some(existing_metadata)))
    }
}

pub fn get_status(
    paths: &DvsPaths,
    filter: Option<&StatusFilter>,
) -> Result<crate::BatchOutcome<StatusSuccess, StatusError>> {
    let dvs_directory = paths.metadata_folder();
    log::debug!("Scanning metadata folder: {}", dvs_directory.display());
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

    let pool = get_threadpool(entries.len())?;

    let mut rows: Vec<Row> = pool.install(|| {
        entries
            .into_par_iter()
            .map(|dvs_path| {
                let relative = match dvs_path.strip_prefix(&dvs_directory) {
                    Ok(r) => r.with_extension(""),
                    Err(e) => {
                        return Row::Err(StatusError::RelativePath {
                            metadata_path: dvs_path.clone(),
                            reason: e.to_string(),
                        });
                    }
                };
                if let Some(f) = filter {
                    if !f.matches(&relative) {
                        return Row::Skip;
                    }
                }
                match get_file_status(paths, &relative, cache.as_ref()) {
                    Ok((status, metadata)) => Row::Ok(StatusSuccess {
                        path: relative,
                        status,
                        metadata,
                    }),
                    Err(e) => {
                        let reason = e.to_string();
                        if reason.contains("blake3") || reason.contains("hash") {
                            Row::Err(StatusError::HashFailure {
                                path: relative,
                                reason,
                            })
                        } else {
                            Row::Err(StatusError::MetadataRead {
                                path: relative,
                                reason,
                            })
                        }
                    }
                }
            })
            .collect()
    });
    rows.sort_by(|a, b| status_row_key(a).cmp(status_row_key(b)));

    let mut outcome = crate::BatchOutcome::<StatusSuccess, StatusError>::new();
    for row in rows {
        match row {
            Row::Ok(success) => outcome.ok.push(success),
            Row::Err(err) => outcome.err.push(err),
            Row::Skip => {}
        }
    }

    log::debug!(
        "Found {} tracked files ({} errors)",
        outcome.ok.len(),
        outcome.err.len()
    );
    Ok(outcome)
}

/// Successful status row for a single tracked file.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct StatusSuccess {
    pub path: PathBuf,
    pub status: Status,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub metadata: Option<FileMetadata>,
}

/// Structured failure for a single file in `get_status`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum StatusError {
    /// Internal: could not determine the path relative to the `.dvs` folder.
    RelativePath { metadata_path: PathBuf, reason: String },
    /// Reading the `.dvs` metadata file failed.
    MetadataRead { path: PathBuf, reason: String },
    /// Hashing the on-disk file to compare against metadata failed.
    HashFailure { path: PathBuf, reason: String },
}

impl StatusError {
    pub fn kind(&self) -> &'static str {
        match self {
            StatusError::RelativePath { .. } => "relative_path",
            StatusError::MetadataRead { .. } => "metadata_read",
            StatusError::HashFailure { .. } => "hash_failure",
        }
    }
}

#[derive(Debug)]
enum Row {
    Ok(StatusSuccess),
    Err(StatusError),
    Skip,
}

fn status_row_key(row: &Row) -> &Path {
    match row {
        Row::Ok(success) => success.path.as_path(),
        Row::Err(StatusError::RelativePath { metadata_path, .. }) => metadata_path.as_path(),
        Row::Err(StatusError::MetadataRead { path, .. }) => path.as_path(),
        Row::Err(StatusError::HashFailure { path, .. }) => path.as_path(),
        Row::Skip => Path::new(""),
    }
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
        .unwrap()
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
        let (status, metadata) = get_file_status(&paths, "new.txt", Some(&cache)).unwrap();
        assert_eq!(status, Status::Untracked);
        assert!(metadata.is_none());
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
            .save(
                Uuid::new_v4(),
                &file_path,
                backend,
                &paths,
                "synced.txt",
                None,
            )
            .unwrap();

        let cache = make_cache(&paths);
        let (status, metadata) = get_file_status(&paths, "synced.txt", Some(&cache)).unwrap();
        assert_eq!(status, Status::Current);
        assert!(metadata.is_some());
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
            .save(
                Uuid::new_v4(),
                &file_path,
                backend,
                &paths,
                "deleted.txt",
                None,
            )
            .unwrap();

        // Delete the original file
        fs::remove_file(&file_path).unwrap();

        let cache = make_cache(&paths);
        let (status, metadata) = get_file_status(&paths, "deleted.txt", Some(&cache)).unwrap();
        assert_eq!(status, Status::Absent);
        assert!(metadata.is_some());
    }

    #[test]
    fn status_error_serializes_with_snake_case_kind() {
        let cases: Vec<(StatusError, &str)> = vec![
            (
                StatusError::RelativePath {
                    metadata_path: "a".into(),
                    reason: "x".into(),
                },
                "relative_path",
            ),
            (
                StatusError::MetadataRead {
                    path: "a".into(),
                    reason: "x".into(),
                },
                "metadata_read",
            ),
            (
                StatusError::HashFailure {
                    path: "a".into(),
                    reason: "x".into(),
                },
                "hash_failure",
            ),
        ];
        for (err, expected) in cases {
            assert_eq!(err.kind(), expected);
            let json = serde_json::to_value(&err).unwrap();
            assert_eq!(json["kind"], expected);
        }
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
            .save(
                Uuid::new_v4(),
                &file_path,
                backend,
                &paths,
                "modified.txt",
                None,
            )
            .unwrap();

        // Modify the file
        fs::write(&file_path, b"changed content").unwrap();

        let cache = make_cache(&paths);
        let (status, metadata) = get_file_status(&paths, "modified.txt", Some(&cache)).unwrap();
        assert_eq!(status, Status::Unsynced);
        assert!(metadata.is_some());
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
                .save(Uuid::new_v4(), &file_path, backend, &paths, name, None)
                .unwrap();
        }

        let outcome = get_status(&paths, None).unwrap();
        assert!(outcome.err.is_empty());
        assert_eq!(outcome.ok.len(), 3);

        // All should be Current
        for status in &outcome.ok {
            assert_eq!(status.status, Status::Current);
            assert!(status.metadata.is_some());
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
            .save(Uuid::new_v4(), &file_a, backend, &paths, "a.txt", None)
            .unwrap();
        let hash_h1 = metadata_a.hashes.blake3.clone();

        // Add file B with content "bar" (hash H2)
        let file_b = create_file(&root, "b.txt", b"bar");
        let metadata_b = FileMetadata::from_file(&file_b, Compression::Zstd, None).unwrap();
        metadata_b
            .save(Uuid::new_v4(), &file_b, backend, &paths, "b.txt", None)
            .unwrap();
        let hash_h2 = metadata_b.hashes.blake3.clone();
        assert_ne!(hash_h1, hash_h2);

        // Change file B's content to "foo" (now hash H1)
        fs::write(&file_b, b"foo").unwrap();

        // Run add on B with new content
        let metadata_b_new = FileMetadata::from_file(&file_b, Compression::Zstd, None).unwrap();
        assert_eq!(metadata_b_new.hashes.blake3, hash_h1);

        metadata_b_new
            .save(Uuid::new_v4(), &file_b, backend, &paths, "b.txt", None)
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
        let (status, _metadata) = get_file_status(&paths, "b.txt", Some(&cache)).unwrap();
        assert_eq!(status, Status::Current);
    }

    /// Helper to set up a repo with files at various directory depths.
    /// Returns (TempDir, DvsPaths) with tracked files:
    ///   "a.txt", "dir1/b.txt", "dir1/sub/c.txt", "dir2/d.txt"
    fn setup_filtered_repo() -> (tempfile::TempDir, DvsPaths) {
        let (tmp, root) = create_temp_git_repo();
        let (config, _dvs_dir) = init_dvs_repo(&root);
        let backend = config.backend();
        let paths = make_paths(&root, &config);

        for name in ["a.txt", "dir1/b.txt", "dir1/sub/c.txt", "dir2/d.txt"] {
            let file_path = create_file(&root, name, name.as_bytes());
            let metadata = FileMetadata::from_file(&file_path, Compression::Zstd, None).unwrap();
            metadata
                .save(Uuid::new_v4(), &file_path, backend, &paths, name, None)
                .unwrap();
        }
        (tmp, paths)
    }

    fn run_filter_cases(cases: Vec<(&[&str], &str, bool)>, recursive: bool) {
        for (filter_paths, test_path, expected) in cases {
            let filter = StatusFilter {
                paths: filter_paths
                    .iter()
                    .filter_map(|p| normalize_path(PathBuf::from(p)))
                    .collect(),
                recursive,
            };
            assert_eq!(
                filter.matches(Path::new(test_path)),
                expected,
                "filter={filter_paths:?} recursive={recursive} path={test_path:?}"
            );
        }
    }

    #[test]
    fn status_filter_matches_non_recursive() {
        // (filter_paths, test_path, expected)
        let cases: Vec<(&[&str], &str, bool)> = vec![
            // direct child matches
            (&["dir1"], "dir1/b.txt", true),
            // nested child does NOT match
            (&["dir1"], "dir1/sub/c.txt", false),
            // exact file match
            (&["dir2/d.txt"], "dir2/d.txt", true),
            // exact file: different file does NOT match
            (&["dir2/d.txt"], "dir2/e.txt", false),
            // "." matches root-level files
            (&["."], "a.txt", true),
            // "." does NOT match nested files
            (&["."], "dir1/b.txt", false),
            // "../foo" escapes root → dropped, matches nothing
            (&["../foo"], "foo", false),
            // "dir1/../dir2" normalizes to "dir2", matches direct child
            (&["dir1/../dir2"], "dir2/d.txt", true),
            // "dir1/.." normalizes to root, matches root-level files
            (&["dir1/.."], "a.txt", true),
        ];
        run_filter_cases(cases, false);
    }

    #[test]
    fn status_filter_matches_recursive() {
        // (filter_paths, test_path, expected)
        let cases: Vec<(&[&str], &str, bool)> = vec![
            // direct child matches
            (&["dir1"], "dir1/b.txt", true),
            // nested child matches
            (&["dir1"], "dir1/sub/c.txt", true),
            // unrelated dir does NOT match
            (&["dir1"], "dir2/d.txt", false),
            // "." matches everything recursively
            (&["."], "a.txt", true),
            (&["."], "dir1/b.txt", true),
            (&["."], "dir1/sub/c.txt", true),
            // "../foo" escapes root → dropped
            (&["../foo"], "foo", false),
            // "a/../../x" escapes root → dropped
            (&["a/../../x"], "x", false),
            // "dir1/../dir2" normalizes to "dir2", matches descendants
            (&["dir1/../dir2"], "dir2/d.txt", true),
        ];
        run_filter_cases(cases, true);
    }

    #[test]
    fn get_status_with_filter() {
        let (_tmp, paths) = setup_filtered_repo();

        // Relative path filter
        let filter = StatusFilter {
            paths: vec![PathBuf::from("dir1")],
            recursive: false,
        };
        let outcome = get_status(&paths, Some(&filter)).unwrap();
        assert!(outcome.err.is_empty());
        assert_eq!(outcome.ok.len(), 1);
        assert_eq!(outcome.ok[0].path, PathBuf::from("dir1/b.txt"));

        // Absolute path filter via from_user_paths
        let abs_path = paths.repo_root().join("dir1/b.txt");
        let filter = StatusFilter::from_user_paths(vec![abs_path], false, &paths);
        let outcome = get_status(&paths, Some(&filter)).unwrap();
        assert!(outcome.err.is_empty());
        assert_eq!(outcome.ok.len(), 1);
        assert_eq!(outcome.ok[0].path, PathBuf::from("dir1/b.txt"));
    }

    #[test]
    fn from_user_paths_with_subdirectory_cwd() {
        let (_tmp, root) = create_temp_git_repo();
        let (_config, _dvs_dir) = init_dvs_repo(&root);

        // Create subdirectories so canonicalize works in DvsPaths::new
        fs::create_dir_all(root.join("subdir/deep")).unwrap();
        fs::create_dir_all(root.join("dir2")).unwrap();

        // From subdir/: ../foo → resolves to "foo" (valid)
        let paths = DvsPaths::new(
            fs::canonicalize(root.join("subdir")).unwrap(),
            root.to_path_buf(),
            ".dvs",
        )
        .unwrap();
        let filter = StatusFilter::from_user_paths(vec![PathBuf::from("../foo")], false, &paths);
        assert_eq!(filter.paths, vec![PathBuf::from("foo")]);

        // From subdir/: ../../foo → escapes root → dropped (empty)
        let filter = StatusFilter::from_user_paths(vec![PathBuf::from("../../foo")], false, &paths);
        assert!(
            filter.paths.is_empty(),
            "../../foo should escape root and be dropped"
        );

        // From subdir/deep/: ../../foo → resolves to "foo" (valid, 2 levels up = root)
        let paths_deep = DvsPaths::new(
            fs::canonicalize(root.join("subdir/deep")).unwrap(),
            root.to_path_buf(),
            ".dvs",
        )
        .unwrap();
        let filter =
            StatusFilter::from_user_paths(vec![PathBuf::from("../../foo")], false, &paths_deep);
        assert_eq!(filter.paths, vec![PathBuf::from("foo")]);

        // From subdir/: ../dir2/file.txt → resolves to "dir2/file.txt"
        let filter =
            StatusFilter::from_user_paths(vec![PathBuf::from("../dir2/file.txt")], false, &paths);
        assert_eq!(filter.paths, vec![PathBuf::from("dir2/file.txt")]);

        // From subdir/: absolute path with .. like <root>/subdir/../a.txt → normalizes to "a.txt"
        let abs_with_dotdot = root.join("subdir/../a.txt");
        let filter = StatusFilter::from_user_paths(vec![abs_with_dotdot], false, &paths);
        assert_eq!(filter.paths, vec![PathBuf::from("a.txt")]);
    }
}
