use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::ops::Range;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use anyhow::{Result, anyhow, bail};
use rayon::prelude::*;
use serde::{Deserialize, Serialize};

use crate::api::v1::{BatchKind, CompleteBatch, ManifestEntry};
use crate::backends::{Backend, ManifestItem, ManifestLimits, PresentSize, Stored};
use crate::cache;
use crate::cache::{HashCache, try_open_cache};
use crate::config::Compression;
use crate::files::metadata::FileMetadata;
use crate::git::current_git_context;
use crate::gitignore::add_to_gitignore;
use crate::paths::{DvsPaths, ProjectPath};
use crate::progress::OnFileStart;
use crate::remote::api_error;
use crate::utils::get_threadpool;

/// The largest manifest body sent, whatever the server allows.
const MAX_MANIFEST_BYTES: usize = 10 * 1024 * 1024;
/// Uploads to a server at once.
const MAX_PARALLEL_UPLOADS: usize = 4;

/// What happened to one file.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AddOutcome {
    /// Its bytes were sent (in a dry run: would be)
    Added,
    /// The project already had the content, so only the path was recorded
    Present,
    /// Its `.dvs` file already matched; nothing was sent
    Skipped,
    Failed,
    /// Not tried, because an earlier step of the upload failed
    NotAttempted,
}

/// Result of adding a single file.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AddResult {
    pub path: PathBuf,
    pub outcome: AddOutcome,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hash: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub size: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stored_size: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    /// The server's error code, when it gave one
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub code: Option<String>,
}

impl AddResult {
    /// Failed, or never attempted.
    pub fn is_failure(&self) -> bool {
        matches!(self.outcome, AddOutcome::Failed | AddOutcome::NotAttempted)
    }
}

/// An error kept for every file it applies to.
#[derive(Debug, Clone)]
struct Failure {
    error: String,
    code: Option<String>,
}

impl From<&anyhow::Error> for Failure {
    fn from(e: &anyhow::Error) -> Self {
        Self {
            error: format!("{e:#}"),
            code: api_error(e).map(|a| a.code.clone()),
        }
    }
}

/// A file to add, hashed.
struct Hashed {
    path: PathBuf,
    project_path: ProjectPath,
    hashes: crate::Hashes,
    size: u64,
    /// Its `.dvs` file already records this content
    unchanged: bool,
}

impl Hashed {
    fn result(&self, outcome: AddOutcome, stored_size: Option<u64>) -> AddResult {
        AddResult {
            path: self.path.clone(),
            outcome,
            hash: Some(self.hashes.blake3.clone()),
            size: Some(self.size),
            stored_size,
            error: None,
            code: None,
        }
    }

    fn failed(&self, outcome: AddOutcome, failure: &Failure) -> AddResult {
        AddResult {
            error: Some(failure.error.clone()),
            code: failure.code.clone(),
            ..self.result(outcome, None)
        }
    }

    fn item(&self) -> ManifestItem {
        ManifestItem {
            path: self.project_path.clone(),
            hashes: self.hashes.clone(),
            size: self.size,
        }
    }
}

fn hash_file(path: &Path, paths: &DvsPaths, cache: Option<&Mutex<HashCache>>) -> Result<Hashed> {
    let project_path = ProjectPath::from_path(path).map_err(|e| anyhow!("{e}"))?;
    let (hashes, size) =
        cache::hashes_for_file(&paths.file_path(path), &path.to_string_lossy(), cache)?;
    let dvs_file = paths.metadata_path(path);
    let unchanged = dvs_file.is_file()
        && FileMetadata::read(&dvs_file).is_ok_and(|m| m.hashes == hashes && m.size == size);
    Ok(Hashed {
        path: path.to_path_buf(),
        project_path,
        hashes,
        size,
        unchanged,
    })
}

