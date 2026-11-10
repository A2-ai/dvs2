//! `get --version`: an earlier version of one tracked file, written to a side
//! file. It runs through `get_files` like any get; this module only builds
//! the request and finds the version in the file's history.

use std::path::{Path, PathBuf};

use anyhow::Result;

use crate::audit::Action;
use crate::error::{Error, ErrorCode};
use crate::files::get::normalize;
use crate::{Backend, Compression, DvsPaths, Hashes, PathFilter};

const MIN_PREFIX: usize = 8;

/// One tracked file at an earlier version, and where to write it.
#[derive(Debug, Clone)]
pub struct VersionRequest {
    /// The tracked file, relative to the project root
    pub path: PathBuf,
    /// A hash or a prefix of one, lowercase
    pub version: String,
    /// The side file, absolute
    pub to: PathBuf,
}

fn invalid(message: String) -> anyhow::Error {
    Error::new(ErrorCode::InvalidArguments, message).into()
}

impl VersionRequest {
    /// `user_paths` must name one file; it and `to` are relative to the
    /// current directory.
    pub fn new(
        paths: &DvsPaths,
        user_paths: Vec<PathBuf>,
        version: &str,
        to: &Path,
    ) -> Result<Self> {
        let [user_path] = <[_; 1]>::try_from(user_paths)
            .map_err(|_| invalid("--version takes exactly one file".into()))?;
        if paths.cwd().join(&user_path).is_dir() {
            return Err(invalid(format!(
                "--version takes a file, not a folder: {}",
                user_path.display()
            )));
        }
        let filter = PathFilter::from_user_paths(vec![user_path.clone()], false, paths);
        let path = match filter.paths() {
            [relative] if !relative.as_os_str().is_empty() => relative.clone(),
            _ => {
                return Err(invalid(format!(
                    "--version takes a file inside the project: {}",
                    user_path.display()
                )));
            }
        };
        let version = version.to_ascii_lowercase();
        if version.len() < MIN_PREFIX || !version.bytes().all(|c| c.is_ascii_hexdigit()) {
            return Err(invalid(format!(
                "--version takes a hash or a prefix of at least {MIN_PREFIX} hex characters"
            )));
        }
        Ok(VersionRequest {
            path,
            version,
            to: normalize(&paths.cwd().join(to)),
        })
    }
}

/// The hash and compression of the one stored version of `path` that starts
/// with `version`.
pub(crate) fn resolve(
    backend: &Backend,
    path: &Path,
    version: &str,
) -> Result<(Hashes, Compression)> {
    let mut found: Vec<(Hashes, Compression)> = Vec::new();
    for entry in backend.get_audit_entries(&[path.to_path_buf()])? {
        if let Action::Add {
            file, compression, ..
        } = entry.action
            && file.path == path
            && file.hashes.blake3.starts_with(version)
            && !found.iter().any(|(h, _)| h.blake3 == file.hashes.blake3)
        {
            found.push((file.hashes, compression));
        }
    }
    match found.len() {
        1 => Ok(found.remove(0)),
        0 => Err(Error::new(
            ErrorCode::FileFailed,
            format!("{} has no version {version}", path.display()),
        )
        .into()),
        n => Err(invalid(format!(
            "{version} matches {n} versions of {}; give more of the hash",
            path.display()
        ))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testutil::{create_temp_git_repo, init_dvs_repo};

    #[test]
    fn a_short_prefix_or_two_files_are_refused() {
        let (_tmp, root) = create_temp_git_repo();
        let (config, _) = init_dvs_repo(&root);
        let paths = DvsPaths::new(root.clone(), root, config.metadata_folder_name()).unwrap();
        let to = Path::new("out.csv");

        let short = VersionRequest::new(&paths, vec!["a.csv".into()], "0123abc", to);
        let two = VersionRequest::new(&paths, vec!["a.csv".into(), "b.csv".into()], "0123abcd", to);

        for result in [short, two] {
            let code = result.unwrap_err().downcast_ref::<Error>().unwrap().code;
            assert_eq!(code, ErrorCode::InvalidArguments);
        }
    }
}
