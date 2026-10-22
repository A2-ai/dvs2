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

    /// An observer that records each call as a short line, in order, plus
    /// every finished row as JSON.
    #[derive(Default)]
    pub struct Recorder {
        pub seen: std::sync::Mutex<Vec<String>>,
        pub rows: std::sync::Mutex<Vec<serde_json::Value>>,
    }

    impl Recorder {
        pub fn seen(&self) -> Vec<String> {
            self.seen.lock().unwrap().clone()
        }

        fn push(&self, line: String) {
            self.seen.lock().unwrap().push(line);
        }
    }

    impl crate::OperationObserver for Recorder {
        fn planned(&self, p: &crate::OperationPlan) {
            let (files, bytes) = (p.files_total, p.bytes_total);
            let (to_files, to_bytes) = (p.files_to_transfer, p.bytes_to_transfer);
            self.push(format!("plan {files} {bytes} {to_files} {to_bytes}"));
        }

        fn phase_started(&self, phase: crate::Phase) {
            self.push(format!("start {}", phase.as_str()));
        }

        fn phase_finished(&self, phase: crate::Phase) {
            self.push(format!("end {}", phase.as_str()));
        }

        fn transfer_started(&self, path: &Path, _total_bytes: u64) -> Option<crate::FileProgress> {
            self.push(format!("transfer {}", path.display()));
            None
        }

        fn file_finished(&self, row: crate::FileRow<'_>) {
            let (path, json) = match row {
                crate::FileRow::Add(r) => (&r.path, serde_json::to_value(r)),
                crate::FileRow::Get(r) => (&r.path, serde_json::to_value(r)),
                crate::FileRow::Status(r) => (&r.path, serde_json::to_value(r)),
            };
            self.push(format!("row {}", path.display()));
            self.rows.lock().unwrap().push(json.unwrap());
        }
    }
}
