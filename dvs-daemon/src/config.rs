//! Daemon configuration.

use std::path::PathBuf;
use crate::DaemonError;

/// Configuration for the DVS daemon.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct DaemonConfig {
    /// Repository root path.
    pub repo_root: PathBuf,
    /// Socket path for IPC.
    pub socket_path: PathBuf,
    /// Paths to watch for changes.
    pub watch_paths: Vec<PathBuf>,
    /// Debounce delay in milliseconds.
    pub debounce_ms: u64,
    /// Whether to auto-add new files matching patterns.
    pub auto_add: bool,
    /// Glob patterns for auto-add.
    pub auto_add_patterns: Vec<String>,
    /// Whether to auto-sync modified tracked files.
    pub auto_sync: bool,
    /// Log level.
    pub log_level: String,
}

impl Default for DaemonConfig {
    fn default() -> Self {
        Self {
            repo_root: PathBuf::new(),
            socket_path: PathBuf::new(),
            watch_paths: vec![],
            debounce_ms: 500,
            auto_add: false,
            auto_add_patterns: vec![],
            auto_sync: true,
            log_level: "info".to_string(),
        }
    }
}

impl DaemonConfig {
    /// Load configuration from file.
    pub fn load(_path: &std::path::Path) -> Result<Self, DaemonError> {
        todo!("Load daemon config from file")
    }

    /// Save configuration to file.
    pub fn save(&self, _path: &std::path::Path) -> Result<(), DaemonError> {
        todo!("Save daemon config to file")
    }

    /// Validate the configuration.
    pub fn validate(&self) -> Result<(), DaemonError> {
        todo!("Validate daemon config")
    }
}
