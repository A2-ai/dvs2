//! `hash`: report the hashes of selected working files, tracked or not.
//!
//! Uses and fills the shared hash cache; never writes the baseline or any
//! `.dvs` record. Selection follows `status` (paths, `-r`, globs), but also
//! matches untracked files that a path or glob names.

use std::collections::BTreeMap;
use std::ffi::OsStr;
use std::path::{Path, PathBuf};
use std::time::Instant;

use anyhow::Result;
use rayon::prelude::*;
use serde::Serialize;
use walkdir::WalkDir;

use crate::cache::{self, HashSource};
use crate::globbing::build_glob_matcher;
use crate::hashes::Hashes;
use crate::operation::{
    CANCELLED_CODE, FileRow, LazyPhase, OperationContext, OperationPlan, OperationRow, Phase,
    PhaseGuard, RowState, RunSummary, duration_ms,
};
use crate::paths::{DvsPaths, PathFilter};

/// One selected file. Every field is always present (null when not
/// applicable). Field names match status's `working` object.
#[derive(Debug, Clone, Serialize)]
pub struct HashRow {
    /// Relative to the project root
    pub path: PathBuf,
    pub hashes: Option<Hashes>,
    pub size: Option<u64>,
    pub hash_source: Option<HashSource>,
    /// Time spent on this file; null when it did no work
    pub duration_ms: Option<u64>,
    pub code: Option<&'static str>,
    pub message: Option<String>,
}

impl OperationRow for HashRow {
    fn state(&self) -> RowState {
        match self.code {
            None => RowState::Succeeded,
            Some(CANCELLED_CODE) => RowState::Cancelled,
            Some(_) => RowState::Failed,
        }
    }
}

/// The rows of a hash run, sorted by path.
#[derive(Debug, Clone)]
pub struct HashReport {
    pub files: Vec<HashRow>,
    /// The cancel flag was set when the run finished
    pub cancelled: bool,
}

impl HashReport {
    pub fn summary(&self) -> RunSummary {
        RunSummary::of(&self.files, self.cancelled)
    }
}

/// Status's code and message for a working path that is or goes through a
/// symlink, which is never followed or hashed.
const SYMLINK_CODE: &str = "local_read_failed";
const SYMLINK_MESSAGE: &str = "working path contains a symlink";

/// Hashes the files selected by `filter`/`glob_pattern`, sorted by path.
/// Selection is `status`'s plus untracked files a path or glob names. Nothing
/// selected is silently dropped: missing paths, symlinks (never followed) and
/// unreadable directories each get an error row.
/// Reports each row to `ctx`'s observer as it finishes.
/// Once `ctx` is cancelled, files not yet started get a `cancelled` row.
pub fn hash_files(
    paths: &DvsPaths,
    filter: Option<&PathFilter>,
    glob_pattern: Option<&str>,
    ctx: &OperationContext,
) -> Result<HashReport> {
    let observer = ctx.observer();
    let scan = PhaseGuard::start(observer, Phase::Scan);
    let selected = select_working_files(paths, filter, glob_pattern)?;
    scan.finish();
    observer.planned(&OperationPlan {
        files_total: selected.len() as u64,
        bytes_total: selected
            .values()
            .map(|s| match s {
                Selected::File(size) => *size,
                Selected::Problem(..) => 0,
            })
            .sum(),
        files_to_transfer: 0,
        bytes_to_transfer: 0,
    });

    let mut to_hash = Vec::with_capacity(selected.len());
    let mut rows = Vec::new();
    for (path, selected) in selected {
        match selected {
            Selected::File(_) => to_hash.push(path),
            Selected::Problem(code, message) => {
                let row = error_row(path, code, message, None);
                observer.file_finished(FileRow::Hash(&row));
                rows.push(row);
            }
        }
    }

    let hashing = LazyPhase::new(observer, Phase::Hash);
    let cache = cache::try_open_cache(paths);
    let pool = crate::utils::get_threadpool(to_hash.len().max(1))?;
    rows.extend(pool.install(|| {
        to_hash
            .into_par_iter()
            .map(|path| {
                let row = if ctx.is_cancelled() {
                    let message = "cancelled before this file was hashed".to_string();
                    error_row(path, CANCELLED_CODE, message, None)
                } else {
                    hash_one(path, paths, cache.as_ref(), &hashing)
                };
                observer.file_finished(FileRow::Hash(&row));
                row
            })
            .collect::<Vec<_>>()
    }));
    hashing.finish();
    rows.sort_by(|a, b| a.path.cmp(&b.path));
    Ok(HashReport {
        files: rows,
        cancelled: ctx.is_cancelled(),
    })
}

