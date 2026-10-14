//! Commands that ask a dvs server about a project: projects, groups, access, the files
//! uploaded, a path's versions, activity, and yanks. Every request is a `dvs` library
//! call; this module only reads arguments and prints.

use std::collections::BTreeMap;
use std::io::{BufRead, IsTerminal, Write};
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use clap::{Args, Subcommand};
use serde_json::{Value, json};
use tabled::Tabled;
use url::Url;
use uuid::Uuid;

use dvs::api::v1::{
    AccessView, ActivityBatch, ActivityItem, FileStatus as RemoteStatus, FileView, GrantView,
    GroupView, ProjectView, Role, StatusFile, StatusRelation, VersionView, YankView,
};
use dvs::backends::server::ServerBackend;
use dvs::config::Config;
use dvs::paths::DvsPaths;
use dvs::queries::{ActivityQuery, FilesQuery, GroupsQuery, ProjectsQuery};
use dvs::remote::api_error;
use dvs::{
    Backend, FileStatus, StatusDetail, format_size, format_time, parse_size, short_commit,
    short_hash, since,
};

use crate::output::{self, Mode, or_blank};

/// `--project` and `--server`: which project and server a command asks, over the ones
/// `dvs.toml` names.
#[derive(Debug, Clone, Default)]
pub struct Target {
    pub project: Option<String>,
    pub server: Option<Url>,
}

/// The server, and the project when `need_project`, for a command run in `cwd`.
pub fn backend(target: &Target, cwd: &Path, need_project: bool) -> Result<ServerBackend> {
    let config = Config::find(cwd).transpose()?;
    let (configured, local) = match config.as_ref().map(Config::backend) {
        Some(Backend::Server(s)) => (Some((s.name.clone(), s.url.clone())), false),
        Some(Backend::Local(_)) => (None, true),
        None => (None, false),
    };
    let url = match (&target.server, &configured) {
        (Some(url), _) => url.clone(),
        (None, Some((_, url))) => url.clone(),
        (None, None) if local => bail!(
            "This project keeps its files in local storage, not on a dvs server; give \
             --server URL to ask a server"
        ),
        (None, None) => bail!(
            "Not in a DVS repository attached to a server; give --server URL{}",
            if need_project {
                " and --project NAME"
            } else {
                ""
            }
        ),
    };
    let name = match (&target.project, &configured) {
        (Some(name), _) => name.clone(),
        // dvs.toml's project, unless --server points somewhere else
        (None, Some((name, configured_url))) if *configured_url == url => name.clone(),
        _ if need_project => bail!("Which project on {url}? Give --project NAME"),
        _ => String::new(),
    };
    Ok(ServerBackend::new(name, None, url))
}

/// The checkout `cwd` is in, when it is `server`'s project: only then do its paths
/// name the project's paths and its `.dvs` files describe the project's content.
fn checkout_of(server: &ServerBackend, cwd: &Path) -> Result<Option<DvsPaths>> {
    let Some(config) = Config::find(cwd) else {
        return Ok(None);
    };
    let config = config?;
    match config.backend() {
        Backend::Server(s) if s.url == server.url && s.name == server.name => {
            Ok(Some(DvsPaths::from_cwd(&config)?))
        }
        _ => Ok(None),
    }
}

/// A path a command was given, as the server names it (relative to the project's top
/// folder): from where the command runs in the project's checkout, else as typed.
fn project_path(server: &ServerBackend, cwd: &Path, path: &str) -> Result<String> {
    let Some(paths) = checkout_of(server, cwd)? else {
        return Ok(path.trim_start_matches("./").to_string());
    };
    let folder = path.ends_with('/');
    let mut relative = paths
        .project_relative(Path::new(path))
        .with_context(|| format!("{path} is outside this project"))?;
    if folder && !relative.is_empty() {
        relative.push('/');
    }
    Ok(relative)
}

fn time_from(since_value: Option<&str>, from: Option<String>) -> Result<Option<String>> {
    match (since_value, from) {
        (Some(s), _) => Ok(Some(since(s)?)),
        (None, from) => Ok(from),
    }
}

// ---------------------------------------------------------------------------- projects

#[derive(Debug, Subcommand)]
pub enum ProjectCommand {
    /// List the projects you can read
    Ls {
        /// Only projects where your role is at least this
        #[clap(long, value_parser = ["read", "write", "admin"])]
        role: Option<String>,
        /// Only locked projects
        #[clap(long)]
        locked: bool,
        /// Only projects whose name contains this
        #[clap(long)]
        search: Option<String>,
        #[clap(long, default_value_t = 100)]
        limit: usize,
        /// Continue a `--json` listing from its `next_cursor`
        #[clap(long)]
        cursor: Option<String>,
    },
    /// Show the project: your role, whether it is locked, its groups and what it holds
    Info,
    /// Refuse every upload, from everyone, until a project admin unlocks it. Downloads
    /// and browsing still work, and nothing is deleted.
    Lock {
        /// Why, shown on the project and in its activity
        #[clap(long)]
        reason: Option<String>,
    },
    /// Allow uploads again
    Unlock {
        /// Why, kept in the project's activity
        #[clap(long)]
        reason: Option<String>,
    },
}

#[derive(Tabled)]
struct ProjectRow {
    name: String,
    #[tabled(rename = "your role")]
    role: String,
    locked: String,
    files: String,
    size: String,
    #[tabled(rename = "last activity")]
    last_activity: String,
}

