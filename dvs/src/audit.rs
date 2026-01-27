use std::path::PathBuf;

use crate::Hashes;
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
