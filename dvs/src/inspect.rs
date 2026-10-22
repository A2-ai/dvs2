//! Local facts for machine-readable status, using cached hashes when available.
use crate::cache::{self, FileStat};
use crate::operation::{FileRow, LazyPhase, OperationContext, OperationPlan, Phase, PhaseGuard};
use crate::{DvsPaths, ErrorCode, FileMetadata, Hashes, PathFilter, Status};
use anyhow::Result;
use fs_err as fs;
use rayon::prelude::*;
use serde::{Serialize, Serializer};
use std::path::PathBuf;
use std::sync::Mutex;

#[derive(Clone, Debug, Serialize)]
pub struct Baseline {
    pub blake3: String,
    pub size: u64,
}
#[derive(Clone, Debug, Serialize)]
pub struct WorkingCopy {
    /// Whether the working file is present and available for inspection.
    pub state: &'static str,
    pub hashes: Option<Hashes>,
    pub size: Option<u64>,
    pub hash_source: Option<&'static str>,
}
#[derive(Clone, Debug, Serialize)]
pub struct StatusFacts {
    pub path: PathBuf,
    /// `None` on an error row, which `code` and `error` describe.
    #[serde(serialize_with = "serialize_status")]
    pub status: Option<Status>,
    pub metadata: Option<FileMetadata>,
    pub working: WorkingCopy,
    pub baseline: Option<Baseline>,
    pub code: Option<ErrorCode>,
    pub error: Option<String>,
}

impl StatusFacts {
    /// The row's status as shown to people and in results: `error` on an error row.
    pub fn status_name(&self) -> String {
        status_name(self.status)
    }
}

fn status_name(status: Option<Status>) -> String {
    status.map_or_else(|| "error".to_string(), |s| s.to_string())
}

fn serialize_status<S: Serializer>(status: &Option<Status>, s: S) -> Result<S::Ok, S::Error> {
    s.serialize_str(&status_name(*status))
}

/// How a present working file relates to its record and to the local baseline.
fn classify(
    hashes: &Hashes,
    size: u64,
    metadata: &FileMetadata,
    baseline: Option<&Baseline>,
) -> Status {
    if *hashes == metadata.hashes && size == metadata.size {
        Status::Current
    } else {
        match baseline {
            Some(b) if b.blake3 == hashes.blake3 && b.size == size => Status::Stale,
            Some(_) => Status::Modified,
            None => Status::Unsynced,
        }
    }
}

/// Inspect tracked files in parallel. Each row goes to `ctx`'s observer as
/// soon as it completes, so callers can stream rows; the returned rows are
/// sorted by path. `hash` is reported only once a file is actually hashed.
///
/// Paths in the result are relative to `paths.repo_root()`, independent of cwd.
pub fn inspect_status(
    paths: &DvsPaths,
    filter: Option<&PathFilter>,
    glob_pattern: Option<&str>,
    ctx: &OperationContext,
) -> Result<Vec<StatusFacts>> {
    let observer = ctx.observer();
    let scan = PhaseGuard::start(observer, Phase::Scan);
    let (entries, cache, pool) = status_scan_inputs(paths, filter, glob_pattern)?;
    // What hashing will read: the working files that are regular files. An
    // extra stat per file, so only when someone hears the plan.
    let bytes_total = if ctx.is_observed() {
        pool.install(|| {
            entries
                .par_iter()
                .filter_map(|path| fs::symlink_metadata(paths.file_path(path)).ok())
                .filter(|meta| meta.is_file())
                .map(|meta| meta.len())
                .sum()
        })
    } else {
        0
    };
    scan.finish();
    observer.planned(&OperationPlan {
        files_total: entries.len() as u64,
        bytes_total,
        files_to_transfer: 0,
        bytes_to_transfer: 0,
    });
    let hashing = LazyPhase::new(observer, Phase::Hash);
    let mut results = pool.install(|| {
        entries
            .into_par_iter()
            .map(|path| {
                let row = inspect_path(path, paths, &cache, &hashing);
                observer.file_finished(FileRow::Status(&row));
                row
            })
            .collect::<Vec<_>>()
    });
    hashing.finish();
    results.sort_by(|a, b| a.path.cmp(&b.path));
    Ok(results)
}

fn status_scan_inputs(
    paths: &DvsPaths,
    filter: Option<&PathFilter>,
    glob_pattern: Option<&str>,
) -> Result<(
    Vec<PathBuf>,
    Option<Mutex<cache::HashCache>>,
    rayon::ThreadPool,
)> {
    let glob = crate::globbing::build_glob_matcher(glob_pattern)?;
    let cwd_filter = if filter.is_none() && glob_pattern.is_some() {
        Some(PathFilter::cwd_scoped(false, paths))
    } else {
        None
    };
    let filter = filter.or(cwd_filter.as_ref());
    let cache = cache::try_open_cache(paths);
    let entries: Vec<_> = paths
        .tracked_paths()
        .into_iter()
        .filter(|path| !filter.is_some_and(|f| !f.matches(path, glob.as_ref())))
        .collect();
    let pool = crate::utils::get_threadpool(entries.len().max(1))?;
    Ok((entries, cache, pool))
}

