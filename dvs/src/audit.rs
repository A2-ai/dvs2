use std::path::PathBuf;

use crate::Hashes;
use anyhow::Result;
use jiff::Timestamp;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

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
}

impl AuditEntry {
    pub fn new(operation_id: Uuid, file: AuditFile) -> Self {
        let timestamp = Timestamp::now().as_second();
        let user = whoami::username().unwrap_or_else(|_| "unknown".to_string());

        Self {
            operation_id: operation_id.to_string(),
            timestamp,
            user,
            file,
        }
    }
}

pub fn parse_audit_log(bytes: &[u8]) -> Result<Vec<AuditEntry>> {
    let content = std::str::from_utf8(bytes)?;
    content
        .lines()
        .filter(|line| !line.trim().is_empty())
        .map(|line| Ok(serde_json::from_str(line)?))
        .collect()
}

impl std::fmt::Display for AuditEntry {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let AuditEntry {
            operation_id: _,
            timestamp,
            user,
            file: _audit_file,
        } = self;
        let timestamp =
            Timestamp::from_second(*timestamp).expect("timestamp found in audit log is invalid");
        write!(f, "{}", timestamp)?;
        write!(f, "{}", user)?;

        Ok(())
    }
}
