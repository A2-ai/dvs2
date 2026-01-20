//! DVS configuration types.

use fs_err as fs;
use std::path::PathBuf;
use serde::{Deserialize, Serialize};
use crate::HashAlgo;

/// DVS project configuration.
///
/// Stored in one of:
/// - `dvs.yaml` (with `yaml-config` feature, default)
/// - `dvs.toml` (with `toml-config` feature, without `yaml-config`)
/// - `dvs.json` (fallback when neither feature is enabled)
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

    /// Hash algorithm for content addressing.
    /// Defaults to BLAKE3 if not specified.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hash_algo: Option<HashAlgo>,
}

impl Config {
    /// Create a new configuration.
    pub fn new(storage_dir: PathBuf, permissions: Option<u32>, group: Option<String>) -> Self {
        Self {
            storage_dir,
            permissions,
            group,
            hash_algo: None,
        }
    }

    /// Create a new configuration with a specific hash algorithm.
    pub fn with_hash_algo(
        storage_dir: PathBuf,
        permissions: Option<u32>,
        group: Option<String>,
        hash_algo: HashAlgo,
    ) -> Self {
        Self {
            storage_dir,
            permissions,
            group,
            hash_algo: Some(hash_algo),
        }
    }

    /// Get the configured hash algorithm, or the default.
    pub fn hash_algorithm(&self) -> HashAlgo {
        self.hash_algo.unwrap_or_else(crate::helpers::hash::default_algorithm)
    }

    /// Load configuration from a directory.
    ///
    /// Looks for the config file (dvs.yaml, dvs.toml, or dvs.json depending
    /// on feature flags) in the given directory.
    pub fn load_from_dir(dir: &std::path::Path) -> Result<Self, crate::DvsError> {
        let config_path = dir.join(Self::config_filename());
        Self::load(&config_path)
    }

    /// Load configuration from a file.
    ///
    /// Format is determined by feature flags:
    /// - `yaml-config`: YAML format (default)
    /// - `toml-config` (without yaml-config): TOML format
    /// - Neither: JSON format (fallback)
    pub fn load(path: &std::path::Path) -> Result<Self, crate::DvsError> {
        let contents = fs::read_to_string(path)?;

        #[cfg(feature = "yaml-config")]
        {
            let config: Config = serde_yaml::from_str(&contents)?;
            Ok(config)
        }

        #[cfg(all(feature = "toml-config", not(feature = "yaml-config")))]
        {
            let config: Config = toml::from_str(&contents)?;
            Ok(config)
        }

        #[cfg(all(not(feature = "yaml-config"), not(feature = "toml-config")))]
        {
            let config: Config = serde_json::from_str(&contents)?;
            Ok(config)
        }
    }

    /// Save configuration to a file.
    ///
    /// Format is determined by feature flags:
    /// - `yaml-config`: YAML format (default)
    /// - `toml-config` (without yaml-config): TOML format
    /// - Neither: JSON format (fallback)
    pub fn save(&self, path: &std::path::Path) -> Result<(), crate::DvsError> {
        #[cfg(feature = "yaml-config")]
        {
            let yaml = serde_yaml::to_string(self)?;
            fs::write(path, yaml)?;
        }

        #[cfg(all(feature = "toml-config", not(feature = "yaml-config")))]
        {
            let toml_str = toml::to_string_pretty(self)?;
            fs::write(path, toml_str)?;
        }

        #[cfg(all(not(feature = "yaml-config"), not(feature = "toml-config")))]
        {
            let json = serde_json::to_string_pretty(self)?;
            fs::write(path, json)?;
        }

        Ok(())
    }

    /// Get the default config file name.
    ///
    /// Returns filename based on feature flags:
    /// - `yaml-config`: "dvs.yaml" (default)
    /// - `toml-config` (without yaml-config): "dvs.toml"
    /// - Neither: "dvs.json" (fallback)
    pub const fn config_filename() -> &'static str {
        #[cfg(feature = "yaml-config")]
        {
            "dvs.yaml"
        }

        #[cfg(all(feature = "toml-config", not(feature = "yaml-config")))]
        {
            "dvs.toml"
        }

        #[cfg(all(not(feature = "yaml-config"), not(feature = "toml-config")))]
        {
            "dvs.json"
        }
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

        let config_path = temp_dir.join(Config::config_filename());
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

        let config_path = temp_dir.join(Config::config_filename());

        // Write minimal config in appropriate format
        #[cfg(feature = "yaml-config")]
        fs::write(&config_path, "storage_dir: /storage\n").unwrap();

        #[cfg(all(feature = "toml-config", not(feature = "yaml-config")))]
        fs::write(&config_path, "storage_dir = \"/storage\"\n").unwrap();

        #[cfg(all(not(feature = "yaml-config"), not(feature = "toml-config")))]
        fs::write(&config_path, r#"{"storage_dir": "/storage"}"#).unwrap();

        // Load
        let loaded = Config::load(&config_path).unwrap();
        assert_eq!(loaded.storage_dir, PathBuf::from("/storage"));
        assert!(loaded.permissions.is_none());
        assert!(loaded.group.is_none());

        // Cleanup
        let _ = fs::remove_dir_all(&temp_dir);
    }
}
