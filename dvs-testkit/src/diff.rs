//! Snapshot diffing utilities.

use std::collections::BTreeSet;
use std::path::PathBuf;

use crate::snapshot::{WorkspaceSnapshot, ObjectPresence};

/// Difference between two workspace snapshots.
#[derive(Debug, Clone, Default)]
pub struct SnapshotDiff {
    /// List of mismatches found.
    pub mismatches: Vec<Mismatch>,
}

/// A mismatch between expected and actual state.
#[derive(Debug, Clone)]
pub enum Mismatch {
    /// File tracked in expected but not actual.
    MissingTrackedFile { path: PathBuf },

    /// File tracked in actual but not expected.
    ExtraTrackedFile { path: PathBuf },

    /// Checksum mismatch for a tracked file.
    ChecksumMismatch {
        path: PathBuf,
        expected: String,
        actual: String,
    },

    /// Storage object expected but missing.
    MissingStorageObject { path: PathBuf },

    /// Config expected but not found.
    MissingConfig,

    /// Config found but not expected.
    UnexpectedConfig,

    /// Gitignore pattern mismatch.
    GitignoreMismatch {
        expected_has_dvs: bool,
        actual_has_dvs: bool,
    },

    /// Storage object count mismatch.
    StorageCountMismatch { expected: usize, actual: usize },
}

impl std::fmt::Display for Mismatch {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Mismatch::MissingTrackedFile { path } => {
                write!(f, "Missing tracked file: {}", path.display())
            }
            Mismatch::ExtraTrackedFile { path } => {
                write!(f, "Extra tracked file: {}", path.display())
            }
            Mismatch::ChecksumMismatch { path, expected, actual } => {
                write!(
                    f,
                    "Checksum mismatch for {}: expected {}, got {}",
                    path.display(),
                    expected,
                    actual
                )
            }
            Mismatch::MissingStorageObject { path } => {
                write!(f, "Missing storage object for: {}", path.display())
            }
            Mismatch::MissingConfig => write!(f, "Missing config ({})", dvs_core::Config::config_filename()),
            Mismatch::UnexpectedConfig => write!(f, "Unexpected config ({})", dvs_core::Config::config_filename()),
            Mismatch::GitignoreMismatch { expected_has_dvs, actual_has_dvs } => {
                write!(
                    f,
                    "Gitignore *.dvs pattern: expected {}, got {}",
                    expected_has_dvs, actual_has_dvs
                )
            }
            Mismatch::StorageCountMismatch { expected, actual } => {
                write!(
                    f,
                    "Storage object count: expected {}, got {}",
                    expected, actual
                )
            }
        }
    }
}

impl SnapshotDiff {
    /// Compare two snapshots.
    pub fn compare(expected: &WorkspaceSnapshot, actual: &WorkspaceSnapshot) -> Self {
        let mut mismatches = Vec::new();

        // Compare tracked files
        let expected_paths: BTreeSet<_> = expected.tracked_files.keys().collect();
        let actual_paths: BTreeSet<_> = actual.tracked_files.keys().collect();

        // Files in expected but not actual
        for path in expected_paths.difference(&actual_paths) {
            mismatches.push(Mismatch::MissingTrackedFile {
                path: (*path).clone(),
            });
        }

        // Files in actual but not expected
        for path in actual_paths.difference(&expected_paths) {
            mismatches.push(Mismatch::ExtraTrackedFile {
                path: (*path).clone(),
            });
        }

        // Compare common files
        for path in expected_paths.intersection(&actual_paths) {
            let exp = &expected.tracked_files[*path];
            let act = &actual.tracked_files[*path];

            if exp.checksum != act.checksum {
                mismatches.push(Mismatch::ChecksumMismatch {
                    path: (*path).clone(),
                    expected: exp.checksum.clone(),
                    actual: act.checksum.clone(),
                });
            }

            if exp.storage_exists == ObjectPresence::Present
                && act.storage_exists != ObjectPresence::Present
            {
                mismatches.push(Mismatch::MissingStorageObject {
                    path: (*path).clone(),
                });
            }
        }

        // Compare config presence
        match (expected.config.is_some(), actual.config.is_some()) {
            (true, false) => mismatches.push(Mismatch::MissingConfig),
            (false, true) => mismatches.push(Mismatch::UnexpectedConfig),
            _ => {}
        }

        // Compare gitignore
        if expected.gitignore_has_dvs != actual.gitignore_has_dvs {
            mismatches.push(Mismatch::GitignoreMismatch {
                expected_has_dvs: expected.gitignore_has_dvs,
                actual_has_dvs: actual.gitignore_has_dvs,
            });
        }

        Self { mismatches }
    }