fn project_row(p: &ProjectView) -> ProjectRow {
    let stats = p.stats.as_ref();
    ProjectRow {
        name: p.name.clone(),
        role: p.your_role.to_string(),
        locked: if p.locked { "locked" } else { "" }.to_string(),
        files: stats.map(|s| s.paths.to_string()).unwrap_or_default(),
        size: stats
            .map(|s| format_size(s.logical_bytes))
            .unwrap_or_default(),
        last_activity: stats
            .and_then(|s| s.last_activity_at.as_ref())
            .map(format_time)
            .unwrap_or_default(),
    }
}

fn grant_note(g: &GrantView) -> &'static str {
    if g.removed {
        "deleted from Arx: gives nobody access"
    } else {
        ""
    }
}

fn print_project(p: &ProjectView) {
    println!("Project:   {}", p.name);
    println!("Your role: {}", p.your_role);
    match (&p.lock, p.locked) {
        (Some(lock), _) => println!(
            "Locked:    by {} on {}{}; uploads are refused, downloads still work",
            lock.locked_by,
            format_time(&lock.locked_at),
            lock.reason
                .as_deref()
                .map(|r| format!(": {r}"))
                .unwrap_or_default()
        ),
        (None, true) => println!("Locked:    uploads are refused, downloads still work"),
        (None, false) => println!("Locked:    no"),
    }
    println!(
        "Created:   {} by {}",
        format_time(&p.created_at),
        p.created_by
    );
    if let Some(s) = &p.stats {
        println!(
            "Holds:     {} file{}, {} in their latest versions, {} of distinct content stored",
            s.paths,
            if s.paths == 1 { "" } else { "s" },
            format_size(s.logical_bytes),
            format_size(s.unique_stored_bytes)
        );
        if let Some(t) = &s.last_activity_at {
            println!("Activity:  last on {}", format_time(t));
        }
    }
    if let Some(url) = &p.repo_commit_url {
        println!("Commits:   {url}");
    }
    println!("Groups:");
    for g in &p.grants {
        let note = grant_note(g);
        println!(
            "  {} {}{}",
            g.group,
            g.role,
            if note.is_empty() {
                String::new()
            } else {
                format!(" ({note})")
            }
        );
    }
}

pub fn project(mode: Mode, target: &Target, cwd: &Path, command: ProjectCommand) -> Result<()> {
    match command {
        ProjectCommand::Ls {
            role,
            locked,
            search,
            limit,
            cursor,
        } => {
            let server = backend(target, cwd, false)?;
            let mut query = ProjectsQuery {
                role,
                // The server lists unlocked projects unless asked
                locked: Some(if locked { "true" } else { "any" }.into()),
                q: search,
                stats: Some(true),
                limit: Some(limit),
                cursor,
            };
            let first = server.projects(&query)?;
            output::list(
                mode,
                first,
                |c| {
                    query.cursor = Some(c.to_string());
                    server.projects(&query)
                },
                |items| {
                    if items.is_empty() {
                        println!("No projects{}", if locked { " are locked" } else { "" });
                    } else {
                        output::table(items.iter().map(project_row), &["files", "size"]);
                    }
                },
            )
        }
        ProjectCommand::Info => {
            let p = backend(target, cwd, true)?.project()?;
            if mode.is_text() {
                print_project(&p);
                Ok(())
            } else {
                output::json(&p)
            }
        }
        ProjectCommand::Lock { reason } => set_locked(mode, target, cwd, true, reason),
        ProjectCommand::Unlock { reason } => set_locked(mode, target, cwd, false, reason),
    }
}

fn set_locked(
    mode: Mode,
    target: &Target,
    cwd: &Path,
    locked: bool,
    reason: Option<String>,
) -> Result<()> {
    let p = backend(target, cwd, true)?.set_locked(locked, reason)?;
    if !mode.is_text() {
        return output::json(&p);
    }
    if locked {
        println!(
            "Locked {}: uploads are refused until a project admin unlocks it; downloads still work.",
            p.name
        );
    } else {
        println!("Unlocked {}: uploads are allowed again.", p.name);
    }
    Ok(())
}

// ------------------------------------------------------------------------------ groups

#[derive(Debug, Args)]
pub struct GroupsArgs {
    /// Only groups you are in
    #[clap(long)]
    mine: bool,
    /// Only groups whose name contains this
    #[clap(long)]
    search: Option<String>,
    #[clap(long, default_value_t = 100)]
    limit: usize,
    /// Continue a `--json` listing from its `next_cursor`
    #[clap(long)]
    cursor: Option<String>,
}

#[derive(Tabled)]
struct GroupRow {
    group: String,
    members: i64,
    #[tabled(rename = "you")]
    member: String,
    note: String,
}

pub fn groups(mode: Mode, target: &Target, cwd: &Path, args: GroupsArgs) -> Result<()> {
    let server = backend(target, cwd, false)?;
    let mut query = GroupsQuery {
        q: args.search,
        mine: args.mine.then_some(true),
        limit: Some(args.limit),
        cursor: args.cursor,
    };
    let first = server.groups(&query)?;
    output::list(
        mode,
        first,
        |c| {
            query.cursor = Some(c.to_string());
            server.groups(&query)
        },
        |items: &[GroupView]| {
            if items.is_empty() {
                println!("No Arx groups match");
                return;
            }
            let rows = items.iter().map(|g| GroupRow {
                group: g.name.clone(),
                members: g.known_members,
                member: if g.is_member { "member" } else { "" }.into(),
                note: if g.is_admin_group {
                    "global admins: admin on every project"
                } else {
                    ""
                }
                .into(),
            });
            output::table(rows, &["members"]);
        },
    )
}

