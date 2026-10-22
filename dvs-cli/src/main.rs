mod get_cmd;
mod output;

use std::io::IsTerminal;
use std::path::PathBuf;

use anyhow::{Result, anyhow, bail};
use clap::{Parser, Subcommand};
use indicatif::{MultiProgress, ProgressBar, ProgressStyle};
use serde_json::json;
use tabled::Tabled;
use tabled::settings::{Alignment, Modify, location::ByColumnName, object::Rows};
use url::Url;

use dvs::auth::{
    CredentialPolicy, delete_token, get_client_approve_url, get_token, get_token_with,
    initiate_device_approval, poll_for_token, revoke_token, store_token,
};
use dvs::config::{Config, OnConflict};
use dvs::globbing::{resolve_paths_for_add, select_paths_for_get};
use dvs::init::init;
use dvs::paths::{CONFIG_FILE_NAME, DvsPaths};
use dvs::{
    AddDetail, Backend, Compression, ErrorCode, FileMetadata, FileProgress, GetOptions, Outcome,
    PathFilter, Status, StatusDetail, add_files, format_size, get_files, get_status,
    set_num_threads,
};

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
        /// The project name
        name: String,
        /// The URL of the dvs server
        url: Url,
        /// If you want to use a folder name other than `.dvs` for storing the metadata files
        #[clap(long)]
        metadata_folder_name: Option<String>,
        /// Unix group to use for permissions
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
    },
    /// Retrieves the given files from dvs storage. You can use a glob or paths.
    /// If you pass a directory and a glob, the glob will be ran from that directory.
    /// With no paths, gets the tracked files directly in the current directory;
    /// add -r for everything beneath it.
    #[command(next_display_order = 100)]
    Get {
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
        /// What to do with files whose local version dvs did not put there
        /// (local changes, or no record): abort changes nothing, skip leaves
        /// them and gets the rest, overwrite replaces them. Defaults to
        /// DVS_GET_ON_CONFLICT, then [get] on_conflict in dvs.toml, then abort
        #[clap(long, value_name = "POLICY", value_parser = clap::builder::PossibleValuesParser::new(OnConflict::CHOICES))]
        on_conflict: Option<String>,
        /// Replace files with local changes (same as --on-conflict overwrite)
        #[clap(long, short, conflicts_with = "on_conflict")]
        force: bool,
    },
    /// Show the project configuration and the server info is using a dvs-server backend
    Info,
    /// Log in to a DVS server, the one in the config unless a URL is given
    Login { url: Option<Url> },
    /// Log out from a DVS server
    Logout,
}

