//! The functions that ask a dvs server: login, logout and info; attaching a checkout;
//! projects, access, the files uploaded, a path's versions and activity; and how local
//! files compare with the server. Every request is a `dvs` library call; this module
//! only shapes the answers for R. Access changes, project creation and yanks stay in
//! the CLI.
//!
//! Lists come back a page at a time through a [`Pager`], so the R side can stop
//! between pages (an interrupt, or `max_rows`).

use std::collections::HashMap;
use std::path::Path;
use std::sync::{Arc, Mutex};

use anyhow::Result;
use miniextendr_api::externalptr::ExternalPtr;
use miniextendr_api::serde::AsSerialize;
use miniextendr_api::{DataFrame, miniextendr};
use serde::Serialize;
use url::Url;
use uuid::Uuid;

use dvs::api::v1::{
    ActivityItem, FileStatus as RemoteStatus, FileView, GrantView, Page, ProjectView, StatusFile,
    StatusRelation, VersionView,
};
use dvs::auth::{delete_token_if, get_token, store_token};
use dvs::backends::server::ServerBackend;
use dvs::config::Config;
use dvs::init::write_attached;
use dvs::paths::{CONFIG_FILE_NAME, DvsPaths};
use dvs::queries::{ActivityQuery, FilesQuery, ProjectsQuery};
use dvs::remote::{self, LoginPoll, PendingLogin, TOKEN_ENV, TokenSource, token_for};
use dvs::{Backend, DvsError, ErrorCode, ProjectPath, parse_size, since};

use crate::conditions::{client_error, or_raise, raise};
use crate::interrupt::off_r_thread;

/// Rows a page asks for; the server may give fewer.
const PAGE_SIZE: usize = 1000;

// region: Where to ask

fn config_here() -> Result<Option<Config>> {
    let cwd = std::env::current_dir()?;
    Config::find(&cwd).transpose()
}

/// The server project the working directory is in.
fn project_here() -> Result<ServerBackend> {
    match config_here()?.as_ref().map(Config::backend) {
        Some(Backend::Server(s)) => Ok(s.clone()),
        Some(Backend::Local(_)) => Err(DvsError::new(
            ErrorCode::NotAServerProject,
            "This works on server projects only: this project keeps its files in local storage",
        )
        .into()),
        None => Err(DvsError::new(
            ErrorCode::NotInRepository,
            "Not in a DVS repository; use dvs_attach() to attach this folder to a server project",
        )
        .into()),
    }
}

fn parse_url(url: &str) -> Result<Url> {
    Url::parse(url)
        .map_err(|e| client_error("invalid_argument", format!("{url:?} is not a URL: {e}")))
}

/// `url`, or the server of the project the working directory is in.
fn server_url(url: Option<String>) -> Result<Url> {
    if let Some(url) = url {
        return parse_url(&url);
    }
    match config_here()?.as_ref().map(Config::backend) {
        Some(Backend::Server(s)) => Ok(s.url.clone()),
        Some(Backend::Local(_)) => Err(DvsError::new(
            ErrorCode::NotAServerProject,
            "This project keeps its files in local storage, not on a dvs server; give `url` \
             to ask a server",
        )
        .into()),
        None => Err(DvsError::new(
            ErrorCode::NotInRepository,
            "Not in a DVS repository; give the server's `url`",
        )
        .into()),
    }
}

/// A path as the server names it (from the project's top folder), given relative to
/// the working directory; a trailing `/` (a folder) is kept.
fn project_path(path: &str) -> Result<String> {
    let Some(config) = config_here()? else {
        return Ok(path.trim_start_matches("./").to_string());
    };
    let paths = DvsPaths::from_cwd(&config)?;
    let mut relative = paths.project_relative(Path::new(path)).ok_or_else(|| {
        client_error(
            "invalid_argument",
            format!("{path} is outside this project"),
        )
    })?;
    if path.ends_with('/') && !relative.is_empty() {
        relative.push('/');
    }
    Ok(relative)
}

/// `since` (7d, 12h, 2w) as a time, or `from`.
fn time_from(since_value: Option<String>, from: Option<String>) -> Result<Option<String>> {
    match (since_value, from) {
        (Some(_), Some(_)) => Err(client_error(
            "invalid_argument",
            "give `since` or `from`, not both",
        )),
        (Some(s), None) => since(&s)
            .map(Some)
            .map_err(|e| client_error("invalid_argument", format!("since: {e}"))),
        (None, from) => Ok(from),
    }
}

