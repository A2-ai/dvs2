use std::path::{Path, PathBuf};

use anyhow::{Result, bail};
use fs_err as fs;
use uuid::Uuid;

use crate::audit::{Action, AuditEntry};
use crate::backends::Backend;
use crate::backends::local::{project_name_from_folder, validate_project_name};
use crate::config::Config;
use crate::paths;

/// A project of that name is in the storage directory, and the name was not
/// given: it came from the folder, so joining it may be an accident.
#[derive(Debug)]
pub struct ProjectExists {
    pub name: String,
    pub storage: PathBuf,
}

impl std::fmt::Display for ProjectExists {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "a project named \"{}\" already exists in {}. Give its name to join it, \
             or another name to start a new project",
            self.name,
            self.storage.display()
        )
    }
}

impl std::error::Error for ProjectExists {}

/// What `init` did.
#[derive(Debug, Clone, PartialEq)]
pub struct Initialized {
    pub root: PathBuf,
    /// The project in the storage directory, `None` on a server
    pub project: Option<String>,
    /// The project was there already, and this project joined it
    pub joined: bool,
}

/// Creates the metadata folder of the project in `root_dir`, if it doesn't exist yet.
/// Both `init` and `attach` need it: the commands that track files expect it.
pub fn create_metadata_folder(root_dir: impl AsRef<Path>, config: &Config) -> Result<()> {
    let metadata_dir = root_dir.as_ref().join(config.metadata_folder_name());
    log::debug!("Creating metadata folder: {}", metadata_dir.display());
    if let Err(e) = fs::create_dir(&metadata_dir)
        && e.kind() != std::io::ErrorKind::AlreadyExists
    {
        return Err(e.into());
    }
    Ok(())
}