#[derive(Parser)]
#[clap(version, author, about, subcommand_negates_reqs = true)]
pub struct Cli {
    /// Output results as JSON
    #[clap(long, global = true, display_order = 0)]
    pub json: bool,

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
            hash: m.hashes.blake3.as_str(),
            created_by: m.created_by.as_str(),
            add_time: m.add_time.to_string(),
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
fn run_device_login(url: &Url, machine: bool) -> Result<()> {
    if machine {
        return Err(output::coded(
            "authentication_required",
            "Device authorization requires interactive login; run dvs login without --json",
        ));
    }
    let device_auth = initiate_device_approval(url)?;
    let approve_url = get_client_approve_url(url, &device_auth.user_code)?;
    println!(
        "Go to {approve_url} and enter the code {}",
        device_auth.user_code
    );
    let polling_result = poll_for_token(url, &device_auth.device_code)?;
    store_token(url, &polling_result.access_token)?;
    println!("You are now logged in!");
    Ok(())
}

fn try_main(cli: Cli) -> Result<()> {
    if let Some(n) = cli.threads {
        set_num_threads(n);
    }
    let current_dir = std::env::current_dir()?;
    // C5: machine mode (--json) never changes stored credentials: a rejected
    // token or an unparsable store is reported and left in place. People get
    // them repaired, and are told to run `dvs login`. Only login and logout
    // change credentials on purpose.
    let credentials = if cli.json {
        CredentialPolicy::Preserve
    } else {
        CredentialPolicy::Repair
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
                    // Initializing on the server requires authentication, so run the
                    // login flow first if we don't have a token for it yet.
                    if get_token_with(&url, credentials)?.is_none() {
                        run_device_login(&url, cli.json)?;
                    }
                    let mut config = Config::new_server(name, url, Some(group));
                    config.set_credential_policy(credentials);
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
            if cli.json {
                let mut result = output::envelope("init", false, vec![], 0);
                result["project_root"] = json!(repo_root);
                output::emit(result);
            } else {
                println!("DVS Initialized at {repo_root:?}");
            }
        }
        Command::Attach {
            name,
            url,
            metadata_folder_name,
        } => {
            let config_path = current_dir.join(CONFIG_FILE_NAME);
            if config_path.exists() {
                bail!("Configuration file already exists");
            }

            // Initializing on the server requires authentication, so run the
            // login flow first if we don't have a token for it yet.
            if get_token_with(&url, credentials)?.is_none() {
                run_device_login(&url, cli.json)?;
            }
            let mut config = Config::new_server(name, url, None);
            config.set_credential_policy(credentials);
            config.set_compression(None);
            if let Some(m) = metadata_folder_name {
                config.set_metadata_folder_name(m);
            }

            if let Backend::Server(s) = config.backend() {
                s.attach()?;
                config.save(current_dir)?;
                if cli.json {
                    output::emit(output::envelope("attach", false, vec![], 0));
                }
            }
        }
        Command::Add {
            paths,
            glob,
            message,
            dry_run,
        } => {
            let config = find_config(&current_dir, credentials)?;
            let dvs_paths = DvsPaths::from_cwd(&config)?;
            let all_paths: Vec<_> = resolve_paths_for_add(paths, glob.as_deref(), &dvs_paths)?
                .into_iter()
                .collect();
            if all_paths.is_empty() {
                return Err(output::coded("no_files_selected", "No files to add"));
            }

            let show_progress = !cli.json && !dry_run && std::io::stderr().is_terminal();

            let on_file_start = make_progress_callback(config.progress_bytes_threshold());
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
            let has_errors = results
                .iter()
                .any(|r| matches!(r.detail, AddDetail::Error { .. }));
            if cli.json {
                let rows = results.iter().map(|r| {
                    (
                        serde_json::to_value(r).expect("add results serialize"),
                        matches!(r.detail, AddDetail::Error { .. }),
                    )
                });
                let (files, failed) = output::file_results(rows);
                output::emit(output::envelope("add", dry_run, files, failed));
            } else {
                for result in &results {
                    match &result.detail {
                        AddDetail::Error { error: err } => {
                            eprintln!("Error adding {}: {err}", result.path.display());
                        }
                        AddDetail::Success {
                            outcome,
                            hash,
                            size,
                            stored_size,
                        } => match outcome {
                            Outcome::Copied => {
                                let msg = if dry_run { "To add" } else { "Added" };
                                let stored_info = if size != stored_size {
                                    format!(" --> saved [{}]", format_size(*stored_size))
                                } else {
                                    String::new()
                                };
                                println!(
                                    "{msg}: {} [{}]{stored_info} as {hash}",
                                    result.path.display(),
                                    format_size(*size),
                                );
                            }
                            Outcome::Present => {}
                        },
                    }
                }
            }
            if has_errors {
                if !cli.json {
                    eprintln!("Error: Some files failed to add");
                }
                return Err(Exit(1).into());
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
        } => {
            let config = find_config(&current_dir, credentials)?;
            let dvs_paths = DvsPaths::from_cwd(&config)?;
            let show_all = !current && !absent && !unsynced;

            let filter = if user_paths.is_empty() {
                None
            } else {
                Some(PathFilter::from_user_paths(
                    user_paths, recursive, &dvs_paths,
                ))
            };
            let mut statuses = get_status(&dvs_paths, filter.as_ref(), glob.as_deref())?;
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
            if cli.json {
                let rows = statuses.iter().map(|r| {
                    (
                        serde_json::to_value(r).expect("status results serialize"),
                        matches!(r.detail, StatusDetail::Error { .. }),
                    )
                });
                let (files, failed) = output::file_results(rows);
                output::emit(output::envelope("status", false, files, failed));
            } else if statuses.is_empty() {
                if show_all {
                    println!("No tracked files");
                } else {
                    println!("No tracked files matching the filter")
                }
            } else {
                for file_status in &statuses {
                    if let StatusDetail::Error { error } = &file_status.detail {
                        eprintln!(
                            "Error getting status for {}: {error}",
                            file_status.path.display()
                        );
                    }
                }

                if with_metadata {
                    let rows: Vec<StatusRowFull> = statuses
                        .iter()
                        .filter_map(|fs| match &fs.detail {
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
                        })
                        .collect();
                    let mut table = tabled::Table::new(rows);
                    table
                        .with(Modify::new(ByColumnName::new("size")).with(Alignment::right()))
                        .with(Modify::new(Rows::first()).with(Alignment::left()));
                    println!("{table}");
                } else {
                    let rows: Vec<StatusRow> = statuses
                        .iter()
                        .filter_map(|fs| match &fs.detail {
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
                        })
                        .collect();
                    let mut table = tabled::Table::new(rows);
                    table
                        .with(Modify::new(ByColumnName::new("size")).with(Alignment::right()))
                        .with(Modify::new(Rows::first()).with(Alignment::left()));
                    println!("{table}");
                }
            }
            if has_errors {
                if !cli.json {
                    eprintln!("Error: Some files failed to get status");
                }
                return Err(Exit(1).into());
            }
        }
        Command::Get {
            paths,
            glob,
            recursive,
            dry_run,
            on_conflict,
            force,
        } => {
            let setup = || -> Result<_> {
                let config = find_config(&current_dir, credentials)?;
                let explicit = if force {
                    Some(OnConflict::Overwrite)
                } else {
                    on_conflict
                        .map(|v| v.parse().map_err(|e: String| anyhow!(e)))
                        .transpose()?
                };
                let (on_conflict, source) = config.get_on_conflict(explicit)?;
                let dvs_paths = DvsPaths::from_cwd(&config)?;
                let selection =
                    select_paths_for_get(paths, glob.as_deref(), &dvs_paths, recursive)?;
                Ok((config, on_conflict, source, dvs_paths, selection))
            };
            let (config, on_conflict, source, dvs_paths, selection) = setup()?;

            let show_progress = !cli.json && !dry_run && std::io::stderr().is_terminal();
            let on_file_start = make_progress_callback(config.progress_bytes_threshold());
            let report = get_files(
                selection,
                &dvs_paths,
                config.backend(),
                GetOptions {
                    dry_run,
                    on_conflict,
                    source,
                },
                if show_progress {
                    Some(&on_file_start)
                } else {
                    None
                },
            );
            if cli.json {
                let mut value = get_cmd::to_json(&report);
                value["error"] = report
                    .problem
                    .as_ref()
                    .map(|message| json!({"code": "operation_failed", "message": message}))
                    .unwrap_or(json!(null));
                output::emit(value);
            } else {
                get_cmd::print_human(&report);
            }
            let code = get_cmd::exit_code(&report);
            if code != get_cmd::EXIT_OK {
                return Err(Exit(code).into());
            }
        }
        Command::Info => {
            let config = find_config(&current_dir, credentials)?;
            let compression = config.compression().to_string();
            let dvs_paths = DvsPaths::from_cwd(&config)?;
            let info_json = |fields: serde_json::Value| {
                let mut result = output::envelope("info", false, vec![], 0);
                for (key, value) in fields.as_object().expect("info fields are an object") {
                    result[key] = value.clone();
                }
                result["project_root"] = json!(dvs_paths.repo_root());
                result["metadata_directory"] = json!(dvs_paths.metadata_folder());
                result["protocol"] = json!({"versions": [1], "capabilities": ["json_results", "get_conflict_policy", "connectivity_status"]});
                output::emit(result);
            };

            match config.backend() {
                Backend::Local(l) => {
                    if cli.json {
                        info_json(json!({
                            "version": dvs::VERSION,
                            "backend": "local",
                            "storage": l.path,
                            "compression": compression,
                            "connection": "not_applicable",
                        }));
                    } else {
                        println!("dvs {}", dvs::VERSION);
                        println!("backend     local");
                        println!("storage     {}", l.path.display());
                        println!("compression {compression}");
                    }
                }
                Backend::Server(s) => {
                    // Not logged in is an answer, not an error: no token, or
                    // nowhere a login could have been stored (`auth_required`)
                    let logged_in = if cli.json {
                        match get_token_with(&s.url, credentials) {
                            Ok(token) => token.is_some(),
                            Err(error)
                                if dvs::backend_error_code(&error)
                                    == Some(ErrorCode::AuthRequired) =>
                            {
                                false
                            }
                            Err(error) => {
                                return Err(output::coded(
                                    ErrorCode::CredentialsUnreadable.as_str(),
                                    format!("Cannot read credentials: {error}"),
                                ));
                            }
                        }
                    } else {
                        get_token(&s.url)?.is_some()
                    };
                    let (access, connection, connection_error) = if logged_in {
                        match s.access() {
                            Ok(access) => (access, "available", None),
                            Err(error)
                                if cli.json
                                    && dvs::backend_error_code(&error)
                                        == Some(ErrorCode::ServerUnavailable) =>
                            {
                                (None, "unavailable", Some(format!("{error:#}")))
                            }
                            Err(error) => return Err(error),
                        }
                    } else {
                        (None, "unknown", None)
                    };
                    if cli.json {
                        info_json(json!({
                            "version": dvs::VERSION,
                            "backend": "server",
                            "project": s.name,
                            "server": s.url,
                            "compression": compression,
                            "logged_in": logged_in,
                            "connection": connection,
                            "connection_error": connection_error,
                            "user": access.as_ref().map(|a| &a.user),
                            "access": if !logged_in || connection == "available" {
                                json!(access.as_ref().map_or("none", |a| a.access.as_str()))
                            } else {
                                serde_json::Value::Null
                            },
                            "groups": if connection == "unavailable" {
                                serde_json::Value::Null
                            } else {
                                json!(access.as_ref().map_or(&[][..], |a| &a.groups))
                            },
                        }));
                    } else {
                        let project = match (&access, logged_in) {
                            (Some(a), true) => format!("{} ({})", s.name, a.access),
                            (None, true) if connection == "available" => {
                                format!("{} (no access)", s.name)
                            }
                            _ => s.name.clone(),
                        };
                        println!("dvs {}", dvs::VERSION);
                        println!("backend     server");
                        println!("project     {project}");
                        println!("server      {}", s.url);
                        if !logged_in {
                            println!("user        not logged in, run `dvs login`");
                        } else if let Some(a) = &access {
                            match &a.user.email {
                                Some(email) => {
                                    println!("user        {} ({email})", a.user.username)
                                }
                                None => println!("user        {}", a.user.username),
                            }
                            if !a.groups.is_empty() {
                                println!("groups      {}", a.groups.join(", "));
                            }
                        }
                        println!("compression {compression}");
                    }
                }
            }
        }
        Command::Login { url: Some(url) } => {
            run_device_login(&url, cli.json)?;
        }
        Command::Login { url: None } => {
            let config = find_config(&current_dir, credentials)?;

            if let Some(url) = config.server_url() {
                run_device_login(url, cli.json)?;
            } else {
                bail!("This project is not using a dvs server");
            }
        }
        Command::Logout => {
            let config = find_config(&current_dir, credentials)?;

            if let Some(url) = config.server_url() {
                let Some(token) = get_token(url)? else {
                    if cli.json {
                        let mut result = output::envelope("logout", false, vec![], 0);
                        result["revoked"] = json!(null);
                        result["warnings"] = json!([]);
                        output::emit(result);
                    } else {
                        println!("You were not logged in");
                    }
                    return Ok(());
                };
                // The local copy goes regardless: the server may be unreachable, and keeping
                // a token the user asked to drop is worse than one revoke call too few.
                let revoke_error = revoke_token(url, &token).err();
                if cli.json {
                    if let Err(delete_error) = delete_token(url) {
                        let mut result = output::problem("logout", false, &delete_error);
                        result["revoked"] = json!(revoke_error.is_none());
                        result["warnings"] = json!(
                            revoke_error
                                .as_ref()
                                .map(|e| json!({"code": "revoke_failed", "message": e.to_string()}))
                                .into_iter()
                                .collect::<Vec<_>>()
                        );
                        output::emit(result);
                        return Err(Exit(1).into());
                    }
                    let mut result = output::envelope("logout", false, vec![], 0);
                    result["revoked"] = json!(revoke_error.is_none());
                    result["warnings"] = json!(
                        revoke_error
                            .as_ref()
                            .map(|e| json!({"code": "revoke_failed", "message": e.to_string()}))
                            .into_iter()
                            .collect::<Vec<_>>()
                    );
                    output::emit(result);
                } else {
                    if let Some(e) = &revoke_error {
                        eprintln!("Warning: could not revoke the token on the server: {e}");
                    }
                    delete_token(url)?;
                    println!("Logged out successfully");
                }
            } else {
                bail!("This project is not using a dvs server");
            }
        }
    }
    Ok(())
}

/// A command that already reported its own result and only needs to set the
/// exit status.
#[derive(Debug)]
struct Exit(i32);

impl std::fmt::Display for Exit {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "exit {}", self.0)
    }
}