fn size_arg(name: &str, value: Option<String>) -> Result<Option<u64>> {
    value
        .map(|v| {
            parse_size(&v).map_err(|e| client_error("invalid_argument", format!("{name}: {e}")))
        })
        .transpose()
}

// endregion

// region: Pages

/// The rows of a page, made into a data frame on R's thread.
type Rows = Box<dyn FnOnce() -> Result<DataFrame> + Send>;
type FetchPage = Box<dyn FnMut(Option<String>) -> Result<(Rows, Option<String>)> + Send>;

struct PagerState {
    cursor: Option<String>,
    done: bool,
}

/// A list read a page at a time. Each page is fetched off R's thread, so waiting for
/// it can be interrupted.
#[derive(miniextendr_api::ExternalPtr)]
pub struct Pager {
    fetch: Arc<Mutex<FetchPage>>,
    state: Mutex<PagerState>,
}

impl Pager {
    fn new<T: 'static, R: Serialize + Send + 'static>(
        mut page: impl FnMut(Option<String>) -> Result<Page<T>> + Send + 'static,
        row: impl Fn(&T) -> R + Send + 'static,
    ) -> ExternalPtr<Pager> {
        let fetch: FetchPage = Box::new(move |cursor| {
            let page = page(cursor)?;
            let rows: Vec<R> = page.items.iter().map(&row).collect();
            let rows: Rows = Box::new(move || Ok(miniextendr_api::serde::vec_to_dataframe(&rows)?));
            Ok((rows, page.next_cursor))
        });
        ExternalPtr::new(Pager {
            fetch: Arc::new(Mutex::new(fetch)),
            state: Mutex::new(PagerState {
                cursor: None,
                done: false,
            }),
        })
    }
}

/// The next page of a list, as a data frame; an empty one once the list has ended.
///
/// @param pager A pager from one of the list functions.
/// @keywords internal
#[miniextendr(r_name = "dvs_pager_next_impl")]
pub(crate) fn dvs_pager_next(pager: ExternalPtr<Pager>) -> DataFrame {
    let cursor = {
        let mut state = pager.state.lock().unwrap_or_else(|p| p.into_inner());
        if state.done {
            return or_raise(
                miniextendr_api::serde::vec_to_dataframe::<u8>(&[]).map_err(Into::into),
            );
        }
        state.cursor.take()
    };
    let fetch = Arc::clone(&pager.fetch);
    let page = off_r_thread(move || {
        let mut fetch = fetch.lock().unwrap_or_else(|p| p.into_inner());
        fetch(cursor)
    });
    let mut state = pager.state.lock().unwrap_or_else(|p| p.into_inner());
    match page.and_then(|(rows, next)| Ok((rows()?, next))) {
        Ok((rows, next)) => {
            state.done = next.is_none();
            state.cursor = next;
            rows
        }
        Err(e) => {
            state.done = true;
            drop(state);
            raise(e)
        }
    }
}

/// Whether a list has no more pages.
///
/// @param pager A pager from one of the list functions.
/// @keywords internal
#[miniextendr(r_name = "dvs_pager_done_impl")]
pub(crate) fn dvs_pager_done(pager: ExternalPtr<Pager>) -> bool {
    pager.state.lock().unwrap_or_else(|p| p.into_inner()).done
}

// endregion

// region: Login

/// Polls in a row that got no answer and still count as waiting: a proxy closing an
/// idle connection, or a moment of network trouble.
const MAX_UNANSWERED_POLLS: u32 = 3;

/// A device login waiting for approval. Each poll is one request made when R asks
/// for it, so when R stops waiting nothing goes on polling.
#[derive(miniextendr_api::ExternalPtr)]
pub struct LoginHandle {
    url: Url,
    wait: Mutex<LoginWait>,
}

struct LoginWait {
    pending: PendingLogin,
    unanswered: u32,
}

#[derive(Serialize)]
pub(crate) struct LoginState {
    url: String,
    /// Where to approve the login, and the code that page must show
    uri: String,
    code: String,
    /// Seconds to wait before the next poll
    interval: f64,
    done: bool,
    /// When the token expires, once the login is done
    expires_at: Option<String>,
}

impl LoginHandle {
    fn state(&self, wait: &LoginWait, expires_at: Option<String>) -> LoginState {
        let auth = &wait.pending.authorization;
        LoginState {
            url: self.url.to_string(),
            uri: auth.verification_uri_complete.clone(),
            code: auth.user_code.clone(),
            interval: wait.pending.interval as f64,
            done: expires_at.is_some(),
            expires_at,
        }
    }
}