// ------------------------------------------------------------------------------ access

#[derive(Debug, Subcommand)]
pub enum AccessCommand {
    /// Give an Arx group a role on the project: read, write or admin
    Grant {
        /// The Arx group's name, exactly
        group: String,
        #[clap(value_parser = ["read", "write", "admin"])]
        role: String,
        /// Do not ask before removing your own admin role
        #[clap(long, short)]
        yes: bool,
    },
    /// Remove an Arx group's access to the project
    Revoke {
        /// The Arx group's name, exactly
        group: String,
        /// The grant of the group deleted from Arx, not of a group created again under
        /// the same name
        #[clap(long)]
        removed: bool,
        /// Do not ask before removing your own admin role
        #[clap(long, short)]
        yes: bool,
    },
}

#[derive(Tabled)]
struct GrantRow {
    group: String,
    role: String,
    note: String,
}

fn print_access(project: &str, view: &AccessView) {
    if view.grants.is_empty() {
        println!("No group has access to {project}");
    } else {
        output::table(
            view.grants.iter().map(|g| GrantRow {
                group: g.group.clone(),
                role: g.role.to_string(),
                note: grant_note(g).into(),
            }),
            &[],
        );
    }
    match view.your_role {
        Some(role) => println!("Your role: {role}"),
        None => println!("You no longer have access to {project}"),
    }
}

/// Ask before a change that removes the caller's own admin role; `--yes` answers for
/// them, and without a terminal nothing is changed.
fn confirm_self_demotion(project: &str, yes: bool, mode: Mode) -> Result<()> {
    if yes {
        return Ok(());
    }
    let question = format!(
        "This removes your own admin role on {project}: you could no longer change its access."
    );
    if !mode.is_text() || !std::io::stdin().is_terminal() {
        bail!("{question} Nothing was changed; run it again with --yes to confirm.");
    }
    eprint!("{question} Continue? [y/N] ");
    std::io::stderr().flush()?;
    let mut answer = String::new();
    std::io::stdin().lock().read_line(&mut answer)?;
    if matches!(answer.trim().to_ascii_lowercase().as_str(), "y" | "yes") {
        Ok(())
    } else {
        bail!("Nothing was changed")
    }
}

/// Run an access change; when the server says it removes the caller's own admin role,
/// ask, then send it again confirmed.
fn change_access<T>(
    project: &str,
    yes: bool,
    mode: Mode,
    mut send: impl FnMut(bool) -> Result<T>,
) -> Result<T> {
    let result = match send(yes) {
        Err(e) if api_error(&e).is_some_and(|a| a.code == "self_demotion_requires_confirm") => {
            confirm_self_demotion(project, yes, mode)?;
            send(true)
        }
        other => other,
    };
    result.map_err(|e| match api_error(&e) {
        // The code (and so the exit code and --json's code) stays the server's
        Some(a) if a.code == "precondition_failed" => {
            let mut a = a.clone();
            a.message = format!(
                "{project}'s access changed while this ran, so nothing was changed; run it \
                 again to apply it to the new list"
            );
            a.into()
        }
        _ => e,
    })
}

/// The id of the Arx group named exactly `name`.
fn group_id(server: &ServerBackend, access: &AccessView, name: &str) -> Result<Uuid> {
    if let Some(g) = access.grants.iter().find(|g| g.group == name && !g.removed) {
        return Ok(g.group_id);
    }
    // The search matches names containing `name`: follow its pages to the exact one
    let mut query = GroupsQuery {
        q: Some(name.to_string()),
        limit: Some(1000),
        ..Default::default()
    };
    loop {
        let page = server.groups(&query)?;
        if let Some(g) = page.items.iter().find(|g| g.name == name) {
            return Ok(g.id);
        }
        match page.next_cursor {
            Some(cursor) => query.cursor = Some(cursor),
            None => bail!(
                "No Arx group is named {name:?} (names are exact, case included); `dvs groups \
                 --search {name}` lists similar ones"
            ),
        }
    }
}

