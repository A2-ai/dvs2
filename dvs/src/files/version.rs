//! `get --version`: write an earlier version of a tracked file to a side file,
//! leaving the tracked file, its `.dvs` file and dvs's local state alone.

use std::path::{Path, PathBuf};

use anyhow::Result;
use fs_err as fs;
use serde::Serialize;
use uuid::Uuid;

use crate::audit::Action;
use crate::error::{Error, ErrorCode};
use crate::files::get::normalize;
use crate::files::metadata::FileMetadata;
use crate::gitignore::add_to_gitignore;
use crate::{Backend, Compression, DvsPaths, Hashes, PathFilter, ProjectPath, RetrieveRequest};

const MIN_PREFIX: usize = 8;

#[derive(Debug, Clone, Serialize)]
pub struct VersionGet {
    /// The tracked file, relative to the project root
    pub path: PathBuf,
    /// Where the version was written
    pub to: PathBuf,
    pub hash: String,
    pub size: u64,
    /// Whether `to` was added to a `.gitignore`
    pub ignored: bool,
}

fn coded(code: ErrorCode, message: impl Into<String>) -> anyhow::Error {
    Error::new(code, message).into()
}

/// The hash and compression of the one stored version of `path` that starts
/// with `version`.
fn resolve(backend: &Backend, path: &Path, version: &str) -> Result<(Hashes, Compression)> {
    let prefix = version.to_ascii_lowercase();
    if prefix.len() < MIN_PREFIX || !prefix.bytes().all(|c| c.is_ascii_hexdigit()) {
        return Err(coded(
            ErrorCode::InvalidArguments,
            format!("--version takes a hash or a prefix of at least {MIN_PREFIX} hex characters"),
        ));
    }
    let mut found: Vec<(Hashes, Compression)> = Vec::new();
    for entry in backend.get_audit_entries(&[path.to_path_buf()])? {
        if let Action::Add {
            file, compression, ..
        } = entry.action
            && file.path == path
            && file.hashes.blake3.starts_with(&prefix)
            && !found.iter().any(|(h, _)| h.blake3 == file.hashes.blake3)
        {
            found.push((file.hashes, compression));
        }
    }
    match found.len() {
        1 => Ok(found.remove(0)),
        0 => Err(coded(
            ErrorCode::FileFailed,
            format!("{} has no version {version}", path.display()),
        )),
        _ => Err(coded(
            ErrorCode::InvalidArguments,
            format!(
                "{version} matches {} versions of {}; give more of the hash",
                found.len(),
                path.display()
            ),
        )),
    }
}

/// Refuses a destination that exists, is tracked or is in the metadata folder.
fn check_destination(paths: &DvsPaths, to: &Path) -> Result<()> {
    let refuse = |why: &str| {
        Err(coded(
            ErrorCode::InvalidArguments,
            format!("cannot write to {}: {why}", to.display()),
        ))
    };
    if fs::symlink_metadata(to).is_ok() {
        return refuse("it already exists");
    }
    if let Ok(relative) = to.strip_prefix(paths.repo_root()) {
        if to.starts_with(paths.metadata_folder()) {
            return refuse("it is inside the dvs metadata folder");
        }
        if paths.metadata_path(relative).is_file() {
            return refuse("it is a tracked file");
        }
    }
    Ok(())
}

/// The project-relative path of the one file `user_path` names.
fn project_file(paths: &DvsPaths, user_path: &Path) -> Result<PathBuf> {
    let invalid = |why: &str| {
        Err(coded(
            ErrorCode::InvalidArguments,
            format!("--version {why}: {}", user_path.display()),
        ))
    };
    if paths.cwd().join(user_path).is_dir() {
        return invalid("takes a file, not a folder");
    }
    let filter = PathFilter::from_user_paths(vec![user_path.to_path_buf()], false, paths);
    match filter.paths() {
        [relative] if !relative.as_os_str().is_empty() => Ok(relative.clone()),
        _ => invalid("takes a file inside the project"),
    }
}

