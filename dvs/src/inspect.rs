//! Local facts for machine-readable status, using cached hashes when available.
use crate::cache::{self, FileStat};
use crate::error::ErrorCode;
use crate::operation::{FileRow, LazyPhase, OperationContext, OperationPlan, Phase, PhaseGuard};
use crate::{DvsPaths, FileMetadata, Hashes, PathFilter, Status};
use anyhow::Result;
use fs_err as fs;
use rayon::prelude::*;
use serde::Serialize;
use std::path::PathBuf;
use std::sync::Mutex;

#[derive(Clone, Debug, Serialize)]
pub struct Baseline {
    pub blake3: String,
    pub size: u64,
}
#[derive(Clone, Debug, Serialize)]
pub struct WorkingCopy {
    pub state: &'static str,
    pub hashes: Option<Hashes>,
    pub size: Option<u64>,
    pub hash_source: Option<&'static str>,
}
#[derive(Clone, Debug, Serialize)]
pub struct StatusFacts {
    pub path: PathBuf,
    pub status: Status,
    pub metadata: Option<FileMetadata>,
    pub working: WorkingCopy,
    pub baseline: Option<Baseline>,
    pub code: Option<&'static str>,
    pub error: Option<String>,
}

/// Inspect tracked files in parallel. Each row goes to `ctx`'s observer as
/// soon as it completes, so callers can stream rows; the returned rows are
/// sorted by path. `hash` is reported only once a file is actually hashed.
/// Once `ctx` is cancelled, files not yet started get a `cancelled` row.
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
    // What hashing will read: the working files that are regular files
    let bytes_total = pool.install(|| {
        entries
            .par_iter()
            .filter_map(|path| fs::symlink_metadata(paths.file_path(path)).ok())
            .filter(|meta| meta.is_file())
            .map(|meta| meta.len())
            .sum()
    });
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
                let row = if ctx.is_cancelled() {
                    cancelled_row(path)
                } else {
                    inspect_path(path, paths, &cache, &hashing)
                };
                observer.file_finished(FileRow::Status(&row));
                row
            })
            .collect::<Vec<_>>()
    });
    hashing.finish();
    results.sort_by(|a, b| a.path.cmp(&b.path));
    Ok(results)
}