fn inspect_path(
    path: PathBuf,
    paths: &DvsPaths,
    cache: &Option<Mutex<cache::HashCache>>,
    hashing: &LazyPhase<'_>,
) -> StatusFacts {
    let mut row = StatusFacts {
        path: path.clone(),
        status: None,
        metadata: None,
        working: WorkingCopy {
            state: "unknown",
            hashes: None,
            size: None,
            hash_source: None,
        },
        baseline: None,
        code: None,
        error: None,
    };
    let facts = (|| -> Result<()> {
        row.code = Some(ErrorCode::BadMetadata);
        row.metadata = Some(serde_json::from_reader(fs::File::open(
            paths.metadata_path(&path),
        )?)?);
        row.code = Some(ErrorCode::IoError);
        let key = path.to_string_lossy();
        row.baseline = cache
            .as_ref()
            .and_then(|c| c.lock().ok()?.baseline(&key).ok().flatten())
            .map(|(blake3, size)| Baseline { blake3, size });
        let full = paths.file_path(&path);
        // Never hash a symlink or traverse a symlinked parent outside the project.
        let mut component = paths.repo_root().to_path_buf();
        for part in path.components() {
            component.push(part.as_os_str());
            match fs::symlink_metadata(&component) {
                Ok(meta) if meta.file_type().is_symlink() => {
                    anyhow::bail!("working path contains a symlink")
                }
                Ok(_) => (),
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => break,
                Err(e) => return Err(e.into()),
            }
        }
        let stat = match FileStat::from_path(&full) {
            Ok(stat) => stat,
            Err(e)
                if e.downcast_ref::<std::io::Error>()
                    .is_some_and(|e| e.kind() == std::io::ErrorKind::NotFound) =>
            {
                row.status = Some(Status::Absent);
                row.working.state = "absent";
                return Ok(());
            }
            Err(e) => return Err(e),
        };
        anyhow::ensure!(full.is_file(), "working path is not a regular file");
        row.working.size = Some(stat.size);
        // Hashing starts here: a cache lookup counts, while
        // absent and failed rows never open the phase.
        hashing.begin();
        let hit = cache
            .as_ref()
            .and_then(|c| c.lock().ok()?.lookup(&key, &stat).ok().flatten());
        let (hashes, size, source) = match hit {
            Some(hashes) => (hashes, stat.size, "cache"),
            None => {
                let (hashes, size) = cache::hashes_for_file(&full, &key, cache.as_ref())?;
                (hashes, size, "computed")
            }
        };
        row.status = Some(classify(
            &hashes,
            size,
            row.metadata.as_ref().unwrap(),
            row.baseline.as_ref(),
        ));
        row.working = WorkingCopy {
            state: "present",
            hashes: Some(hashes),
            size: Some(size),
            hash_source: Some(source),
        };
        Ok(())
    })();
    match facts {
        Ok(()) => row.code = None,
        Err(e) => {
            row.status = None;
            row.error = Some(format!("{e:#}"));
        }
    }
    row
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Compression;
    use crate::operation::OperationObserver;
    use crate::testutil::{create_file, create_temp_git_repo, init_dvs_repo};
    use std::sync::mpsc;
    use uuid::Uuid;
    fn fixture() -> (tempfile::TempDir, DvsPaths) {
        let (temp, root) = create_temp_git_repo();
        let (config, _) = init_dvs_repo(&root);
        let paths =
            DvsPaths::new(root.clone(), root.clone(), config.metadata_folder_name()).unwrap();
        let full = create_file(&root, "data.txt", b"original");
        let (hashes, size) = Hashes::compute_from_path(&full, &[]).unwrap();
        // Serialize via the actual metadata type so the fixture tracks the file format.
        let metadata = FileMetadata::from_file(&full, crate::Compression::Zstd, None).unwrap();
        fs::write(
            paths.metadata_path(std::path::Path::new("data.txt")),
            serde_json::to_vec(&metadata).unwrap(),
        )
        .unwrap();
        assert_eq!(hashes.blake3, metadata.hashes.blake3);
        assert_eq!(size, metadata.size);
        (temp, paths)
    }
    #[test]
    fn cache_miss_computes_status_facts() {
        let (_temp, paths) = fixture();
        let rows = inspect_status(&paths, None, None, &OperationContext::new()).unwrap();
        assert_eq!(rows[0].status, Some(Status::Current));
        assert_eq!(rows[0].working.hash_source, Some("computed"));
        #[cfg(feature = "cache")]
        {
            let rows = inspect_status(&paths, None, None, &OperationContext::new()).unwrap();
            assert_eq!(rows[0].status, Some(Status::Current));
            assert_eq!(rows[0].working.hash_source, Some("cache"));
            // A changed stat invalidates the cache without reading new file content.
            fs::write(paths.repo_root().join("data.txt"), b"changed content").unwrap();
            assert_eq!(
                inspect_status(&paths, None, None, &OperationContext::new()).unwrap()[0].status,
                Some(Status::Unsynced)
            );
        }
    }
    #[test]
    fn missing_and_bad_metadata_are_distinct() {
        let (_temp, paths) = fixture();
        fs::remove_file(paths.repo_root().join("data.txt")).unwrap();
        let rows = inspect_status(&paths, None, None, &OperationContext::new()).unwrap();
        assert_eq!(rows[0].status, Some(Status::Absent));
        assert!(rows[0].error.is_none());
        fs::write(
            paths.metadata_path(std::path::Path::new("data.txt")),
            b"broken",
        )
        .unwrap();
        let rows = inspect_status(&paths, None, None, &OperationContext::new()).unwrap();
        assert_eq!(rows[0].status, None);
        assert_eq!(serde_json::to_value(&rows[0]).unwrap()["status"], "error");
        assert_eq!(rows[0].code, Some(ErrorCode::BadMetadata));
    }

    #[test]
    fn callback_receives_a_completed_row_before_the_scan_returns() {
        let (_temp, paths) = fixture();
        struct Pausing {
            rows: Mutex<mpsc::Sender<StatusFacts>>,
            resume: Mutex<mpsc::Receiver<()>>,
        }
        impl OperationObserver for Pausing {
            fn file_finished(&self, row: FileRow<'_>) {
                let FileRow::Status(row) = row else {
                    panic!("status only reports status rows")
                };
                self.rows.lock().unwrap().send(row.clone()).unwrap();
                self.resume.lock().unwrap().recv().unwrap();
            }
        }
        let (row_sender, row_receiver) = mpsc::channel();
        let (continue_sender, continue_receiver) = mpsc::channel();
        let ctx = OperationContext::new().with_observer(std::sync::Arc::new(Pausing {
            rows: Mutex::new(row_sender),
            resume: Mutex::new(continue_receiver),
        }));

        std::thread::scope(|scope| {
            let scan = scope.spawn(|| inspect_status(&paths, None, None, &ctx));
            let row = row_receiver
                .recv_timeout(std::time::Duration::from_secs(5))
                .unwrap();
            assert_eq!(row.status, Some(Status::Current));
            assert!(!scan.is_finished());
            continue_sender.send(()).unwrap();
            scan.join().unwrap().unwrap();
        });
    }

    #[test]
    #[cfg(feature = "cache")]
    fn baseline_distinguishes_stale_from_modified() {
        let (_temp, paths) = fixture();
        let cache = cache::open_cache(&paths).unwrap();
        let rows = inspect_status(&paths, None, None, &OperationContext::new()).unwrap();
        let original = rows[0].working.hashes.as_ref().unwrap();
        cache
            .set_baseline(
                "data.txt",
                &original.blake3,
                rows[0].metadata.as_ref().unwrap().size,
            )
            .unwrap();
        let mut target = rows[0].metadata.clone().unwrap();
        target.hashes.blake3 = "b".repeat(64);
        fs::write(
            paths.metadata_path(std::path::Path::new("data.txt")),
            serde_json::to_vec(&target).unwrap(),
        )
        .unwrap();
        let stale = inspect_status(&paths, None, None, &OperationContext::new()).unwrap();
        assert_eq!(stale[0].status, Some(Status::Stale));
        fs::write(paths.repo_root().join("data.txt"), b"user edits").unwrap();
        let modified = inspect_status(&paths, None, None, &OperationContext::new()).unwrap();
        assert_eq!(modified[0].status, Some(Status::Modified));
    }

    #[test]
    fn status_returns_empty_vec_for_repo_with_no_tracked_files() {
        let (_tmp, root) = create_temp_git_repo();
        let (config, _dvs_dir) = init_dvs_repo(&root);
        let paths =
            DvsPaths::new(root.clone(), root.clone(), config.metadata_folder_name()).unwrap();

        let statuses = inspect_status(&paths, None, None, &OperationContext::new()).unwrap();
        assert!(statuses.is_empty());
    }

    /// Helper to set up a repo with files at various directory depths.
    /// Returns (TempDir, DvsPaths) with tracked files:
    ///   "a.txt", "dir1/b.txt", "dir1/sub/c.txt", "dir2/d.txt"
    fn setup_filtered_repo() -> (tempfile::TempDir, DvsPaths) {
        let (tmp, root) = create_temp_git_repo();
        let (config, _dvs_dir) = init_dvs_repo(&root);
        let backend = config.backend();
        let paths =
            DvsPaths::new(root.clone(), root.clone(), config.metadata_folder_name()).unwrap();

        for name in ["a.txt", "dir1/b.txt", "dir1/sub/c.txt", "dir2/d.txt"] {
            let file_path = create_file(&root, name, name.as_bytes());
            let mut metadata =
                FileMetadata::from_file(&file_path, Compression::Zstd, None).unwrap();
            metadata
                .save(Uuid::new_v4(), &file_path, backend, &paths, name, None)
                .unwrap();
        }
        (tmp, paths)
    }

    #[test]
    fn status_with_filter() {
        let (_tmp, paths) = setup_filtered_repo();

        // Relative path filter
        let filter = PathFilter {
            paths: vec![PathBuf::from("dir1")],
            recursive: false,
        };
        let statuses =
            inspect_status(&paths, Some(&filter), None, &OperationContext::new()).unwrap();
        assert_eq!(statuses.len(), 1);
        assert_eq!(statuses[0].path, PathBuf::from("dir1/b.txt"));

        // Absolute path filter via from_user_paths
        let abs_path = paths.repo_root().join("dir1/b.txt");
        let filter = PathFilter::from_user_paths(vec![abs_path], false, &paths);
        let statuses =
            inspect_status(&paths, Some(&filter), None, &OperationContext::new()).unwrap();
        assert_eq!(statuses.len(), 1);
        assert_eq!(statuses[0].path, PathBuf::from("dir1/b.txt"));
    }

    #[test]
    fn status_whole_repo_applies_glob() {
        let (_tmp, paths) = setup_filtered_repo();

        // No filter (whole repo) + glob: the glob still applies, matched against
        // the full repo-relative path. `*.txt` (literal separator) hits only
        // root-level files.
        let statuses =
            inspect_status(&paths, None, Some("*.txt"), &OperationContext::new()).unwrap();
        assert_eq!(statuses.len(), 1);
        assert_eq!(statuses[0].path, PathBuf::from("a.txt"));

        // `**/*.txt` reaches every tracked file.
        let statuses =
            inspect_status(&paths, None, Some("**/*.txt"), &OperationContext::new()).unwrap();
        assert_eq!(statuses.len(), 4);
    }

    #[test]
    fn status_bare_glob_anchors_to_cwd_subdir() {
        let (_tmp, paths) = setup_filtered_repo();
        let root = paths.repo_root().to_path_buf();
        // cwd = dir1: a bare glob must scope to the cwd (like add/get), not the
        // repo root. Pre-fix it globbed the whole repo from any directory.
        let sub =
            DvsPaths::new(fs::canonicalize(root.join("dir1")).unwrap(), root, ".dvs").unwrap();

        // Literal separator: only the direct child dir1/b.txt, not a.txt/dir2.
        let got: Vec<_> = inspect_status(&sub, None, Some("*.txt"), &OperationContext::new())
            .unwrap()
            .into_iter()
            .map(|f| f.path)
            .collect();
        assert_eq!(got, vec![PathBuf::from("dir1/b.txt")]);

        // Recursive glob reaches dir1/sub/c.txt, still scoped under dir1.
        let got: Vec<_> = inspect_status(&sub, None, Some("**/*.txt"), &OperationContext::new())
            .unwrap()
            .into_iter()
            .map(|f| f.path)
            .collect();
        assert_eq!(
            got,
            vec![PathBuf::from("dir1/b.txt"), PathBuf::from("dir1/sub/c.txt")]
        );
    }

    #[test]
    fn status_reports_a_hash_phase_when_a_file_is_hashed() {
        use crate::testutil::{Recorder, Seen};
        let (_temp, paths) = fixture();
        let recorder = std::sync::Arc::new(Recorder::default());
        let ctx = OperationContext::new().with_observer(recorder.clone());
        inspect_status(&paths, None, None, &ctx).unwrap();
        assert_eq!(
            recorder.outline(),
            vec![
                Seen::PhaseStarted(Phase::Scan),
                Seen::PhaseFinished(Phase::Scan),
                Seen::Planned(OperationPlan {
                    files_total: 1,
                    bytes_total: 8,
                    files_to_transfer: 0,
                    bytes_to_transfer: 0,
                }),
                Seen::PhaseStarted(Phase::Hash),
                Seen::PhaseFinished(Phase::Hash),
            ]
        );
    }
}