/// Split the manifest into calls within the server's limits and 10 MB.
fn manifest_chunks(items: &[ManifestItem], limits: ManifestLimits) -> Vec<Range<usize>> {
    // `{"entries":[` and `]}`
    const ENVELOPE: usize = 16;
    let max_bytes = limits.max_body_bytes.min(MAX_MANIFEST_BYTES);
    let max_entries = limits.max_entries.max(1);
    let mut chunks = Vec::new();
    let (mut start, mut bytes) = (0, ENVELOPE);
    for (i, item) in items.iter().enumerate() {
        let entry = ManifestEntry {
            path: item.path.as_str().to_string(),
            hash: item.hashes.get_blake3().to_string(),
            size: item.size,
        };
        // The entry and its comma
        let size = serde_json::to_vec(&entry).map_or(max_bytes, |v| v.len()) + 1;
        if i > start && (i - start >= max_entries || bytes + size > max_bytes) {
            chunks.push(start..i);
            (start, bytes) = (i, ENVELOPE);
        }
        bytes += size;
    }
    if start < items.len() {
        chunks.push(start..items.len());
    }
    chunks
}

/// Adds files matching a glob pattern to DVS.
///
/// The pattern is matched against files relative to cwd.
/// Files are stored with paths relative to repo_root.
///
/// One batch: every file is hashed, files whose `.dvs` already matches are skipped,
/// the rest go in the manifest, each missing content is sent once, and the `.dvs`
/// files are written for the contents the project now has. Content is never removed
/// to undo a failed metadata write, because other paths may share it.
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

    let hashed: Vec<_> = pool.install(|| {
        valid_paths
            .par_iter()
            .map(|path| (path, hash_file(path, paths, cache.as_ref())))
            .collect()
    });
    let mut results = BTreeMap::new();
    let mut to_send = Vec::new();
    for (path, hashed) in hashed {
        match hashed {
            Ok(h) if h.unchanged => {
                results.insert(path.clone(), h.result(AddOutcome::Skipped, None));
            }
            Ok(h) => to_send.push(h),
            Err(e) => {
                log::warn!("Failed to add {}: {e:#}", path.display());
                let failure = Failure::from(&e);
                results.insert(
                    path.clone(),
                    AddResult {
                        path: path.clone(),
                        outcome: AddOutcome::Failed,
                        hash: None,
                        size: None,
                        stored_size: None,
                        error: Some(failure.error),
                        code: failure.code,
                    },
                );
            }
        }
    }

    // The manifest goes in path order
    to_send.sort_by(|a, b| a.path.cmp(&b.path));

    let mut progressed = HashSet::new();
    if dry_run {
        let checked: Vec<_> = pool.install(|| {
            to_send
                .par_iter()
                .map(|h| (h, backend.exists(&h.hashes)))
                .collect()
        });
        for (h, exists) in checked {
            let result = match exists {
                Ok(true) => h.result(AddOutcome::Present, None),
                Ok(false) => h.result(AddOutcome::Added, None),
                Err(e) => h.failed(AddOutcome::Failed, &Failure::from(&e)),
            };
            results.insert(h.path.clone(), result);
        }
    } else if !to_send.is_empty() {
        upload(
            &to_send,
            paths,
            backend,
            message,
            compression,
            on_file_start,
            &mut results,
            &mut progressed,
        )?;
    }

    // Every file reports its end, uploaded or not
    if let Some(on_file_start) = on_file_start {
        for result in results.values() {
            if !progressed.contains(&result.path) {
                let progress = on_file_start(&result.path, result.size.unwrap_or(0));
                (progress.on_done)(!result.is_failure());
            }
        }
    }

    if !dry_run {
        let recorded: Vec<_> = results
            .values()
            .filter(|r| !r.is_failure())
            .map(|r| r.path.clone())
            .collect();
        if !recorded.is_empty()
            && let Err(e) = add_to_gitignore(paths.repo_root(), &recorded)
        {
            log::warn!("Failed to update .gitignore: {e}");
        }
    }

    Ok(results.into_values().collect())
}

