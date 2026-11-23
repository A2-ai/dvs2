//! `fetch`: write an earlier version of a file to the scratchpad.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use anyhow::{Result, bail};
use fs_err as fs;
use serde::Serialize;
use uuid::Uuid;

use crate::cache::FileStat;
use crate::error::{Error, ErrorCode};
use crate::files::get::{
    GetCode, destination_is_contained, relative_to, retrieve_verified, shell_path,
};
use crate::gitignore::add_to_gitignore;
use crate::paths::SCRATCHPAD_FOLDER_NAME;
use crate::{Backend, Compression, DvsPaths, Hashes, PathFilter, ProjectPath, audit};

/// The shortest hash prefix accepted, as `dvs history` shows it.
pub const SHORT_HASH_LEN: usize = 12;

/// How to pick the version to fetch.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FetchCriteria {
    /// The full hash, or at least its first [`SHORT_HASH_LEN`] characters
    Hash(String),
}

/// `user_path` relative to the project root. The file need not exist.
fn project_path(user_path: &Path, paths: &DvsPaths) -> Result<ProjectPath> {
    let filter = PathFilter::from_user_paths(vec![user_path.to_path_buf()], false, paths);
    let [relative] = filter.paths() else {
        let message = format!("{} is outside the project", user_path.display());
        bail!(Error::invalid_arguments(message));
    };
    ProjectPath::from_path(relative)
        .map_err(|e| Error::invalid_arguments(format!("{}: {e}", user_path.display())).into())
}

/// The version of `path` picked by `criteria`. `user_path` is for the messages.
fn find_version(
    path: &ProjectPath,
    user_path: &Path,
    criteria: &FetchCriteria,
    backend: &Backend,
) -> Result<(Hashes, Compression)> {
    match criteria {
        FetchCriteria::Hash(hash) => find_by_hash(path, user_path, hash, backend),
    }
}

/// The version of `path` whose hash starts with `hash`.
fn find_by_hash(
    path: &ProjectPath,
    user_path: &Path,
    hash: &str,
    backend: &Backend,
) -> Result<(Hashes, Compression)> {
    let prefix = hash.to_ascii_lowercase();
    if prefix.len() < SHORT_HASH_LEN || !prefix.chars().all(|c| c.is_ascii_hexdigit()) {
        bail!(Error::invalid_arguments(format!(
            "the hash must be at least {SHORT_HASH_LEN} hex characters: {hash}"
        )));
    }

    // Keyed by hash: a version added twice is one version
    let versions: HashMap<String, (Hashes, Compression)> = backend
        .get_audit_entries(&[path.to_native()])?
        .into_iter()
        .filter_map(|entry| match entry.action {
            audit::Action::Add {
                file, compression, ..
            } if file.hashes.blake3.starts_with(&prefix) => {
                Some((file.hashes.blake3.clone(), (file.hashes, compression)))
            }
            _ => None,
        })
        .collect();

    let shown = user_path.display();
    let count = versions.len();
    match versions.into_values().next() {
        Some(version) if count == 1 => Ok(version),
        Some(_) => bail!(Error::invalid_arguments(format!(
            "{prefix} matches {count} versions of {shown}; give more of the hash"
        ))),
        None => bail!(Error::invalid_arguments(format!(
            "{shown} has no version with a hash starting with {prefix}; \
             list its versions with: dvs history {}",
            shell_path(user_path)
        ))),
    }
}

/// A version written to the scratchpad.
#[derive(Debug, Clone, Serialize)]
pub struct Fetched {
    /// Relative to the project root
    pub path: PathBuf,
    /// Relative to the current directory
    pub written_to: PathBuf,
    pub hash: String,
    pub size: u64,
    /// The copy was already there: nothing was downloaded
    pub already_there: bool,
}