pub fn access(
    mode: Mode,
    target: &Target,
    cwd: &Path,
    command: Option<AccessCommand>,
) -> Result<()> {
    let server = backend(target, cwd, true)?;
    let view = server.access()?;
    let project = server.name.clone();
    let changed = match command {
        None => view,
        Some(AccessCommand::Grant { group, role, yes }) => {
            let role = Role::parse(&role).expect("clap checks the role");
            let id = group_id(&server, &view, &group)?;
            let after = change_access(&project, yes, mode, |confirm| {
                server.put_grant(id, role, confirm, Some(view.version))
            })?;
            if mode.is_text() {
                println!("{group} now has {role} on {project}.");
            }
            after
        }
        Some(AccessCommand::Revoke {
            group,
            removed,
            yes,
        }) => {
            let grant = view
                .grants
                .iter()
                .find(|g| g.group == group && g.removed == removed)
                .or_else(|| {
                    // Only a deleted group's grant has the name: that one
                    (!removed)
                        .then(|| view.grants.iter().find(|g| g.group == group))
                        .flatten()
                });
            let Some(grant) = grant else {
                bail!("{group} has no grant on {project}; `dvs access` lists the groups that do");
            };
            let id = grant.group_id;
            change_access(&project, yes, mode, |confirm| {
                server.delete_grant(id, confirm, Some(view.version))
            })?;
            if mode.is_text() {
                println!("{group} no longer has access to {project}.");
            }
            match server.access() {
                Ok(after) => after,
                // The revoke took the caller's own access
                Err(e) if api_error(&e).is_some_and(|a| a.code == "project_not_found") => {
                    AccessView {
                        grants: vec![],
                        your_role: None,
                        version: view.version,
                    }
                }
                // The revoke is done, so this is no failure: say what is not known
                Err(e) => {
                    eprintln!(
                        "warning: the grant was revoked, but {project}'s access could not be \
                         read again ({e:#}); `dvs access` shows it"
                    );
                    if !mode.is_text() {
                        output::json(&Value::Null)?;
                    }
                    return Ok(());
                }
            }
        }
    };
    if mode.is_text() {
        print_access(&project, &changed);
        Ok(())
    } else {
        output::json(&changed)
    }
}

// ------------------------------------------------------------------------- remote status

/// How a file's content relates to the server, in words.
fn relation(s: &RemoteStatus) -> String {
    let mut words = match s.relation {
        StatusRelation::Latest => "latest".to_string(),
        StatusRelation::NewerUploadExists => format!(
            "newer upload{}{}",
            s.latest_by
                .as_deref()
                .map(|by| format!(" by {by}"))
                .unwrap_or_default(),
            s.latest_at
                .as_ref()
                .map(|t| format!(" on {}", format_time(t)))
                .unwrap_or_default()
        ),
        StatusRelation::UnknownPath => "never uploaded at this path".to_string(),
        StatusRelation::NotPlacedHere => "not uploaded at this path".to_string(),
    };
    if !s.recorded {
        words.push_str("; the server does not have this content");
    }
    if s.yanked {
        words.push_str(&format!(
            "; yanked{}",
            s.yank_reason
                .as_deref()
                .map(|r| format!(": {r}"))
                .unwrap_or_default()
        ));
    }
    words
}

#[derive(Tabled)]
struct RemoteRow {
    path: String,
    status: String,
    size: String,
    server: String,
}

/// `dvs status --remote`: each tracked file's `.dvs` content compared with the server.
pub fn remote_status(
    mode: Mode,
    target: &Target,
    cwd: &Path,
    statuses: &[FileStatus],
) -> Result<()> {
    let server = backend(target, cwd, true)?;
    let mut files = Vec::new();
    for s in statuses {
        if let StatusDetail::Success {
            metadata: Some(m), ..
        } = &s.detail
        {
            let path = dvs::ProjectPath::from_path(&s.path)
                .map_err(|e| anyhow::anyhow!("{e}"))?
                .as_str()
                .to_string();
            files.push(StatusFile {
                path,
                hash: m.hashes.blake3.clone(),
            });
        }
    }
    let remote: BTreeMap<String, RemoteStatus> = server
        .remote_status(&files)?
        .into_iter()
        .map(|r| (r.path.clone(), r))
        .collect();
    let key = |s: &FileStatus| {
        dvs::ProjectPath::from_path(&s.path)
            .map(|p| p.as_str().to_string())
            .unwrap_or_default()
    };
    if !mode.is_text() {
        let rows: Vec<Value> = statuses
            .iter()
            .map(|s| {
                let mut v = serde_json::to_value(s).unwrap_or(Value::Null);
                if let Some(obj) = v.as_object_mut() {
                    obj.insert("remote".into(), json!(remote.get(&key(s))));
                }
                v
            })
            .collect();
        if mode == Mode::JsonLines {
            for row in &rows {
                output::json(row)?;
            }
            return Ok(());
        }
        return output::json(&rows);
    }
    let rows = statuses.iter().filter_map(|s| match &s.detail {
        StatusDetail::Success { status, metadata } => Some(RemoteRow {
            path: s.path.display().to_string(),
            status: status.to_string(),
            size: metadata
                .as_ref()
                .map(|m| format_size(m.size))
                .unwrap_or_default(),
            server: remote.get(&key(s)).map(relation).unwrap_or_default(),
        }),
        StatusDetail::Error { .. } => None,
    });
    output::table(rows, &["size"]);
    Ok(())
}

// --------------------------------------------------------------------------- ls-remote

#[derive(Debug, Args)]
pub struct LsRemoteArgs {
    /// Only paths starting with this, from the project's top folder, e.g. `data/derived/`
    prefix: Option<String>,
    /// Only paths matching this: `*` within a folder, `**` across folders
    #[clap(long)]
    glob: Option<String>,
    /// Only files at least this large, e.g. 100MiB
    #[clap(long, value_parser = parse_size_arg)]
    min_size: Option<u64>,
    /// Only files at most this large
    #[clap(long, value_parser = parse_size_arg)]
    max_size: Option<u64>,
    /// Only files whose latest version this person uploaded (username, email or `me`)
    #[clap(long)]
    user: Vec<String>,
    /// Only files uploaded in the last 7d, 12h, 2w…
    #[clap(long, conflicts_with = "from")]
    since: Option<String>,
    /// Only files uploaded at or after this date (YYYY-MM-DD) or time (RFC 3339)
    #[clap(long)]
    from: Option<String>,
    /// Only files uploaded before the end of this date, or before this time
    #[clap(long)]
    to: Option<String>,
    /// Only files whose latest version is yanked
    #[clap(long)]
    yanked: bool,
    /// `path`, `uploaded_at`, `size`, `versions` or `user`; `-` first for descending,
    /// e.g. `-size`
    #[clap(long, allow_hyphen_values = true)]
    sort: Option<String>,
    #[clap(long, default_value_t = 100)]
    limit: usize,
    /// Continue a `--json` listing from its `next_cursor`
    #[clap(long)]
    cursor: Option<String>,
}