fn error_row(
    path: PathBuf,
    code: &'static str,
    message: String,
    duration_ms: Option<u64>,
) -> HashRow {
    HashRow {
        path,
        hashes: None,
        size: None,
        hash_source: None,
        duration_ms,
        code: Some(code),
        message: Some(message),
    }
}

/// Hashes one selected file. The checks that need no reading come first and
/// fail with a `null` duration: such a row did no hashing work, and only a
/// file about to be read starts the `hash` phase.
fn hash_one(
    path: PathBuf,
    paths: &DvsPaths,
    cache: Option<&std::sync::Mutex<cache::HashCache>>,
    hashing: &LazyPhase<'_>,
) -> HashRow {
    let root = paths.repo_root();
    let full = paths.file_path(&path);
    // A tracked file is checked here, as status checks it: the walk only
    // vouches for the files it found
    if crosses_symlink(root, &full) {
        return error_row(path, SYMLINK_CODE, SYMLINK_MESSAGE.to_string(), None);
    }
    match std::fs::symlink_metadata(&full) {
        Ok(meta) if meta.is_file() => (),
        Ok(_) => {
            return error_row(
                path,
                "local_read_failed",
                "working path is not a regular file".to_string(),
                None,
            );
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            return error_row(
                path,
                "io_error",
                "tracked file is absent from the working tree".to_string(),
                None,
            );
        }
        Err(e) => {
            return error_row(path, "io_error", e.to_string(), None);
        }
    }
    hashing.begin();
    let start = Instant::now();
    let key = path.to_string_lossy().into_owned();
    match cache::hashes_for_file_with_source(&full, &key, cache) {
        Ok((hashes, size, source)) => HashRow {
            path,
            hashes: Some(hashes),
            size: Some(size),
            hash_source: Some(source),
            duration_ms: Some(duration_ms(start)),
            code: None,
            message: None,
        },
        Err(e) => {
            let code = if e.downcast_ref::<std::io::Error>().is_some() {
                "io_error"
            } else {
                "operation_failed"
            };
            error_row(path, code, format!("{e:#}"), Some(duration_ms(start)))
        }
    }
}

