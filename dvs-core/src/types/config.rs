//! DVS configuration types.

use std::path::PathBuf;
use serde::{Deserialize, Serialize};

/// DVS project configuration, stored in `dvs.yaml`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Config {
    /// Path to the external storage directory.
    pub storage_dir: PathBuf,

    /// Optional file permissions (octal, e.g., 0o664).
    #[serde(default)]
    pub permissions: Option<u32>,

    /// Optional Linux group for stored files.
    #[serde(default)]
    pub group: Option<String>,
}

impl Config {
    /// Create a new configuration.
    pub fn new(storage_dir: PathBuf, permissions: Option<u32>, group: Option<String>) -> Self {
        Self {
            storage_dir,
            permissions,
            group,
        }
    }

    /// Load configuration from a YAML file.
    pub fn load(path: &std::path::Path) -> Result<Self, crate::DvsError> {
        todo!("Load config from YAML file")
    }

    /// Save configuration to a YAML file.
    pub fn save(&self, path: &std::path::Path) -> Result<(), crate::DvsError> {
        todo!("Save config to YAML file")
    }

    /// Get the default config file name.
    pub const fn config_filename() -> &'static str {
        "dvs.yaml"
    }
}
