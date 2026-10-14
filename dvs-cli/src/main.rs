mod output;
mod server_cmds;

use std::io::IsTerminal;
use std::path::{Path, PathBuf};

use anyhow::{Result, anyhow};
use clap::{Parser, Subcommand};
use indicatif::{MultiProgress, ProgressBar, ProgressStyle};
use serde_json::json;
use tabled::Tabled;
use url::Url;

use dvs::auth::{delete_token, get_token};
use dvs::config::Config;
use dvs::globbing::{resolve_paths_for_add, resolve_paths_for_get};
use dvs::init::{attach, init};
use dvs::paths::DvsPaths;
use dvs::remote::{self, ApiError, TOKEN_ENV, TokenSource, api_error, token_for};
use dvs::{
    AddOutcome, Backend, Compression, FileMetadata, FileProgress, GetDetail, Outcome, PathFilter,
    Status, StatusDetail, add_files, format_size, format_time, get_files, get_status,
    set_num_threads, short_hash,
};

use dvs::{DvsError, ErrorCode};
use output::Mode;
use server_cmds::Target;

#[derive(Debug, Subcommand)]
pub enum Init {
    Local {
        /// Where the data will be stored
        path: PathBuf,
        /// If you want to use a root folder other than the current directory
        #[clap(long)]
        root_dir: Option<PathBuf>,
        /// If you want to use a folder name other than `.dvs` for storing the metadata files
        #[clap(long)]
        metadata_folder_name: Option<String>,
        /// Unix group to set on storage directory and files
        #[clap(long)]
        group: Option<String>,
        /// Disable compression of stored files. Compression defaults to zstd
        #[clap(long)]
        no_compression: bool,
    },
    Server {
        /// The project name: lowercase letters, digits and hyphens, starting with a letter
        /// or digit, at most 100 characters
        name: String,
        /// The URL of the dvs server
        url: Url,
        /// If you want to use a folder name other than `.dvs` for storing the metadata files
        #[clap(long)]
        metadata_folder_name: Option<String>,
        /// The Arx group that gets admin on the new project, by its exact name
        #[clap(long)]
        group: String,
        /// Disable compression of stored files. Compression defaults to zstd
        #[clap(long)]
        no_compression: bool,
    },
}