/// One upload batch for the files in `to_send`, writing each file's result.
#[allow(clippy::too_many_arguments)]
fn upload(
    to_send: &[Hashed],
    paths: &DvsPaths,
    backend: &Backend,
    message: Option<String>,
    compression: Compression,
    on_file_start: Option<&OnFileStart>,
    results: &mut BTreeMap<PathBuf, AddResult>,
    progressed: &mut HashSet<PathBuf>,
) -> Result<()> {
    let limits = backend.manifest_limits()?;
    let git = current_git_context(paths.repo_root());
    let batch = backend.open_batch(BatchKind::Upload, message.as_deref(), &git, compression)?;

    // The manifest, in chunks. After a chunk fails, later chunks are not sent; the
    // contents of earlier chunks are still uploaded.
    let items: Vec<_> = to_send.iter().map(Hashed::item).collect();
    let mut missing = BTreeSet::new();
    let mut present: HashMap<String, PresentSize> = HashMap::new();
    let mut project_compression = compression;
    let mut recorded = vec![false; to_send.len()];
    let mut failed_chunk = false;
    for range in manifest_chunks(&items, limits) {
        if failed_chunk {
            let failure = Failure {
                error: "not sent, because an earlier part of the manifest failed".into(),
                code: None,
            };
            for h in &to_send[range] {
                results.insert(h.path.clone(), h.failed(AddOutcome::NotAttempted, &failure));
            }
            continue;
        }
        match backend.manifest(&batch, &items[range.clone()]) {
            Ok(reply) => {
                missing.extend(reply.missing);
                present.extend(reply.present);
                project_compression = reply.compression;
                recorded[range].fill(true);
            }
            Err(e) => {
                log::warn!("The manifest failed: {e:#}");
                let failure = Failure::from(&e);
                for h in &to_send[range] {
                    results.insert(h.path.clone(), h.failed(AddOutcome::Failed, &failure));
                }
                failed_chunk = true;
            }
        }
    }

    // Each missing content once, from the first file holding it
    let mut sources: BTreeMap<&str, &Hashed> = BTreeMap::new();
    for (h, _) in to_send.iter().zip(&recorded).filter(|(_, r)| **r) {
        let hash = h.hashes.get_blake3();
        if missing.contains(hash) {
            sources.entry(hash).or_insert(h);
        }
    }
    let stored: HashMap<String, Result<Stored, Failure>> = if sources.is_empty() {
        HashMap::new()
    } else {
        let threads = match backend {
            Backend::Server(_) => MAX_PARALLEL_UPLOADS.min(sources.len()),
            Backend::Local(_) => get_threadpool(sources.len())?.current_num_threads(),
        };
        let pool = rayon::ThreadPoolBuilder::new()
            .num_threads(threads)
            .build()?;
        pool.install(|| {
            sources
                .par_iter()
                .map(|(hash, h)| {
                    let progress = on_file_start.map(|f| f(&h.path, h.size));
                    let on_bytes = progress.as_ref().map(|p| &*p.on_bytes);
                    let result = backend
                        .put(&batch, &h.hashes, &paths.file_path(&h.path), on_bytes)
                        .map_err(|e| {
                            log::warn!("Failed to upload {}: {e:#}", h.path.display());
                            Failure::from(&e)
                        });
                    if let Some(p) = &progress {
                        (p.on_done)(result.is_ok());
                    }
                    (hash.to_string(), result)
                })
                .collect()
        })
    };
    progressed.extend(sources.values().map(|h| h.path.clone()));

    // The `.dvs` file of every file whose content the project now has
    for (h, _) in to_send.iter().zip(&recorded).filter(|(_, r)| **r) {
        let hash = h.hashes.get_blake3();
        let (outcome, stored_size) = match (stored.get(hash), present.get(hash)) {
            (Some(Ok(s)), _) => (AddOutcome::Added, s.stored_size),
            (Some(Err(failure)), _) => {
                results.insert(h.path.clone(), h.failed(AddOutcome::Failed, failure));
                continue;
            }
            (None, Some(p)) => (AddOutcome::Present, p.stored_size),
            (None, None) => {
                let failure = Failure {
                    error: "the server did not say whether it has this content".into(),
                    code: None,
                };
                results.insert(h.path.clone(), h.failed(AddOutcome::Failed, &failure));
                continue;
            }
        };
        let metadata = FileMetadata::from_hashes(
            h.hashes.clone(),
            h.size,
            project_compression,
            message.clone(),
        );
        let result = match metadata.write(&paths.metadata_path(&h.path)) {
            Ok(()) => {
                log::info!("Added {} ({outcome:?})", h.path.display());
                h.result(outcome, Some(stored_size))
            }
            Err(e) => {
                log::warn!("{e:#}");
                h.failed(AddOutcome::Failed, &Failure::from(&e))
            }
        };
        results.insert(h.path.clone(), result);
    }

    let mut counts = CompleteBatch::default();
    for r in results.values() {
        match r.outcome {
            AddOutcome::Added | AddOutcome::Present => counts.ok += 1,
            AddOutcome::Skipped => counts.skipped += 1,
            AddOutcome::Failed | AddOutcome::NotAttempted => counts.failed += 1,
        }
    }
    if let Err(e) = backend.complete(&batch, &counts) {
        log::warn!("The files are recorded, but closing the upload failed: {e:#}");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Hashes;
    use crate::audit::Action;
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

    fn add(paths: &DvsPaths, backend: &Backend, files: &[&str], dry_run: bool) -> Vec<AddResult> {
        add_files(
            files.iter().map(PathBuf::from).collect(),
            paths,
            backend,
            None,
            Compression::Zstd,
            dry_run,
            None,
        )
        .unwrap()
    }

    fn outcomes(results: &[AddResult]) -> Vec<(&str, AddOutcome)> {
        results
            .iter()
            .map(|r| (r.path.to_str().unwrap(), r.outcome))
            .collect()
    }

    fn added_paths(backend: &Backend) -> Vec<PathBuf> {
        backend
            .get_audit_entries(&[])
            .unwrap()
            .into_iter()
            .filter_map(|e| match e.action {
                Action::Add { file, .. } => Some(file.path),
                _ => None,
            })
            .collect()
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
        // Shares its content with a.txt, so it is not uploaded
        create_file(&root, "d.txt", b"a");

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
            vec![
                "a.txt".into(),
                "b.txt".into(),
                "c.txt".into(),
                "d.txt".into(),
            ],
            &paths,
            backend,
            None,
            Compression::Zstd,
            false,
            Some(&on_file_start),
        )
        .unwrap();

        assert_eq!(results.len(), 4);
        assert!(results.iter().all(|r| r.outcome == AddOutcome::Added));
        assert_eq!(done_calls.load(Ordering::SeqCst), 4);
        assert_eq!(success_calls.load(Ordering::SeqCst), 4);
    }

    #[test]
    fn unchanged_files_are_skipped_without_a_batch() {
        let (_tmp, root) = create_temp_git_repo();
        let (config, _) = init_dvs_repo(&root);
        let backend = config.backend();
        let paths = make_paths(&root, &config);
        create_file(&root, "a.txt", b"a");

        assert_eq!(
            outcomes(&add(&paths, backend, &["a.txt"], false)),
            [("a.txt", AddOutcome::Added)]
        );
        let again = add(&paths, backend, &["a.txt"], false);
        assert_eq!(outcomes(&again), [("a.txt", AddOutcome::Skipped)]);
        assert!(again[0].hash.is_some());
        assert_eq!(
            added_paths(backend).len(),
            1,
            "a skipped file is not logged"
        );

        // A changed file is sent again
        create_file(&root, "a.txt", b"changed");
        assert_eq!(
            outcomes(&add(&paths, backend, &["a.txt"], false)),
            [("a.txt", AddOutcome::Added)]
        );
    }

    #[test]
    fn copy_then_add_records_the_path_without_sending_bytes() {
        let (_tmp, root) = create_temp_git_repo();
        let (config, _) = init_dvs_repo(&root);
        let backend = config.backend();
        let paths = make_paths(&root, &config);
        create_file(&root, "a.txt", b"shared content");
        add(&paths, backend, &["a.txt"], false);

        fs::copy(root.join("a.txt"), root.join("b.txt")).unwrap();
        let results = add(&paths, backend, &["a.txt", "b.txt"], false);
        assert_eq!(
            outcomes(&results),
            [
                ("a.txt", AddOutcome::Skipped),
                ("b.txt", AddOutcome::Present)
            ]
        );
        assert!(results[1].stored_size.is_some());
        assert!(paths.metadata_path(Path::new("b.txt")).is_file());
        // One log entry per path
        assert_eq!(
            added_paths(backend),
            [PathBuf::from("a.txt"), PathBuf::from("b.txt")]
        );
    }

    #[test]
    fn a_dry_run_changes_nothing() {
        let (_tmp, root) = create_temp_git_repo();
        let (config, _) = init_dvs_repo(&root);
        let backend = config.backend();
        let paths = make_paths(&root, &config);
        create_file(&root, "a.txt", b"a");
        add(&paths, backend, &["a.txt"], false);
        create_file(&root, "b.txt", b"a");
        create_file(&root, "c.txt", b"c");

        let results = add(&paths, backend, &["a.txt", "b.txt", "c.txt"], true);
        assert_eq!(
            outcomes(&results),
            [
                ("a.txt", AddOutcome::Skipped),
                ("b.txt", AddOutcome::Present),
                ("c.txt", AddOutcome::Added)
            ]
        );
        assert!(!paths.metadata_path(Path::new("b.txt")).exists());
        assert!(!paths.metadata_path(Path::new("c.txt")).exists());
        let c = Hashes::compute_from_path(&root.join("c.txt"), &[])
            .unwrap()
            .0;
        assert!(!backend.exists(&c).unwrap());
        assert_eq!(added_paths(backend).len(), 1);
    }

    #[test]
    fn a_failed_metadata_write_keeps_shared_content() {
        let (_tmp, root) = create_temp_git_repo();
        let (config, _) = init_dvs_repo(&root);
        let backend = config.backend();
        let paths = make_paths(&root, &config);
        create_file(&root, "a.txt", b"same");
        create_file(&root, "b.txt", b"same");
        // A directory where b.txt's metadata goes, so its write fails
        fs::create_dir_all(paths.metadata_path(Path::new("b.txt")).join("occupied")).unwrap();

        let results = add(&paths, backend, &["a.txt", "b.txt"], false);
        assert_eq!(
            outcomes(&results),
            [("a.txt", AddOutcome::Added), ("b.txt", AddOutcome::Failed)]
        );
        assert!(
            results[1].error.as_deref().unwrap().contains("metadata"),
            "{:?}",
            results[1].error
        );
        let hashes = FileMetadata::read(&paths.metadata_path(Path::new("a.txt")))
            .unwrap()
            .hashes;
        assert!(backend.exists(&hashes).unwrap(), "the content stays");
    }

    #[test]
    fn manifests_split_at_either_limit() {
        let item = |i: usize| ManifestItem {
            path: ProjectPath::from_canonical(&format!("dir/file-{i:04}.csv")).unwrap(),
            hashes: Hashes {
                blake3: format!("{i:064x}"),
                md5: None,
            },
            size: i as u64,
        };
        let items: Vec<_> = (0..10).map(item).collect();
        let unlimited = ManifestLimits {
            max_entries: usize::MAX,
            max_body_bytes: usize::MAX,
        };
        let whole = manifest_chunks(&items, unlimited);
        assert_eq!((whole.len(), whole[0].clone()), (1, 0..10));
        assert_eq!(manifest_chunks(&[], unlimited), Vec::<Range<usize>>::new());

        let by_count = ManifestLimits {
            max_entries: 4,
            ..unlimited
        };
        assert_eq!(manifest_chunks(&items, by_count), [0..4, 4..8, 8..10]);

        // Each entry is the same size; room for three
        let entry = serde_json::to_vec(&ManifestEntry {
            path: items[0].path.as_str().into(),
            hash: items[0].hashes.blake3.clone(),
            size: 0,
        })
        .unwrap()
        .len();
        let by_bytes = ManifestLimits {
            max_body_bytes: 16 + 3 * (entry + 1),
            ..unlimited
        };
        let chunks = manifest_chunks(&items, by_bytes);
        assert_eq!(chunks, [0..3, 3..6, 6..9, 9..10]);
        for range in chunks {
            let body = serde_json::to_vec(&crate::api::v1::Manifest {
                entries: items[range]
                    .iter()
                    .map(|i| ManifestEntry {
                        path: i.path.as_str().into(),
                        hash: i.hashes.blake3.clone(),
                        size: i.size,
                    })
                    .collect(),
            })
            .unwrap();
            assert!(body.len() <= by_bytes.max_body_bytes, "{}", body.len());
        }
    }
}