/// Start a device login with the server: ask it for the code to approve.
///
/// @param url The server's URL, or `NULL` for the server of the project here.
/// @keywords internal
#[miniextendr(r_name = "dvs_login_start_impl")]
pub(crate) fn dvs_login_start(
    #[miniextendr(default = "NULL")] url: Option<String>,
) -> ExternalPtr<LoginHandle> {
    let url = or_raise(server_url(url));
    let base = url.clone();
    let pending = or_raise(off_r_thread(move || remote::start_login(&base)));
    ExternalPtr::new(LoginHandle {
        url,
        wait: Mutex::new(LoginWait {
            pending,
            unanswered: 0,
        }),
    })
}

/// Where a login is, without asking the server: its approval URL and code, and how
/// long to wait before polling.
///
/// @param handle The handle `dvs_login_start_impl()` returned.
/// @keywords internal
#[miniextendr(r_name = "dvs_login_state_impl")]
pub(crate) fn dvs_login_state(handle: ExternalPtr<LoginHandle>) -> AsSerialize<LoginState> {
    let wait = handle.wait.lock().unwrap_or_else(|p| p.into_inner());
    AsSerialize(handle.state(&wait, None))
}

/// Ask the server once whether the login was approved; once it was, the token is
/// stored. Raises `login_expired` or `login_denied` when it will not be.
///
/// @param handle The handle `dvs_login_start_impl()` returned.
/// @keywords internal
#[miniextendr(r_name = "dvs_login_poll_impl")]
pub(crate) fn dvs_login_poll(handle: ExternalPtr<LoginHandle>) -> AsSerialize<LoginState> {
    let pending = handle
        .wait
        .lock()
        .unwrap_or_else(|p| p.into_inner())
        .pending
        .clone();
    let base = handle.url.clone();
    // The token is stored here, after an answer R waited for: an interrupted poll that
    // ends in an approval stores nothing
    let poll = off_r_thread(move || remote::poll_login_unstored(&base, &pending));
    let mut wait = handle.wait.lock().unwrap_or_else(|p| p.into_inner());
    let expires_at = match poll {
        Ok(LoginPoll::Done(token)) => {
            if let Err(e) = store_token(&handle.url, &token.access_token) {
                drop(wait);
                raise(e)
            }
            Some(token.expires_at.to_string())
        }
        Ok(LoginPoll::Pending) => {
            wait.unanswered = 0;
            None
        }
        Ok(LoginPoll::SlowDown) => {
            wait.unanswered = 0;
            wait.pending.slow_down();
            None
        }
        Err(e) if remote::is_no_response(&e) && wait.unanswered < MAX_UNANSWERED_POLLS => {
            wait.unanswered += 1;
            None
        }
        Err(e) => {
            drop(wait);
            raise(e)
        }
    };
    AsSerialize(handle.state(&wait, expires_at))
}

/// Whether there is a token for the server: `DVS_TOKEN`, or one `dvs_login()` stored.
///
/// @param url The server's URL.
/// @keywords internal
#[miniextendr(r_name = "dvs_has_token_impl")]
pub(crate) fn dvs_has_token(url: String) -> bool {
    or_raise((|| Ok(token_for(&parse_url(&url)?)?.is_some()))())
}

#[derive(Serialize)]
pub(crate) struct LogoutResult {
    url: String,
    logged_out: bool,
    notes: Vec<String>,
}

/// Revoke the stored token on the server and forget it here.
///
/// @param url The server's URL, or `NULL` for the server of the project here.
/// @keywords internal
#[miniextendr(r_name = "dvs_logout_impl")]
pub(crate) fn dvs_logout(
    #[miniextendr(default = "NULL")] url: Option<String>,
) -> AsSerialize<LogoutResult> {
    or_raise(off_r_thread(move || {
        let url = server_url(url)?;
        let mut notes = Vec::new();
        let env_token = std::env::var(TOKEN_ENV)
            .ok()
            .map(|t| t.trim().to_string())
            .filter(|t| !t.is_empty());
        if env_token.is_some() {
            notes.push(format!(
                "{TOKEN_ENV} is set; it is not revoked. Unset it to stop using it."
            ));
        }
        let Some(token) = get_token(&url)? else {
            return Ok(AsSerialize(LogoutResult {
                url: url.to_string(),
                logged_out: false,
                notes,
            }));
        };
        if env_token.as_deref() == Some(token.as_str()) {
            notes.push(format!(
                "The stored token is the one in {TOKEN_ENV}, so it is not revoked; it is only \
                 forgotten here."
            ));
        } else {
            match remote::revoke(&url, &token) {
                Ok(_) => {}
                Err(e) if remote::is_no_response(&e) => notes.push(format!(
                    "Could not reach {url} to revoke the token ({e:#}). It is forgotten here, \
                     but stays valid on the server until it expires."
                )),
                Err(e) => notes.push(format!(
                    "{url} did not revoke the token ({e:#}). It is forgotten here, but may \
                     stay valid on the server until it expires."
                )),
            }
        }
        // Only the token revoked: a login since then (an interrupted logout's request
        // can end after one) is kept
        if !delete_token_if(&url, &token)? {
            notes.push(format!(
                "A newer login to {url} was stored meanwhile; it is kept."
            ));
        }
        Ok(AsSerialize(LogoutResult {
            url: url.to_string(),
            logged_out: true,
            notes,
        }))
    }))
}