#[derive(Debug, Subcommand)]
pub enum Command {
    /// Starts a new dvs project.
    /// This will create a `dvs.toml` file in the current folder of where the user is calling the CLI
    /// from.
    #[command(next_display_order = 100, subcommand)]
    Init(Init),
    /// Attach to a project created via the dvs server UI.
    /// If the project is found, it will write a `dvs.toml` file based on the information from
    /// the server
    #[command(next_display_order = 100)]
    Attach {
        /// Project name: has to match the existing project on the server
        name: String,
        /// The URL of the dvs server
        url: Url,
        /// If you want to use a folder name other than `.dvs` for storing the metadata files
        #[clap(long)]
        metadata_folder_name: Option<String>,
    },
    /// Adds the given files to dvs. You can use a glob or paths.
    /// If you pass a directory and a glob, the glob will be ran from that directory.
    /// At least one path or --glob must be provided
    #[command(next_display_order = 100)]
    Add {
        #[clap(required_unless_present = "glob")]
        paths: Vec<PathBuf>,
        #[clap(long)]
        glob: Option<String>,
        /// An optional message to add
        #[clap(long, short)]
        message: Option<String>,
        /// Show what would be added without making any actual changes
        #[clap(long)]
        dry_run: bool,
    },
    /// Gets the status of each files in the current repository
    #[command(next_display_order = 100)]
    Status {
        /// Paths (files or directories) to check status for
        paths: Vec<PathBuf>,
        /// Recursively include files in subdirectories for given directories
        /// Without this flag, directories return only their direct children.
        #[clap(long, short, conflicts_with = "glob")]
        recursive: bool,
        #[clap(long, short, conflicts_with = "recursive")]
        glob: Option<String>,
        /// Include the files that are current
        #[clap(long)]
        current: bool,
        /// Include the files that are absent
        #[clap(long)]
        absent: bool,
        /// Include the files that are unsynced
        #[clap(long)]
        unsynced: bool,
        /// Show all metadata columns in the table output
        #[clap(long)]
        with_metadata: bool,
        /// Also compare each file's `.dvs` content with the server: the latest upload at
        /// its path, a newer one, or one the server does not have
        #[clap(long)]
        remote: bool,
    },
    /// Retrieves the given files from dvs storage. You can use a glob or paths.
    /// If you pass a directory and a glob, the glob will be ran from that directory.
    /// At least one path or --glob must be provided; to restore every tracked file,
    /// pass `--glob '**/*'`.
    #[command(next_display_order = 100)]
    Get {
        #[clap(required_unless_present = "glob")]
        paths: Vec<PathBuf>,
        #[clap(long, short, conflicts_with = "recursive")]
        glob: Option<String>,
        /// Recursively include files in subdirectories for directory inputs.
        /// Without this flag, directories return only their direct children.
        #[clap(long, short, conflicts_with = "glob")]
        recursive: bool,
        /// Show what would be retrieved without making any actual changes
        #[clap(long)]
        dry_run: bool,
    },
    /// Log in to a DVS server, the one in the config unless a URL is given
    Login { url: Option<Url> },
    /// Log out from a DVS server, the one in the config unless a URL is given: the
    /// stored token is revoked, then forgotten. A `DVS_TOKEN` is left alone.
    Logout { url: Option<Url> },
    /// Show the server, the client, who you are and your role on this project
    Info {
        /// A server to ask, instead of this project's
        url: Option<Url>,
    },
    /// Projects on the server: list them, show this one, lock or unlock it
    #[command(subcommand)]
    Project(server_cmds::ProjectCommand),
    /// List Arx groups: the names `dvs access grant` takes
    Groups(server_cmds::GroupsArgs),
    /// Show which Arx groups have access to the project, or grant or revoke it
    Access {
        #[command(subcommand)]
        command: Option<server_cmds::AccessCommand>,
    },
    /// List the files uploaded to the project, each in its latest version
    LsRemote(server_cmds::LsRemoteArgs),
    /// Show every version uploaded at a path, newest first
    Log {
        /// The file, from where you are in the checkout (or from the project's top folder
        /// outside one)
        path: String,
    },
    /// Show what happened in the project, newest first: uploads, downloads, and changes
    /// such as locks, access and yanks
    Activity(server_cmds::ActivityArgs),
    /// Stop everyone downloading a content, wherever it was uploaded, until it is
    /// unyanked. Nothing is deleted.
    Yank {
        /// A tracked file (its `.dvs` version), a path on the server (its latest
        /// version), or a content hash or its first 8 or more characters
        target: String,
        /// Why, shown to anyone who tries to download it
        #[clap(long)]
        reason: String,
    },
    /// Allow a yanked content to be downloaded again
    Unyank {
        /// A path the yanked content was at, or its hash or first 8 or more characters
        target: String,
        /// Why, kept in the project's activity
        #[clap(long)]
        reason: Option<String>,
    },
}

/// Some files failed; the codes the server gave decide the exit code.
#[derive(Debug)]
struct FilesFailed {
    action: &'static str,
    codes: Vec<String>,
}

impl std::fmt::Display for FilesFailed {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Some files failed to {}", self.action)
    }
}

impl std::error::Error for FilesFailed {}

/// The stable code of an error, for `--json`: the server's, or `files_failed`, or
/// `error`.
fn error_code(e: &anyhow::Error) -> &str {
    if e.downcast_ref::<FilesFailed>().is_some() {
        return "files_failed";
    }
    dvs::error_code(e).unwrap_or("error")
}

/// A client-side error with its code.
fn coded(code: ErrorCode, message: impl Into<String>) -> anyhow::Error {
    DvsError::new(code, message).into()
}

/// A per-file error with the code, when there is one, after it.
fn with_code(error: &str, code: Option<&str>) -> String {
    match code {
        Some(code) if !code.is_empty() => format!("{error} ({code})"),
        _ => error.to_string(),
    }
}

const EXIT_CODES: &str = "\
Exit codes:
  0  success
  1  any other failure, such as some files failing
  2  usage error
  3  not logged in, or the token was rejected
  4  the project already exists
  5  project not found, or you have no access to it

