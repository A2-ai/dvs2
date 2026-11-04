use std::ffi::OsStr;
use std::path::{Component, Path, PathBuf};

use anyhow::{Result, anyhow, bail};
use fs_err as fs;
use serde::Deserialize;
use walkdir::WalkDir;

use crate::config::ConfigError;
use crate::error::{Error, ErrorCode};
use crate::{Compression, DvsPaths, FileMetadata, Hashes};

pub const CONFIG_FILE_NAME: &str = "dvs.yaml";

pub fn is_dvs1(repo_root: &Path) -> bool {
    repo_root.join(CONFIG_FILE_NAME).is_file()
}

/// The dvs1 `dvs.yaml`.
#[derive(Debug, Clone, PartialEq)]
pub struct Dvs1Config {
    pub storage_dir: PathBuf,
    /// File mode, e.g. `0o664`
    pub permissions: Option<u32>,
    pub group: Option<String>,
}

impl Dvs1Config {
    pub fn read(repo_root: &Path) -> Result<Self> {
        let path = repo_root.join(CONFIG_FILE_NAME);
        let content = fs::read_to_string(&path)?;
        Self::parse(&content, repo_root)
            .map_err(|e| ConfigError(format!("Failed to parse {}: {e}", path.display())).into())
    }

    // dvs1 writes three flat scalar keys, so this avoids a YAML dependency and
    // refuses any other value syntax for them rather than guessing. Like dvs1,
    // other keys are ignored.
    fn parse(content: &str, repo_root: &Path) -> Result<Self> {
        let (mut storage_dir, mut permissions, mut group) = (None, None, None);
        let mut in_known_key = false;
        for line in content.lines() {
            let trimmed = line.trim();
            if trimmed.is_empty() || trimmed.starts_with('#') || trimmed == "---" {
                continue;
            }
            // A nested or multi-line value belongs to the key above it
            if line.starts_with([' ', '\t', '-']) {
                if in_known_key {
                    bail!("unsupported value at `{line}`");
                }
                continue;
            }
            let Some((key, value)) = trimmed.split_once(':') else {
                bail!("unexpected line `{line}`");
            };
            let key = key.trim();
            in_known_key = matches!(key, "storage_dir" | "permissions" | "group");
            if !in_known_key {
                continue;
            }
            let value = scalar(value.trim())?;
            match key {
                "storage_dir" => storage_dir = value,
                "permissions" => permissions = value.and_then(|v| u32::from_str_radix(&v, 8).ok()),
                _ => group = value,
            }
        }
        let storage_dir: PathBuf = storage_dir
            .ok_or_else(|| anyhow!("missing storage_dir"))?
            .into();
        let storage_dir = if storage_dir.is_absolute() {
            storage_dir
        } else {
            normalized(&repo_root.join(storage_dir))
        };
        Ok(Self {
            storage_dir,
            permissions,
            group,
        })
    }
}

/// Resolves `.` and `..` without touching the filesystem.
fn normalized(path: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for component in path.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => match out.components().next_back() {
                Some(Component::Normal(_)) => {
                    out.pop();
                }
                Some(Component::RootDir | Component::Prefix(_)) => {}
                _ => out.push(component),
            },
            c => out.push(c),
        }
    }
    out
}

/// A YAML scalar; `None` for null or empty.
fn scalar(value: &str) -> Result<Option<String>> {
    let unquoted = if let Some(inner) = value.strip_prefix('\'').and_then(|v| v.strip_suffix('\''))
    {
        inner.replace("''", "'")
    } else if let Some(inner) = value.strip_prefix('"').and_then(|v| v.strip_suffix('"')) {
        if inner.contains('\\') {
            bail!("unsupported escape in {value}");
        }
        inner.to_string()
    } else if matches!(value, "" | "~" | "null" | "Null" | "NULL") {
        return Ok(None);
    } else if value.starts_with(['|', '>', '[', '{', '&', '*', '!']) {
        bail!("unsupported value `{value}`");
    } else {
        value.to_string()
    };
    Ok((!unquoted.is_empty()).then_some(unquoted))
}

/// A dvs1 `<file>.dvs` record, kept next to the data file.
#[derive(Debug, Clone, Deserialize)]
pub struct Dvs1Metadata {
    pub blake3_checksum: String,
    #[serde(alias = "file_size_bytes")]
    pub size: u64,
    #[serde(alias = "time_stamp")]
    pub add_time: String,
    pub message: String,
    pub saved_by: String,
}

