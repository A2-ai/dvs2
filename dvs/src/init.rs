use std::path::{Path, PathBuf};

use anyhow::{Result, bail};
use fs_err as fs;

use crate::config::Config;
use crate::paths;

/// Starts a new dvs project.
/// We need a ready to use Config object + the current directory the user is in
pub fn init(root_dir: impl AsRef<Path>, config: Config) -> Result<PathBuf> {
    let root_dir = root_dir.as_ref();
    if root_dir.join(paths::CONFIG_FILE_NAME).exists() {
        bail!("Configuration already exists in {}", root_dir.display());
    }
    config.save(root_dir)?;

    let metadata_dir = root_dir.join(config.metadata_folder_name());
    let config_path = root_dir.join(paths::CONFIG_FILE_NAME);
    let metadata_existed = metadata_dir.exists();

    let result = (|| {
        log::debug!("Creating metadata folder: {}", metadata_dir.display());
        fs::create_dir(&metadata_dir)?;
        log::debug!("Initializing backend");
        config.backend().init()
    })();

    if let Err(e) = result {
        // Best-effort cleanup of local artifacts we created
        if !metadata_existed {
            let _ = fs::remove_dir_all(&metadata_dir);
        }
        let _ = fs::remove_file(&config_path);
        return Err(e.into());
    }

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

    #[test]
    fn init_succeeds_in_subdirectory_of_initialized_project() {
        let (_tmp, root) = create_temp_git_repo();
        let storage = root.join(".storage");

        // Initialize the parent project
        let config = Config::new_local(&storage, None, None).unwrap();
        init(&root, config).unwrap();

        // Create a subdirectory and initialize a nested project there
        let subdir = root.join("nested");
        fs::create_dir(&subdir).unwrap();
        let nested_storage = subdir.join(".storage");
        let nested_config = Config::new_local(&nested_storage, None, None).unwrap();
        let result = init(&subdir, nested_config);
        assert!(
            result.is_ok(),
            "init in subdirectory should succeed: {result:?}"
        );
        assert!(subdir.join("dvs.toml").is_file());
        assert!(subdir.join(".dvs").is_dir());
    }

    #[test]
    fn init_cleans_up_on_backend_failure() {
        let (_tmp, root) = create_temp_git_repo();
        // Point storage at an impossible path so backend init fails
        let storage = Path::new("/dev/null/impossible");

        let config = Config::new_local(storage, None, None).unwrap();
        let result = init(&root, config);
        assert!(result.is_err());

        // Local artifacts should have been cleaned up
        assert!(
            !root.join("dvs.toml").exists(),
            "dvs.toml should be cleaned up"
        );
        assert!(!root.join(".dvs").exists(), ".dvs should be cleaned up");

        // A retry with a valid storage path should now succeed
        let valid_storage = root.join(".storage");
        let config = Config::new_local(&valid_storage, None, None).unwrap();
        init(&root, config).unwrap();
        assert!(root.join("dvs.toml").is_file());
        assert!(root.join(".dvs").is_dir());
    }

    #[test]
    fn init_preserves_preexisting_metadata_dir_on_failure() {
        let (_tmp, root) = create_temp_git_repo();
        // Create a .dvs directory before init runs
        let metadata_dir = root.join(".dvs");
        fs::create_dir(&metadata_dir).unwrap();
        let marker = metadata_dir.join("marker.txt");
        fs::write(&marker, "keep me").unwrap();

        let storage = Path::new("/dev/null/impossible");
        let config = Config::new_local(storage, None, None).unwrap();
        let result = init(&root, config);
        assert!(result.is_err());

        // dvs.toml should be cleaned up
        assert!(
            !root.join("dvs.toml").exists(),
            "dvs.toml should be cleaned up"
        );
        // Pre-existing .dvs directory and its contents should be preserved
        assert!(metadata_dir.is_dir(), ".dvs should be preserved");
        assert!(
            marker.is_file(),
            "marker file inside .dvs should be preserved"
        );
    }
}