// endregion

// region: Info and projects

#[derive(Serialize)]
pub(crate) struct InfoResult {
    server: String,
    server_version: String,
    api_versions: Vec<i32>,
    client_version: String,
    user: Option<String>,
    email: Option<String>,
    global_admin: Option<bool>,
    groups: Vec<String>,
    token_source: Option<&'static str>,
    token_expires_at: Option<String>,
    project: Option<String>,
    project_role: Option<String>,
    project_locked: Option<bool>,
}

/// Who you are on the server, where the token comes from and when it expires, the
/// server's and this client's versions, and your role on the project here.
///
/// @param url The server's URL, or `NULL` for the server of the project here.
/// @keywords internal
#[miniextendr(r_name = "dvs_info_impl")]
pub(crate) fn dvs_info(
    #[miniextendr(default = "NULL")] url: Option<String>,
) -> AsSerialize<InfoResult> {
    or_raise(off_r_thread(move || {
        let project = match &url {
            Some(_) => None,
            None => match config_here()?.as_ref().map(Config::backend) {
                Some(Backend::Server(s)) => Some(s.clone()),
                _ => None,
            },
        };
        let server = match &project {
            Some(p) => p.clone(),
            None => ServerBackend::new(String::new(), None, server_url(url)?),
        };
        let info = server.server_info()?;
        let source = server.token_source();
        let me = source.map(|_| server.me()).transpose()?;
        let role = match (&project, &me) {
            (Some(p), Some(_)) => Some(p.project()?),
            _ => None,
        };
        Ok(AsSerialize(InfoResult {
            server: server.url.to_string(),
            server_version: info.server_version,
            api_versions: info.api_versions.iter().map(|v| *v as i32).collect(),
            client_version: dvs::VERSION.to_string(),
            user: me
                .as_ref()
                .map(|m| m.username.clone().unwrap_or_else(|| m.sub.clone())),
            email: me.as_ref().and_then(|m| m.email.clone()),
            global_admin: me.as_ref().map(|m| m.global_admin),
            groups: me.as_ref().map(|m| m.groups.clone()).unwrap_or_default(),
            token_source: source.map(|s| match s {
                TokenSource::Environment => "DVS_TOKEN",
                TokenSource::Stored => "dvs_login",
            }),
            token_expires_at: me
                .as_ref()
                .and_then(|m| m.token.as_ref())
                .map(|t| t.expires_at.to_string()),
            project: project.as_ref().map(|p| p.name.clone()),
            project_role: role.as_ref().map(|r| r.your_role.to_string()),
            project_locked: role.as_ref().map(|r| r.locked),
        }))
    }))
}

#[derive(Serialize)]
pub(crate) struct ProjectResult {
    name: String,
    server: String,
    your_role: String,
    locked: bool,
    locked_by: Option<String>,
    locked_at: Option<String>,
    lock_reason: Option<String>,
    compression: String,
    created_at: String,
    created_by: String,
    repo_commit_url: Option<String>,
    files: Option<u64>,
    versions: Option<u64>,
    uploads: Option<u64>,
    size: Option<u64>,
    stored_size: Option<u64>,
    last_upload_at: Option<String>,
    last_activity_at: Option<String>,
    grant_group: Vec<String>,
    grant_role: Vec<String>,
    grant_removed: Vec<bool>,
    grant_group_id: Vec<String>,
}

