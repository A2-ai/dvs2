pub mod audit;
pub mod auth;
pub mod backends;
mod cache;
pub mod config;
pub mod error;
mod files;
pub mod git;
mod gitignore;
pub mod globbing;
mod hashes;
pub mod init;
pub mod operation;
pub mod paths;
pub mod progress;
pub(crate) mod utils;

pub use backends::{Backend, RetrieveRequest, StoreRequest, StoreResult};
pub use config::Compression;
pub use error::{Error, ErrorCode, backend_error_code};
pub use files::add::{AddDetail, AddReport, AddResult, add_files};
pub use files::get::{
    Action, Before, GetCode, GetCounts, GetOptions, GetOutcome, GetReport, GetRow, GetVerdict,
    StoredCopy, get_files, shell_path,
};
pub use files::metadata::FileMetadata;
pub use files::status::{Baseline, StatusFacts, WorkingCopy, inspect_status};
pub use files::types::{Outcome, Status};
pub use hashes::{HashAlg, Hashes};
pub use operation::{
    ByteTotals, Counts, FileRow, OperationContext, OperationObserver, OperationPlan, Phase,
    TransferObserver,
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

    /// An observer that records every call in order.
    #[derive(Default)]
    pub struct Recorder {
        pub seen: std::sync::Mutex<Vec<Seen>>,
    }

    impl Recorder {
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
    }

    impl crate::OperationObserver for Recorder {
        fn planned(&self, plan: &crate::OperationPlan) {
            self.seen.lock().unwrap().push(Seen::Planned(*plan));
        }

        fn phase_started(&self, phase: crate::Phase) {
            self.seen.lock().unwrap().push(Seen::PhaseStarted(phase));
        }

        fn phase_finished(&self, phase: crate::Phase) {
            self.seen.lock().unwrap().push(Seen::PhaseFinished(phase));
        }

        fn transfer_started(&self, path: &Path, _total_bytes: u64) -> Option<crate::FileProgress> {
            self.seen
                .lock()
                .unwrap()
                .push(Seen::TransferStarted(path.to_path_buf()));
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
                crate::FileRow::Status(r) => {
                    (r.path.clone(), r.code.map(|c| c.as_str().to_string()), None)
                }
            };
            self.seen.lock().unwrap().push(Seen::Finished {
                path,
                code,
                duration_ms,
            });
        }
    }
}