With --json, an error is printed on stderr as {\"error\":{\"code\":…,\"message\":…}}.";

/// 3 not logged in or token rejected, 5 project not found or no access, 4 project
/// exists, 1 anything else. When several apply, the first in that list wins. Clap
/// exits with 2 on usage errors before any of this.
fn exit_code(e: &anyhow::Error) -> i32 {
    let codes: Vec<&str> = match e.downcast_ref::<FilesFailed>() {
        Some(f) => f.codes.iter().map(String::as_str).collect(),
        None => api_error(e).map(|a| a.code.as_str()).into_iter().collect(),
    };
    let api = |code: &&str| ApiError::new(0, code, "");
    if codes.iter().any(|c| api(c).is_auth()) {
        3
    } else if codes.iter().any(|c| api(c).is_access()) {
        5
    } else if codes.contains(&"project_exists") {
        4
    } else {
        1
    }
}

/// The URL given, or the server of the project here.
fn server_url(url: Option<Url>, current_dir: &std::path::Path) -> Result<Url> {
    if let Some(url) = url {
        return Ok(url);
    }
    let config = Config::find(current_dir).ok_or_else(|| {
        coded(
            ErrorCode::NotInRepository,
            "Not in a DVS repository; give the server URL",
        )
    })??;
    match config.server_url() {
        Some(url) => Ok(url.clone()),
        None => Err(coded(
            ErrorCode::NotAServerProject,
            "This project is not using a dvs server",
        )),
    }
}

#[derive(Parser)]
#[clap(version, author, about, subcommand_negates_reqs = true, after_help = EXIT_CODES)]
pub struct Cli {
    /// Output results as JSON: a list as {"items", "next_cursor"}
    #[clap(long, global = true, display_order = 0)]
    pub json: bool,

    /// Output a list as one JSON object a line, following every page
    #[clap(long, global = true, conflicts_with = "json", display_order = 0)]
    pub json_lines: bool,

    /// The project to ask, instead of the one in dvs.toml
    #[clap(long, global = true, display_order = 2)]
    pub project: Option<String>,

    /// The server to ask, instead of the one in dvs.toml
    #[clap(long, global = true, display_order = 2)]
    pub server: Option<Url>,

    /// Number of threads for parallel operations (0 = auto-detect)
    #[clap(long, global = true, display_order = 1)]
    pub threads: Option<usize>,

    #[clap(subcommand)]
    pub command: Command,
}

#[derive(Tabled)]
struct StatusRow {
    path: String,
    status: String,
    size: String,
}

#[derive(Default, Tabled)]
struct StatusRowFull<'a> {
    path: String,
    status: String,
    size: String,
    hash: &'a str,
    created_by: &'a str,
    add_time: String,
    compression: String,
    message: &'a str,
}

impl<'a> From<&'a FileMetadata> for StatusRowFull<'a> {
    fn from(m: &'a FileMetadata) -> Self {
        Self {
            size: format_size(m.size),
            hash: short_hash(&m.hashes.blake3),
            created_by: m.created_by.as_str(),
            add_time: format_time(&m.add_time),
            compression: m.compression.to_string(),
            message: m.message.as_deref().unwrap_or(""),
            ..Default::default()
        }
    }
}

fn progress_style() -> ProgressStyle {
    ProgressStyle::with_template("{bar:40} {pos_fmt}/{len_fmt} ({percent}%) | {msg}")
        .unwrap()
        .with_key(
            "pos_fmt",
            |state: &indicatif::ProgressState, w: &mut dyn std::fmt::Write| {
                write!(w, "{}", format_size(state.pos())).unwrap()
            },
        )
        .with_key(
            "len_fmt",
            |state: &indicatif::ProgressState, w: &mut dyn std::fmt::Write| {
                write!(w, "{}", format_size(state.len().unwrap_or(0))).unwrap()
            },
        )
}

fn make_progress_callback(threshold: u64) -> impl Fn(&std::path::Path, u64) -> FileProgress {
    let mp = MultiProgress::new();
    let style = progress_style();
    move |path: &std::path::Path, size: u64| {
        if size <= threshold {
            return FileProgress {
                on_bytes: Box::new(|_| {}),
                on_done: Box::new(|_| {}),
            };
        }
        let pb = mp.add(ProgressBar::new(size));
        pb.set_style(style.clone());
        pb.set_message(path.display().to_string());
        let pb2 = pb.clone();
        FileProgress {
            on_bytes: Box::new(move |n| pb.inc(n)),
            on_done: Box::new(move |ok| {
                if ok {
                    pb2.finish_and_clear()
                } else {
                    pb2.abandon()
                }
            }),
        }
    }
}

/// Runs the interactive device-authorization flow and stores the resulting token.
fn run_device_login(url: &Url) -> Result<()> {
    if std::env::var(TOKEN_ENV).is_ok_and(|t| !t.trim().is_empty()) {
        eprintln!("Note: {TOKEN_ENV} is set, so dvs will keep using it instead of this login");
    }
    let token = remote::login(url, |start| {
        eprintln!(
            "Open {} and check that it shows the code {}",
            start.verification_uri_complete, start.user_code
        );
    })?;
    eprintln!(
        "You are now logged in to {url} until {}",
        format_time(&token.expires_at)
    );
    Ok(())
}