/// Writes the version of `user_path` picked by `criteria` to
/// `.dvs-scratchpad/<hash prefix>/<project path>`, replacing what is there.
pub fn fetch_file(
    user_path: &Path,
    criteria: &FetchCriteria,
    paths: &DvsPaths,
    backend: &Backend,
) -> Result<Fetched> {
    let path = project_path(user_path, paths)?;
    let (hashes, compression) = find_version(&path, user_path, criteria, backend)?;
    let scratchpad = paths.repo_root().join(SCRATCHPAD_FOLDER_NAME);
    let target = scratchpad
        .join(&hashes.blake3[..SHORT_HASH_LEN])
        .join(path.to_native());
    let fetched = |size, already_there| Fetched {
        path: path.to_native(),
        written_to: relative_to(paths.cwd(), &target),
        hash: hashes.blake3.clone(),
        size,
        already_there,
    };

    fs::create_dir_all(&scratchpad)?;
    if !destination_is_contained(&scratchpad, &target) {
        bail!(Error::new(
            ErrorCode::OperationFailed,
            format!(
                "{} leads outside {SCRATCHPAD_FOLDER_NAME} (a symlink?); dvs will not write there",
                target.display()
            )
        ));
    }

    // Hashed every time: the copy may have been edited since it was fetched
    if let Ok((existing, size)) = Hashes::compute_from_path(&target, &[])
        && existing == hashes
    {
        return Ok(fetched(size, true));
    }
    let stat = FileStat::from_path(&target).ok();
    let written = retrieve_verified(
        backend,
        &target,
        &hashes,
        compression,
        path.clone(),
        &stat,
        Uuid::new_v4(),
        None,
    );
    if let Err((code, message)) = written {
        // Leave no empty folders behind
        for dir in target.ancestors().skip(1) {
            if !dir.starts_with(&scratchpad) || fs::remove_dir(dir).is_err() {
                break;
            }
        }
        let code = match code {
            GetCode::Shared(code) => code,
            _ => ErrorCode::OperationFailed,
        };
        bail!(Error::new(
            code,
            format!("{}: {message}", user_path.display())
        ));
    }
    if let Err(e) = add_to_gitignore(paths.repo_root(), &[PathBuf::from(SCRATCHPAD_FOLDER_NAME)]) {
        log::warn!("Failed to update .gitignore: {e}");
    }
    Ok(fetched(fs::metadata(&target)?.len(), false))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Config;
    use crate::error::backend_error_code;
    use crate::testutil::{create_file, create_temp_git_repo, init_dvs_repo};
    use crate::{OperationContext, add_files};
    use tempfile::TempDir;

    /// A project where `data/a.csv` was added as old, new, old, new, and the
    /// hash of old.
    fn repo() -> (TempDir, Config, DvsPaths, String) {
        let (tmp, root) = create_temp_git_repo();
        let (config, _) = init_dvs_repo(&root);
        let paths = DvsPaths::new(root.clone(), root.clone(), ".dvs").unwrap();
        let mut hashes = Vec::new();
        for content in [b"old", b"new", b"old", b"new"] {
            let file = create_file(&root, "data/a.csv", content);
            hashes.push(Hashes::compute_from_path(&file, &[]).unwrap().0.blake3);
            let ctx = OperationContext::new();
            let compression = config.compression();
            add_files(
                vec!["data/a.csv".into()],
                &paths,
                config.backend(),
                None,
                compression,
                false,
                &ctx,
            )
            .unwrap();
        }
        (tmp, config, paths, hashes.swap_remove(0))
    }

    fn fetch(paths: &DvsPaths, config: &Config, name: &str, hash: &str) -> Result<Fetched> {
        let criteria = FetchCriteria::Hash(hash.to_string());
        fetch_file(Path::new(name), &criteria, paths, config.backend())
    }

    #[test]
    fn a_version_is_found_by_its_hash_or_the_start_of_it() {
        let (_tmp, config, paths, old) = repo();

        for (name, hash) in [
            ("data/a.csv", &old[..SHORT_HASH_LEN - 1]),
            ("data/a.csv", "zzzzzzzzzzzz"),
            ("data/a.csv", "000000000000"),
            ("never-added.csv", &old[..]),
            ("../outside.csv", &old[..]),
        ] {
            let error = fetch(&paths, &config, name, hash).unwrap_err();
            let code = backend_error_code(&error);
            assert_eq!(
                code,
                Some(ErrorCode::InvalidArguments),
                "{name} {hash}: {error:#}"
            );
        }
        assert!(!paths.repo_root().join(SCRATCHPAD_FOLDER_NAME).exists());

        // Added twice, still one version
        for hash in [&old[..], &old[..SHORT_HASH_LEN], &old.to_uppercase()] {
            let fetched = fetch(&paths, &config, "data/a.csv", hash).unwrap();
            assert_eq!(fetched.hash, old, "{hash}");
        }
    }

    #[test]
    fn the_version_is_written_to_the_scratchpad_and_nothing_else_changes() {
        let (_tmp, config, paths, old) = repo();
        let root = paths.repo_root();
        let record = fs::read(paths.metadata_path(Path::new("data/a.csv"))).unwrap();
        let in_data = DvsPaths::new(root.join("data"), root.to_path_buf(), ".dvs").unwrap();
        let fetch = || fetch(&in_data, &config, "a.csv", &old[..SHORT_HASH_LEN]).unwrap();
        let copy = Path::new(SCRATCHPAD_FOLDER_NAME)
            .join(&old[..SHORT_HASH_LEN])
            .join("data/a.csv");

        let first = fetch();
        assert_eq!(first.written_to, Path::new("..").join(&copy));
        assert_eq!(
            (first.hash.as_str(), first.size, first.already_there),
            (&old[..], 3, false)
        );
        assert_eq!(fs::read(root.join(&copy)).unwrap(), b"old");
        assert!(fetch().already_there);

        // An edited copy is replaced
        fs::write(root.join(&copy), b"edited").unwrap();
        assert!(!fetch().already_there);
        assert_eq!(fs::read(root.join(&copy)).unwrap(), b"old");

        assert_eq!(fs::read(root.join("data/a.csv")).unwrap(), b"new");
        assert_eq!(
            fs::read(paths.metadata_path(Path::new("data/a.csv"))).unwrap(),
            record
        );
        let gitignore = fs::read_to_string(root.join(".gitignore")).unwrap();
        assert_eq!(gitignore, format!("/{SCRATCHPAD_FOLDER_NAME}\n"));
    }

    #[test]
    fn a_failed_fetch_leaves_nothing_behind() {
        let (_tmp, config, paths, old) = repo();
        let root = paths.repo_root();
        let failure = || {
            fetch(&paths, &config, "data/a.csv", &old)
                .unwrap_err()
                .to_string()
        };

        #[cfg(unix)]
        {
            // The hash folder is a symlink to the project root
            let folder = root
                .join(SCRATCHPAD_FOLDER_NAME)
                .join(&old[..SHORT_HASH_LEN]);
            fs::create_dir(root.join(SCRATCHPAD_FOLDER_NAME)).unwrap();
            std::os::unix::fs::symlink("..", &folder).unwrap();
            // Refused whether or not the working file is already that version
            for content in [&b"new"[..], b"old"] {
                fs::write(root.join("data/a.csv"), content).unwrap();
                assert!(failure().contains("leads outside"));
                assert_eq!(fs::read(root.join("data/a.csv")).unwrap(), content);
            }
            fs::remove_dir_all(root.join(SCRATCHPAD_FOLDER_NAME)).unwrap();
        }

        let stored = Hashes {
            blake3: old.clone(),
            md5: None,
        };
        config.backend().remove(&stored).unwrap();
        assert!(failure().contains("not in storage"));
        assert!(!root.join(SCRATCHPAD_FOLDER_NAME).exists());
        assert!(!root.join(".gitignore").exists());
    }
}
