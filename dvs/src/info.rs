//! What `info` reports about a project: the fields of `dvs info --json`,
//! which R's `dvs_info()` returns too.

use std::path::PathBuf;

use serde::Serialize;
use url::Url;

use crate::auth::get_token;
use crate::backends::Backend;
use crate::backends::server::{AccessUser, ServerBackend};
use crate::config::{Config, OnConflict};
use crate::error::{Error, ErrorCode, backend_error_code};
use crate::paths::DvsPaths;

/// A project's configuration and, for a server project, the login and the
/// server's answer.
#[derive(Serialize)]
pub struct ProjectInfo {
    pub version: &'static str,
    #[serde(flatten)]
    pub backend: BackendInfo,
    /// What get does with local changes, `None` when the setting is invalid
    pub on_conflict: Option<OnConflict>,
    /// Where `on_conflict` was set, `None` when the setting is invalid
    pub on_conflict_source: Option<&'static str>,
    pub project_root: PathBuf,
    pub metadata_directory: PathBuf,
}

// One value per `info` call, so the size of the server variant does not matter
#[allow(clippy::large_enum_variant)]
#[derive(Serialize)]
#[serde(tag = "backend", rename_all = "snake_case")]
pub enum BackendInfo {
    Local {
        storage: PathBuf,
        compression: String,
        /// Always `not_applicable`
        connection: &'static str,
    },
    Server {
        project: String,
        server: Url,
        /// Set on the server and not kept in dvs.toml, so always `None`
        compression: Option<String>,
        /// A compression line in dvs.toml, which the server's setting overrides
        ignored_compression: Option<String>,
        logged_in: bool,
        /// `available`, `unavailable`, or `unknown` when not logged in
        connection: &'static str,
        /// Why `connection` is `unavailable`
        connection_error: Option<Error>,
        user: Option<AccessUser>,
        /// The server's level (`read`, `write`, `admin`), `not_logged_in`,
        /// `none` without membership, or `None` when the server could not say
        access: Option<String>,
        /// `None` when the server could not say
        groups: Option<Vec<String>>,
    },
}

/// What `info` reports. Not being logged in, having no access and a server
/// that does not answer are part of the answer. A credentials store that
/// cannot be read and a rejected login (`auth_expired`) are errors.
pub fn project_info(config: &Config, paths: &DvsPaths) -> anyhow::Result<ProjectInfo> {
    let on_conflict = config.get_on_conflict(None).ok();
    let backend = match config.backend() {
        Backend::Local(l) => BackendInfo::Local {
            storage: l.path.clone(),
            compression: config.compression().to_string(),
            connection: "not_applicable",
        },
        Backend::Server(s) => server_info(config, s)?,
    };
    Ok(ProjectInfo {
        version: crate::VERSION,
        backend,
        on_conflict: on_conflict.as_ref().map(|(policy, _)| *policy),
        on_conflict_source: on_conflict.as_ref().map(|(_, source)| source.as_str()),
        project_root: paths.repo_root().to_path_buf(),
        metadata_directory: paths.metadata_folder(),
    })
}

fn server_info(config: &Config, s: &ServerBackend) -> anyhow::Result<BackendInfo> {
    let logged_in = match get_token(&s.url) {
        Ok(token) => token.is_some(),
        Err(error) if backend_error_code(&error) == Some(ErrorCode::AuthRequired) => false,
        Err(error) => {
            return Err(Error::new(
                ErrorCode::CredentialsUnreadable,
                format!("Cannot read credentials: {error}"),
            )
            .into());
        }
    };
    let (access, connection, connection_error) = if logged_in {
        match s.access() {
            Ok(access) => (access, "available", None),
            // A 403 is an answer (no membership). A rejected login stays an error.
            Err(error) => match backend_error_code(&error) {
                Some(ErrorCode::AccessDenied) => (None, "available", None),
                Some(
                    code @ (ErrorCode::ServerUnavailable
                    | ErrorCode::ProjectNotFound
                    | ErrorCode::ServerTooOld),
                ) => (
                    None,
                    "unavailable",
                    Some(Error::new(code, format!("{error:#}"))),
                ),
                _ => return Err(error),
            },
        }
    } else {
        (None, "unknown", None)
    };
    let level = if !logged_in {
        Some("not_logged_in".to_string())
    } else if connection == "available" {
        Some(
            access
                .as_ref()
                .map_or("none", |a| a.access.as_str())
                .to_string(),
        )
    } else {
        None
    };
    let groups = if connection == "unavailable" {
        None
    } else {
        Some(access.as_ref().map_or_else(Vec::new, |a| a.groups.clone()))
    };
    Ok(BackendInfo::Server {
        project: s.name.clone(),
        server: s.url.clone(),
        compression: None,
        ignored_compression: config.configured_compression().map(|c| c.to_string()),
        logged_in,
        connection,
        connection_error,
        user: access.map(|a| a.user),
        access: level,
        groups,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testutil::{create_temp_git_repo, init_dvs_repo};

    #[test]
    fn a_local_project_reports_its_storage_and_compression() {
        let (_tmp, root) = create_temp_git_repo();
        let (config, _) = init_dvs_repo(&root);
        let paths =
            DvsPaths::new(root.clone(), root.clone(), config.metadata_folder_name()).unwrap();
        let info = serde_json::to_value(project_info(&config, &paths).unwrap()).unwrap();
        assert_eq!(info["backend"], "local");
        assert_eq!(info["compression"], "zstd");
        assert_eq!(info["connection"], "not_applicable");
        assert_eq!(info["on_conflict"], "abort");
        assert_eq!(info["on_conflict_source"], "default");
        assert!(info.get("logged_in").is_none());
    }
}