/// A selected path: a file to hash (with its size at scan time, 0 if unknown)
/// or the problem the scan found with it, which becomes its error row.
enum Selected {
    File(u64),
    Problem(&'static str, String),
}

/// Repo-root-relative paths matching the selection, sorted.
fn select_working_files(
    paths: &DvsPaths,
    filter: Option<&PathFilter>,
    glob_pattern: Option<&str>,
) -> Result<BTreeMap<PathBuf, Selected>> {
    let glob = build_glob_matcher(glob_pattern)?;
    // Same defaulting as status: a bare glob is matched from the cwd
    let cwd_filter = if filter.is_none() && glob_pattern.is_some() {
        Some(PathFilter::cwd_scoped(false, paths))
    } else {
        None
    };
    let filter = filter.or(cwd_filter.as_ref());
    let root = paths.repo_root();
    let metadata_root = paths.metadata_folder();
    let size_of = |relative: &Path| {
        std::fs::symlink_metadata(root.join(relative))
            .ok()
            .filter(|m| m.is_file())
            .map_or(0, |m| m.len())
    };

    // What status selects: the tracked files the filter matches, present or not
    let mut selected: BTreeMap<PathBuf, Selected> = paths
        .tracked_paths()
        .into_iter()
        .filter(|path| filter.is_none_or(|f| f.matches(path, glob.as_ref())))
        .map(|path| {
            let size = size_of(&path);
            (path, Selected::File(size))
        })
        .collect();
    // Untracked files only where a path or glob names them
    let Some(filter) = filter else {
        return Ok(selected);
    };
    let symlink = || Selected::Problem(SYMLINK_CODE, SYMLINK_MESSAGE.to_string());
    for named in filter.paths() {
        let walk_root = root.join(named);
        if crosses_symlink(root, &walk_root) {
            selected.entry(named.clone()).or_insert_with(symlink);
            continue;
        }
        match std::fs::symlink_metadata(&walk_root) {
            Ok(_) => (),
            // A missing path is only a problem if it names no tracked file
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                if !selected.keys().any(|path| path.starts_with(named)) {
                    selected.insert(
                        named.clone(),
                        Selected::Problem("io_error", format!("{}: {e}", named.display())),
                    );
                }
                continue;
            }
            Err(e) => {
                selected.insert(named.clone(), Selected::Problem("io_error", e.to_string()));
                continue;
            }
        }
        // Without a glob, a non-recursive selection only reaches direct children
        let max_depth = if glob.is_none() && !filter.recursive {
            1
        } else {
            usize::MAX
        };
        let walker = WalkDir::new(&walk_root)
            .max_depth(max_depth)
            .follow_links(false)
            .follow_root_links(false)
            .into_iter()
            .filter_entry(|e| e.file_name() != OsStr::new(".git") && e.path() != metadata_root);
        for entry in walker {
            let entry = match entry {
                Ok(entry) => entry,
                // A directory that cannot be read hides whatever is in it
                Err(e) => {
                    let path = e.path().unwrap_or(&walk_root);
                    if let Ok(relative) = path.strip_prefix(root) {
                        let message = match e.io_error() {
                            Some(io) => format!("{}: {io}", relative.display()),
                            None => e.to_string(),
                        };
                        selected.insert(
                            relative.to_path_buf(),
                            Selected::Problem("io_error", message),
                        );
                    }
                    continue;
                }
            };
            let Ok(relative) = entry.path().strip_prefix(root) else {
                continue;
            };
            let matched = filter.matches(relative, glob.as_ref());
            if entry.file_type().is_symlink() {
                // Never followed. Reported when selected, or when it stands
                // for a directory the walk would otherwise have entered
                let hides_a_directory = entry.depth() < max_depth
                    && std::fs::metadata(entry.path()).is_ok_and(|m| m.is_dir());
                if matched || hides_a_directory {
                    selected
                        .entry(relative.to_path_buf())
                        .or_insert_with(symlink);
                }
            } else if entry.file_type().is_file() && matched {
                let size = entry.metadata().map(|m| m.len()).unwrap_or(0);
                selected
                    .entry(relative.to_path_buf())
                    .or_insert(Selected::File(size));
            }
        }
    }
    Ok(selected)
}