/// A file the cancelled operation never looked at.
fn cancelled_row(path: PathBuf) -> StatusFacts {
    StatusFacts {
        path,
        status: Status::Error,
        metadata: None,
        working: WorkingCopy {
            state: "unknown",
            hashes: None,
            size: None,
            hash_source: None,
        },
        baseline: None,
        code: Some(ErrorCode::Cancelled.as_str()),
        error: Some("cancelled before this file was checked".to_string()),
    }
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
        status: Status::Error,
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
        row.code = Some("bad_metadata");
        row.metadata = Some(serde_json::from_reader(fs::File::open(
            paths.metadata_path(&path),
        )?)?);
        row.code = Some("local_read_failed");
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
                row.status = Status::Absent;
                row.working.state = "absent";
                return Ok(());
            }
            Err(e) => return Err(e),
        };
        anyhow::ensure!(full.is_file(), "working path is not a regular file");
        row.working.size = Some(stat.size);
        // Hashing starts here, as in `hash`: a cache lookup counts, while
        // cancelled, absent and failed rows never open the phase.
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
        let target = row.metadata.as_ref().unwrap();
        let matches_metadata = hashes == target.hashes && size == target.size;
        let matches_baseline = row
            .baseline
            .as_ref()
            .is_some_and(|baseline| baseline.blake3 == hashes.blake3 && baseline.size == size);
        row.status = if matches_metadata {
            Status::Current
        } else if matches_baseline {
            Status::Stale
        } else if row.baseline.is_some() {
            Status::Modified
        } else {
            Status::Unsynced
        };
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
            row.status = Status::Error;
            row.error = Some(format!("{e:#}"));
        }
    }
    row
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::operation::OperationObserver;
    use crate::testutil::{create_file, create_temp_git_repo, init_dvs_repo};
    use std::sync::Mutex;
    use std::sync::mpsc;
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
        assert_eq!(rows[0].status, Status::Current);
        assert_eq!(rows[0].working.hash_source, Some("computed"));
        #[cfg(feature = "cache")]
        {
            let rows = inspect_status(&paths, None, None, &OperationContext::new()).unwrap();
            assert_eq!(rows[0].status, Status::Current);
            assert_eq!(rows[0].working.hash_source, Some("cache"));
            // A changed stat invalidates the cache without reading new file content.
            fs::write(paths.repo_root().join("data.txt"), b"changed content").unwrap();
            assert_eq!(
                inspect_status(&paths, None, None, &OperationContext::new()).unwrap()[0].status,
                Status::Unsynced
            );
        }
    }
    #[test]
    fn missing_and_bad_metadata_are_distinct() {
        let (_temp, paths) = fixture();
        fs::remove_file(paths.repo_root().join("data.txt")).unwrap();
        let rows = inspect_status(&paths, None, None, &OperationContext::new()).unwrap();
        assert_eq!(rows[0].status, Status::Absent);
        assert!(rows[0].error.is_none());
        fs::write(
            paths.metadata_path(std::path::Path::new("data.txt")),
            b"broken",
        )
        .unwrap();
        let rows = inspect_status(&paths, None, None, &OperationContext::new()).unwrap();
        assert_eq!(rows[0].status, Status::Error);
        assert_eq!(rows[0].code, Some("bad_metadata"));
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
            assert_eq!(row.status, Status::Current);
            assert!(!scan.is_finished());
            continue_sender.send(()).unwrap();
            scan.join().unwrap().unwrap();
        });
    }

    #[test]
    #[cfg(feature = "cache")]
    fn baseline_distinguishes_clean_old_version_from_edits() {
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
        assert_eq!(stale[0].status, Status::Stale);
        fs::write(paths.repo_root().join("data.txt"), b"user edits").unwrap();
        let modified = inspect_status(&paths, None, None, &OperationContext::new()).unwrap();
        assert_eq!(modified[0].status, Status::Modified);
    }

    #[test]
    fn cancelled_status_reports_unchecked_files_as_cancelled() {
        use crate::testutil::{Recorder, Seen};
        let (_temp, paths) = fixture();
        let token = crate::CancelToken::new();
        token.cancel();
        let recorder = std::sync::Arc::new(Recorder::default());
        let ctx = OperationContext::new()
            .with_cancel(token)
            .with_observer(recorder.clone());
        let rows = inspect_status(&paths, None, None, &ctx).unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].status, Status::Error);
        assert_eq!(rows[0].code, Some("cancelled"));
        assert!(rows[0].working.hashes.is_none());
        assert_eq!(recorder.finished().len(), 1);
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
                // Nothing was hashed, so no hash phase (K29)
            ]
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

    #[test]
    fn status_cancelled_mid_scan_checks_only_started_files() {
        use crate::testutil::Recorder;
        let (_temp, paths) = fixture();
        for i in 0..48 {
            let name = format!("f{i:02}.txt");
            let full = create_file(paths.repo_root(), &name, name.as_bytes());
            let metadata = FileMetadata::from_file(&full, crate::Compression::Zstd, None).unwrap();
            fs::write(
                paths.metadata_path(std::path::Path::new(&name)),
                serde_json::to_vec(&metadata).unwrap(),
            )
            .unwrap();
        }
        let token = crate::CancelToken::new();
        let recorder = std::sync::Arc::new(Recorder::cancelling_on_row(token.clone(), 1));
        let ctx = OperationContext::new()
            .with_cancel(token)
            .with_observer(recorder.clone());
        let rows = inspect_status(&paths, None, None, &ctx).unwrap();
        assert_eq!(rows.len(), 49);
        assert_eq!(recorder.finished().len(), 49);
        let cancelled = rows.iter().filter(|r| r.code == Some("cancelled")).count();
        let checked = rows.iter().filter(|r| r.status == Status::Current).count();
        assert_eq!(cancelled + checked, 49);
        assert!(checked >= 1 && cancelled >= 1, "{checked} checked");
    }
}
