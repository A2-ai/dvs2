use std::path::Path;

use crate::backends::Backend as BackendTrait;
use crate::backends::local::LocalBackend;
use crate::paths::{CONFIG_FILE_NAME, DEFAULT_METADATA_FOLDER_NAME, find_repo_root};
use anyhow::{Context, Result};
use fs_err as fs;
use serde::{Deserialize, Serialize};

#[derive(Debug, Serialize, Deserialize, Clone, PartialEq)]
#[serde(untagged)]
pub enum Backend {
    Local(LocalBackend),
}

#[derive(Debug, Serialize, Deserialize, Clone, PartialEq)]
pub struct Config {
    /// By default, all the metadata files (the .dvs files) will be stored in a `.dvs` folder
    /// at the root of the repository
    /// If this option is set, dvs will use that folder name instead of `.dvs`
    metadata_folder_name: Option<String>,
    backend: Backend,
}

impl Config {
    pub fn new_local(
        path: impl AsRef<Path>,
        permissions: Option<String>,
        group: Option<String>,
    ) -> Result<Config> {
        let backend = LocalBackend::new(path.as_ref(), permissions, group)?;
        Ok(Config {
            metadata_folder_name: None,
            backend: Backend::Local(backend),
        })
    }

    pub fn save(&self, directory: impl AsRef<Path>) -> Result<()> {
        let config_path = directory.as_ref().join(CONFIG_FILE_NAME);
        let content = toml::to_string_pretty(&self)?;
        fs::write(&config_path, content)?;
        log::info!("Configuration saved to {}", config_path.display());
        Ok(())
    }

    pub fn find(current_directory: impl AsRef<Path>) -> Option<Result<Self>> {
        let repo_root = find_repo_root(current_directory)?;
        let config_path = repo_root.join(CONFIG_FILE_NAME);
        log::debug!("Looking for config at {}", config_path.display());
        if config_path.exists() {
            let content = match fs::read_to_string(&config_path) {
                Ok(c) => c,
                Err(e) => return Some(Err(e.into())),
            };
            Some(
                toml::from_str(&content)
                    .with_context(|| format!("Failed to parse {}", config_path.display())),
            )
        } else {
            log::debug!("No config file found at {}", config_path.display());
            None
        }
    }

    pub fn set_metadata_folder_name(&mut self, name: String) {
        self.metadata_folder_name = Some(name);
    }

    pub fn metadata_folder_name(&self) -> &str {
        if let Some(name) = &self.metadata_folder_name {
            name.as_str()
        } else {
            DEFAULT_METADATA_FOLDER_NAME
        }
    }

    pub fn backend(&self) -> &dyn BackendTrait {
        match &self.backend {
            Backend::Local(b) => b,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testutil::create_temp_git_repo;

    #[test]
    fn config_save_and_find_roundtrip() {
        let (_tmp, root) = create_temp_git_repo();
        let storage = root.join(".storage");

        let original = Config::new_local(&storage, None, None).unwrap();
        original.save(&root).unwrap();

        let loaded = Config::find(&root).unwrap().unwrap();
        assert_eq!(original, loaded);
    }

    #[test]
    fn config_find_returns_none_without_config_file() {
        let (_tmp, root) = create_temp_git_repo();
        assert!(Config::find(&root).is_none());
    }

    #[test]
    fn new_local_validates_permissions_format() {
        let tmp = tempfile::tempdir().unwrap();
        let storage = tmp.path().join(".storage");

        // Valid octal permissions should work
        assert!(Config::new_local(&storage, Some("755".to_string()), None).is_ok());
        assert!(Config::new_local(&storage, Some("0755".to_string()), None).is_ok());
        assert!(Config::new_local(&storage, Some("777".to_string()), None).is_ok());

        // Invalid permissions should fail
        let result = Config::new_local(&storage, Some("999".to_string()), None);
        assert!(result.is_err());
        assert!(
            result
                .unwrap_err()
                .to_string()
                .contains("Invalid permission mode")
        );

        let result = Config::new_local(&storage, Some("abc".to_string()), None);
        assert!(result.is_err());
        assert!(
            result
                .unwrap_err()
                .to_string()
                .contains("Invalid permission mode")
        );
    }

    #[test]
    fn new_local_validates_group_exists() {
        let tmp = tempfile::tempdir().unwrap();
        let storage = tmp.path().join(".storage");

        // Non-existent group should fail
        let result = Config::new_local(&storage, None, Some("nonexistent_group_12345".to_string()));
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("not found"));
    }

    #[test]
    fn config_with_custom_metadata_folder() {
        let (_tmp, root) = create_temp_git_repo();
        let storage = root.join(".storage");

        let mut config = Config::new_local(&storage, None, None).unwrap();
        config.set_metadata_folder_name(".custom_dvs".to_string());
        config.save(&root).unwrap();

        let loaded = Config::find(&root).unwrap().unwrap();
        assert_eq!(loaded.metadata_folder_name(), ".custom_dvs");
    }
}