/// Explain why `dvs status PATHS` lists nothing for some of the paths.
/// `tracked` is every tracked file the paths match, `shown` those left after the
/// `--current`/`--absent`/`--unsynced` filters, named by `states` when there are any.
fn unmatched_paths(
    user_paths: &[PathBuf],
    tracked: &[dvs::FileStatus],
    shown: &[dvs::FileStatus],
    states: Option<&str>,
    recursive: bool,
    dvs_paths: &DvsPaths,
) -> Vec<String> {
    let mut notes = Vec::new();
    for user_path in user_paths {
        let Some(wanted) = dvs_paths.project_relative(user_path) else {
            notes.push(format!("{} is outside this project", user_path.display()));
            continue;
        };
        let wanted = Path::new(&wanted);
        let matches = |list: &[dvs::FileStatus]| list.iter().any(|s| s.path.starts_with(wanted));
        if matches(shown) {
            continue;
        }
        let shown = user_path.display();
        let full = dvs_paths.file_path(wanted);
        if let Some(states) = states
            && matches(tracked)
        {
            notes.push(if full.is_dir() {
                format!("No tracked files in {shown} are {states}")
            } else {
                format!("{shown} is tracked, but not {states}")
            });
            continue;
        }
        notes.push(if full.is_dir() {
            if recursive {
                format!("No tracked files in {shown}")
            } else {
                format!(
                    "No tracked files directly in {shown}; add --recursive to include its \
                     subfolders"
                )
            }
        } else if full.exists() {
            format!("{shown} is not tracked: `dvs add {shown}` tracks it")
        } else {
            format!("{shown} does not exist and is not tracked")
        });
    }
    notes
}