fn project_result(p: ProjectView, server: &Url) -> ProjectResult {
    let stats = p.stats.as_ref();
    let lock = p.lock.as_ref();
    ProjectResult {
        server: server.to_string(),
        your_role: p.your_role.to_string(),
        locked: p.locked,
        locked_by: lock.map(|l| l.locked_by.clone()),
        locked_at: lock.map(|l| l.locked_at.to_string()),
        lock_reason: lock.and_then(|l| l.reason.clone()),
        created_at: p.created_at.to_string(),
        files: stats.map(|s| s.paths),
        versions: stats.map(|s| s.versions),
        uploads: stats.map(|s| s.placements),
        size: stats.map(|s| s.logical_bytes),
        stored_size: stats.map(|s| s.unique_stored_bytes),
        last_upload_at: stats.and_then(|s| s.last_upload_at.map(|t| t.to_string())),
        last_activity_at: stats.and_then(|s| s.last_activity_at.map(|t| t.to_string())),
        grant_group: p.grants.iter().map(|g| g.group.clone()).collect(),
        grant_role: p.grants.iter().map(|g| g.role.to_string()).collect(),
        grant_removed: p.grants.iter().map(|g| g.removed).collect(),
        grant_group_id: p.grants.iter().map(|g| g.group_id.to_string()).collect(),
        name: p.name,
        compression: p.compression,
        created_by: p.created_by,
        repo_commit_url: p.repo_commit_url,
    }
}

/// Attach the working directory to a server project: check that it exists and that
/// you can read it, then write `dvs.toml` and create the metadata folder.
///
/// @param name The project's name on the server.
/// @param url The server's URL.
/// @param metadata_folder_name Name of the metadata folder. Defaults to `.dvs`.
/// @keywords internal
#[miniextendr(r_name = "dvs_attach_impl")]
pub(crate) fn dvs_attach(
    name: String,
    url: String,
    #[miniextendr(default = "NULL")] metadata_folder_name: Option<String>,
) -> AsSerialize<ProjectResult> {
    or_raise((|| {
        let url = parse_url(&url)?;
        let mut config = Config::new_server(name, url.clone(), None);
        if let Some(m) = metadata_folder_name {
            config.set_metadata_folder_name(m);
        }
        let root = std::env::current_dir()?;
        if root.join(CONFIG_FILE_NAME).exists() {
            return Err(DvsError::new(
                ErrorCode::AlreadyInitialized,
                "dvs is already initialized (dvs.toml exists)",
            )
            .into());
        }
        let Backend::Server(server) = config.backend() else {
            unreachable!("a server config")
        };
        let server = server.clone();
        // Only the lookup runs off R's thread: dvs.toml is written here, once R has
        // the answer, so an interrupted attach writes nothing
        let project = off_r_thread(move || server.project())?;
        write_attached(&root, config)?;
        Ok(AsSerialize(project_result(project, &url)))
    })())
}

/// The project here as the server describes it to you.
///
/// @keywords internal
#[miniextendr(r_name = "dvs_project_info_impl")]
pub(crate) fn dvs_project_info() -> AsSerialize<ProjectResult> {
    or_raise(off_r_thread(move || {
        let server = project_here()?;
        Ok(AsSerialize(project_result(server.project()?, &server.url)))
    }))
}

#[derive(Serialize)]
struct ProjectRow {
    name: String,
    your_role: String,
    locked: bool,
    locked_by: Option<String>,
    locked_at: Option<String>,
    lock_reason: Option<String>,
    files: Option<u64>,
    size: Option<u64>,
    stored_size: Option<u64>,
    last_activity_at: Option<String>,
    created_at: String,
    created_by: String,
}

fn project_row(p: &ProjectView) -> ProjectRow {
    let stats = p.stats.as_ref();
    let lock = p.lock.as_ref();
    ProjectRow {
        name: p.name.clone(),
        your_role: p.your_role.to_string(),
        locked: p.locked,
        locked_by: lock.map(|l| l.locked_by.clone()),
        locked_at: lock.map(|l| l.locked_at.to_string()),
        lock_reason: lock.and_then(|l| l.reason.clone()),
        files: stats.map(|s| s.paths),
        size: stats.map(|s| s.logical_bytes),
        stored_size: stats.map(|s| s.unique_stored_bytes),
        last_activity_at: stats.and_then(|s| s.last_activity_at.map(|t| t.to_string())),
        created_at: p.created_at.to_string(),
        created_by: p.created_by.clone(),
    }
}

