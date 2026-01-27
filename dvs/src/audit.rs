use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};

use anyhow::Result;
use fs_err::{File, OpenOptions};
use jiff::Zoned;
use serde::{Deserialize, Serialize};

use crate::file::Hashes;
use crate::lock::FileLock;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Operation {
    Add,
    Get,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Outcome {
    Success,
    Failure(String),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuditFile {
    pub path: String,
    pub hashes: Hashes,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuditEntry {
    pub timestamp: String,
    pub operation: Operation,
    pub user: String,
    pub files: Vec<AuditFile>,
    pub outcome: Outcome,
}

impl AuditEntry {
    /// Creates a new audit entry with the current timestamp and user.
    pub fn new(operation: Operation, files: Vec<AuditFile>, outcome: Outcome) -> Self {
        let timestamp = Zoned::now()
            .timestamp()
            .strftime("%Y-%m-%dT%H:%M:%SZ")
            .to_string();
        let user = whoami::username().unwrap_or_else(|_| "unknown".to_string());

        Self {
            timestamp,
            operation,
            user,
            files,
            outcome,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct AuditLog {
    path: PathBuf,
}

impl AuditLog {
    pub fn new(path: impl AsRef<Path>) -> Self {
        Self {
            path: path.as_ref().to_path_buf(),
        }
    }

    pub fn log(&self, entry: &AuditEntry) -> Result<()> {
        let _lock = FileLock::acquire(&self.path)?;
        let mut file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.path)?;

        let json = serde_json::to_string(entry)?;
        writeln!(file, "{}", json)?;
        Ok(())
    }

    pub fn read(&self) -> Result<Vec<AuditEntry>> {
        if !self.path.exists() {
            return Ok(Vec::new());
        }

        let file = File::open(&self.path)?;
        let reader = BufReader::new(file);
        let mut entries = Vec::new();

        for line in reader.lines() {
            let line = line?;
            if line.trim().is_empty() {
                continue;
            }
            let entry: AuditEntry = serde_json::from_str(&line)?;
            entries.push(entry);
        }

        Ok(entries)
    }
}
