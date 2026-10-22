pub mod audit;
pub mod auth;
pub mod backends;
pub mod cache;
pub mod config;
pub mod error;
mod files;
pub mod git;
mod gitignore;
pub mod globbing;
mod hashes;
pub mod init;
pub mod inspect;
pub mod operation;
pub mod paths;
pub mod progress;
pub(crate) mod utils;

pub use backends::{Backend, RetrieveRequest, StoreRequest, StoreResult};
pub use cache::HashSource;
pub use config::Compression;
pub use error::{BackendError, ErrorCode, backend_error_code};
pub use files::add::{AddDetail, AddErrorCode, AddReport, AddResult, add_files};
pub use files::get::{
    Action, Before, GetCode, GetCounts, GetOptions, GetOutcome, GetReport, GetRow, StoredCopy,
    get_files,
};
pub use files::hash::{HashRow, hash_files};
pub use files::metadata::FileMetadata;
pub use files::status::{FileStatus, StatusDetail, get_status};
pub use files::types::{Outcome, Status};
pub use hashes::{HashAlg, Hashes};
pub use operation::{
    ByteTotals, CancelToken, FileRow, NoopObserver, OperationContext, OperationObserver,
    OperationPlan, Phase, TransferObserver,
};
pub use paths::{AddPathStatus, DvsPaths, PathFilter, ProjectPath, find_repo_root};
pub use progress::FileProgress;
pub use utils::{format_size, set_num_threads};

pub const VERSION: &str = env!("CARGO_PKG_VERSION");

#[cfg(test)]
pub mod testutil {
    use fs_err as fs;
    use std::path::{Path, PathBuf};
    use tempfile::TempDir;

    use crate::config::Config;
    use crate::init::init;

    /// Creates a temporary directory with a .git folder (simulating a git repo).
    /// Returns the TempDir (owns the directory) and the path to the repo root.
    ///
    /// IMPORTANT: Keep the TempDir alive for the duration of the test,
    /// otherwise the directory gets deleted.
    pub fn create_temp_git_repo() -> (TempDir, PathBuf) {
        let tmp = tempfile::tempdir().unwrap();
        // The repo lives in a subdirectory of the tempdir so that storage can
        // be placed as a sibling (outside the repo), matching real usage and
        // the storage-inside-repo guard in `init`.
        let repo_root = fs::canonicalize(tmp.path()).unwrap().join("repo");
        fs::create_dir(&repo_root).unwrap();
        fs::create_dir(repo_root.join(".git")).unwrap();
        (tmp, repo_root)
    }