/// The projects you can read on the server.
///
/// @param role Only projects where your role is at least this.
/// @param locked `TRUE` for locked projects only, `FALSE` for unlocked ones only,
///   `NULL` for both.
/// @param url The server's URL, or `NULL` for the server of the project here.
/// @keywords internal
#[miniextendr(r_name = "dvs_projects_impl")]
pub(crate) fn dvs_projects(
    #[miniextendr(default = "NULL")] role: Option<String>,
    #[miniextendr(default = "NULL")] locked: Option<bool>,
    #[miniextendr(default = "NULL")] url: Option<String>,
) -> ExternalPtr<Pager> {
    let server = ServerBackend::new(String::new(), None, or_raise(server_url(url)));
    let mut query = ProjectsQuery {
        role,
        // The server lists unlocked projects unless asked
        locked: Some(
            match locked {
                Some(true) => "true",
                Some(false) => "false",
                None => "any",
            }
            .to_string(),
        ),
        stats: Some(true),
        limit: Some(PAGE_SIZE),
        ..Default::default()
    };
    Pager::new(
        move |cursor| {
            query.cursor = cursor;
            server.projects(&query)
        },
        project_row,
    )
}

#[derive(Serialize)]
struct GrantRow {
    group: String,
    role: String,
    removed: bool,
    group_id: String,
}

fn grant_row(g: &GrantView) -> GrantRow {
    GrantRow {
        group: g.group.clone(),
        role: g.role.to_string(),
        removed: g.removed,
        group_id: g.group_id.to_string(),
    }
}

/// The groups with access to the project here and their roles.
///
/// @keywords internal
#[miniextendr(r_name = "dvs_access_impl")]
pub(crate) fn dvs_access() -> DataFrame {
    or_raise((|| {
        let server = project_here()?;
        let access = off_r_thread(move || server.access())?;
        let rows: Vec<GrantRow> = access.grants.iter().map(grant_row).collect();
        Ok(miniextendr_api::serde::vec_to_dataframe(&rows)?)
    })())
}

// endregion

// region: Files, versions, activity

#[derive(Serialize)]
struct FileRow {
    path: String,
    size: u64,
    stored_size: u64,
    hash: String,
    uploaded_at: String,
    uploaded_by: Option<String>,
    branch: Option<String>,
    commit: Option<String>,
    git_state: Option<String>,
    versions: u64,
    first_uploaded_at: String,
    yanked: bool,
}

fn file_row(f: &FileView) -> FileRow {
    FileRow {
        path: f.path.clone(),
        size: f.size,
        stored_size: f.stored_size,
        hash: f.hash.clone(),
        uploaded_at: f.uploaded_at.to_string(),
        uploaded_by: f.uploaded_by.clone(),
        branch: f.branch.clone(),
        commit: f.commit.clone(),
        git_state: f.git_state.clone(),
        versions: f.versions,
        first_uploaded_at: f.first_uploaded_at.to_string(),
        yanked: f.yanked,
    }
}

/// The files uploaded to the project here: each path with its latest upload.
///
/// @param prefix Only paths starting with this, from the project's top folder.
/// @param glob Only paths matching this: `*` within a folder, `**` across folders.
/// @param min_size,max_size Only files at least / at most this large: bytes, or a
///   size such as `"100MiB"`.
/// @param user Only files whose latest version these people uploaded.
/// @param since Only files uploaded in the last `7d`, `12h`, `2w`, ….
/// @param from,to Only files uploaded from / until this date or time.
/// @param branch Only files whose latest upload came from these branches.
/// @param hash Only files whose latest version has this hash or hash prefix.
/// @param yanked Only files whose latest version is (or is not) yanked.
/// @param sort `path`, `uploaded_at`, `size`, `versions` or `user`; `-` first for
///   descending.
/// @keywords internal
#[allow(clippy::too_many_arguments)]
#[miniextendr(r_name = "dvs_remote_files_impl")]
pub(crate) fn dvs_remote_files(
    #[miniextendr(default = "NULL")] prefix: Option<String>,
    #[miniextendr(default = "NULL")] glob: Option<String>,
    #[miniextendr(default = "NULL")] min_size: Option<String>,
    #[miniextendr(default = "NULL")] max_size: Option<String>,
    #[miniextendr(default = "character(0)")] user: Vec<String>,
    #[miniextendr(default = "NULL")] since: Option<String>,
    #[miniextendr(default = "NULL")] from: Option<String>,
    #[miniextendr(default = "NULL")] to: Option<String>,
    #[miniextendr(default = "character(0)")] branch: Vec<String>,
    #[miniextendr(default = "NULL")] hash: Option<String>,
    #[miniextendr(default = "NULL")] yanked: Option<bool>,
    #[miniextendr(default = "NULL")] sort: Option<String>,
) -> ExternalPtr<Pager> {
    let (server, mut query) = or_raise((|| {
        let query = FilesQuery {
            path_prefix: prefix,
            path_glob: glob,
            hash,
            user,
            from: time_from(since, from)?,
            to,
            branch,
            min_size: size_arg("min_size", min_size)?,
            max_size: size_arg("max_size", max_size)?,
            yanked,
            sort,
            limit: Some(PAGE_SIZE),
            ..Default::default()
        };
        Ok((project_here()?, query))
    })());
    Pager::new(
        move |cursor| {
            query.cursor = cursor;
            server.files(&query)
        },
        file_row,
    )
}