/// Writes version `version` of the tracked file `user_path` to `to`, both
/// relative to the current directory. The download names the real path, so
/// the server records it against the upload of that version.
pub fn get_version(
    paths: &DvsPaths,
    backend: &Backend,
    user_path: &Path,
    version: &str,
    to: &Path,
) -> Result<VersionGet> {
    let path = &project_file(paths, user_path)?;
    let to = &normalize(&paths.cwd().join(to));
    let (hashes, compression) = resolve(backend, path, version)?;
    check_destination(paths, to)?;
    let project_path = ProjectPath::from_path(path)
        .map_err(|e| coded(ErrorCode::InvalidArguments, e.to_string()))?;

    let parent = to.parent().unwrap_or(Path::new("."));
    fs::create_dir_all(parent)?;
    let tmp = parent.join(format!(".dvs-tmp.{}", Uuid::new_v4()));
    let result = (|| {
        let retrieved = backend.retrieve(RetrieveRequest {
            hashes: &hashes,
            target: &tmp,
            compression,
            path: project_path,
            operation_id: Uuid::new_v4(),
            on_bytes: None,
        })?;
        if !retrieved {
            return Err(coded(
                ErrorCode::FileFailed,
                format!(
                    "version {} of {} is not in storage",
                    hashes.blake3,
                    path.display()
                ),
            ));
        }
        let actual = FileMetadata::from_file(&tmp, Compression::None, None)?;
        if actual.hashes.blake3 != hashes.blake3 {
            return Err(coded(
                ErrorCode::FileFailed,
                "the stored content does not match the version's hash",
            ));
        }
        check_destination(paths, to)?;
        fs::rename(&tmp, to)?;
        Ok(actual.size)
    })();
    if result.is_err() {
        let _ = fs::remove_file(&tmp);
    }
    let size = result?;

    let ignored = match to.strip_prefix(paths.repo_root()) {
        Ok(relative) => {
            add_to_gitignore(paths.repo_root(), &[relative.to_path_buf()])?;
            paths.repo_root().join(".git").exists()
        }
        Err(_) => false,
    };
    Ok(VersionGet {
        path: path.to_path_buf(),
        to: to.to_path_buf(),
        hash: hashes.blake3,
        size,
        ignored,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::OperationContext;
    use crate::add_files;
    use crate::testutil::{create_file, create_temp_git_repo, init_dvs_repo};
    use tempfile::TempDir;

    struct Repo {
        _tmp: TempDir,
        root: PathBuf,
        config: crate::config::Config,
        paths: DvsPaths,
    }

    impl Repo {
        fn new() -> Self {
            let (_tmp, root) = create_temp_git_repo();
            let (config, _) = init_dvs_repo(&root);
            let paths =
                DvsPaths::new(root.clone(), root.clone(), config.metadata_folder_name()).unwrap();
            Self {
                _tmp,
                root,
                config,
                paths,
            }
        }

        /// Writes and adds `content` at `name`, returning its hash.
        fn add(&self, name: &str, content: &[u8]) -> String {
            create_file(&self.root, name, content);
            add_files(
                vec![name.into()],
                &self.paths,
                self.config.backend(),
                None,
                Compression::Zstd,
                false,
                &OperationContext::new(),
            )
            .unwrap();
            FileMetadata::from_file(self.root.join(name), Compression::None, None)
                .unwrap()
                .hashes
                .blake3
        }

        fn get(&self, name: &str, version: &str, to: &str) -> Result<VersionGet> {
            get_version(
                &self.paths,
                self.config.backend(),
                Path::new(name),
                version,
                Path::new(to),
            )
        }
    }

    fn code(error: &anyhow::Error) -> ErrorCode {
        error.downcast_ref::<Error>().unwrap().code
    }

    #[test]
    fn an_old_version_lands_beside_the_tracked_file() {
        let repo = Repo::new();
        let v1 = repo.add("data.csv", b"v1");
        repo.add("data.csv", b"v2");
        let sidecar = fs::read(repo.paths.metadata_path(Path::new("data.csv"))).unwrap();

        let got = repo
            .get("data.csv", &v1[..8], "compare/data_v1.csv")
            .unwrap();

        assert_eq!(got.hash, v1);
        assert_eq!(
            fs::read(repo.root.join("compare/data_v1.csv")).unwrap(),
            b"v1"
        );
        assert_eq!(fs::read(repo.root.join("data.csv")).unwrap(), b"v2");
        assert_eq!(
            fs::read(repo.paths.metadata_path(Path::new("data.csv"))).unwrap(),
            sidecar
        );
        let ignore = fs::read_to_string(repo.root.join("compare/.gitignore")).unwrap();
        assert!(ignore.lines().any(|l| l == "/data_v1.csv"));
    }

    #[test]
    fn a_destination_outside_the_repo_is_not_ignored() {
        let repo = Repo::new();
        let v1 = repo.add("data.csv", b"v1");

        let got = repo.get("data.csv", &v1, "../out.csv").unwrap();

        let outside = repo.root.parent().unwrap();
        assert_eq!(got.to, outside.join("out.csv"));
        assert!(!got.ignored);
        assert!(!outside.join(".gitignore").exists());
    }

    #[test]
    fn a_hash_the_file_never_had_is_refused() {
        let repo = Repo::new();
        repo.add("data.csv", b"v1");
        let other = repo.add("other.csv", b"other");

        let error = repo.get("data.csv", &other, "out.csv").unwrap_err();

        assert_eq!(code(&error), ErrorCode::FileFailed);
        assert!(!repo.root.join("out.csv").exists());
    }

    #[test]
    fn a_short_prefix_is_refused() {
        let repo = Repo::new();
        let v1 = repo.add("data.csv", b"v1");

        let error = repo.get("data.csv", &v1[..7], "out.csv").unwrap_err();

        assert_eq!(code(&error), ErrorCode::InvalidArguments);
    }

    #[test]
    fn an_existing_or_tracked_destination_is_never_overwritten() {
        let repo = Repo::new();
        let v1 = repo.add("data.csv", b"v1");
        repo.add("data.csv", b"v2");
        create_file(&repo.root, "mine.csv", b"mine");

        for to in ["mine.csv", "data.csv"] {
            let error = repo.get("data.csv", &v1, to).unwrap_err();
            assert_eq!(code(&error), ErrorCode::InvalidArguments);
        }
        assert_eq!(fs::read(repo.root.join("mine.csv")).unwrap(), b"mine");
        assert_eq!(fs::read(repo.root.join("data.csv")).unwrap(), b"v2");
    }
}
