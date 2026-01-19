//! DVS configuration types.

use fs_err as fs;
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
        let contents = fs::read_to_string(path)?;
        let config: Config = serde_yaml::from_str(&contents)?;
        Ok(config)
    }

    /// Save configuration to a YAML file.
    pub fn save(&self, path: &std::path::Path) -> Result<(), crate::DvsError> {
        let yaml = serde_yaml::to_string(self)?;
        fs::write(path, yaml)?;
        Ok(())
    }

    /// Get the default config file name.
    pub const fn config_filename() -> &'static str {
        "dvs.yaml"
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_config_new() {
        let config = Config::new(
            PathBuf::from("/storage"),
            Some(0o664),
            Some("data".to_string()),
        );
        assert_eq!(config.storage_dir, PathBuf::from("/storage"));
        assert_eq!(config.permissions, Some(0o664));
        assert_eq!(config.group, Some("data".to_string()));
    }

    #[test]
    fn test_config_roundtrip() {
        let temp_dir = std::env::temp_dir().join("dvs-test-config-roundtrip");
        let _ = fs::create_dir_all(&temp_dir);

        let config_path = temp_dir.join("dvs.yaml");
        let config = Config::new(
            PathBuf::from("/my/storage"),
            Some(0o660),
            Some("mygroup".to_string()),
        );

        // Save
        config.save(&config_path).unwrap();

        // Load
        let loaded = Config::load(&config_path).unwrap();
        assert_eq!(loaded.storage_dir, config.storage_dir);
        assert_eq!(loaded.permissions, config.permissions);
        assert_eq!(loaded.group, config.group);

        // Cleanup
        let _ = fs::remove_dir_all(&temp_dir);
    }

    #[test]
    fn test_config_minimal() {
        let temp_dir = std::env::temp_dir().join("dvs-test-config-minimal");
        let _ = fs::create_dir_all(&temp_dir);

        let config_path = temp_dir.join("dvs.yaml");

        // Write minimal YAML
        fs::write(&config_path, "storage_dir: /storage\n").unwrap();

        // Load
        let loaded = Config::load(&config_path).unwrap();
        assert_eq!(loaded.storage_dir, PathBuf::from("/storage"));
        assert!(loaded.permissions.is_none());
        assert!(loaded.group.is_none());

        // Cleanup
        let _ = fs::remove_dir_all(&temp_dir);
    }
}