#[derive(Serialize)]
struct VersionRow {
    hash: String,
    size: u64,
    stored_size: u64,
    first_uploaded_at: String,
    last_uploaded_at: String,
    uploads: u64,
    downloads: u64,
    yanked: bool,
    other_paths: u64,
}

fn version_row(v: &VersionView) -> VersionRow {
    VersionRow {
        hash: v.hash.clone(),
        size: v.size,
        stored_size: v.stored_size,
        first_uploaded_at: v.first_uploaded_at.to_string(),
        last_uploaded_at: v.last_uploaded_at.to_string(),
        uploads: v.placements,
        downloads: v.downloads,
        yanked: v.yanked,
        other_paths: v.other_paths,
    }
}

/// Every version uploaded at a path of the project here, newest first.
///
/// @param path The file, relative to the working directory.
/// @keywords internal
#[miniextendr(r_name = "dvs_log_impl")]
pub(crate) fn dvs_log(path: String) -> ExternalPtr<Pager> {
    let (server, path) = or_raise((|| Ok((project_here()?, project_path(&path)?)))());
    Pager::new(
        move |cursor| server.versions(&path, cursor.as_deref(), Some(PAGE_SIZE)),
        version_row,
    )
}

#[derive(Serialize)]
struct ActivityRow {
    at: String,
    kind: String,
    user: Option<String>,
    global_admin: bool,
    path: Option<String>,
    hash: Option<String>,
    size: Option<u64>,
    stored_size: Option<u64>,
    branch: Option<String>,
    commit: Option<String>,
    git_state: Option<String>,
    message: Option<String>,
    reason: Option<String>,
    batch: Option<String>,
    client: Option<String>,
    detail: Option<String>,
    id: String,
}

fn activity_row(item: &ActivityItem) -> ActivityRow {
    let reason = item.detail.as_ref().and_then(|d| {
        d.get("after")
            .and_then(|a| a.get("reason"))
            .and_then(|r| r.as_str())
            .map(str::to_string)
    });
    ActivityRow {
        at: item.at.to_string(),
        kind: item.kind.clone(),
        user: item.user.clone(),
        global_admin: item.global_admin,
        path: item.path.clone(),
        hash: item.hash.clone(),
        size: item.size,
        stored_size: item.stored_size,
        branch: item.branch.clone(),
        commit: item.commit.clone(),
        git_state: item.git_state.clone(),
        message: item.message.clone(),
        reason,
        batch: item.batch_id.map(|b| b.to_string()),
        client: item.client.clone(),
        detail: item.detail.as_ref().map(|d| d.to_string()),
        id: item.id.clone(),
    }
}