impl Dvs1Metadata {
    pub fn read(path: &Path) -> Result<Self> {
        let metadata = serde_json::from_reader(fs::File::open(path)?).map_err(|e| {
            Error::new(
                ErrorCode::BadMetadata,
                format!("cannot read {}: {e}", path.display()),
            )
        })?;
        Ok(metadata)
    }

    /// The same record as dvs2 metadata. dvs1 blobs are stored raw.
    pub fn to_metadata(&self) -> Result<FileMetadata> {
        let add_time = self.add_time.parse::<jiff::Timestamp>().map_err(|e| {
            Error::new(
                ErrorCode::BadMetadata,
                format!("invalid add_time `{}`: {e}", self.add_time),
            )
        })?;
        Ok(FileMetadata {
            hashes: Hashes {
                blake3: self.blake3_checksum.clone(),
                md5: None,
            },
            size: self.size,
            created_by: self.saved_by.clone(),
            add_time,
            compression: Compression::None,
            message: (!self.message.is_empty()).then(|| self.message.clone()),
        })
    }
}

/// The dvs1 metadata file for a data file: `x.csv` -> `x.csv.dvs`.
pub fn metadata_path(data_path: &Path) -> PathBuf {
    let mut s = data_path.as_os_str().to_owned();
    s.push(".dvs");
    PathBuf::from(s)
}

/// Repo-root-relative paths of every data file with a dvs1 metadata file in
/// the working tree, committed or not. `metadata_folder_name` is the dvs2
/// metadata folder, whose `.dvs` files are not dvs1's.
pub fn tracked_paths(repo_root: &Path, metadata_folder_name: &str) -> Vec<PathBuf> {
    let metadata_files =
        git_listed(repo_root).unwrap_or_else(|| walked(repo_root, metadata_folder_name));
    let mut paths: Vec<PathBuf> = metadata_files
        .into_iter()
        .filter(|rel| !rel.starts_with(metadata_folder_name))
        .filter(|rel| rel.extension() == Some(OsStr::new("dvs")))
        // The index still lists files deleted from the working tree
        .filter(|rel| repo_root.join(rel).is_file())
        .map(|rel| rel.with_extension(""))
        .collect();
    paths.sort();
    // An unmerged file is listed once per conflict stage
    paths.dedup();
    paths
}

/// The `tracked_paths` a merge brought into the dvs2 checkout `paths`, less
/// any `.dvs` file it tracks as data and, unless `unreadable`, any that does
/// not read as dvs1 metadata.
pub fn leftover_paths(
    paths: &DvsPaths,
    metadata_folder_name: &str,
    unreadable: bool,
) -> Vec<PathBuf> {
    let root = paths.repo_root();
    tracked_paths(root, metadata_folder_name)
        .into_iter()
        .filter(|path| {
            let sidecar = metadata_path(path);
            !paths.metadata_path(&sidecar).is_file()
                && (unreadable || Dvs1Metadata::read(&root.join(&sidecar)).is_ok())
        })
        .collect()
}

/// `None` when this is not a git checkout.
fn git_listed(repo_root: &Path) -> Option<Vec<PathBuf>> {
    let listed = crate::git::output(
        repo_root,
        &[
            "ls-files",
            "-z",
            "--cached",
            "--others",
            "--exclude-standard",
            "--",
            "*.dvs",
        ],
    )?;
    Some(
        listed
            .split('\0')
            .filter(|p| !p.is_empty())
            .map(PathBuf::from)
            .collect(),
    )
}

