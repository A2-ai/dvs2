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
/// An empty `only_files` set returns every entry, that is the whole log. A
/// non-empty set keeps only `Add` entries whose path is in the set and drops
/// all `Init` entries.
pub fn parse_audit_log(
    reader: impl BufRead,
    only_files: &HashSet<PathBuf>,
) -> Result<Vec<AuditEntry>> {
    reader
        .lines()
        .map(|line| line.map_err(anyhow::Error::from))
        .filter_map(|line| match line {
            Ok(l) if l.trim().is_empty() => None,
            other => Some(other),
        })
        .map(|line| Ok(serde_json::from_str::<AuditEntry>(&line?)?))
        .filter(|entry| match entry {
            Ok(e) => {
                if only_files.is_empty() {
                    true
                } else {
                    match &e.action {
                        Action::Add { file, .. } => only_files.contains(&file.path),
                        Action::Init { .. } => false,
                    }
                }
            }
            Err(_) => true, // propagate errors
        })
        .collect()
}