/// The project's activity, newest first: uploads, downloads and project changes.
///
/// @param kind `upload`, `download`, or a project change such as `lock` or `yank`.
/// @param user Only what these people did.
/// @param since Only the last `7d`, `12h`, `2w`, ….
/// @param from,to From / until this date or time.
/// @param path Files, a folder ending in `/`, or a pattern with `*`.
/// @param hash A content hash, or its first 8 or more characters.
/// @param branch The branches the person's checkout was on.
/// @param commit The commit the person's checkout was on: 7 or more characters.
/// @param batch One run of `dvs_add()` or `dvs_get()`, by its id.
/// @keywords internal
#[allow(clippy::too_many_arguments)]
#[miniextendr(r_name = "dvs_activity_impl")]
pub(crate) fn dvs_activity(
    #[miniextendr(default = "character(0)")] kind: Vec<String>,
    #[miniextendr(default = "character(0)")] user: Vec<String>,
    #[miniextendr(default = "NULL")] since: Option<String>,
    #[miniextendr(default = "NULL")] from: Option<String>,
    #[miniextendr(default = "NULL")] to: Option<String>,
    #[miniextendr(default = "character(0)")] path: Vec<String>,
    #[miniextendr(default = "NULL")] hash: Option<String>,
    #[miniextendr(default = "character(0)")] branch: Vec<String>,
    #[miniextendr(default = "NULL")] commit: Option<String>,
    #[miniextendr(default = "NULL")] batch: Option<String>,
) -> ExternalPtr<Pager> {
    let (server, mut query) = or_raise((|| {
        let batch = batch
            .map(|b| {
                Uuid::parse_str(&b).map_err(|e| {
                    client_error(
                        "invalid_argument",
                        format!("batch: {b:?} is not a batch id: {e}"),
                    )
                })
            })
            .transpose()?;
        let mut query = ActivityQuery {
            hash,
            user,
            from: time_from(since, from)?,
            to,
            branch,
            commit,
            kind,
            batch,
            limit: Some(PAGE_SIZE),
            ..Default::default()
        };
        // The server takes any number of exact paths, or one folder or one pattern;
        // the filters combine with AND, so a folder with anything else matches less
        // than asked
        let narrowing = path
            .iter()
            .filter(|p| p.ends_with('/') || p.contains('*'))
            .count();
        if narrowing > 0 && path.len() > 1 {
            return Err(client_error(
                "invalid_argument",
                "path: give any number of files, or one folder (ending in `/`) or one \
                 pattern (with `*`) alone",
            ));
        }
        for p in &path {
            query.set_path(&project_path(p)?);
        }
        Ok((project_here()?, query))
    })());
    Pager::new(
        move |cursor| {
            query.cursor = cursor;
            server.activity(&query)
        },
        activity_row,
    )
}

#[derive(Serialize)]
struct RemoteStatusRow {
    path: String,
    remote_status: &'static str,
    on_server: bool,
    yanked: bool,
    yank_reason: Option<String>,
    newer_uploads: u64,
    latest_hash: Option<String>,
    latest_at: Option<String>,
    latest_by: Option<String>,
    latest_branch: Option<String>,
}

/// `s` as a row for the local file at `path`, as `dvs_status()` wrote it.
fn remote_status_row(path: &str, s: &RemoteStatus) -> RemoteStatusRow {
    RemoteStatusRow {
        path: path.to_string(),
        remote_status: match s.relation {
            StatusRelation::Latest => "latest",
            StatusRelation::NewerUploadExists => "newer_upload_exists",
            StatusRelation::UnknownPath => "unknown_path",
            StatusRelation::NotPlacedHere => "not_placed_here",
        },
        on_server: s.recorded,
        yanked: s.yanked,
        yank_reason: s.yank_reason.clone(),
        newer_uploads: s.newer_uploads,
        latest_hash: s.latest_hash.clone(),
        latest_at: s.latest_at.map(|t| t.to_string()),
        latest_by: s.latest_by.clone(),
        latest_branch: s.latest_branch.clone(),
    }
}

/// Local files compared with the server: how each file's content relates to what
/// was uploaded at its path.
///
/// @param paths Paths from the project's top folder, as `dvs_status()` gives them
///   (with the platform's separator).
/// @param hashes Their content hashes.
/// @return One row per path, its `path` as given.
/// @keywords internal
#[miniextendr(r_name = "dvs_remote_status_impl")]
pub(crate) fn dvs_remote_status(paths: Vec<String>, hashes: Vec<String>) -> DataFrame {
    or_raise((|| {
        let server = project_here()?;
        // The server names paths with `/`, whatever the platform
        let keys = paths
            .iter()
            .map(|p| {
                ProjectPath::from_path(Path::new(p))
                    .map(|k| k.as_str().to_string())
                    .map_err(|e| client_error("invalid_argument", format!("{p}: {e}")))
            })
            .collect::<Result<Vec<String>>>()?;
        let files: Vec<StatusFile> = keys
            .iter()
            .zip(hashes)
            .map(|(path, hash)| StatusFile {
                path: path.clone(),
                hash,
            })
            .collect();
        let remote = off_r_thread(move || server.remote_status(&files))?;
        let by_key: HashMap<&str, &RemoteStatus> =
            remote.iter().map(|r| (r.path.as_str(), r)).collect();
        let rows: Vec<RemoteStatusRow> = paths
            .iter()
            .zip(&keys)
            .filter_map(|(path, key)| by_key.get(key.as_str()).map(|r| remote_status_row(path, r)))
            .collect();
        Ok(miniextendr_api::serde::vec_to_dataframe(&rows)?)
    })())
}

// endregion
