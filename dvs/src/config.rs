use std::path::{Path, PathBuf};

use anyhow::{Result, anyhow};
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

/// Parse a permission string as an octal mode.
/// Returns the mode as an u32.
fn parse_permissions(perms: &str) -> Result<u32> {
    let mode = u32::from_str_radix(perms, 8).map_err(|_| {
        anyhow!(
            "Invalid permission mode '{}': must be octal (e.g., '770')",
            perms
        )
    })?;
    if mode > 0o7777 {
        anyhow::bail!(
            "Invalid permission mode '{}': value {} exceeds maximum 7777",
            perms,
            mode
        );
    }
    Ok(mode)
}

/// Resolve a group name to its GID.
#[cfg(unix)]
fn resolve_group(group_name: &str) -> Result<nix::unistd::Gid> {
    use nix::unistd::Group;
    let group =
        Group::from_name(group_name)?.ok_or_else(|| anyhow!("Group '{}' not found", group_name))?;
    Ok(nix::unistd::Gid::from_raw(group.gid.as_raw()))
}

#[cfg(not(unix))]
fn resolve_group(_group_name: &str) -> Result<()> {
    Ok(())
}

impl LocalBackend {
    pub fn permissions(&self) -> Option<&str> {
        self.permissions.as_deref()
    }

    pub fn group(&self) -> Option<&str> {
        self.group.as_deref()
    }

    /// Apply configured permissions and group to a path.
    /// No-op on non-Unix or if neither permissions nor group are set.
    #[cfg(unix)]
    pub fn apply_perms(&self, path: impl AsRef<Path>) -> Result<()> {
        use nix::unistd::chown;
        use std::os::unix::fs::PermissionsExt;

        let path = path.as_ref();

        if let Some(perms) = &self.permissions {
            let mode = parse_permissions(perms)?;
            let permissions = std::fs::Permissions::from_mode(mode);
            fs::set_permissions(path, permissions)?;
        }

        if let Some(group_name) = &self.group {
            let gid = resolve_group(group_name)?;
            chown(path, None, Some(gid))?;
        }

        Ok(())
    }

    #[cfg(not(unix))]
    pub fn apply_perms(&self, _path: impl AsRef<Path>) -> Result<()> {
        Ok(())
    }
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
    pub fn new_local(
        path: impl AsRef<Path>,
        permissions: Option<String>,
        group: Option<String>,
    ) -> Result<Config> {
        // Validate permissions and group before creating config
        if let Some(ref perms) = permissions {
            parse_permissions(perms)?;
        }
        if let Some(ref grp) = group {
            resolve_group(grp)?;
        }

        let backend = Backend::Local(LocalBackend {
            path: path.as_ref().to_path_buf(),
            permissions,
            group,
        });
        Ok(Config {
            metadata_folder_name: None,
            backend,
        })
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
}
