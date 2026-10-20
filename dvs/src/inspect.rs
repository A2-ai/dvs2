//! Local facts for machine-readable status, using cached hashes when available.
use crate::cache::{self, FileStat};
use crate::{DvsPaths, FileMetadata, Hashes, PathFilter};
use anyhow::Result;
use fs_err as fs;
use rayon::prelude::*;
use serde::Serialize;
use std::path::PathBuf;

#[derive(Debug, Serialize)]
pub struct Baseline {
    pub blake3: String,
    pub size: u64,
}
#[derive(Debug, Serialize)]
pub struct WorkingCopy {
    /// Whether the working file is present and available for inspection.
    pub state: &'static str,
    pub hashes: Option<Hashes>,
    pub size: Option<u64>,
    pub hash_source: Option<&'static str>,
}
#[derive(Debug, Serialize)]
pub struct StatusFacts {
    pub path: PathBuf,
    pub status: &'static str,
    pub metadata: Option<FileMetadata>,
    pub working: WorkingCopy,
    pub baseline: Option<Baseline>,
    /// Local checkout state: `current` when matching metadata; otherwise
    /// `clean`/`modified` relative to the checkout baseline, or `absent`/
    /// `unknown` when the working file or baseline is unavailable.
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
    let mut results = pool.install(|| {
        entries
            .into_par_iter()
            .map(|path| {
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
                            let (hashes, size) =
                                cache::hashes_for_file(&full, &key, cache.as_ref())?;
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
            })
            .collect::<Vec<_>>()
    });
    results.sort_by(|a, b| a.path.cmp(&b.path));
    Ok(results)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testutil::{create_file, create_temp_git_repo, init_dvs_repo};
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