fn try_main(cli: Cli) -> Result<()> {
    // Warnings (a failed `complete`, a retried upload) show unless RUST_LOG says otherwise,
    // as `warning: ...` unless RUST_LOG asks for the full format
    let mut logger =
        env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("dvs=warn"));
    if std::env::var_os("RUST_LOG").is_none() {
        logger.format(|buf, record| {
            use std::io::Write;
            let level = record.level().as_str().to_ascii_lowercase();
            writeln!(buf, "{level}: {}", record.args())
        });
    }
    logger.init();

    if let Some(n) = cli.threads {
        set_num_threads(n);
    }
    let current_dir = std::env::current_dir()?;
    let mode = if cli.json_lines {
        Mode::JsonLines
    } else if cli.json {
        Mode::Json
    } else {
        Mode::Text
    };
    let json_out = !mode.is_text();
    let target = Target {
        project: cli.project.clone(),
        server: cli.server.clone(),
    };

    match cli.command {
        Command::Init(init_cmd) => {
            let (config, root_dir, metadata_folder_name) = match init_cmd {
                Init::Local {
                    path: storage_path,
                    root_dir,
                    metadata_folder_name,
                    group,
                    no_compression,
                } => {
                    let mut config = Config::new_local(&storage_path, group)?;
                    if no_compression {
                        config.set_compression(Some(Compression::None));
                    }
                    (config, root_dir, metadata_folder_name)
                }
                Init::Server {
                    name,
                    url,
                    metadata_folder_name,
                    group,
                    no_compression,
                } => {
                    // Creating the project needs a token, so log in first if there is none
                    if token_for(&url)?.is_none() {
                        run_device_login(&url)?;
                    }
                    let mut config = Config::new_server(name, url, Some(group));
                    if no_compression {
                        config.set_compression(Some(Compression::None));
                    }
                    (config, None, metadata_folder_name)
                }
            };

            let mut config = config;
            if let Some(m) = metadata_folder_name {
                config.set_metadata_folder_name(m);
            }
            let root = root_dir.unwrap_or(current_dir);

            let repo_root = init(&root, config)?;
            if json_out {
                println!("{}", json!({"status": "initialized"}));
            } else {
                println!("DVS Initialized at {repo_root:?}");
            }
        }
        Command::Attach {
            name,
            url,
            metadata_folder_name,
        } => {
            // Reading the project needs a token, so log in first if there is none
            if token_for(&url)?.is_none() {
                run_device_login(&url)?;
            }
            let mut config = Config::new_server(name, url, None);
            if let Some(m) = metadata_folder_name {
                config.set_metadata_folder_name(m);
            }
            let project = attach(&current_dir, config)?;
            if json_out {
                println!("{}", json!({"status": "attached", "project": project}));
            } else {
                println!(
                    "Attached {} (your role: {}){}",
                    project.name,
                    project.your_role.as_str(),
                    if project.locked {
                        ", locked: uploads are refused"
                    } else {
                        ""
                    }
                );
            }
        }
        Command::Add {
            paths,
            glob,
            message,
            dry_run,
        } => {
            let config = Config::find_required(&current_dir)?;
            let dvs_paths = DvsPaths::from_cwd(&config)?;
            let all_paths: Vec<_> = resolve_paths_for_add(paths, glob.as_deref(), &dvs_paths)?
                .into_iter()
                .collect();
            if all_paths.is_empty() {
                return Err(coded(ErrorCode::NothingToAdd, "No files to add"));
            }

            let show_progress = !json_out && !dry_run && std::io::stderr().is_terminal();

            let on_file_start = make_progress_callback(config.progress_bytes_threshold());
            let had_message = message.is_some();
            let results = add_files(
                all_paths,
                &dvs_paths,
                config.backend(),
                message,
                config.compression(),
                dry_run,
                if show_progress {
                    Some(&on_file_start)
                } else {
                    None
                },
            )?;
            let codes: Vec<String> = results
                .iter()
                .filter(|r| r.is_failure())
                .map(|r| r.code.clone().unwrap_or_default())
                .collect();
            if json_out {
                println!("{}", serde_json::to_string(&results)?);
            } else {
                for result in &results {
                    let path = result.path.display();
                    let size = format_size(result.size.unwrap_or(0));
                    let hash = result.hash.as_deref().map(short_hash).unwrap_or("");
                    let error = with_code(
                        result.error.as_deref().unwrap_or(""),
                        result.code.as_deref(),
                    );
                    match result.outcome {
                        AddOutcome::Added => {
                            let msg = if dry_run { "To add" } else { "Added" };
                            let stored_info = match result.stored_size {
                                Some(s) if Some(s) != result.size => {
                                    format!(" --> saved [{}]", format_size(s))
                                }
                                _ => String::new(),
                            };
                            println!("{msg}: {path} [{size}]{stored_info} as {hash}");
                        }
                        AddOutcome::Present => {
                            let msg = if dry_run { "To record" } else { "Recorded" };
                            println!("{msg}: {path} [{size}] as {hash} (content already stored)");
                        }
                        AddOutcome::Skipped => {
                            println!("Unchanged: {path} [{size}] as {hash} (already tracked)")
                        }
                        AddOutcome::Failed => eprintln!("Error adding {path}: {error}"),
                        AddOutcome::NotAttempted => eprintln!("Not attempted: {path}: {error}"),
                    }
                }
                let unchanged = results
                    .iter()
                    .filter(|r| r.outcome == AddOutcome::Skipped)
                    .count();
                if had_message && unchanged > 0 {
                    eprintln!(
                        "The message was not recorded for the {unchanged} unchanged file{}: a \
                         message is kept only with a new version",
                        if unchanged == 1 { "" } else { "s" }
                    );
                }
                let recorded = results
                    .iter()
                    .any(|r| matches!(r.outcome, AddOutcome::Added | AddOutcome::Present));
                if recorded && !dry_run {
                    eprintln!(
                        "Next: commit the new .dvs files in {}/ and .gitignore to Git, so others \
                         can `dvs get` these files",
                        config.metadata_folder_name()
                    );
                }
            }
            if !codes.is_empty() {
                return Err(FilesFailed {
                    action: "add",
                    codes,
                }
                .into());
            }
        }
        Command::Status {
            paths: user_paths,
            recursive,
            current,
            absent,
            unsynced,
            with_metadata,
            glob,
            remote,
        } => {
            let config = Config::find_required(&current_dir)?;
            let dvs_paths = DvsPaths::from_cwd(&config)?;
            let show_all = !current && !absent && !unsynced;

            let filter = if user_paths.is_empty() {
                None
            } else {
                Some(PathFilter::from_user_paths(
                    user_paths.clone(),
                    recursive,
                    &dvs_paths,
                ))
            };
            let mut statuses = get_status(&dvs_paths, filter.as_ref(), glob.as_deref())?;
            let tracked = statuses.clone();
            let states = [
                (current, "current"),
                (absent, "absent"),
                (unsynced, "unsynced"),
            ]
            .into_iter()
            .filter_map(|(on, name)| on.then_some(name))
            .collect::<Vec<_>>()
            .join(" or ");
            if !show_all {
                statuses.retain(|x| match &x.detail {
                    StatusDetail::Success { status, .. } => {
                        (current && *status == Status::Current)
                            || (absent && *status == Status::Absent)
                            || (unsynced && *status == Status::Unsynced)
                    }
                    StatusDetail::Error { .. } => true,
                });
            }
            let has_errors = statuses
                .iter()
                .any(|s| matches!(s.detail, StatusDetail::Error { .. }));
            // Paths given that match nothing tracked: say why, instead of a bare "none"
            let unmatched = if glob.is_none() {
                unmatched_paths(
                    &user_paths,
                    &tracked,
                    &statuses,
                    (!show_all).then_some(states.as_str()),
                    recursive,
                    &dvs_paths,
                )
            } else {
                Vec::new()
            };
            if remote {
                if !json_out {
                    // The table leaves out files whose status failed: name each first
                    for file_status in &statuses {
                        if let StatusDetail::Error { error } = &file_status.detail {
                            eprintln!(
                                "Error getting status for {}: {error}",
                                file_status.path.display()
                            );
                        }
                    }
                }
                server_cmds::remote_status(mode, &target, &current_dir, &statuses)?;
                if !json_out {
                    for note in &unmatched {
                        eprintln!("{note}");
                    }
                }
            } else if json_out {
                println!("{}", serde_json::to_string(&statuses)?);
            } else if statuses.is_empty() {
                if !unmatched.is_empty() {
                    for note in &unmatched {
                        println!("{note}");
                    }
                } else if let Some(g) = &glob {
                    println!("No tracked file matches {g}");
                } else if !show_all {
                    println!("No tracked files have the status asked for");
                } else if user_paths.is_empty() {
                    println!("No tracked files: `dvs add PATH` tracks one");
                } else {
                    println!("No tracked files there");
                }
            } else {
                for note in &unmatched {
                    eprintln!("{note}");
                }
                for file_status in &statuses {
                    if let StatusDetail::Error { error } = &file_status.detail {
                        eprintln!(
                            "Error getting status for {}: {error}",
                            file_status.path.display()
                        );
                    }
                }

                if with_metadata {
                    let rows = statuses.iter().filter_map(|fs| match &fs.detail {
                        StatusDetail::Success { status, metadata } => {
                            let mut row = metadata
                                .as_ref()
                                .map(StatusRowFull::from)
                                .unwrap_or_default();
                            row.path = fs.path.display().to_string();
                            row.status = status.to_string();
                            Some(row)
                        }
                        StatusDetail::Error { .. } => None,
                    });
                    output::table(rows, &["size"]);
                } else {
                    let rows = statuses.iter().filter_map(|fs| match &fs.detail {
                        StatusDetail::Success { status, metadata } => {
                            let size = match metadata {
                                Some(m) => format_size(m.size),
                                None => String::new(),
                            };
                            Some(StatusRow {
                                path: fs.path.display().to_string(),
                                status: status.to_string(),
                                size,
                            })
                        }
                        StatusDetail::Error { .. } => None,
                    });
                    output::table(rows, &["size"]);
                }
            }
            if has_errors {
                return Err(anyhow!("Some files failed to get status"));
            }
        }
        Command::Get {
            paths,
            glob,
            recursive,
            dry_run,
        } => {
            let config = Config::find_required(&current_dir)?;
            let dvs_paths = DvsPaths::from_cwd(&config)?;
            let all_paths: Vec<_> =
                resolve_paths_for_get(paths, glob.as_deref(), &dvs_paths, recursive)?
                    .into_iter()
                    .collect();
            if all_paths.is_empty() {
                return Err(coded(ErrorCode::NothingToGet, "No files to get"));
            }

            let show_progress = !json_out && !dry_run && std::io::stderr().is_terminal();

            let on_file_start = make_progress_callback(config.progress_bytes_threshold());
            let results = get_files(
                all_paths,
                &dvs_paths,
                config.backend(),
                dry_run,
                if show_progress {
                    Some(&on_file_start)
                } else {
                    None
                },
            )?;
            let codes: Vec<String> = results
                .iter()
                .filter_map(|r| match &r.detail {
                    GetDetail::Error { code, .. } => Some(code.clone().unwrap_or_default()),
                    GetDetail::Success { .. } => None,
                })
                .collect();
            if json_out {
                println!("{}", serde_json::to_string(&results)?);
            } else {
                let mut total_files = 0u64;
                let mut total_bytes = 0u64;
                let mut matching = 0u64;
                for result in &results {
                    match &result.detail {
                        GetDetail::Success { outcome, size } => {
                            if *outcome == Outcome::Copied {
                                println!(
                                    "{}{} [{}]",
                                    if dry_run { "Would get: " } else { "" },
                                    result.path.display(),
                                    format_size(*size)
                                );
                                total_files += 1;
                                total_bytes += size;
                            } else {
                                matching += 1;
                            }
                        }
                        GetDetail::Error { error, code } => eprintln!(
                            "Error: {} - {}",
                            result.path.display(),
                            with_code(error, code.as_deref())
                        ),
                    }
                }
                if total_files > 0 {
                    println!(
                        "{}: {} file{}, {}",
                        if dry_run { "Would get" } else { "Total" },
                        total_files,
                        if total_files == 1 { "" } else { "s" },
                        format_size(total_bytes)
                    );
                } else if codes.is_empty() {
                    println!(
                        "Nothing to get: {matching} file{} already match{} {} .dvs file{}",
                        if matching == 1 { "" } else { "s" },
                        if matching == 1 { "es" } else { "" },
                        if matching == 1 { "its" } else { "their" },
                        if matching == 1 { "" } else { "s" },
                    );
                }
            }
            if !codes.is_empty() {
                return Err(FilesFailed {
                    action: "get",
                    codes,
                }
                .into());
            }
        }
        Command::Login { url } => {
            let url = server_url(url, &current_dir)?;
            run_device_login(&url)?;
        }
        Command::Logout { url } => {
            let url = server_url(url, &current_dir)?;
            let env_token = std::env::var(TOKEN_ENV)
                .ok()
                .map(|t| t.trim().to_string())
                .filter(|t| !t.is_empty());
            if env_token.is_some() {
                eprintln!(
                    "Note: {TOKEN_ENV} is set; it is not revoked. Unset it to stop using it."
                );
            }
            match get_token(&url)? {
                None => println!("You were not logged in to {url}"),
                Some(token) => {
                    if env_token.as_deref() == Some(token.as_str()) {
                        eprintln!(
                            "The stored token is the one in {TOKEN_ENV}, so it is not revoked; \
                             it is only forgotten here."
                        );
                    } else {
                        match remote::revoke(&url, &token) {
                            Ok(_) => {}
                            Err(e) if remote::is_no_response(&e) => eprintln!(
                                "Could not reach {url} to revoke the token ({e:#}). It is \
                                 forgotten here, but stays valid on the server until it expires."
                            ),
                            Err(e) => eprintln!(
                                "{url} did not revoke the token ({e:#}). It is forgotten here, \
                                 but may stay valid on the server until it expires."
                            ),
                        }
                    }
                    delete_token(&url)?;
                    println!("Logged out of {url}");
                }
            }
        }
        Command::Info { url } => {
            let config = match &url {
                Some(_) => None,
                None => Config::find(&current_dir).transpose()?,
            };
            let project = config.as_ref().and_then(|c| match c.backend() {
                Backend::Server(s) => Some(s.clone()),
                Backend::Local(_) => None,
            });
            let url = match (url, &project, &config) {
                (Some(url), _, _) => url,
                (None, Some(s), _) => s.url.clone(),
                (None, None, Some(_)) => {
                    return Err(coded(
                        ErrorCode::NotAServerProject,
                        "This project is not using a dvs server",
                    ));
                }
                (None, None, None) => {
                    return Err(coded(
                        ErrorCode::NotInRepository,
                        "Not in a DVS repository; give the server URL",
                    ));
                }
            };
            let server = project.clone().unwrap_or_else(|| {
                dvs::backends::server::ServerBackend::new(String::new(), None, url.clone())
            });
            let info = server.server_info()?;
            let source = server.token_source();
            let me = server.me();
            let role = match (&project, &me) {
                (Some(p), Ok(_)) => Some(p.project()),
                _ => None,
            };
            if json_out {
                println!(
                    "{}",
                    json!({
                        "server": url,
                        "server_version": info.server_version,
                        "api_versions": info.api_versions,
                        "client_version": dvs::VERSION,
                        "token_source": source,
                        "me": me.as_ref().ok(),
                        "project": role.as_ref().and_then(|r| r.as_ref().ok()),
                    })
                );
            } else {
                println!(
                    "Server:  {url} (dvs-server {}, API {:?})",
                    info.server_version, info.api_versions
                );
                println!("Client:  dvs {}", dvs::VERSION);
                if let Ok(me) = &me {
                    let name = me.username.as_deref().unwrap_or(&me.sub);
                    match &me.email {
                        Some(email) => println!("User:    {name} <{email}>"),
                        None => println!("User:    {name}"),
                    }
                    let from = match source {
                        Some(TokenSource::Environment) => TOKEN_ENV,
                        _ => "dvs login",
                    };
                    match &me.token {
                        Some(t) => println!(
                            "Token:   from {from}, expires {}",
                            format_time(&t.expires_at)
                        ),
                        None => println!("Token:   from {from}"),
                    }
                }
                if let Some(Ok(p)) = &role {
                    println!(
                        "Project: {}, your role: {}{}",
                        p.name,
                        p.your_role.as_str(),
                        if p.locked {
                            " (locked: uploads are refused)"
                        } else {
                            ""
                        }
                    );
                }
            }
            me?;
            if let Some(role) = role {
                role?;
            }
        }
        Command::Project(command) => server_cmds::project(mode, &target, &current_dir, command)?,
        Command::Groups(args) => server_cmds::groups(mode, &target, &current_dir, args)?,
        Command::Access { command } => server_cmds::access(mode, &target, &current_dir, command)?,
        Command::LsRemote(args) => server_cmds::ls_remote(mode, &target, &current_dir, args)?,
        Command::Log { path } => server_cmds::log(mode, &target, &current_dir, &path)?,
        Command::Activity(args) => server_cmds::activity(mode, &target, &current_dir, args)?,
        Command::Yank {
            target: what,
            reason,
        } => server_cmds::yank(mode, &target, &current_dir, &what, &reason)?,
        Command::Unyank {
            target: what,
            reason,
        } => server_cmds::unyank(mode, &target, &current_dir, &what, reason)?,
    }
    Ok(())
}