fn walked(repo_root: &Path, metadata_folder_name: &str) -> Vec<PathBuf> {
    WalkDir::new(repo_root)
        .into_iter()
        .filter_entry(|e| {
            e.file_name() != ".git" && !(e.depth() == 1 && e.file_name() == metadata_folder_name)
        })
        .filter_map(|e| e.ok())
        .filter(|e| e.file_type().is_file())
        .filter_map(|e| e.path().strip_prefix(repo_root).ok().map(Path::to_path_buf))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testutil::{create_file, create_temp_git_repo};

    #[test]
    fn is_dvs1_when_dvs_yaml_is_present() {
        let (_tmp, root) = create_temp_git_repo();
        assert!(!is_dvs1(&root));
        create_file(&root, CONFIG_FILE_NAME, b"storage_dir: /data/proj\n");
        assert!(is_dvs1(&root));
    }

    #[test]
    fn config_reads_storage_dir_permissions_and_group() {
        let (_tmp, root) = create_temp_git_repo();
        let yaml = "storage_dir: /data/proj\npermissions: 664\ngroup: 'my team'\n";
        create_file(&root, CONFIG_FILE_NAME, yaml.as_bytes());

        let config = Dvs1Config::read(&root).unwrap();
        assert_eq!(
            config,
            Dvs1Config {
                storage_dir: PathBuf::from("/data/proj"),
                permissions: Some(0o664),
                group: Some("my team".to_string()),
            }
        );
    }

    #[test]
    fn config_treats_null_and_empty_as_absent() {
        let config = Dvs1Config::parse(
            "storage_dir: /data/proj\npermissions: null\ngroup: ''\n",
            Path::new("/repo"),
        )
        .unwrap();
        assert_eq!((config.permissions, config.group), (None, None));
    }

    #[test]
    fn config_refuses_yaml_it_does_not_understand() {
        let err = Dvs1Config::parse("storage_dir: |\n  /data/proj\n", Path::new("/repo"));
        assert!(err.is_err());
    }

    #[test]
    fn metadata_converts_to_raw_dvs2_metadata() {
        let (_tmp, root) = create_temp_git_repo();
        let json = r#"{
            "blake3_checksum": "abcdef",
            "size": 11,
            "add_time": "2024-01-02T03:04:05.123Z",
            "message": "",
            "saved_by": "tester"
        }"#;
        let path = create_file(&root, "x.csv.dvs", json.as_bytes());

        let metadata = Dvs1Metadata::read(&path).unwrap().to_metadata().unwrap();
        assert_eq!(metadata.hashes.blake3, "abcdef");
        assert_eq!(metadata.hashes.md5, None);
        assert_eq!(metadata.size, 11);
        assert_eq!(metadata.created_by, "tester");
        assert_eq!(metadata.add_time.to_string(), "2024-01-02T03:04:05.123Z");
        assert_eq!(metadata.compression, Compression::None);
        assert_eq!(metadata.message, None);
    }

    #[test]
    fn metadata_reads_v0_0_1_field_names() {
        let (_tmp, root) = create_temp_git_repo();
        let json = r#"{
            "blake3_checksum": "abcdef",
            "file_size_bytes": 11,
            "time_stamp": "2024-01-02T03:04:05.123Z",
            "message": "",
            "saved_by": "tester"
        }"#;
        let path = create_file(&root, "x.csv.dvs", json.as_bytes());

        let metadata = Dvs1Metadata::read(&path).unwrap();
        assert_eq!(metadata.size, 11);
        assert_eq!(metadata.add_time, "2024-01-02T03:04:05.123Z");
    }

    #[test]
    fn tracked_paths_skip_the_dvs2_metadata_folder() {
        let (_tmp, root) = create_temp_git_repo();
        create_file(&root, "x.csv.dvs", b"{}");
        create_file(&root, "sub/y.csv.dvs", b"{}");
        create_file(&root, ".dvs/z.csv.dvs", b"{}");
        create_file(&root, ".git/w.dvs", b"{}");

        assert_eq!(
            tracked_paths(&root, ".dvs"),
            vec![PathBuf::from("sub/y.csv"), PathBuf::from("x.csv")]
        );
    }

    #[test]
    fn tracked_paths_in_git_list_untracked_but_not_ignored_files() {
        let (_tmp, root) = create_temp_git_repo();
        fs::remove_dir(root.join(".git")).unwrap();
        crate::git::output(&root, &["init", "-q"]).unwrap();
        create_file(&root, ".gitignore", b"ignored/\n");
        create_file(&root, "x.csv.dvs", b"{}");
        create_file(&root, "ignored/y.csv.dvs", b"{}");
        create_file(&root, "added.csv.dvs", b"{}");
        create_file(&root, "deleted.csv.dvs", b"{}");
        create_file(&root, ".dvs/z.csv.dvs", b"{}");
        crate::git::output(
            &root,
            &["add", "added.csv.dvs", "deleted.csv.dvs", ".dvs/z.csv.dvs"],
        )
        .unwrap();
        fs::remove_file(root.join("deleted.csv.dvs")).unwrap();

        assert_eq!(
            tracked_paths(&root, ".dvs"),
            vec![PathBuf::from("added.csv"), PathBuf::from("x.csv")]
        );
    }
}
