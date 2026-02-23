use std::path::{Path, PathBuf};

use anyhow::{Result, bail};
use fs_err as fs;

use crate::config::Config;

/// Starts a new dvs project.
/// We need a ready to use Config object + the current directory the user is in
pub fn init(root_dir: impl AsRef<Path>, config: Config) -> Result<PathBuf> {
    let root_dir = root_dir.as_ref();
    if Config::find(&root_dir).is_some() {
        bail!("Configuration already exists in {}", root_dir.display());
    }
    config.save(&root_dir)?;
    log::debug!(
        "Creating metadata folder: {}",
        root_dir.join(config.metadata_folder_name()).display()
    );
    fs::create_dir(root_dir.join(config.metadata_folder_name()))?;
    log::debug!("Initializing backend");
    config.backend().init()?;
    log::info!("DVS repository initialized successfully");
    Ok(root_dir.to_path_buf())
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
}
