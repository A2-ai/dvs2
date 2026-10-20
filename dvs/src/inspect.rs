//! Local facts for machine-readable status, using cached hashes when available.
use crate::cache::{self, FileStat};
use crate::{DvsPaths, FileMetadata, Hashes, PathFilter};
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
    pub status: &'static str,
    pub metadata: Option<FileMetadata>,
    pub working: WorkingCopy,
    pub baseline: Option<Baseline>,
    /// Local checkout state relative to the baseline and current metadata.
    pub state: &'static str,
    pub code: Option<&'static str>,
    pub error: Option<String>,
}

/// Paths in the result are relative to `paths.repo_root()`, independent of cwd.
pub fn inspect_status(
    paths: &DvsPaths,
    filter: Option<&PathFilter>,
    glob_pattern: Option<&str>,
) -> Result<Vec<StatusFacts>> {
    let (entries, cache, pool) = status_scan_inputs(paths, filter, glob_pattern)?;
    let mut results = pool.install(|| {
        entries
            .into_par_iter()
            .map(|path| inspect_path(path, paths, &cache))
            .collect::<Vec<_>>()
    });
    results.sort_by(|a, b| a.path.cmp(&b.path));
    Ok(results)
}

/// Inspect tracked files in parallel and call `on_row` as each local status
/// check completes. Callers that need additional backend facts can resolve
/// those before publishing the row.
pub fn inspect_status_with_callback(
    paths: &DvsPaths,
    filter: Option<&PathFilter>,
    glob_pattern: Option<&str>,
    on_row: impl Fn(&StatusFacts) + Sync + Send,
) -> Result<()> {
    let (entries, cache, pool) = status_scan_inputs(paths, filter, glob_pattern)?;
    pool.install(|| {
        entries.into_par_iter().for_each(|path| {
            let row = inspect_path(path, paths, &cache);
            on_row(&row);
        });
    });
    Ok(())
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
) -> StatusFacts {
    let mut row = StatusFacts {
        path: path.clone(),
        status: "error",
        metadata: None,
        working: WorkingCopy {
            state: "unknown",
            hashes: None,
            size: None,
            hash_source: None,
        },
        baseline: None,
        state: "unknown",
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
                row.status = "absent";
                row.state = "absent";
                row.working.state = "absent";
                return Ok(());
            }
            Err(e) => return Err(e),
        };
        anyhow::ensure!(full.is_file(), "working path is not a regular file");
        row.working.size = Some(stat.size);
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
        row.status = if hashes == target.hashes && size == target.size {
            "current"
        } else {
            "unsynced"
        };
        row.state = if row.status == "current" {
            "current"
        } else {
            match &row.baseline {
                Some(b) if b.blake3 == hashes.blake3 && b.size == size => "clean",
                Some(_) => "modified",
                None => "unknown",
            }
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
            row.status = "error";
            row.error = Some(format!("{e:#}"));
        }
    }
    row
}

#[cfg(test)]
mod tests {
    use super::*;
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
        let rows = inspect_status(&paths, None, None).unwrap();
        assert_eq!(rows[0].status, "current");
        assert_eq!(rows[0].working.hash_source, Some("computed"));
        #[cfg(feature = "cache")]
        {
            let rows = inspect_status(&paths, None, None).unwrap();
            assert_eq!(rows[0].status, "current");
            assert_eq!(rows[0].working.hash_source, Some("cache"));
            // A changed stat invalidates the cache without reading new file content.
            fs::write(paths.repo_root().join("data.txt"), b"changed content").unwrap();
            assert_eq!(
                inspect_status(&paths, None, None).unwrap()[0].status,
                "unsynced"
            );
        }
    }
    #[test]
    fn missing_and_bad_metadata_are_distinct() {
        let (_temp, paths) = fixture();
        fs::remove_file(paths.repo_root().join("data.txt")).unwrap();
        let rows = inspect_status(&paths, None, None).unwrap();
        assert_eq!(rows[0].status, "absent");
        assert!(rows[0].error.is_none());
        fs::write(
            paths.metadata_path(std::path::Path::new("data.txt")),
            b"broken",
        )
        .unwrap();
        let rows = inspect_status(&paths, None, None).unwrap();
        assert_eq!(rows[0].status, "error");
        assert_eq!(rows[0].code, Some("bad_metadata"));
    }

    #[test]
    fn callback_receives_a_completed_row_before_the_scan_returns() {
        let (_temp, paths) = fixture();
        let (row_sender, row_receiver) = mpsc::channel();
        let (continue_sender, continue_receiver) = mpsc::channel();
        let continue_receiver = Mutex::new(continue_receiver);

        std::thread::scope(|scope| {
            let scan = scope.spawn(|| {
                inspect_status_with_callback(&paths, None, None, |row| {
                    row_sender.send(row.clone()).unwrap();
                    continue_receiver.lock().unwrap().recv().unwrap();
                })
            });
            let row = row_receiver
                .recv_timeout(std::time::Duration::from_secs(5))
                .unwrap();
            assert_eq!(row.status, "current");
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
        let rows = inspect_status(&paths, None, None).unwrap();
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
        assert_eq!(
            inspect_status(&paths, None, None).unwrap()[0].state,
            "clean"
        );
        fs::write(paths.repo_root().join("data.txt"), b"user edits").unwrap();
        assert_eq!(
            inspect_status(&paths, None, None).unwrap()[0].state,
            "modified"
        );
    }
}
