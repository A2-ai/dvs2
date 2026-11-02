//! The commands for server projects, as in the CLI: init server, attach, info,
//! login, logout and availability.

use dvs::ErrorCode;
use dvs::auth::{
    Logout, TOKEN_POLL_INTERVAL, get_client_approve_url, get_token, initiate_device_approval,
    poll_token_once, store_token,
};
use dvs::config::Config;
use dvs::init::{attach, check_attachable, check_uninitialized, init};
use dvs::paths::DvsPaths;
use miniextendr_api::dataframe::IntoDataFrame;
use miniextendr_api::serde::AsSerialize;
use miniextendr_api::{BuiltDataFrame, DataFrameRow, miniextendr};
use serde::Serialize;
use url::Url;

use crate::{CompressionChoice, DvsError, find_config};

/// The URL of the project's dvs server. A local project is an error, as in
/// the CLI.
fn server_url(config: &Config) -> Result<&Url, DvsError> {
    config
        .server_url()
        .ok_or_else(|| anyhow::anyhow!("This project is not using a dvs server").into())
}

fn parse_url(url: &str, param: &str) -> Result<Url, DvsError> {
    Url::parse(url).map_err(|e| {
        DvsError::new(
            ErrorCode::InvalidArguments,
            format!("invalid value '{url}' for `{param}`: {e}"),
        )
    })
}

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

/// The device login that `dvs_login()` waits on.
#[derive(Serialize)]
pub(crate) struct DeviceLogin {
    server: String,
    approve_url: String,
    user_code: String,
    device_code: String,
    /// Seconds between two asks
    interval: f64,
}

/// @title Log in to a dvs server
/// @description
/// Logs in to the dvs server of the project that holds the working
/// directory, or to `url`, as `dvs login` does. It shows a web address and a
/// code, then waits until you enter the code there, up to about ten minutes.
/// The token goes where the CLI keeps it, so one login serves both.
///
/// It needs an interactive R session. Elsewhere it raises a `dvs_error` with
/// `code` `"auth_required"`, as `dvs login --json` does. A local project
/// raises one with `"operation_failed"`.
///
/// @param url The URL of the dvs server, if not the project's.
/// @param interactive_session Whether R is interactive.
/// @return The server, the web address and code to show, the device code to
///   ask with and the seconds between two asks.
/// @keywords internal
#[miniextendr(r_name = "dvs_login_start_impl", internal, call = caller)]
pub(crate) fn dvs_login_start(
    #[miniextendr(default = "NULL", no_na)] url: Option<String>,
    #[miniextendr(no_na)] interactive_session: bool,
) -> Result<AsSerialize<DeviceLogin>, DvsError> {
    let server = match url {
        Some(url) => parse_url(&url, "url")?,
        None => server_url(&find_config()?)?.clone(),
    };
    if !interactive_session {
        return Err(DvsError::new(
            ErrorCode::AuthRequired,
            "Device authorization requires interactive login; run dvs_login() in an interactive R session",
        ));
    }
    let device = initiate_device_approval(&server)?;
    let approve_url = get_client_approve_url(&server, &device.user_code)?;
    Ok(AsSerialize(DeviceLogin {
        server: server.to_string(),
        approve_url: approve_url.to_string(),
        user_code: device.user_code,
        device_code: device.device_code,
        interval: TOKEN_POLL_INTERVAL.as_secs_f64(),
    }))
}

/// @title Ask once whether a device login was approved
/// @description
/// One step of the wait in `dvs_login()`. Stores the token once the login is
/// approved.
///
/// @param server The `server` of `dvs_login_start_impl()`.
/// @param device_code Its `device_code`.
/// @param attempt How many times it asked before. Past the limit the code
///   has expired, and that is an error.
/// @return `TRUE` once logged in, `FALSE` while the login waits.
/// @keywords internal
#[miniextendr(r_name = "dvs_login_poll_impl", internal, call = caller)]
pub(crate) fn dvs_login_poll(
    #[miniextendr(no_na)] server: String,
    #[miniextendr(no_na)] device_code: String,
    #[miniextendr(no_na)] attempt: i32,
) -> Result<bool, DvsError> {
    let server = parse_url(&server, "server")?;
    let attempt = u32::try_from(attempt).unwrap_or_default();
    match poll_token_once(&server, &device_code, attempt)? {
        Some(token) => {
            store_token(&server, &token.access_token)?;
            Ok(true)
        }
        None => Ok(false),
    }
}

/// @title Log out of the dvs server
/// @description
/// Logs out of the dvs server of the project that holds the working
/// directory, as `dvs logout` does: asks the server to revoke your token,
/// then deletes it from this computer. The token is deleted even when the
/// server cannot revoke it. That is a warning in the result, not an error.
/// Not being logged in is not an error either.
///
/// @return A list with the fields of `dvs logout --json`: `revoked` (`TRUE`
///   when the server revoked the token, `FALSE` when it could not, `NULL`
///   when you were not logged in) and `warnings`, a tibble with `code` and
///   `message`. It has a `"revoke_failed"` row with the reason when the server
///   could not revoke the token.
///
///   A local project raises a `dvs_error` with `code` `"operation_failed"`.
/// @keywords internal
#[miniextendr(r_name = "dvs_logout_impl", internal, call = caller)]
pub(crate) fn dvs_logout() -> Result<AsSerialize<Logout>, DvsError> {
    let config = find_config()?;
    let logout = dvs::auth::logout(server_url(&config)?).map_err(|e| DvsError::from(e.error))?;
    Ok(AsSerialize(logout))
}

