use std::path::{Path, PathBuf};

use anyhow::{Result, bail};
use fs_err as fs;
use uuid::Uuid;

use crate::audit::AuditEntry;
use crate::config::{Backend, Config};
use crate::paths;

/// Starts a new dvs project.
/// We need a ready to use Config object + the current directory the user is in
pub fn init(root_dir: impl AsRef<Path>, mut config: Config) -> Result<PathBuf> {
    let root_dir = root_dir.as_ref();

    // The storage directory must not live inside the repository, otherwise it
    // would be tracked alongside the user's files.
    match &mut config.backend {
        Backend::Local(b) => {
            let storage_path = &b.path;
            let abs_storage = if storage_path.exists() {
                fs::canonicalize(storage_path)?
            } else if let Some(parent) = storage_path.parent().filter(|p| p.exists()) {
                fs::canonicalize(parent)?.join(storage_path.file_name().unwrap())
            } else {
                std::path::absolute(storage_path)?
            };
            let abs_root = fs::canonicalize(root_dir)?;
            if abs_storage.starts_with(&abs_root) {
                bail!("The given storage path is within the repository.");
            }
            b.path = abs_storage;
        }
        Backend::Server(_) => { /* nothing to do here */ }
    }

    if root_dir.join(paths::CONFIG_FILE_NAME).exists() {
        bail!("dvs is already initialized (dvs.toml exists)");
    }

    log::debug!("Initializing backend");
    if config.backend().init(config.compression())? {
        bail!("dvs is already initialized (backend storage exists)");
    }

    config.save(root_dir)?;

    let metadata_dir = root_dir.join(config.metadata_folder_name());
    let config_path = root_dir.join(paths::CONFIG_FILE_NAME);

    log::debug!("Creating metadata folder: {}", metadata_dir.display());
    if let Err(e) = fs::create_dir(&metadata_dir) {
        // Roll back the config file we just wrote.
        let _ = fs::remove_file(&config_path);
        return Err(e.into());
    }

    match &config.backend {
        Backend::Local(b) => {
            let audit_entry =
                AuditEntry::new_init(Uuid::new_v4(), config.clone(), root_dir.to_path_buf());
            if let Err(e) = b.log_audit_entry(&audit_entry) {
                log::error!("Failed to write init audit log {audit_entry:?}: {e}");
            }
        }
        Backend::Server(_) => { /* noop */ }
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
        let storage = root.parent().unwrap().join(".storage");

        let config = Config::new_local(&storage, None).unwrap();
        init(&root, config).unwrap();

        // Config file should exist
        assert!(root.join("dvs.toml").is_file());
        // Metadata folder should exist
        assert!(root.join(".dvs").is_dir());
        // Storage folder should exist
        assert!(storage.is_dir());
    }

    #[test]
    fn init_logs_audit_entry() {
        let (_tmp, root) = create_temp_git_repo();
        let storage = root.parent().unwrap().join(".storage");

        let config = Config::new_local(&storage, None).unwrap();
        init(&root, config.clone()).unwrap();

        let entries = config.backend().get_audit_entries(&[]).unwrap();
        assert_eq!(entries.len(), 1);
        assert!(matches!(
            entries[0].action,
            crate::audit::Action::Init { .. }
        ));
    }

    #[test]
    fn init_fails_if_already_initialized() {
        let (_tmp, root) = create_temp_git_repo();
        let storage = root.parent().unwrap().join(".storage");

        let config = Config::new_local(&storage, None).unwrap();
        init(&root, config.clone()).unwrap();

        // Second init should fail because dvs.toml exists
        let result = init(&root, config.clone());
        assert!(result.is_err(), "second init should fail");
    }

    #[test]
    fn init_fails_if_backend_already_initialized() {
        let (_tmp, root) = create_temp_git_repo();
        let storage = root.parent().unwrap().join(".storage");

        let config = Config::new_local(&storage, None).unwrap();
        init(&root, config.clone()).unwrap();

        // Remove dvs.toml and .dvs but leave backend storage intact
        fs::remove_file(root.join("dvs.toml")).unwrap();
        fs::remove_dir_all(root.join(".dvs")).unwrap();

        let result = init(&root, config);
        assert!(
            result.is_err(),
            "should detect backend is already initialized"
        );
    }

    #[test]
    fn init_rejects_storage_inside_repo() {
        let (_tmp, root) = create_temp_git_repo();
        // Storage directory nested inside the repository root.
        let storage = root.join("inside").join(".storage");

        let config = Config::new_local(&storage, None).unwrap();
        let result = init(&root, config);
        assert!(
            result.is_err(),
            "init should reject storage inside the repository"
        );
        assert!(
            result
                .unwrap_err()
                .to_string()
                .contains("within the repository"),
            "error should explain the storage path is inside the repo"
        );
        // Nothing should have been created.
        assert!(!root.join("dvs.toml").exists());
    }

    #[test]
    fn init_succeeds_in_subdirectory_of_initialized_project() {
        let (_tmp, root) = create_temp_git_repo();
        let storage = root.parent().unwrap().join(".storage");

        // Initialize the parent project
        let config = Config::new_local(&storage, None).unwrap();
        init(&root, config).unwrap();

        // Create a subdirectory and initialize a nested project there
        let subdir = root.join("nested");
        fs::create_dir(&subdir).unwrap();
        let nested_storage = root.parent().unwrap().join(".nested-storage");
        let nested_config = Config::new_local(&nested_storage, None).unwrap();
        let result = init(&subdir, nested_config);
        assert!(
            result.is_ok(),
            "init in subdirectory should succeed: {result:?}"
        );
        assert!(subdir.join("dvs.toml").is_file());
        assert!(subdir.join(".dvs").is_dir());
    }

    #[cfg(unix)]
    #[test]
    fn init_cleans_up_on_backend_failure() {
        let (_tmp, root) = create_temp_git_repo();
        // Point storage at an impossible path so backend init fails
        let storage = Path::new("/dev/null/impossible");

        let config = Config::new_local(storage, None).unwrap();
        let result = init(&root, config);
        assert!(result.is_err());

        // Local artifacts should have been cleaned up
        assert!(
            !root.join("dvs.toml").exists(),
            "dvs.toml should be cleaned up"
        );
        assert!(!root.join(".dvs").exists(), ".dvs should be cleaned up");

        // A retry with a valid storage path should now succeed
        let valid_storage = root.parent().unwrap().join(".storage");
        let config = Config::new_local(&valid_storage, None).unwrap();
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
        let config = Config::new_local(storage, None).unwrap();
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
