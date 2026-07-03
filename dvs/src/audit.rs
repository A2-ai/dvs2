use std::collections::HashSet;
use std::io::BufRead;
use std::path::PathBuf;

use crate::Hashes;
use crate::config::{Compression, Config};
use anyhow::Result;
use jiff::Timestamp;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Action {
    Add {
        file: AuditFile,
        compression: Compression,
    },
    Init {
        settings: Config,
        project_path: PathBuf,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuditFile {
    pub path: PathBuf,
    pub hashes: Hashes,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuditEntry {
    pub operation_id: String,
    #[serde(with = "jiff::fmt::serde::timestamp::second::required")]
    pub timestamp: Timestamp,
    pub user: String,
    pub action: Action,
}

impl AuditEntry {
    pub fn new_add(operation_id: Uuid, file: AuditFile, compression: Compression) -> Self {
        let timestamp = Timestamp::now();
        let user = whoami::username().unwrap_or_else(|_| "unknown".to_string());

        Self {
            operation_id: operation_id.to_string(),
            timestamp,
            user,
            action: Action::Add { file, compression },
        }
    }

    pub fn new_init(operation_id: Uuid, config: Config, project_path: PathBuf) -> Self {
        let timestamp = Timestamp::now();
        let user = whoami::username().unwrap_or_else(|_| "unknown".to_string());
        let project_path = project_path.canonicalize().unwrap_or(project_path);

        Self {
            operation_id: operation_id.to_string(),
            timestamp,
            user,
            action: Action::Init {
                settings: config,
                project_path,
            },
        }
    }
}

/// Parse an audit log into entries, optionally filtered to specific files.
///
/// Lines that cannot be decoded as an [`AuditEntry`] are skipped with a
/// warning instead of failing the read. IO errors still propagate.
///
/// An empty `only_files` set returns every entry, that is the whole log. A
/// non-empty set keeps only `Add` entries whose path is in the set and drops
/// all `Init` entries.
pub fn parse_audit_log(
    reader: impl BufRead,
    only_files: &HashSet<PathBuf>,
) -> Result<Vec<AuditEntry>> {
    let mut entries = Vec::new();
    for (idx, line) in reader.lines().enumerate() {
        let line = line?;
        if line.trim().is_empty() {
            continue;
        }
        let entry = match serde_json::from_str::<AuditEntry>(&line) {
            Ok(entry) => entry,
            Err(e) => {
                log::warn!("Skipping undecodable audit log line {}: {e}", idx + 1);
                continue;
            }
        };
        let keep = if only_files.is_empty() {
            true
        } else {
            match &entry.action {
                Action::Add { file, .. } => only_files.contains(&file.path),
                Action::Init { .. } => false,
            }
        };
        if keep {
            entries.push(entry);
        }
    }
    Ok(entries)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    fn entry_json(operation_id: &str, path: &str) -> String {
        serde_json::to_string(&AuditEntry {
            operation_id: operation_id.to_string(),
            timestamp: Timestamp::from_second(1000000000).unwrap(),
            user: "alice".to_string(),
            action: Action::Add {
                file: AuditFile {
                    path: PathBuf::from(path),
                    hashes: Hashes {
                        blake3: "abc123def456789012345678901234ab".to_string(),
                        md5: None,
                    },
                },
                compression: Compression::Zstd,
            },
        })
        .unwrap()
    }

    #[test]
    fn parse_audit_log_skips_undecodable_lines() {
        let valid1 = entry_json("op-1", "file1.txt");
        let valid2 = entry_json("op-2", "file2.txt");
        // A torn line, e.g. a concurrent append truncated mid-entry
        let torn = &valid1[..valid1.len() / 2];
        let log = format!("{valid1}\n{torn}\nnot json at all\n{valid2}\n");

        let entries = parse_audit_log(Cursor::new(log), &HashSet::new()).unwrap();
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].operation_id, "op-1");
        assert_eq!(entries[1].operation_id, "op-2");
    }

    #[test]
    fn parse_audit_log_skips_undecodable_lines_with_filter() {
        let valid1 = entry_json("op-1", "file1.txt");
        let valid2 = entry_json("op-2", "file2.txt");
        let log = format!("{valid1}\n{{\"garbage\":\n{valid2}\n");

        let only_files = HashSet::from([PathBuf::from("file2.txt")]);
        let entries = parse_audit_log(Cursor::new(log), &only_files).unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].operation_id, "op-2");
    }
}