/// @title Create a project on a dvs server
/// @description
/// Creates the project `name` on the dvs server at `url` and makes the
/// working directory its dvs project, as `dvs init server` does. Without a
/// stored token for the server it logs in first, as `dvs_login()` does, after
/// the checks that need no server: a `dvs.toml`, or a metadata folder with
/// files in it, is an error before any login.
///
/// @param name The project name on the server.
/// @param url The URL of the dvs server.
/// @param group The server group whose members can read and write the
///   project. You must be one of them.
/// @param metadata_folder_name Name of the metadata folder. Defaults to `.dvs`.
/// @param compression Compression method for stored files. One of `"zstd"`
///   (default) or `"none"`. The server keeps it, so `dvs.toml` has no
///   `compression` line.
/// @return The project root, or `NA` when there is no token for the server
///   yet. Then nothing was written.
/// @keywords internal
#[miniextendr(r_name = "dvs_init_server_impl", internal, call = caller)]
pub(crate) fn dvs_init_server(
    #[miniextendr(no_na)] name: String,
    #[miniextendr(no_na)] url: String,
    #[miniextendr(no_na)] group: String,
    #[miniextendr(default = "NULL", no_na)] metadata_folder_name: Option<String>,
    #[miniextendr(match_arg, default = "\"zstd\"")] compression: CompressionChoice,
) -> Result<Option<String>, DvsError> {
    let url = parse_url(&url, "url")?;
    let root = std::env::current_dir()?;
    let mut config = Config::new_server(name, url.clone(), Some(group));
    config.set_compression(Some(compression.into()));
    if let Some(m) = metadata_folder_name {
        config.set_metadata_folder_name(m);
    }
    check_uninitialized(&root, &config)?;
    if get_token(&url)?.is_none() {
        return Ok(None);
    }
    Ok(Some(init(&root, config)?.to_string_lossy().into_owned()))
}

/// @title Attach to a project on a dvs server
/// @description
/// Makes the working directory a dvs project of the project `name` that
/// exists on the dvs server at `url`, as `dvs attach` does. It checks that the
/// server has the project and that you can reach it, then writes the metadata
/// folder and `dvs.toml`. Without a stored token for the server it logs in
/// first, as `dvs_login()` does. A `dvs.toml` in the working directory is an
/// error before any login.
///
/// @param name The project name on the server.
/// @param url The URL of the dvs server.
/// @param metadata_folder_name Name of the metadata folder. Defaults to `.dvs`.
/// @return The project root, or `NA` when there is no token for the server
///   yet. Then nothing was written.
/// @keywords internal
#[miniextendr(r_name = "dvs_attach_impl", internal, call = caller)]
pub(crate) fn dvs_attach(
    #[miniextendr(no_na)] name: String,
    #[miniextendr(no_na)] url: String,
    #[miniextendr(default = "NULL", no_na)] metadata_folder_name: Option<String>,
) -> Result<Option<String>, DvsError> {
    let url = parse_url(&url, "url")?;
    let root = std::env::current_dir()?;
    check_attachable(&root)?;
    if get_token(&url)?.is_none() {
        return Ok(None);
    }
    attach(&root, name, url, metadata_folder_name)?;
    Ok(Some(root.to_string_lossy().into_owned()))
}

/// One row of the `dvs_availability()` frame.
#[derive(DataFrameRow)]
struct AvailabilityFileRow {
    hash: String,
    state: String,
    code: Option<String>,
    message: Option<String>,
}

/// @title Check whether content is stored
/// @description
/// Looks up each BLAKE3 hash in the project's storage without downloading
/// anything, as `dvs availability` does: the local storage folder, or the dvs
/// server. The hashes are the ones `dvs_status()` and `dvs_add()` report, and
/// a hash may be given more than once.
///
/// A storage it cannot read (no login, no access, a server that does not
/// answer, a missing storage folder) is not an error: every row is
/// `"unknown"`, with the reason.
///
/// @param hashes BLAKE3 hashes, 64 hex digits each, in any case.
/// @return A tibble with one row per hash, in the order given: `hash` (in
///   lowercase), `state` (`"present"`, `"absent"`, or `"unknown"` when the
///   storage could not say), and `code` and `message` (why the storage could
///   not say, `NA` otherwise), the fields of `dvs availability --json`.
///
///   A hash that is not 64 hex digits, or no hashes, raises a `dvs_error`
///   with `code` `"invalid_arguments"` before any lookup.
/// @keywords internal
#[miniextendr(r_name = "dvs_availability_impl", internal, call = caller)]
pub(crate) fn dvs_availability(
    #[miniextendr(no_na)] hashes: Vec<String>,
) -> Result<BuiltDataFrame, DvsError> {
    let config = find_config()?;
    let rows = dvs::availability(config.backend(), &hashes)?;
    Ok(rows
        .into_iter()
        .map(|row| AvailabilityFileRow {
            hash: row.hash,
            state: row.state.as_str().to_string(),
            code: row.code.map(|code| code.as_str().to_string()),
            message: row.message,
        })
        .collect::<Vec<_>>()
        .into_dataframe()?)
}