/// Whether any existing component of `path` below `root` is a symlink. Like
/// status, hash never reads through a symlinked parent, which could lead
/// outside the project.
fn crosses_symlink(root: &Path, path: &Path) -> bool {
    let Ok(relative) = path.strip_prefix(root) else {
        return true;
    };
    let mut component = root.to_path_buf();
    for part in relative.components() {
        component.push(part.as_os_str());
        match std::fs::symlink_metadata(&component) {
            Ok(meta) if meta.file_type().is_symlink() => return true,
            Ok(_) => (),
            Err(_) => break,
        }
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The rows only, which is all most tests need.
    fn hash_files(
        paths: &DvsPaths,
        filter: Option<&PathFilter>,
        glob_pattern: Option<&str>,
        ctx: &OperationContext,
    ) -> Result<Vec<HashRow>> {
        super::hash_files(paths, filter, glob_pattern, ctx).map(|report| report.files)
    }
    use crate::testutil::{Recorder, Seen, create_file, create_temp_git_repo, init_dvs_repo};
    use crate::{Compression, FileMetadata};
    use fs_err as fs;
    use std::sync::Arc;

    /// A project with a tracked `data/tracked.csv` and untracked files beside
    /// and below it.
    fn fixture() -> (tempfile::TempDir, DvsPaths) {
        let (tmp, root) = create_temp_git_repo();
        let (config, _) = init_dvs_repo(&root);
        let paths =
            DvsPaths::new(root.clone(), root.clone(), config.metadata_folder_name()).unwrap();
        let tracked = create_file(&root, "data/tracked.csv", b"tracked");
        let metadata = FileMetadata::from_file(&tracked, Compression::Zstd, None).unwrap();
        metadata.write(&paths, "data/tracked.csv").unwrap();
        create_file(&root, "data/untracked.txt", b"untracked");
        create_file(&root, "data/sub/deep.csv", b"deep");
        create_file(&root, "top.csv", b"top");
        (tmp, paths)
    }

    fn selected(rows: &[HashRow]) -> Vec<&str> {
        rows.iter().map(|r| r.path.to_str().unwrap()).collect()
    }

    #[test]
    fn a_cancel_before_the_run_hashes_nothing() {
        let (_tmp, paths) = fixture();
        let token = crate::CancelToken::new();
        token.cancel();
        let ctx = OperationContext::new().with_cancel(token);
        let report = super::hash_files(&paths, None, None, &ctx).unwrap();
        assert!(!report.files.is_empty());
        assert!(
            report
                .files
                .iter()
                .all(|r| r.state() == RowState::Cancelled && r.hashes.is_none())
        );
        let summary = report.summary();
        assert_eq!(summary.outcome, crate::RunOutcome::Cancelled);
        assert_eq!(summary.counts.cancelled, report.files.len());
    }

    fn filter(paths: &DvsPaths, user: &[&str], recursive: bool) -> PathFilter {
        PathFilter::from_user_paths(user.iter().map(PathBuf::from).collect(), recursive, paths)
    }

    #[test]
    fn hashes_tracked_and_untracked_files_alike() {
        let (_tmp, paths) = fixture();
        let rows = hash_files(
            &paths,
            Some(&filter(&paths, &["data"], false)),
            None,
            &OperationContext::new(),
        )
        .unwrap();
        assert_eq!(selected(&rows), ["data/tracked.csv", "data/untracked.txt"]);
        let (expected, size) =
            Hashes::compute_from_path(&paths.file_path(Path::new("data/untracked.txt")), &[])
                .unwrap();
        assert_eq!(rows[1].hashes, Some(expected));
        assert_eq!(rows[1].size, Some(size));
        assert!(
            rows.iter()
                .all(|r| r.code.is_none() && r.duration_ms.is_some())
        );
    }

    #[test]
    fn selection_follows_status_rules() {
        let (_tmp, paths) = fixture();
        let ctx = OperationContext::new();
        // Recursive directory
        let rows = hash_files(&paths, Some(&filter(&paths, &["data"], true)), None, &ctx).unwrap();
        assert_eq!(
            selected(&rows),
            [
                "data/sub/deep.csv",
                "data/tracked.csv",
                "data/untracked.txt"
            ]
        );
        // A glob is matched relative to the directory
        let rows = hash_files(
            &paths,
            Some(&filter(&paths, &["data"], false)),
            Some("**/*.csv"),
            &ctx,
        )
        .unwrap();
        assert_eq!(selected(&rows), ["data/sub/deep.csv", "data/tracked.csv"]);
        // A bare glob is matched from the cwd (the root here), one level deep
        let rows = hash_files(&paths, None, Some("*.csv"), &ctx).unwrap();
        assert_eq!(selected(&rows), ["top.csv"]);
        // An explicit file
        let rows = hash_files(
            &paths,
            Some(&filter(&paths, &["data/sub/deep.csv"], false)),
            None,
            &ctx,
        )
        .unwrap();
        assert_eq!(selected(&rows), ["data/sub/deep.csv"]);
        // The whole project, named, without the metadata folder or .git
        create_file(paths.repo_root(), ".git/HEAD", b"ref: refs/heads/main");
        let rows = hash_files(&paths, Some(&filter(&paths, &["."], true)), None, &ctx).unwrap();
        let all = selected(&rows);
        for path in [
            "data/sub/deep.csv",
            "data/tracked.csv",
            "data/untracked.txt",
            "dvs.toml",
            "top.csv",
        ] {
            assert!(all.contains(&path), "{path} not in {all:?}");
        }
        assert!(
            all.iter()
                .all(|p| !p.starts_with(".dvs/") && !p.starts_with(".git/")),
            "{all:?}"
        );
        // Nothing named: exactly the tracked files, as status selects them
        let rows = hash_files(&paths, None, None, &ctx).unwrap();
        assert_eq!(selected(&rows), ["data/tracked.csv"]);
    }

    fn track(paths: &DvsPaths, relative: &str, content: &[u8]) {
        let full = create_file(paths.repo_root(), relative, content);
        let metadata = FileMetadata::from_file(&full, Compression::Zstd, None).unwrap();
        metadata.write(paths, relative).unwrap();
    }

    fn row<'a>(rows: &'a [HashRow], path: &str) -> &'a HashRow {
        rows.iter()
            .find(|r| r.path == Path::new(path))
            .unwrap_or_else(|| panic!("no row for {path} in {:?}", selected(rows)))
    }

    #[test]
    fn missing_files_get_error_rows() {
        let (_tmp, paths) = fixture();
        track(&paths, "data/gone.csv", b"gone");
        fs::remove_file(paths.file_path(Path::new("data/gone.csv"))).unwrap();
        let ctx = OperationContext::new();

        // A tracked file absent from disk is selected, as status selects it
        let rows = hash_files(&paths, None, None, &ctx).unwrap();
        assert_eq!(selected(&rows), ["data/gone.csv", "data/tracked.csv"]);
        assert_eq!(rows[0].code, Some("io_error"));
        assert!(rows[0].hashes.is_none() && rows[0].message.is_some());
        assert_eq!(rows[1].code, None);

        // A named path that does not exist, and one that is the absent tracked
        // file (one row, not two)
        let rows = hash_files(
            &paths,
            Some(&filter(&paths, &["nope.csv", "data/gone.csv"], false)),
            None,
            &ctx,
        )
        .unwrap();
        assert_eq!(selected(&rows), ["data/gone.csv", "nope.csv"]);
        assert!(rows.iter().all(|r| r.code == Some("io_error")));
    }

    #[test]
    #[cfg(unix)]
    fn symlinks_are_error_rows_never_followed() {
        use std::os::unix::fs::symlink;
        let (_tmp, paths) = fixture();
        let root = paths.repo_root().to_path_buf();
        symlink(root.join("data/tracked.csv"), root.join("data/link.csv")).unwrap();
        symlink(root.join("data/sub"), root.join("data/linkdir")).unwrap();
        let ctx = OperationContext::new();
        let is_symlink_row = |r: &HashRow| {
            r.code == Some("local_read_failed")
                && r.message.as_deref() == Some("working path contains a symlink")
                && r.hashes.is_none()
        };

        // Named
        let rows = hash_files(
            &paths,
            Some(&filter(
                &paths,
                &["data/link.csv", "data/linkdir/deep.csv"],
                false,
            )),
            None,
            &ctx,
        )
        .unwrap();
        assert_eq!(selected(&rows), ["data/link.csv", "data/linkdir/deep.csv"]);
        assert!(rows.iter().all(is_symlink_row), "{rows:?}");

        // Walked: one row per link, nothing read through the directory link
        let rows = hash_files(&paths, Some(&filter(&paths, &["data"], true)), None, &ctx).unwrap();
        assert_eq!(
            selected(&rows),
            [
                "data/link.csv",
                "data/linkdir",
                "data/sub/deep.csv",
                "data/tracked.csv",
                "data/untracked.txt"
            ]
        );
        assert!(is_symlink_row(row(&rows, "data/link.csv")));
        assert!(is_symlink_row(row(&rows, "data/linkdir")));
        // A glob still reports the directory link it cannot look into
        let rows = hash_files(
            &paths,
            Some(&filter(&paths, &["data"], false)),
            Some("**/*.csv"),
            &ctx,
        )
        .unwrap();
        assert!(is_symlink_row(row(&rows, "data/linkdir")));
        assert!(is_symlink_row(row(&rows, "data/link.csv")));

        // A tracked file that became a symlink
        let tracked = root.join("data/tracked.csv");
        fs::rename(&tracked, root.join("data/real.csv")).unwrap();
        symlink(root.join("data/real.csv"), &tracked).unwrap();
        let rows = hash_files(&paths, None, None, &ctx).unwrap();
        assert_eq!(selected(&rows), ["data/tracked.csv"]);
        assert!(is_symlink_row(&rows[0]));
    }

    #[test]
    #[cfg(unix)]
    fn an_unreadable_directory_is_an_error_row() {
        use std::os::unix::fs::PermissionsExt;
        let (_tmp, paths) = fixture();
        create_file(paths.repo_root(), "data/locked/hidden.csv", b"hidden");
        let locked = paths.file_path(Path::new("data/locked"));
        fs::set_permissions(&locked, std::fs::Permissions::from_mode(0o000)).unwrap();
        if fs::read_dir(&locked).is_ok() {
            // Running as root: permissions do not stop reads
            fs::set_permissions(&locked, std::fs::Permissions::from_mode(0o755)).unwrap();
            return;
        }
        let recorder = Arc::new(Recorder::default());
        let ctx = OperationContext::new().with_observer(recorder.clone());
        let rows = hash_files(&paths, Some(&filter(&paths, &["data"], true)), None, &ctx);
        fs::set_permissions(&locked, std::fs::Permissions::from_mode(0o755)).unwrap();
        let rows = rows.unwrap();
        let locked_row = row(&rows, "data/locked");
        assert_eq!(locked_row.code, Some("io_error"));
        assert!(locked_row.message.is_some() && locked_row.duration_ms.is_none());
        assert_eq!(rows.iter().filter(|r| r.code.is_none()).count(), 3);
        // The scan's error row is reported after the plan, like every row
        let seen = recorder.seen();
        let planned = seen
            .iter()
            .position(|s| matches!(s, Seen::Planned(_)))
            .unwrap();
        let first_row = seen
            .iter()
            .position(|s| matches!(s, Seen::Finished { .. }))
            .unwrap();
        assert!(planned < first_row, "{seen:?}");
        assert_eq!(recorder.finished().len(), rows.len());
    }

    #[test]
    #[cfg(feature = "cache")]
    fn second_hash_comes_from_the_cache_and_baseline_is_untouched() {
        let (_tmp, paths) = fixture();
        let only = filter(&paths, &["data/tracked.csv"], false);
        let ctx = OperationContext::new();
        let first = hash_files(&paths, Some(&only), None, &ctx).unwrap();
        assert_eq!(first[0].hash_source, Some(HashSource::Computed));
        let second = hash_files(&paths, Some(&only), None, &ctx).unwrap();
        assert_eq!(second[0].hash_source, Some(HashSource::Cache));
        assert_eq!(first[0].hashes, second[0].hashes);
        let cache = cache::open_cache(&paths).unwrap();
        assert_eq!(cache.baseline("data/tracked.csv").unwrap(), None);
        // No record appears for an untracked file
        hash_files(
            &paths,
            Some(&filter(&paths, &["top.csv"], false)),
            None,
            &ctx,
        )
        .unwrap();
        assert!(!paths.metadata_path(Path::new("top.csv")).exists());
    }

    #[test]
    #[cfg(unix)]
    fn unreadable_file_is_a_row_error() {
        use std::os::unix::fs::PermissionsExt;
        let (_tmp, paths) = fixture();
        let locked = create_file(paths.repo_root(), "data/locked.csv", b"secret");
        fs::set_permissions(&locked, std::fs::Permissions::from_mode(0o000)).unwrap();
        if fs::read(&locked).is_ok() {
            // Running as root: permissions do not stop reads
            return;
        }
        let rows = hash_files(
            &paths,
            Some(&filter(&paths, &["data"], false)),
            None,
            &OperationContext::new(),
        )
        .unwrap();
        fs::set_permissions(&locked, std::fs::Permissions::from_mode(0o644)).unwrap();
        let row = rows
            .iter()
            .find(|r| r.path == Path::new("data/locked.csv"))
            .unwrap();
        assert_eq!(row.code, Some("io_error"));
        assert!(row.message.is_some() && row.hashes.is_none());
        assert_eq!(rows.iter().filter(|r| r.code.is_none()).count(), 2);
    }
}