fn parse_size_arg(value: &str) -> Result<u64, String> {
    parse_size(value).map_err(|e| format!("{e}; use a size such as 500KiB, 100MiB or 2GiB"))
}

#[derive(Tabled)]
struct FileRow {
    path: String,
    size: String,
    uploaded: String,
    by: String,
    branch: String,
    versions: u64,
}

fn file_row(f: &FileView) -> FileRow {
    FileRow {
        path: if f.yanked {
            format!("{} (yanked)", f.path)
        } else {
            f.path.clone()
        },
        size: format_size(f.size),
        uploaded: format_time(&f.uploaded_at),
        by: or_blank(f.uploaded_by.as_deref()),
        branch: match (f.branch.as_deref(), f.git_state.as_deref()) {
            (Some(b), _) => b.to_string(),
            (None, Some("detached")) => "(detached HEAD)".into(),
            _ => String::new(),
        },
        versions: f.versions,
    }
}

pub fn ls_remote(mode: Mode, target: &Target, cwd: &Path, args: LsRemoteArgs) -> Result<()> {
    let server = backend(target, cwd, true)?;
    let mut query = FilesQuery {
        path_prefix: args.prefix,
        path_glob: args.glob,
        user: args.user,
        from: time_from(args.since.as_deref(), args.from)?,
        to: args.to,
        min_size: args.min_size,
        max_size: args.max_size,
        yanked: args.yanked.then_some(true),
        sort: args.sort,
        limit: Some(args.limit),
        cursor: args.cursor,
        ..Default::default()
    };
    let first = server.files(&query)?;
    let summary = if mode.is_text() {
        Some(server.files_summary(&query)?)
    } else {
        None
    };
    output::list(
        mode,
        first,
        |c| {
            query.cursor = Some(c.to_string());
            server.files(&query)
        },
        |items| {
            if items.is_empty() {
                println!("No file matches");
                return;
            }
            output::table(items.iter().map(file_row), &["size", "versions"]);
            if let Some(s) = &summary {
                println!(
                    "{} file{} · {} in their latest versions · {} of distinct content stored",
                    s.files,
                    if s.files == 1 { "" } else { "s" },
                    format_size(s.logical_bytes),
                    format_size(s.unique_stored_bytes)
                );
            }
        },
    )
}

// --------------------------------------------------------------------------------- log

#[derive(Tabled)]
struct VersionRow {
    hash: String,
    #[tabled(rename = "first uploaded")]
    first: String,
    #[tabled(rename = "last uploaded")]
    last: String,
    size: String,
    uploads: u64,
    downloads: u64,
    #[tabled(rename = "also at")]
    also_at: String,
    note: String,
}

fn version_row(v: &VersionView, latest: bool) -> VersionRow {
    VersionRow {
        hash: short_hash(&v.hash).to_string(),
        first: format_time(&v.first_uploaded_at),
        last: format_time(&v.last_uploaded_at),
        size: format_size(v.size),
        uploads: v.placements,
        downloads: v.downloads,
        also_at: match v.other_paths {
            0 => String::new(),
            1 => "1 other path".into(),
            n => format!("{n} other paths"),
        },
        note: [latest.then_some("latest"), v.yanked.then_some("yanked")]
            .into_iter()
            .flatten()
            .collect::<Vec<_>>()
            .join(", "),
    }
}

/// `dvs log <path>`: every version uploaded at the path, newest first.
pub fn log(mode: Mode, target: &Target, cwd: &Path, path: &str) -> Result<()> {
    let server = backend(target, cwd, true)?;
    let path = project_path(&server, cwd, path)?;
    let first = server.versions(&path, None, Some(1000))?;
    if !mode.is_text() {
        return output::list(
            mode,
            first,
            |c| server.versions(&path, Some(c), Some(1000)),
            |_| {},
        );
    }
    let versions = output::all(first, |c| server.versions(&path, Some(c), Some(1000)))?;
    if versions.is_empty() {
        println!("Nothing was uploaded at {path} in {}", server.name);
        return Ok(());
    }
    output::table(
        versions
            .iter()
            .enumerate()
            .map(|(i, v)| version_row(v, i == 0)),
        &["size", "uploads", "downloads"],
    );
    Ok(())
}

// ---------------------------------------------------------------------------- activity

