use std::collections::HashSet;
use std::io;
use std::path::{Path, PathBuf};
use std::time::Instant;

use crate::cache::FileStat;
use crate::error::{ErrorCode, backend_error_code};
use crate::gitignore::add_to_gitignore;
use crate::operation::{FileRow, OperationContext, OperationPlan, millis};
use crate::{Backend, Compression, DvsPaths, Hashes, PathFilter, ProjectPath, audit};
use serde::Serialize;
use uuid::Uuid;

use super::get::{
    Action, Before, FileError, GetCode, GetCounts, GetOutcome, GetRow, GetVerdict, cancel_row,
    relative_to, retrieve_into, set_row, shell_path,
};

pub const SCRATCHPAD: &str = ".dvs-scratchpad";

const MIN_PREFIX: usize = 8;

#[derive(Debug, Clone, Serialize)]
pub struct FetchRow {
    #[serde(flatten)]
    pub row: GetRow,
    pub written_to: Option<PathBuf>,
    pub hash: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct FetchReport {
    pub outcome: GetOutcome,
    pub counts: GetCounts,
    pub files: Vec<FetchRow>,
}

impl FetchReport {
    pub fn verdict(&self) -> GetVerdict {
        if self.outcome == GetOutcome::Cancelled {
            GetVerdict::Cancelled
        } else if self.counts.failed > 0 {
            GetVerdict::Failed
        } else if self.outcome == GetOutcome::Aborted {
            GetVerdict::HeldBack
        } else {
            GetVerdict::Done
        }
    }
}

#[derive(Debug, Clone)]
struct Resolved {
    path: PathBuf,
    project_path: ProjectPath,
    local_path: PathBuf,
    hashes: Hashes,
    compression: Compression,
}

struct Planned {
    row: FetchRow,
    resolved: Option<Resolved>,
    stat: Option<FileStat>,
}

fn new_row(path: PathBuf, local_path: PathBuf) -> FetchRow {
    FetchRow {
        row: GetRow {
            path,
            local_path,
            before: Before::Absent,
            action: Action::NotRun,
            satisfied: false,
            size: None,
            target_hash: None,
            local_hash: None,
            stored_copy: None,
            code: None,
            message: None,
            hint: None,
            duration_ms: None,
        },
        written_to: None,
        hash: None,
    }
}

fn fail(mut row: FetchRow, before: Before, code: GetCode, message: String) -> FetchRow {
    row.row.before = before;
    set_row(&mut row.row, Action::Failed, Some(code), Some(message));
    row
}

fn invalid(row: FetchRow, before: Before, message: String) -> Box<FetchRow> {
    Box::new(fail(
        row,
        before,
        GetCode::Shared(ErrorCode::InvalidArguments),
        message,
    ))
}

fn resolve(
    user_path: &Path,
    version: &str,
    paths: &DvsPaths,
    backend: &Backend,
) -> Result<Resolved, Box<FetchRow>> {
    let filter = PathFilter::from_user_paths(vec![user_path.to_path_buf()], false, paths);
    let relative = match filter.paths() {
        [p] if paths.metadata_path(p).is_file() => p.clone(),
        _ => {
            let row = new_row(user_path.to_path_buf(), user_path.to_path_buf());
            return Err(invalid(
                row,
                Before::NotTracked,
                "not a tracked file".into(),
            ));
        }
    };
    let mut row = new_row(relative.clone(), user_path.to_path_buf());
    let prefix = version.to_ascii_lowercase();
    if prefix.len() < MIN_PREFIX || !prefix.chars().all(|c| c.is_ascii_hexdigit()) {
        let message =
            format!("version must be at least {MIN_PREFIX} hex characters of a hash: {version}");
        return Err(invalid(row, Before::Unknown, message));
    }
    let project_path = match ProjectPath::from_path(&relative) {
        Ok(p) => p,
        Err(e) => return Err(invalid(row, Before::NotTracked, e.to_string())),
    };
    let entries = match backend.get_audit_entries(&[project_path.to_native()]) {
        Ok(e) => e,
        Err(e) => {
            let code = backend_error_code(&e)
                .map(GetCode::from)
                .unwrap_or(GetCode::Shared(ErrorCode::IoError));
            let message = format!("cannot read its history: {e:#}");
            return Err(Box::new(fail(row, Before::Unknown, code, message)));
        }
    };

    // Newest first: a hash added twice keeps its latest compression
    let mut seen = HashSet::new();
    let mut matches: Vec<(Hashes, Compression)> = entries
        .into_iter()
        .filter_map(|entry| match entry.action {
            audit::Action::Add {
                file, compression, ..
            } if file.hashes.blake3.starts_with(&prefix) => Some((file.hashes, compression)),
            _ => None,
        })
        .filter(|(hashes, _)| seen.insert(hashes.blake3.clone()))
        .collect();
    if let [_] = matches.as_slice() {
        let (hashes, compression) = matches.remove(0);
        return Ok(Resolved {
            path: relative,
            project_path,
            local_path: user_path.to_path_buf(),
            hashes,
            compression,
        });
    }
    let message = match matches.len() {
        0 => format!("no version has a hash starting with {prefix}"),
        n => format!("{prefix} matches {n} versions; give more of the hash"),
    };
    row.row.hint = Some(format!(
        "list its versions with: dvs history {}",
        shell_path(user_path)
    ));
    Err(invalid(row, Before::Unknown, message))
}

fn scratch_target(file: &Resolved) -> PathBuf {
    Path::new(SCRATCHPAD)
        .join(&file.hashes.blake3[..MIN_PREFIX])
        .join(&file.path)
}

// A scratchpad or label folder symlinked to the root resolves to the root,
// so only the root path itself is accepted.
fn inside_scratchpad(paths: &DvsPaths, target: &Path) -> bool {
    let root = paths.repo_root();
    let Ok(real_root) = std::fs::canonicalize(root) else {
        return false;
    };
    let scratchpad = real_root.join(SCRATCHPAD);
    let Some(d) = existing_ancestor(target) else {
        return false;
    };
    std::fs::canonicalize(d)
        .map(|real| (d == root && real == real_root) || real.starts_with(&scratchpad))
        .unwrap_or(false)
}

/// The nearest ancestor of `target` that exists (a symlink counts).
fn existing_ancestor(target: &Path) -> Option<&Path> {
    target
        .ancestors()
        .skip(1)
        .find(|d| std::fs::symlink_metadata(d).is_ok())
}

fn prepare_scratchpad(paths: &DvsPaths) -> Result<(), FileError> {
    add_to_gitignore(paths.repo_root(), &[PathBuf::from(SCRATCHPAD)]).map_err(|e| {
        (
            GetCode::Shared(ErrorCode::IoError),
            format!("cannot add {SCRATCHPAD} to .gitignore: {e:#}"),
        )
    })
}

fn plan_copy(paths: &DvsPaths, resolved: Resolved) -> Planned {
    let mut row = new_row(resolved.path.clone(), resolved.local_path.clone());
    let target = paths.file_path(&scratch_target(&resolved));
    row.written_to = Some(relative_to(paths.cwd(), &target));
    row.hash = Some(resolved.hashes.blake3.clone());
    row.row.target_hash = row.hash.clone();
    let mut item = Planned {
        row,
        resolved: Some(resolved),
        stat: None,
    };
    let failed = |mut item: Planned, before, code, message| {
        item.row = fail(item.row, before, code, message);
        item
    };

    if !inside_scratchpad(paths, &target) {
        return failed(
            item,
            Before::UnsafePath,
            GetCode::UnsafePath,
            "a parent directory leads outside the scratchpad (symlink?); dvs will not write there"
                .to_string(),
        );
    }
    match std::fs::symlink_metadata(&target) {
        Err(e) if e.kind() == io::ErrorKind::NotFound => return item,
        Err(e) => {
            return failed(
                item,
                Before::Unknown,
                GetCode::Shared(ErrorCode::IoError),
                e.to_string(),
            );
        }
        Ok(meta) if !meta.is_file() => {
            return failed(
                item,
                Before::NotAFile,
                GetCode::NotAFile,
                "is not a regular file; dvs will not replace it".to_string(),
            );
        }
        Ok(_) => {}
    }
    let stat = FileStat::from_path(&target).ok();
    let (hashes, size) = match Hashes::compute_from_path(&target, &[]) {
        Ok(h) => h,
        Err(e) => {
            return failed(
                item,
                Before::Unknown,
                GetCode::Shared(ErrorCode::IoError),
                format!("cannot read the copy already there: {e:#}"),
            );
        }
    };
    item.stat = stat;
    item.row.row.local_hash = Some(hashes.blake3.clone());
    if Some(&hashes.blake3) == item.row.hash.as_ref() {
        item.row.row.before = Before::Current;
        item.row.row.size = Some(size);
        set_row(&mut item.row.row, Action::Unchanged, None, None);
    } else {
        item.row.row.before = Before::Modified;
        set_row(
            &mut item.row.row,
            Action::NotRun,
            Some(GetCode::LocalChanges),
            Some("the copy already there was changed".to_string()),
        );
        item.row.row.hint = Some(format!(
            "to replace it: dvs fetch --force {} --version {}",
            shell_path(&item.row.row.local_path),
            item.row.hash.as_deref().unwrap_or_default()
        ));
    }
    item
}

/// Never errors: every problem, bad arguments included, is in the returned rows.
pub fn fetch_files(
    path: &Path,
    version: &str,
    paths: &DvsPaths,
    backend: &Backend,
    force: bool,
    ctx: &OperationContext,
) -> FetchReport {
    let observer = ctx.observer();

    let mut items: Vec<Planned> = [resolve(path, version, paths, backend)]
        .into_iter()
        .map(|r| match r {
            Ok(resolved) => plan_copy(paths, resolved),
            Err(row) => Planned {
                row: *row,
                resolved: None,
                stat: None,
            },
        })
        .collect();

    let blocked = items.iter().any(|i| is_known_problem(&i.row.row, force));
    let transfers: Vec<usize> = if blocked {
        Vec::new()
    } else {
        (0..items.len())
            .filter(|&i| needs_transfer(&items[i].row.row, force))
            .collect()
    };
    observer.planned(&OperationPlan {
        files_total: items.len() as u64,
        bytes_total: 0,
        files_to_transfer: transfers.len() as u64,
        bytes_to_transfer: 0,
    });

    let operation_id = Uuid::new_v4();
    let mut prepared: Option<Result<(), FileError>> = None;
    for &i in &transfers {
        let item = &mut items[i];
        if ctx.is_cancelled() {
            cancel_row(&mut item.row.row);
            continue;
        }
        let started = Instant::now();
        let result = prepared
            .get_or_insert_with(|| prepare_scratchpad(paths))
            .clone()
            .and_then(|()| install_copy(item, paths, backend, operation_id, ctx));
        item.row.row.duration_ms = Some(millis(started.elapsed()));
        let was_conflict = item.row.row.is_conflict();
        match result {
            Ok(size) => {
                let action = if item.row.row.before == Before::Absent {
                    Action::Restored
                } else {
                    Action::Replaced
                };
                let message = was_conflict.then(|| "local changes were discarded".to_string());
                item.row.row.size = Some(size);
                item.row.row.hint = None;
                set_row(&mut item.row.row, action, None, message);
            }
            Err((code, message)) => {
                item.row.row.hint = None;
                set_row(&mut item.row.row, Action::Failed, Some(code), Some(message));
            }
        }
    }

    for item in &items {
        observer.file_finished(FileRow::Get(&item.row.row));
    }
    finish(items, blocked, ctx)
}

fn is_known_problem(row: &GetRow, force: bool) -> bool {
    row.action == Action::Failed || (row.is_conflict() && !force)
}

fn needs_transfer(row: &GetRow, force: bool) -> bool {
    row.action == Action::NotRun && (row.before == Before::Absent || (row.is_conflict() && force))
}

fn install_copy(
    item: &Planned,
    paths: &DvsPaths,
    backend: &Backend,
    operation_id: Uuid,
    ctx: &OperationContext,
) -> Result<u64, FileError> {
    let resolved = item.resolved.as_ref().expect("transfers are resolved");
    let target = paths.file_path(&scratch_target(resolved));
    let progress = ctx.observer().transfer_started(&resolved.path, 0);
    let on_bytes = progress.as_ref().map(|fp| &*fp.on_bytes);
    let result = retrieve_into(
        backend,
        &target,
        &resolved.hashes,
        resolved.compression,
        resolved.project_path.clone(),
        &item.stat,
        operation_id,
        on_bytes,
    );
    if let Some(fp) = &progress {
        (fp.on_done)(result.is_ok());
    }
    result?;
    std::fs::metadata(&target)
        .map(|m| m.len())
        .map_err(|e| (GetCode::Shared(ErrorCode::IoError), e.to_string()))
}

fn finish(items: Vec<Planned>, aborted: bool, ctx: &OperationContext) -> FetchReport {
    let files: Vec<FetchRow> = items.into_iter().map(|i| i.row).collect();
    let mut counts = GetCounts {
        total: files.len(),
        succeeded: files.iter().filter(|r| r.row.satisfied).count(),
        ..GetCounts::default()
    };
    for r in &files {
        let slot = match r.row.action {
            Action::Restored => &mut counts.restored,
            Action::Replaced => &mut counts.replaced,
            Action::Unchanged => &mut counts.unchanged,
            Action::Skipped => &mut counts.skipped,
            Action::NotRun => &mut counts.not_run,
            Action::Cancelled => &mut counts.cancelled,
            Action::Failed => &mut counts.failed,
            Action::WouldRestore => &mut counts.would_restore,
            Action::WouldReplace => &mut counts.would_replace,
        };
        *slot += 1;
    }
    let outcome = if ctx.is_cancelled() {
        GetOutcome::Cancelled
    } else if aborted {
        GetOutcome::Aborted
    } else if counts.succeeded == counts.total {
        GetOutcome::Complete
    } else if counts.succeeded + counts.skipped == 0 {
        GetOutcome::Failed
    } else {
        GetOutcome::Partial
    };
    FetchReport {
        outcome,
        counts,
        files,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cache::{baseline_for, try_open_cache};
    use crate::config::Config;
    use crate::files::metadata::FileMetadata;
    use crate::testutil::{create_file, create_temp_git_repo, init_dvs_repo};
    use crate::{AddDetail, add_files};
    use fs_err as fs;
    use tempfile::TempDir;

    struct TestRepo {
        _tmp: TempDir,
        root: PathBuf,
        config: Config,
        paths: DvsPaths,
    }

    impl TestRepo {
        fn new() -> Self {
            let (_tmp, root) = create_temp_git_repo();
            let (config, _dvs_dir) = init_dvs_repo(&root);
            let paths =
                DvsPaths::new(root.clone(), root.clone(), config.metadata_folder_name()).unwrap();
            Self {
                _tmp,
                root,
                config,
                paths,
            }
        }

        /// Write `content` at `name`, `dvs add` it and return its full hash.
        fn add(&self, name: &str, content: &[u8]) -> String {
            create_file(&self.root, name, content);
            let results = add_files(
                vec![name.into()],
                &self.paths,
                self.config.backend(),
                None,
                Compression::Zstd,
                false,
                &OperationContext::new(),
            )
            .unwrap()
            .files;
            assert!(matches!(results[0].detail, AddDetail::Success { .. }));
            let bytes = fs::read(self.paths.metadata_path(Path::new(name))).unwrap();
            serde_json::from_slice::<FileMetadata>(&bytes)
                .unwrap()
                .hashes
                .blake3
        }

        fn fetch(&self, path: &str, version: &str, force: bool) -> FetchReport {
            fetch_files(
                Path::new(path),
                version,
                &self.paths,
                self.config.backend(),
                force,
                &OperationContext::new(),
            )
        }

        fn scratch(&self, hash: &str, name: &str) -> PathBuf {
            self.root.join(SCRATCHPAD).join(&hash[..8]).join(name)
        }
    }

    fn two_versions() -> (TestRepo, String) {
        let repo = TestRepo::new();
        let old = repo.add("data/a.csv", b"old");
        repo.add("data/a.csv", b"new");
        (repo, old)
    }

    #[test]
    fn old_version_lands_in_the_scratchpad_and_a_second_run_is_unchanged() {
        let (repo, old) = two_versions();

        let report = repo.fetch("data/a.csv", &old[..8].to_uppercase(), false);
        assert_eq!(report.outcome, GetOutcome::Complete, "{report:#?}");
        let r = &report.files[0];
        assert_eq!(
            (r.row.before, r.row.action),
            (Before::Absent, Action::Restored)
        );
        assert_eq!(r.hash.as_deref(), Some(old.as_str()));
        let written = Path::new(SCRATCHPAD).join(&old[..8]).join("data/a.csv");
        assert_eq!(r.written_to.as_deref(), Some(written.as_path()));
        assert_eq!(fs::read(repo.scratch(&old, "data/a.csv")).unwrap(), b"old");

        let again = repo.fetch("data/a.csv", &old[..8], false);
        let r = &again.files[0];
        assert_eq!(
            (r.row.before, r.row.action),
            (Before::Current, Action::Unchanged)
        );
    }

    #[test]
    fn tracking_is_untouched() {
        let (repo, old) = two_versions();
        let record = repo.paths.metadata_path(Path::new("data/a.csv"));
        let record_before = fs::read(&record).unwrap();
        let audit_before = repo.config.backend().get_audit_entries(&[]).unwrap().len();
        let cache = try_open_cache(&repo.paths);
        let baseline_before = baseline_for(cache.as_ref(), "data/a.csv");
        drop(cache);

        let report = repo.fetch("data/a.csv", &old, false);
        assert_eq!(report.outcome, GetOutcome::Complete, "{report:#?}");

        assert_eq!(fs::read(repo.root.join("data/a.csv")).unwrap(), b"new");
        assert_eq!(fs::read(&record).unwrap(), record_before);
        let audit_after = repo.config.backend().get_audit_entries(&[]).unwrap().len();
        assert_eq!(audit_after, audit_before);
        let cache = try_open_cache(&repo.paths);
        assert_eq!(baseline_for(cache.as_ref(), "data/a.csv"), baseline_before);
        assert!(baseline_before.is_some());
    }

    #[test]
    fn first_write_gitignores_the_scratchpad() {
        let (repo, old) = two_versions();
        repo.fetch("data/a.csv", &old, false);
        let gitignore = fs::read_to_string(repo.root.join(".gitignore")).unwrap();
        assert!(
            gitignore.lines().any(|l| l == "/.dvs-scratchpad"),
            "{gitignore}"
        );
    }

    #[test]
    fn an_edited_scratch_copy_is_kept_unless_forced() {
        let (repo, old) = two_versions();
        repo.fetch("data/a.csv", &old, false);
        let copy = repo.scratch(&old, "data/a.csv");
        fs::write(&copy, b"edited").unwrap();

        let report = repo.fetch("data/a.csv", &old, false);
        assert_eq!(report.outcome, GetOutcome::Aborted);
        let r = &report.files[0];
        assert_eq!(
            (r.row.before, r.row.action),
            (Before::Modified, Action::NotRun)
        );
        assert_eq!(r.row.code, Some(GetCode::LocalChanges));
        assert_eq!(fs::read(&copy).unwrap(), b"edited");

        let report = repo.fetch("data/a.csv", &old, true);
        assert_eq!(report.outcome, GetOutcome::Complete);
        assert_eq!(report.files[0].row.action, Action::Replaced);
        assert_eq!(fs::read(&copy).unwrap(), b"old");
    }

    #[test]
    fn bad_versions_fail_as_invalid_arguments() {
        let (repo, old) = two_versions();
        for (version, why) in [
            ("00000000", "no version"),
            (&old[..7], "at least 8"),
            ("zzzzzzzz", "hex characters"),
        ] {
            let report = repo.fetch("data/a.csv", version, false);
            let r = &report.files[0];
            assert!(r.row.message.as_deref().unwrap().contains(why), "{r:#?}");
            assert_eq!(r.row.action, Action::Failed, "{version}");
            assert_eq!(
                r.row.code,
                Some(GetCode::Shared(ErrorCode::InvalidArguments)),
                "{version}"
            );
            assert_eq!(report.outcome, GetOutcome::Aborted, "{version}");
            assert_eq!(report.verdict(), GetVerdict::Failed);
        }
        assert!(!repo.root.join(SCRATCHPAD).exists());
    }

    #[test]
    fn a_prefix_matching_two_versions_asks_for_more_of_the_hash() {
        let (repo, old) = two_versions();
        // A second version sharing the first 8 characters of `old`
        let twin = format!("{}{}", &old[..8], "f".repeat(old.len() - 8));
        let Backend::Local(local) = repo.config.backend() else {
            unreachable!()
        };
        let file = audit::AuditFile {
            path: "data/a.csv".into(),
            hashes: Hashes {
                blake3: twin,
                md5: None,
            },
        };
        let entry = audit::AuditEntry::new_add(Uuid::new_v4(), file, Compression::Zstd, None);
        local.log_audit_entry(&entry).unwrap();

        let report = repo.fetch("data/a.csv", &old[..8], false);
        let r = &report.files[0];
        assert_eq!(
            r.row.code,
            Some(GetCode::Shared(ErrorCode::InvalidArguments))
        );
        assert!(
            r.row
                .message
                .as_deref()
                .unwrap()
                .contains("give more of the hash")
        );
        let report = repo.fetch("data/a.csv", &old, false);
        assert_eq!(report.outcome, GetOutcome::Complete, "{report:#?}");
    }

    #[test]
    fn written_to_is_relative_to_the_working_directory() {
        let (repo, old) = two_versions();
        let in_data = DvsPaths::new(
            repo.root.join("data"),
            repo.root.clone(),
            repo.config.metadata_folder_name(),
        )
        .unwrap();
        let report = fetch_files(
            Path::new("a.csv"),
            &old,
            &in_data,
            repo.config.backend(),
            false,
            &OperationContext::new(),
        );
        let written = Path::new("..")
            .join(SCRATCHPAD)
            .join(&old[..8])
            .join("data/a.csv");
        assert_eq!(
            report.files[0].written_to.as_deref(),
            Some(written.as_path())
        );
        assert_eq!(
            fs::read(repo.root.join("data").join(&written)).unwrap(),
            b"old"
        );
    }

    #[test]
    fn a_version_added_twice_is_one_version() {
        let (repo, old) = two_versions();
        repo.add("data/a.csv", b"old");

        let report = repo.fetch("data/a.csv", &old[..8], false);
        assert_eq!(report.outcome, GetOutcome::Complete, "{report:#?}");
    }

    #[test]
    fn untracked_paths_and_folders_fail_as_invalid_arguments() {
        let (repo, old) = two_versions();
        create_file(&repo.root, "notes.txt", b"untracked");
        for (path, why) in [
            ("notes.txt", "not a tracked file"),
            ("data", "not a tracked file"),
            ("missing.csv", "not a tracked file"),
        ] {
            let report = repo.fetch(path, &old, false);
            let r = &report.files[0];
            assert!(r.row.message.as_deref().unwrap().contains(why), "{r:#?}");
            assert_eq!(
                (r.row.action, r.row.code),
                (
                    Action::Failed,
                    Some(GetCode::Shared(ErrorCode::InvalidArguments))
                ),
                "{path}"
            );
        }
        assert!(!repo.root.join(SCRATCHPAD).exists());
    }

    #[cfg(unix)]
    #[test]
    fn a_label_folder_symlinked_to_the_root_is_refused() {
        let repo = TestRepo::new();
        let old = repo.add("a.csv", b"old");
        repo.add("a.csv", b"new");
        fs::create_dir(repo.root.join(SCRATCHPAD)).unwrap();
        std::os::unix::fs::symlink("..", repo.root.join(SCRATCHPAD).join(&old[..8])).unwrap();

        let report = repo.fetch("a.csv", &old, true);
        let r = &report.files[0];
        assert_eq!(
            (r.row.action, r.row.code),
            (Action::Failed, Some(GetCode::UnsafePath)),
            "{r:#?}"
        );
        assert_eq!(fs::read(repo.root.join("a.csv")).unwrap(), b"new");
    }

    #[test]
    fn a_version_missing_from_storage_fails_and_writes_nothing() {
        let (repo, old) = two_versions();
        let hashes = Hashes {
            blake3: old.clone(),
            md5: None,
        };
        repo.config.backend().remove(&hashes).unwrap();

        let report = repo.fetch("data/a.csv", &old, false);
        let r = &report.files[0];
        assert_eq!(
            (r.row.action, r.row.code),
            (Action::Failed, Some(GetCode::SourceMissing)),
            "{r:#?}"
        );
        assert!(!repo.scratch(&old, "data/a.csv").exists());
    }
}
