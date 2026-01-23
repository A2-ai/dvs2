use std::path::{Path, PathBuf};

use anyhow::Result;
use fs_err as fs;
use serde::{Deserialize, Serialize};

const CONFIG_FILE_NAME: &str = "dvs.toml";
const DEFAULT_FOLDER_NAME: &str = ".dvs";

/// Finds the root of a git repository by walking up from the given directory
/// until a `.git` folder is found.
///
/// Returns `None` if no `.git` folder is found before reaching the filesystem root.
pub fn find_repo_root(start_dir: impl AsRef<Path>) -> Option<PathBuf> {
    let mut dir = start_dir.as_ref();

    loop {
        if dir.join(".git").exists() {
            return Some(dir.to_path_buf());
        }

        dir = dir.parent()?;
    }
}

#[derive(Debug, Serialize, Deserialize, Clone, PartialEq)]
pub struct LocalBackend {
    pub path: PathBuf,
    permissions: Option<String>,
    group: Option<String>,
}

#[derive(Debug, Serialize, Deserialize, Clone, PartialEq)]
#[serde(untagged)]
pub enum Backend {
    Local(LocalBackend),
}

#[derive(Debug, Serialize, Deserialize, Clone, PartialEq)]
pub struct Config {
    metadata_folder_name: Option<String>,
    backend: Backend,
}

impl Config {
    pub fn new_local(path: impl AsRef<Path>) -> Config {
        let backend = Backend::Local(LocalBackend {
            path: path.as_ref().to_path_buf(),
            permissions: None,
            group: None,
        });
        Config {
            metadata_folder_name: None,
            backend,
        }
    }

    pub fn save(&self, directory: impl AsRef<Path>) -> Result<()> {
        let config_path = directory.as_ref().join(CONFIG_FILE_NAME);
        let content = toml::to_string_pretty(&self)?;
        fs::write(config_path, content)?;
        Ok(())
    }

    pub fn find(current_directory: impl AsRef<Path>) -> Option<Result<Self>> {
        let repo_root = find_repo_root(current_directory)?;
        let config_path = repo_root.join(CONFIG_FILE_NAME);
        if config_path.exists() {
            let content = match fs::read_to_string(&config_path) {
                Ok(c) => c,
                Err(e) => return Some(Err(e.into())),
            };
            Some(toml::from_str(&content).map_err(|e| e.into()))
        } else {
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
            DEFAULT_FOLDER_NAME
        }
    }

    pub fn backend(&self) -> &Backend {
        &self.backend
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testutil::create_temp_git_repo;

    #[test]
    fn find_repo_root_at_root() {
        let (_tmp, root) = create_temp_git_repo();
        assert_eq!(find_repo_root(&root), Some(root));
    }

    #[test]
    fn find_repo_root_from_subdirectory() {
        let (_tmp, root) = create_temp_git_repo();
        let subdir = root.join("a/b/c");
        fs::create_dir_all(&subdir).unwrap();
        assert_eq!(find_repo_root(&subdir), Some(root));
    }

    #[test]
    fn find_repo_root_returns_none_without_git() {
        let tmp = tempfile::tempdir().unwrap();
        assert_eq!(find_repo_root(tmp.path()), None);
    }

    #[test]
    fn config_save_and_find_roundtrip() {
        let (_tmp, root) = create_temp_git_repo();
        let storage = root.join(".storage");

        let original = Config::new_local(&storage);
        original.save(&root).unwrap();

        let loaded = Config::find(&root).unwrap().unwrap();
        assert_eq!(original, loaded);
    }

    #[test]
    fn config_find_returns_none_without_config_file() {
        let (_tmp, root) = create_temp_git_repo();
        assert!(Config::find(&root).is_none());
    }
}
