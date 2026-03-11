use serde::{Deserialize, Serialize};

/// Common options shared across all DVS commands.
#[derive(Debug, Clone, Copy, Default)]
pub struct OutputOptions {
    pub dry_run: bool,
    pub verbose: bool,
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
