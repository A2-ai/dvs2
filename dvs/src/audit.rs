use std::collections::HashSet;
use std::io::BufRead;
use std::path::PathBuf;

use crate::Hashes;
use anyhow::Result;
use jiff::Timestamp;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[derive(Debug, Copy, Clone, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Action {
    Add,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuditFile {
    pub path: PathBuf,
    pub hashes: Hashes,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuditEntry {
    pub operation_id: String,
    pub timestamp: i64,
    pub user: String,
    pub file: AuditFile,
    pub action: Action,
}

impl AuditEntry {
    pub fn new_add(operation_id: Uuid, file: AuditFile) -> Self {
        let timestamp = Timestamp::now().as_second();
        let user = whoami::username().unwrap_or_else(|_| "unknown".to_string());

        Self {
            operation_id: operation_id.to_string(),
            timestamp,
            user,
            file,
            action: Action::Add,
        }
    }
}

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
            Ok(e) => only_files.is_empty() || only_files.contains(&e.file.path),
            Err(_) => true, // propagate errors
        })
        .collect()
}