#[derive(Debug, Args)]
pub struct ActivityArgs {
    /// `upload`, `download`, or a project change: `lock`, `unlock`, `access_change`,
    /// `yank`, `unyank`, `project_create`, `settings_change`, `project_rename`.
    /// Repeatable.
    #[clap(long)]
    kind: Vec<String>,
    /// Only what this person did (username, email or `me`); repeatable
    #[clap(long)]
    user: Vec<String>,
    /// Only the last 7d, 12h, 2w…
    #[clap(long, conflicts_with = "from")]
    since: Option<String>,
    /// From this date (YYYY-MM-DD) or time (RFC 3339)
    #[clap(long)]
    from: Option<String>,
    /// Until the end of this date, or before this time
    #[clap(long)]
    to: Option<String>,
    /// A file, a folder ending in `/`, or a pattern with `*`
    #[clap(long)]
    path: Option<String>,
    /// A content hash, or its first 8 or more characters
    #[clap(long)]
    hash: Option<String>,
    /// The branch the person's checkout was on; repeatable
    #[clap(long)]
    branch: Vec<String>,
    /// The commit the person's checkout was on: its first 7 or more characters
    #[clap(long)]
    commit: Option<String>,
    /// One run of `dvs add` or `dvs get`, by the id `--batches` shows
    #[clap(long)]
    batch: Option<Uuid>,
    /// List runs of `dvs add` and `dvs get` instead of single files
    #[clap(long, conflicts_with = "batch")]
    batches: bool,
    /// Write every matching event as CSV to this file (`-` for the screen), oldest first
    #[clap(long, conflicts_with = "batches")]
    csv: Option<PathBuf>,
    /// Replace the `--csv` file if it exists
    #[clap(long, requires = "csv")]
    force: bool,
    #[clap(long, default_value_t = 50)]
    limit: usize,
    /// Continue a `--json` listing from its `next_cursor`
    #[clap(long)]
    cursor: Option<String>,
}

#[derive(Tabled)]
struct EventRow {
    when: String,
    who: String,
    what: String,
    #[tabled(rename = "path or change")]
    detail: String,
    size: String,
    hash: String,
    branch: String,
}

fn grants_of(side: Option<&Value>) -> Vec<(String, String)> {
    side.and_then(|s| s.get("grants"))
        .and_then(Value::as_array)
        .map(|list| {
            list.iter()
                .map(|g| {
                    (
                        g["group"].as_str().unwrap_or("?").to_string(),
                        g["role"].as_str().unwrap_or("?").to_string(),
                    )
                })
                .collect()
        })
        .unwrap_or_default()
}

/// A project change in a few words, from what the server recorded.
fn describe(item: &ActivityItem) -> String {
    let detail = item.detail.as_ref();
    let side = |name: &str| detail.and_then(|d| d.get(name));
    let reason = side("after")
        .and_then(|a| a.get("reason"))
        .and_then(Value::as_str);
    let hash = item.hash.as_deref().map(short_hash).unwrap_or_default();
    let mut words = match item.kind.as_str() {
        "upload" | "download" => return or_blank(item.path.as_deref()),
        "lock" => "locked uploads".to_string(),
        "unlock" => "unlocked uploads".to_string(),
        "yank" => format!("yanked {hash}"),
        "unyank" => format!("unyanked {hash}"),
        "project_create" => "created the project".to_string(),
        "project_rename" => "renamed the project".to_string(),
        "settings_change" => "changed the settings".to_string(),
        "access_change" => {
            let (before, after) = (grants_of(side("before")), grants_of(side("after")));
            let mut changes = Vec::new();
            for (group, role) in &after {
                match before.iter().find(|(g, _)| g == group) {
                    Some((_, old)) if old != role => {
                        changes.push(format!("{group} {old} → {role}"))
                    }
                    Some(_) => {}
                    None => changes.push(format!("{group} granted {role}")),
                }
            }
            for (group, _) in &before {
                if !after.iter().any(|(g, _)| g == group) {
                    changes.push(format!("{group} removed"));
                }
            }
            if changes.is_empty() {
                "changed access".to_string()
            } else {
                format!("access: {}", changes.join(", "))
            }
        }
        other => other.replace('_', " "),
    };
    if let Some(r) = reason {
        words.push_str(&format!(": {r}"));
    }
    words
}

fn event_row(item: &ActivityItem) -> EventRow {
    EventRow {
        when: format_time(&item.at),
        who: match (&item.user, item.global_admin) {
            (Some(u), true) => format!("{u} (global admin)"),
            (Some(u), false) => u.clone(),
            (None, _) => String::new(),
        },
        what: item.kind.replace('_', " "),
        detail: describe(item),
        size: item.size.map(format_size).unwrap_or_default(),
        hash: match item.kind.as_str() {
            "upload" | "download" => item
                .hash
                .as_deref()
                .map(short_hash)
                .unwrap_or_default()
                .into(),
            _ => String::new(),
        },
        branch: or_blank(item.branch.as_deref()),
    }
}

#[derive(Tabled)]
struct BatchRow {
    when: String,
    who: String,
    what: String,
    files: u64,
    size: String,
    branch: String,
    commit: String,
    message: String,
    batch: String,
}

fn batch_row(b: &ActivityBatch) -> BatchRow {
    BatchRow {
        when: format_time(&b.at),
        who: b.user.clone(),
        what: b.kind.clone(),
        files: b.files,
        size: format_size(b.bytes),
        branch: or_blank(b.branch.as_deref()),
        commit: b
            .commit
            .as_deref()
            .map(short_commit)
            .unwrap_or_default()
            .into(),
        message: or_blank(b.message.as_deref()),
        batch: b.batch_id.to_string(),
    }
}

