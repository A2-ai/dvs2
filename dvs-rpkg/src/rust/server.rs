//! The commands for server projects, as in the CLI: info.

use dvs::paths::DvsPaths;
use miniextendr_api::miniextendr;
use miniextendr_api::serde::AsSerialize;

use crate::{DvsError, find_config};

/// @title Show the project's configuration and its server
/// @description
/// Reports the dvs project that holds the working directory, as
/// `dvs info --json` does. For a server project it also says whether you are
/// logged in, whether the server answers, and what you can do on the project.
/// Not being logged in, having no access and a server that does not answer
/// are part of the answer, not errors.
///
/// @return A list with the fields of `dvs info --json`: `version`, `backend`
///   (`"local"` or `"server"`), `on_conflict` and `on_conflict_source` (`NULL`
///   when the setting is invalid), `project_root` and `metadata_directory`.
///   A local project adds `storage`, `compression` (`"zstd"` or `"none"`) and
///   `connection` (`"not_applicable"`). A server project adds `project`,
///   `server`, `compression` (`NULL`: the server sets it),
///   `ignored_compression` (a `compression` line in `dvs.toml`, which the
///   server's setting overrides, `NULL` without one), `logged_in`,
///   `connection` (`"available"`, `"unavailable"`, or `"unknown"` when not
///   logged in), `connection_error` (`code` and `message` when `connection`
///   is `"unavailable"`), `user` (`username` and `email`), `access`
///   (`"read"`, `"write"`, `"admin"`, `"none"` without membership,
///   `"not_logged_in"`, or `NULL` when the server could not say) and `groups`.
///
///   A credentials store that cannot be read raises a `dvs_error` with `code`
///   `"credentials_unreadable"`, and a login the server rejects one with
///   `"auth_expired"`.
/// @keywords internal
#[miniextendr(r_name = "dvs_info_impl", internal, call = caller)]
pub(crate) fn dvs_info() -> Result<AsSerialize<dvs::info::ProjectInfo>, DvsError> {
    let config = find_config()?;
    let paths = DvsPaths::from_cwd(&config)?;
    Ok(AsSerialize(dvs::info::project_info(&config, &paths)?))
}
