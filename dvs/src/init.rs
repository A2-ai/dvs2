use std::path::Path;

use anyhow::{Result, anyhow, bail};
use fs_err as fs;

use crate::config::Config;
use crate::paths::find_repo_root;

/// Starts a new dvs project.
/// We need a ready to use Config object + the current directory the user is in
/// The library handles finding where to create the config file and metadata folder
pub fn init(current_dir: impl AsRef<Path>, config: Config) -> Result<()> {
    if Config::find(&current_dir).is_some() {
        bail!(
            "Configuration already exists in {}",
            current_dir.as_ref().display()
        );
    }
    let repo_root =
        find_repo_root(&current_dir).ok_or_else(|| anyhow!("Cannot find repository root"))?;
    config.save(&repo_root)?;
    log::debug!(
        "Creating metadata folder: {}",
        repo_root.join(config.metadata_folder_name()).display()
    );
    fs::create_dir(repo_root.join(config.metadata_folder_name()))?;
    log::debug!("Initializing backend");
    config.backend().init()?;
    log::info!("DVS repository initialized successfully");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testutil::create_temp_git_repo;

    #[test]
    fn init_creates_config_and_directories() {
        let (_tmp, root) = create_temp_git_repo();
        let storage = root.join(".storage");

        let config = Config::new_local(&storage, None, None).unwrap();
        init(&root, config).unwrap();

        // Config file should exist
        assert!(root.join("dvs.toml").is_file());
        // Metadata folder should exist
        assert!(root.join(".dvs").is_dir());
        // Storage folder should exist
        assert!(storage.is_dir());
    }

    #[test]
    fn init_fails_if_already_initialized() {
        let (_tmp, root) = create_temp_git_repo();
        let storage = root.join(".storage");

        let config = Config::new_local(&storage, None, None).unwrap();
        init(&root, config.clone()).unwrap();

        // Second init should fail
        let result = init(&root, config);
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("already exists"));
    }

    #[test]
    fn init_fails_without_git_repo() {
        let tmp = tempfile::tempdir().unwrap();
        let storage = tmp.path().join(".storage");

        let config = Config::new_local(&storage, None, None).unwrap();
        let result = init(tmp.path(), config);

        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("repository root"));
    }

    #[test]
    fn init_from_subdirectory_creates_at_repo_root() {
        let (_tmp, root) = create_temp_git_repo();
        let subdir = root.join("nested/deep");
        fs::create_dir_all(&subdir).unwrap();
        let storage = root.join(".storage");

        let config = Config::new_local(&storage, None, None).unwrap();
        init(&subdir, config).unwrap();

        // Config should be at repo root, not in subdirectory
        assert!(root.join("dvs.toml").is_file());
        assert!(!subdir.join("dvs.toml").exists());
    }
}