pub fn activity(mode: Mode, target: &Target, cwd: &Path, args: ActivityArgs) -> Result<()> {
    let server = backend(target, cwd, true)?;
    let mut query = ActivityQuery {
        hash: args.hash,
        user: args.user,
        from: time_from(args.since.as_deref(), args.from)?,
        to: args.to,
        branch: args.branch,
        commit: args.commit,
        kind: args.kind,
        batch: args.batch,
        limit: Some(args.limit),
        cursor: args.cursor,
        ..Default::default()
    };
    if let Some(path) = &args.path {
        query.set_path(&project_path(&server, cwd, path)?);
    }
    if let Some(file) = args.csv {
        if file.as_os_str() == "-" {
            server.activity_csv(&query, &mut std::io::stdout().lock())?;
        } else {
            if file.exists() && !args.force {
                bail!(
                    "{} exists; give --force to replace it, or another file name",
                    file.display()
                );
            }
            // Written beside it, then renamed: a failed export leaves no partial file and
            // keeps the old one
            let folder = match file.parent() {
                Some(p) if !p.as_os_str().is_empty() => p,
                _ => Path::new("."),
            };
            let mut out = tempfile::NamedTempFile::new_in(folder)
                .with_context(|| format!("could not write in {}", folder.display()))?;
            let n = server.activity_csv(&query, out.as_file_mut())?;
            let written = if args.force {
                out.persist(&file).map_err(|e| e.error)
            } else {
                out.persist_noclobber(&file).map_err(|e| e.error)
            };
            written.with_context(|| format!("could not write {}", file.display()))?;
            eprintln!(
                "Wrote {} ({}): one row per event, oldest first",
                file.display(),
                format_size(n)
            );
        }
        return Ok(());
    }
    if args.batches {
        let first = server.activity_batches(&query)?;
        return output::list(
            mode,
            first,
            |c| {
                query.cursor = Some(c.to_string());
                server.activity_batches(&query)
            },
            |items| {
                if items.is_empty() {
                    println!("No dvs add or dvs get matches");
                } else {
                    output::table(items.iter().map(batch_row), &["files", "size"]);
                }
            },
        );
    }
    let first = server.activity(&query)?;
    output::list(
        mode,
        first,
        |c| {
            query.cursor = Some(c.to_string());
            server.activity(&query)
        },
        |items| {
            if items.is_empty() {
                println!("No activity matches");
            } else {
                output::table(items.iter().map(event_row), &["size"]);
            }
        },
    )
}

// ------------------------------------------------------------------------------- yanks

fn is_full_hash(s: &str) -> bool {
    s.len() == 64 && s.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'))
}

fn is_hash_prefix(s: &str) -> bool {
    (8..64).contains(&s.len()) && s.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'))
}

/// The path and hash `path`'s `.dvs` file records, when the checkout here is `server`'s
/// project and tracks it. A `.dvs` file that cannot be read is an error, not "untracked":
/// guessing another version would yank the wrong content.
fn tracked_hash(
    server: &ServerBackend,
    cwd: &Path,
    path: &str,
) -> Result<Option<(String, String)>> {
    let Some(paths) = checkout_of(server, cwd)? else {
        return Ok(None);
    };
    let Some(relative) = paths.project_relative(Path::new(path)) else {
        return Ok(None);
    };
    if relative.is_empty() {
        return Ok(None);
    }
    let pointer = paths.metadata_path(Path::new(&relative));
    if !pointer.exists() {
        return Ok(None);
    }
    let metadata = dvs::FileMetadata::read(&pointer).with_context(|| {
        format!(
            "could not read {}, the .dvs file for {relative}; fix it (or finish its merge) \
             first, or give the content hash",
            pointer.display()
        )
    })?;
    Ok(Some((relative, metadata.hashes.blake3)))
}

/// The distinct uploaded contents whose hash starts with `prefix`: every page is read,
/// stopping once two are found, as that already makes the prefix ambiguous.
fn hashes_starting(server: &ServerBackend, prefix: &str) -> Result<Vec<String>> {
    let mut query = ActivityQuery {
        hash: Some(prefix.to_string()),
        kind: vec!["upload".into()],
        limit: Some(1000),
        ..Default::default()
    };
    let mut found = std::collections::BTreeSet::new();
    loop {
        let page = server.activity(&query)?;
        found.extend(page.items.into_iter().filter_map(|i| i.hash));
        match page.next_cursor {
            Some(cursor) if found.len() < 2 => query.cursor = Some(cursor),
            _ => return Ok(found.into_iter().collect()),
        }
    }
}

/// The content a yank is about, and how it was found, for the message.
fn content_to_yank(server: &ServerBackend, cwd: &Path, target: &str) -> Result<(String, String)> {
    if is_full_hash(target) {
        return Ok((target.to_string(), "the content".into()));
    }
    if let Some((path, hash)) = tracked_hash(server, cwd, target)? {
        return Ok((hash, format!("the version this checkout records at {path}")));
    }
    if is_hash_prefix(target) {
        match hashes_starting(server, target)?.as_slice() {
            [one] => return Ok((one.clone(), "the content".into())),
            [] => {}
            _ => bail!("More than one content's hash starts {target}; give more of the hash"),
        }
    }
    let path = project_path(server, cwd, target)?;
    let latest = server.versions(&path, None, Some(1))?;
    match latest.items.first() {
        Some(v) => Ok((
            v.hash.clone(),
            format!("the latest version uploaded at {path}"),
        )),
        None => bail!(
            "Nothing was uploaded at {path} in {}; give a path the project has, or a content hash",
            server.name
        ),
    }
}

