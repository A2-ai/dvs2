use std::sync::mpsc::Sender;

use serde::{Deserialize, Serialize};

/// A single timing measurement emitted during a DVS operation.
///
/// When verbosity >= 3 (`-vvv`), these are collected by a background
/// CSV-writing thread in the CLI.
#[derive(Debug, Clone, Serialize)]
pub struct TimingRecord {
    pub timestamp: String,
    pub user: String,
    pub dvs_version: String,
    pub git_commit: String,
    pub command: String,
    pub file: String,
    pub step: String,
    pub duration_ms: f64,
    pub file_size_bytes: Option<u64>,
    pub num_files: Option<usize>,
    pub compression: String,
    pub hash_algorithm: String,
}

/// Common options shared across all DVS commands.
#[derive(Clone, Default)]
pub struct OutputOptions {
    pub dry_run: bool,
    pub verbosity: u8,
    /// When set, timing records are sent to a background CSV writer.
    pub timing_tx: Option<Sender<TimingRecord>>,
}

impl std::fmt::Debug for OutputOptions {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("OutputOptions")
            .field("dry_run", &self.dry_run)
            .field("verbosity", &self.verbosity)
            .field("timing_tx", &self.timing_tx.as_ref().map(|_| "..."))
            .finish()
    }
}

impl OutputOptions {
    /// Send a timing record if the sender is available.
    pub fn send_timing(&self, record: TimingRecord) {
        if let Some(tx) = &self.timing_tx {
            let _ = tx.send(record);
        }
    }

    /// Create a partially-filled TimingRecord with common fields.
    /// Caller fills in `file`, `step`, `duration_ms`, `file_size_bytes`, `compression`.
    pub fn timing_template(&self, command: &str) -> TimingRecord {
        TimingRecord {
            timestamp: jiff::Timestamp::now().to_string(),
            user: whoami::username().unwrap_or_else(|_| "unknown".into()),
            dvs_version: env!("CARGO_PKG_VERSION").into(),
            git_commit: option_env!("DVS_GIT_COMMIT").unwrap_or("unknown").into(),
            command: command.into(),
            file: String::new(),
            step: String::new(),
            duration_ms: 0.0,
            file_size_bytes: None,
            num_files: None,
            compression: String::new(),
            hash_algorithm: "blake3".into(),
        }
    }
}

/// Outcome of an add or get operation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Outcome {
    /// File was copied to/from storage.
    Copied,
    /// File was already present (no action needed).
    Present,
}

#[derive(Debug, Copy, Clone, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Status {
    /// Local file not tracked in dvs
    Untracked,
    /// Local file exists and matches stored version.
    Current,
    /// Metadata exists but local file is missing.
    Absent,
    /// Local file exists but differs from stored version.
    Unsynced,
}