/// Starts a new dvs project.
/// We need a ready to use Config object + the current directory the user is in
///
/// A local project goes in its own folder of the storage directory. Without a
/// name, it takes the name of `root_dir`'s folder. A given name joins the
/// project of that name when it exists, a name taken from the folder refuses.
pub fn init(root_dir: impl AsRef<Path>, mut config: Config) -> Result<Initialized> {
    let root_dir = root_dir.as_ref();
    let mut name_given = false;

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

            let name = match b.name() {
                Some(name) => {
                    validate_project_name(name)?;
                    name_given = true;
                    name.to_string()
                }
                None => {
                    let folder = abs_root.file_name().map(|f| f.to_string_lossy());
                    folder
                        .as_deref()
                        .and_then(project_name_from_folder)
                        .ok_or_else(|| {
                            anyhow::anyhow!(
                                "no project name can be made from the folder name {:?}: give a \
                                 name of lowercase letters, digits and dashes",
                                folder.as_deref().unwrap_or_default()
                            )
                        })?
                }
            };
            b.set_name(Some(name));
        }
        Backend::Server(_) => { /* nothing to do here */ }
    }

    if root_dir.join(paths::CONFIG_FILE_NAME).exists() {
        bail!("dvs is already initialized (dvs.toml exists)");
    }

    // Check the metadata folder before touching the backend
    let metadata_dir = root_dir.join(config.metadata_folder_name());
    if metadata_dir.exists() && fs::read_dir(&metadata_dir)?.next().is_some() {
        bail!(
            "dvs is already initialized ({} exists and is not empty)",
            config.metadata_folder_name()
        );
    }

    log::debug!("Initializing backend");
    let mut joined = false;
    if config.backend().init(config.compression())? {
        match config.backend() {
            Backend::Server(s) => bail!("the project {} already exists on {}", s.name, s.url),
            Backend::Local(b) => {
                let name = b
                    .name()
                    .expect("init names every local project")
                    .to_string();
                if !name_given {
                    return Err(ProjectExists {
                        name,
                        storage: b.path.clone(),
                    }
                    .into());
                }
                // As dvs-server: a project keeps the compression it was created with
                let created_with = config
                    .backend()
                    .get_audit_entries(&[])?
                    .into_iter()
                    .find_map(|e| match e.action {
                        Action::Init { settings, .. } => Some(settings.compression()),
                        _ => None,
                    });
                if let Some(existing) = created_with
                    && existing != config.compression()
                {
                    bail!(
                        "project {name} already exists with compression {existing}, not {}",
                        config.compression()
                    );
                }
                joined = true;
            }
        }
    }

    // The server decides the compression, so there is nothing to keep past init
    if matches!(config.backend(), Backend::Server(_)) {
        config.set_compression(None);
    }

    config.save(root_dir)?;

    let config_path = root_dir.join(paths::CONFIG_FILE_NAME);

    if let Err(e) = create_metadata_folder(root_dir, &config) {
        // Roll back the config file we just wrote.
        let _ = fs::remove_file(&config_path);
        return Err(e);
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
    let project = match config.backend() {
        Backend::Local(b) => b.name().map(str::to_string),
        Backend::Server(_) => None,
    };
    Ok(Initialized {
        root: root_dir.to_path_buf(),
        project,
        joined,
    })
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
        init(&root, config).unwrap();

        let config = Config::find(&root).unwrap().unwrap();
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

        let error = init(&root, config).unwrap_err();
        let exists = error.downcast_ref::<ProjectExists>().unwrap();
        assert_eq!(exists.name, "repo");
    }

    fn local(storage: &Path, name: Option<&str>) -> Config {
        let mut config = Config::new_local(storage, None).unwrap();
        if let Backend::Local(b) = config.backend_mut() {
            b.set_name(name.map(str::to_string));
        }
        config
    }

    fn repo(tmp: &tempfile::TempDir, folder: &str) -> PathBuf {
        let root = fs::canonicalize(tmp.path()).unwrap().join(folder);
        fs::create_dir_all(root.join(".git")).unwrap();
        root
    }

    #[test]
    fn a_project_gets_its_own_folder_named_as_on_the_server() {
        let tmp = tempfile::tempdir().unwrap();
        let root = repo(&tmp, "My_Analysis 2");
        let storage = tmp.path().join("store");

        let done = init(&root, local(&storage, None)).unwrap();
        assert_eq!(done.project.as_deref(), Some("my-analysis-2"));
        assert!(!done.joined);
        // As dvs-server: the folder is the blake3 hash of the name
        let folder = storage.join(blake3::hash(b"my-analysis-2").to_string());
        assert!(folder.join("audit.log.jsonl").is_file());
        assert!(!storage.join("audit.log.jsonl").exists());

        let saved = Config::find(&root).unwrap().unwrap();
        let Backend::Local(b) = saved.backend() else {
            unreachable!()
        };
        assert_eq!(b.name(), Some("my-analysis-2"));
        assert_eq!(b.path, fs::canonicalize(&storage).unwrap());
        assert_eq!(b.project_dir(), fs::canonicalize(&folder).unwrap());
    }

    #[test]
    fn projects_share_a_store_and_keep_their_own_audit_log() {
        let tmp = tempfile::tempdir().unwrap();
        let storage = tmp.path().join("store");
        init(&repo(&tmp, "pk1"), local(&storage, None)).unwrap();
        init(&repo(&tmp, "pk2"), local(&storage, None)).unwrap();

        for root in [repo(&tmp, "pk1"), repo(&tmp, "pk2")] {
            let config = Config::find(&root).unwrap().unwrap();
            assert_eq!(config.backend().get_audit_entries(&[]).unwrap().len(), 1);
        }
    }

    #[test]
    fn a_given_name_joins_and_a_folder_name_refuses() {
        let tmp = tempfile::tempdir().unwrap();
        let storage = tmp.path().join("store");
        init(&repo(&tmp, "a/analysis"), local(&storage, None)).unwrap();

        // Same folder name, name not given: maybe an accident
        let error = init(&repo(&tmp, "b/analysis"), local(&storage, None)).unwrap_err();
        let exists = error.downcast_ref::<ProjectExists>().unwrap();
        assert_eq!(exists.name, "analysis");
        assert_eq!(exists.storage, fs::canonicalize(&storage).unwrap());
        assert!(!repo(&tmp, "b/analysis").join("dvs.toml").exists());

        // Given: joins, as on dvs-server
        let other = repo(&tmp, "c/elsewhere");
        let done = init(&other, local(&storage, Some("analysis"))).unwrap();
        assert!(done.joined);
        let config = Config::find(&other).unwrap().unwrap();
        let inits = config.backend().get_audit_entries(&[]).unwrap();
        assert_eq!(inits.len(), 2, "the join is in the project's audit log");
    }

    #[test]
    fn a_join_keeps_the_compression_of_the_project() {
        let tmp = tempfile::tempdir().unwrap();
        let storage = tmp.path().join("store");
        init(&repo(&tmp, "pk1"), local(&storage, None)).unwrap();

        let mut config = local(&storage, Some("pk1"));
        config.set_compression(Some(crate::Compression::None));
        let error = init(&repo(&tmp, "other"), config).unwrap_err();
        assert_eq!(
            error.to_string(),
            "project pk1 already exists with compression zstd, not none"
        );
    }

    #[test]
    fn names_follow_the_server_rule() {
        let tmp = tempfile::tempdir().unwrap();
        let storage = tmp.path().join("store");
        for bad in ["Analysis", "-a", "a_b", "project", ""] {
            let error = init(&repo(&tmp, "pk1"), local(&storage, Some(bad))).unwrap_err();
            assert!(
                error
                    .downcast_ref::<crate::backends::local::InvalidProjectName>()
                    .is_some(),
                "{bad:?}: {error}"
            );
        }
        // A folder that leaves no name asks for one
        let error = init(&repo(&tmp, "___"), local(&storage, None)).unwrap_err();
        assert!(error.to_string().contains("give a name"), "{error}");
    }

    #[test]
    fn a_store_from_before_projects_keeps_its_layout_next_to_new_ones() {
        let tmp = tempfile::tempdir().unwrap();
        let storage = tmp.path().join("store");
        // A dvs.toml with no name, as written before projects
        let old = repo(&tmp, "old");
        let mut flat = Config::new_local(&storage, None).unwrap();
        if let Backend::Local(b) = flat.backend_mut() {
            b.path = fs::canonicalize(tmp.path()).unwrap().join("store");
        }
        fs::create_dir_all(&storage).unwrap();
        flat.save(&old).unwrap();
        fs::write(storage.join("audit.log.jsonl"), "").unwrap();
        let saved = Config::find(&old).unwrap().unwrap();
        let Backend::Local(b) = saved.backend() else {
            unreachable!()
        };
        assert_eq!(b.name(), None);
        assert_eq!(b.project_dir(), b.path);

        // A new project in the same storage directory gets its own folder
        let done = init(&repo(&tmp, "new"), local(&storage, None)).unwrap();
        assert_eq!(done.project.as_deref(), Some("new"));
        assert!(storage.join(blake3::hash(b"new").to_string()).is_dir());
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