    /// Check if there are no mismatches.
    pub fn is_empty(&self) -> bool {
        self.mismatches.is_empty()
    }

    /// Get the number of mismatches.
    pub fn len(&self) -> usize {
        self.mismatches.len()
    }

    /// Format the diff as a human-readable report.
    pub fn report(&self) -> String {
        if self.is_empty() {
            return "No differences found.".to_string();
        }

        let mut lines = vec![format!("Found {} differences:", self.mismatches.len())];
        for m in &self.mismatches {
            lines.push(format!("  - {}", m));
        }
        lines.join("\n")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::snapshot::FileSnapshot;
    use std::collections::BTreeMap;

    fn empty_snapshot() -> WorkspaceSnapshot {
        WorkspaceSnapshot {
            tracked_files: BTreeMap::new(),
            storage_objects: BTreeSet::new(),
            config: None,
            manifest: None,
            gitignore_has_dvs: false,
        }
    }

    #[test]
    fn test_identical_snapshots() {
        let s1 = empty_snapshot();
        let s2 = empty_snapshot();

        let diff = SnapshotDiff::compare(&s1, &s2);
        assert!(diff.is_empty());
    }

    #[test]
    fn test_missing_tracked_file() {
        let mut s1 = empty_snapshot();
        s1.tracked_files.insert(
            PathBuf::from("data.csv"),
            FileSnapshot {
                checksum: "abc123".to_string(),
                hash_algo: "Blake3".to_string(),
                file_bytes: 100,
                data_exists: true,
                storage_exists: ObjectPresence::Present,
            },
        );

        let s2 = empty_snapshot();

        let diff = SnapshotDiff::compare(&s1, &s2);
        assert_eq!(diff.len(), 1);
        assert!(matches!(
            &diff.mismatches[0],
            Mismatch::MissingTrackedFile { path } if path == &PathBuf::from("data.csv")
        ));
    }

    #[test]
    fn test_checksum_mismatch() {
        let mut s1 = empty_snapshot();
        s1.tracked_files.insert(
            PathBuf::from("data.csv"),
            FileSnapshot {
                checksum: "abc123".to_string(),
                hash_algo: "Blake3".to_string(),
                file_bytes: 100,
                data_exists: true,
                storage_exists: ObjectPresence::Present,
            },
        );

        let mut s2 = empty_snapshot();
        s2.tracked_files.insert(
            PathBuf::from("data.csv"),
            FileSnapshot {
                checksum: "def456".to_string(),
                hash_algo: "Blake3".to_string(),
                file_bytes: 100,
                data_exists: true,
                storage_exists: ObjectPresence::Present,
            },
        );

        let diff = SnapshotDiff::compare(&s1, &s2);
        assert_eq!(diff.len(), 1);
        assert!(matches!(&diff.mismatches[0], Mismatch::ChecksumMismatch { .. }));
    }

    #[test]
    fn test_config_mismatch() {
        let mut s1 = empty_snapshot();
        s1.config = Some(crate::snapshot::ConfigSnapshot {
            storage_dir: PathBuf::from(".storage"),
            hash_algo: None,
            permissions: None,
            group: None,
        });

        let s2 = empty_snapshot();

        let diff = SnapshotDiff::compare(&s1, &s2);
        assert_eq!(diff.len(), 1);
        assert!(matches!(&diff.mismatches[0], Mismatch::MissingConfig));
    }
}