    /// Creates a file with the given content at the specified path.
    /// Creates parent directories if needed.
    /// Returns the full path to the created file.
    pub fn create_file(dir: &Path, relative_path: &str, content: &[u8]) -> PathBuf {
        let path = dir.join(relative_path);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).ok();
        }
        fs::write(&path, content).unwrap();
        path
    }

    /// Initializes a DVS repository in the given directory.
    /// Creates storage at `{repo_root}/../.storage` (a sibling, outside the
    /// repo) and metadata at `{repo_root}/.dvs`.
    /// Returns (config, dvs_metadata_dir).
    pub fn init_dvs_repo(repo_root: &Path) -> (Config, PathBuf) {
        let storage_dir = repo_root.parent().unwrap().join(".storage");
        let config = Config::new_local(&storage_dir, None).unwrap();
        init(repo_root, config.clone()).unwrap();
        let dvs_dir = repo_root.join(".dvs");
        (config, dvs_dir)
    }

    /// One observer call, as [`Recorder`] saw it.
    #[derive(Debug, Clone, PartialEq)]
    pub enum Seen {
        Planned(crate::OperationPlan),
        PhaseStarted(crate::Phase),
        PhaseFinished(crate::Phase),
        TransferStarted(PathBuf),
        /// A finished row: its path, row code (None on success) and duration
        Finished {
            path: PathBuf,
            code: Option<String>,
            duration_ms: Option<u64>,
        },
    }

    /// An observer that records every call in order and can cancel the
    /// operation once a given number of transfers (or rows) have started.
    #[derive(Default)]
    pub struct Recorder {
        pub seen: std::sync::Mutex<Vec<Seen>>,
        cancel: Option<(crate::CancelToken, usize)>,
        cancel_on_rows: bool,
        /// Cancels when this phase starts (`true`) or finishes (`false`)
        cancel_at_phase: Option<(crate::CancelToken, crate::Phase, bool)>,
        count: std::sync::atomic::AtomicUsize,
    }

    impl Recorder {
        /// Cancels `token` when the `n`th transfer starts.
        pub fn cancelling_on_transfer(token: crate::CancelToken, n: usize) -> Self {
            Self {
                cancel: Some((token, n)),
                ..Self::default()
            }
        }

        /// Cancels `token` when the `n`th row finishes.
        pub fn cancelling_on_row(token: crate::CancelToken, n: usize) -> Self {
            Self {
                cancel: Some((token, n)),
                cancel_on_rows: true,
                ..Self::default()
            }
        }

        /// Cancels `token` as `phase` starts.
        pub fn cancelling_as_phase_starts(token: crate::CancelToken, phase: crate::Phase) -> Self {
            Self {
                cancel_at_phase: Some((token, phase, true)),
                ..Self::default()
            }
        }

        /// Cancels `token` once `phase` has finished.
        pub fn cancelling_after_phase(token: crate::CancelToken, phase: crate::Phase) -> Self {
            Self {
                cancel_at_phase: Some((token, phase, false)),
                ..Self::default()
            }
        }

        fn phase_tick(&self, phase: crate::Phase, starting: bool) {
            if let Some((token, at, on_start)) = &self.cancel_at_phase {
                if *at == phase && *on_start == starting {
                    token.cancel();
                }
            }
        }

        pub fn seen(&self) -> Vec<Seen> {
            self.seen.lock().unwrap().clone()
        }

        /// The finished rows, in completion order.
        pub fn finished(&self) -> Vec<(PathBuf, Option<String>, Option<u64>)> {
            self.seen()
                .into_iter()
                .filter_map(|s| match s {
                    Seen::Finished {
                        path,
                        code,
                        duration_ms,
                    } => Some((path, code, duration_ms)),
                    _ => None,
                })
                .collect()
        }

        /// The one plan reported.
        pub fn plan(&self) -> crate::OperationPlan {
            let plans: Vec<_> = self
                .seen()
                .into_iter()
                .filter_map(|s| match s {
                    Seen::Planned(plan) => Some(plan),
                    _ => None,
                })
                .collect();
            assert_eq!(plans.len(), 1, "expected one plan, got {plans:?}");
            plans[0]
        }

        pub fn transfers(&self) -> Vec<PathBuf> {
            self.seen()
                .into_iter()
                .filter_map(|s| match s {
                    Seen::TransferStarted(path) => Some(path),
                    _ => None,
                })
                .collect()
        }

        /// Plan and phase calls only, in order.
        pub fn outline(&self) -> Vec<Seen> {
            self.seen()
                .into_iter()
                .filter(|s| {
                    matches!(
                        s,
                        Seen::Planned(_) | Seen::PhaseStarted(_) | Seen::PhaseFinished(_)
                    )
                })
                .collect()
        }

        fn tick(&self, rows: bool) {
            if let Some((token, n)) = &self.cancel {
                if rows == self.cancel_on_rows
                    && self.count.fetch_add(1, std::sync::atomic::Ordering::SeqCst) + 1 == *n
                {
                    token.cancel();
                }
            }
        }
    }

    impl crate::OperationObserver for Recorder {
        fn planned(&self, plan: &crate::OperationPlan) {
            self.seen.lock().unwrap().push(Seen::Planned(*plan));
        }

        fn phase_started(&self, phase: crate::Phase) {
            self.seen.lock().unwrap().push(Seen::PhaseStarted(phase));
            self.phase_tick(phase, true);
        }

        fn phase_finished(&self, phase: crate::Phase) {
            self.seen.lock().unwrap().push(Seen::PhaseFinished(phase));
            self.phase_tick(phase, false);
        }

        fn transfer_started(&self, path: &Path, _total_bytes: u64) -> Option<crate::FileProgress> {
            self.seen
                .lock()
                .unwrap()
                .push(Seen::TransferStarted(path.to_path_buf()));
            self.tick(false);
            None
        }

        fn file_finished(&self, row: crate::FileRow<'_>) {
            let (path, code, duration_ms) = match row {
                crate::FileRow::Add(r) => (
                    r.path.clone(),
                    match &r.detail {
                        crate::AddDetail::Success { .. } => None,
                        crate::AddDetail::Error { code, .. } => Some(
                            serde_json::to_value(code)
                                .unwrap()
                                .as_str()
                                .unwrap()
                                .to_string(),
                        ),
                    },
                    r.duration_ms,
                ),
                crate::FileRow::Get(r) => (r.path.clone(), None, None),
                crate::FileRow::Status(r) => (r.path.clone(), r.code.map(str::to_string), None),
                crate::FileRow::Hash(r) => {
                    (r.path.clone(), r.code.map(str::to_string), r.duration_ms)
                }
            };
            self.seen.lock().unwrap().push(Seen::Finished {
                path,
                code,
                duration_ms,
            });
            self.tick(true);
        }
    }
}