impl std::error::Error for Exit {}

fn find_config(current_dir: &std::path::Path, credentials: CredentialPolicy) -> Result<Config> {
    let mut config = Config::find(current_dir)
        .ok_or_else(|| output::coded("not_in_project", "Not in a DVS repository"))??;
    config.set_credential_policy(credentials);
    Ok(config)
}

impl Command {
    fn name(&self) -> &'static str {
        match self {
            Self::Init(_) => "init",
            Self::Attach { .. } => "attach",
            Self::Add { .. } => "add",
            Self::Status { .. } => "status",
            Self::Get { .. } => "get",
            Self::Info => "info",
            Self::Login { .. } => "login",
            Self::Logout => "logout",
        }
    }

    fn dry_run(&self) -> bool {
        matches!(
            self,
            Self::Add { dry_run: true, .. } | Self::Get { dry_run: true, .. }
        )
    }
}

fn main() {
    env_logger::init();
    let cli = match Cli::try_parse() {
        Ok(cli) => cli,
        Err(error) => {
            // Help/version remain the conventional human-readable clap output.
            if error.use_stderr()
                && std::env::args_os()
                    .take_while(|arg| arg != "--")
                    .any(|arg| arg == "--json")
            {
                let error = output::coded("invalid_arguments", error.to_string());
                output::emit(output::problem("cli", false, &error));
                std::process::exit(2);
            }
            error.exit();
        }
    };
    let json = cli.json;
    let command = cli.command.name();
    let dry_run = cli.command.dry_run();
    if let Err(error) = try_main(cli) {
        if let Some(Exit(code)) = error.downcast_ref::<Exit>() {
            std::process::exit(*code);
        }
        if json {
            output::emit(output::problem(command, dry_run, &error));
        } else {
            eprintln!("Error: {error}");
        }
        std::process::exit(output::exit_code(&error));
    }
}