fn print_yank(y: &YankView) {
    let shown: Vec<&str> = y.paths.iter().map(String::as_str).collect();
    let more = y.path_count.saturating_sub(shown.len() as u64);
    println!(
        "It is at {}{}.",
        shown.join(", "),
        if more > 0 {
            format!(" and {more} more")
        } else {
            String::new()
        }
    );
}

pub fn yank(mode: Mode, target: &Target, cwd: &Path, what: &str, reason: &str) -> Result<()> {
    let server = backend(target, cwd, true)?;
    let (hash, found) = content_to_yank(&server, cwd, what)?;
    let y = server.yank(&hash, reason)?;
    if !mode.is_text() {
        return output::json(&y);
    }
    println!(
        "Yanked {} ({found}): nobody can download it until a project admin unyanks it.",
        short_hash(&y.hash)
    );
    print_yank(&y);
    Ok(())
}

/// The yank in force that `target` names: a hash, its start, a tracked file's version,
/// or a path one of the yanked versions was uploaded at.
fn yank_to_end(server: &ServerBackend, cwd: &Path, target: &str) -> Result<String> {
    if is_full_hash(target) {
        return Ok(target.to_string());
    }
    if is_hash_prefix(target) {
        // Every yank in force: the list is not paged
        let yanks = server.yanks()?;
        let found: Vec<&YankView> = yanks
            .iter()
            .filter(|y| y.hash.starts_with(target))
            .collect();
        match found.as_slice() {
            [one] => return Ok(one.hash.clone()),
            [] => {}
            many => bail!(
                "{} yanked contents' hashes start {target}; give more of the hash",
                many.len()
            ),
        }
    }
    if let Some((_, hash)) = tracked_hash(server, cwd, target)?
        && server.yanks()?.iter().any(|y| y.hash == hash)
    {
        return Ok(hash);
    }
    // The path's own versions, all of them, not a yank's sample of its paths
    let path = project_path(server, cwd, target)?;
    let versions = output::all(server.versions(&path, None, Some(1000))?, |c| {
        server.versions(&path, Some(c), Some(1000))
    })?;
    let yanked: Vec<&VersionView> = versions.iter().filter(|v| v.yanked).collect();
    match yanked.as_slice() {
        [one] => Ok(one.hash.clone()),
        [] => bail!(
            "Nothing yanked in {} was uploaded at {path} or has that hash",
            server.name
        ),
        many => bail!(
            "{} yanked versions were uploaded at {path}: {}; give the hash to unyank",
            many.len(),
            many.iter()
                .map(|v| short_hash(&v.hash))
                .collect::<Vec<_>>()
                .join(", ")
        ),
    }
}

pub fn unyank(
    mode: Mode,
    target: &Target,
    cwd: &Path,
    what: &str,
    reason: Option<String>,
) -> Result<()> {
    let server = backend(target, cwd, true)?;
    let hash = yank_to_end(&server, cwd, what)?;
    let y = server.unyank(&hash, reason)?;
    if !mode.is_text() {
        return output::json(&y);
    }
    println!(
        "Unyanked {}: it can be downloaded again.",
        short_hash(&y.hash)
    );
    print_yank(&y);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn item(kind: &str, detail: Value) -> ActivityItem {
        serde_json::from_value(json!({
            "id": "e:1", "kind": kind, "at": "2026-10-13T14:05:00Z", "user": "alice",
            "path": null, "hash": "ab".repeat(32), "size": null, "stored_size": null,
            "batch_id": null, "message": null, "branch": null, "commit": null,
            "git_state": null, "client": null, "detail": detail,
        }))
        .unwrap()
    }

    #[test]
    fn project_changes_read_as_words() {
        let lock = item(
            "lock",
            json!({"after": {"locked": true, "reason": "submitted"}}),
        );
        assert_eq!(describe(&lock), "locked uploads: submitted");
        let yank = item("yank", json!({"after": {"reason": "bad"}}));
        assert_eq!(describe(&yank), format!("yanked {}: bad", "ab".repeat(6)));
        let access = item(
            "access_change",
            json!({
                "before": {"grants": [{"group": "qc", "role": "read"}, {"group": "old", "role": "write"}]},
                "after": {"grants": [{"group": "qc", "role": "write"}, {"group": "new", "role": "read"}]},
            }),
        );
        assert_eq!(
            describe(&access),
            "access: qc read → write, new granted read, old removed"
        );
    }

    #[test]
    fn a_concurrent_access_change_keeps_its_code() {
        let e = change_access("p", true, Mode::Json, |_| -> Result<()> {
            Err(dvs::remote::ApiError::new(412, "precondition_failed", "stale").into())
        })
        .unwrap_err();
        let api = api_error(&e).unwrap();
        assert_eq!(api.code, "precondition_failed");
        assert!(api.message.contains("p's access changed while this ran"));
    }

    #[test]
    fn hashes_are_told_from_paths() {
        assert!(is_full_hash(&"0a".repeat(32)));
        assert!(!is_full_hash(&"0A".repeat(32)));
        assert!(is_hash_prefix("0a1b2c3d"));
        assert!(!is_hash_prefix("0a1b2c3"));
        assert!(!is_hash_prefix("data/a.csv"));
    }
}