/// Parse the command line. A usage error under `--json` is reported in the JSON error
/// form, with exit code 2 like any usage error; help and version print as usual.
fn parse() -> Cli {
    let e = match Cli::try_parse() {
        Ok(cli) => return cli,
        Err(e) => e,
    };
    use clap::error::ErrorKind;
    let json = std::env::args_os()
        .skip(1)
        .take_while(|a| a != "--")
        .any(|a| a == "--json" || a == "--json-lines");
    let shown = matches!(e.kind(), ErrorKind::DisplayHelp | ErrorKind::DisplayVersion);
    if !json || shown {
        e.exit()
    }
    let rendered = e.render().to_string();
    // The first paragraph, without clap's `error: `, is the message
    let message = rendered
        .split("\n\n")
        .next()
        .unwrap_or(&rendered)
        .trim()
        .trim_start_matches("error: ");
    eprintln!(
        "{}",
        json!({"error": {"code": "usage", "message": message}})
    );
    ::std::process::exit(2)
}

fn main() {
    let cli = parse();
    let json_errors = cli.json || cli.json_lines;
    if let Err(e) = try_main(cli) {
        if json_errors {
            // The code has its own field, so the message drops its `(code)` suffix
            let code = error_code(&e);
            let message = format!("{e:#}");
            let message = message
                .strip_suffix(&format!(" ({code})"))
                .unwrap_or(&message);
            let error = json!({"error": {"code": code, "message": message}});
            eprintln!("{error}");
        } else {
            eprintln!("Error: {e:#}");
        }
        ::std::process::exit(exit_code(&e))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn files(codes: &[&str]) -> anyhow::Error {
        FilesFailed {
            action: "add",
            codes: codes.iter().map(|c| c.to_string()).collect(),
        }
        .into()
    }

    #[test]
    fn the_command_line_is_well_formed() {
        use clap::CommandFactory;
        Cli::command().debug_assert();
    }

    #[test]
    fn json_errors_carry_a_stable_code() {
        assert_eq!(error_code(&files(&["hash_mismatch"])), "files_failed");
        assert_eq!(
            error_code(&ApiError::new(423, "project_locked", "locked").into()),
            "project_locked"
        );
        assert_eq!(error_code(&anyhow!("plain")), "error");
    }

    #[test]
    fn the_first_applicable_exit_code_wins() {
        assert_eq!(exit_code(&anyhow!("plain")), 1);
        assert_eq!(
            exit_code(&ApiError::new(409, "project_exists", "").into()),
            4
        );
        assert_eq!(
            exit_code(&ApiError::new(404, "project_not_found", "").into()),
            5
        );
        let wrapped =
            anyhow::Error::from(ApiError::new(401, "token_expired", "")).context("while adding");
        assert_eq!(exit_code(&wrapped), 3);

        assert_eq!(exit_code(&files(&["", "hash_mismatch"])), 1);
        assert_eq!(exit_code(&files(&["", "project_exists"])), 4);
        assert_eq!(
            exit_code(&files(&["project_exists", "insufficient_role"])),
            5
        );
        assert_eq!(
            exit_code(&files(&["insufficient_role", "unauthenticated", ""])),
            3
        );
    }
}
