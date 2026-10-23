use std::fmt;

use serde::{Deserialize, Serialize};

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
    /// Local file matches the baseline, while metadata points to another version.
    Stale,
    /// Local file differs from current metadata and from the local baseline.
    Modified,
    /// Local file differs from metadata, but no baseline exists to explain why.
    Unsynced,
}

impl fmt::Display for Status {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Status::Current => write!(f, "current"),
            Status::Absent => write!(f, "absent"),
            Status::Stale => write!(f, "stale"),
            Status::Modified => write!(f, "modified"),
            Status::Unsynced => write!(f, "unsynced"),
            Status::Untracked => write!(f, "untracked"),
        }
    }
}
