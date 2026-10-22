mod get_cmd;

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
    delete_token, get_client_approve_url, get_token, initiate_device_approval, poll_for_token,
    revoke_token, store_token,
};
use dvs::config::{Config, ConfigError, OnConflict};
use dvs::globbing::{resolve_paths_for_add, select_paths_for_get};
use dvs::init::init;
use dvs::paths::{CONFIG_FILE_NAME, DvsPaths};
use dvs::{
    AddDetail, Backend, Compression, FileMetadata, FileProgress, GetOptions, Outcome, PathFilter,
    Status, StatusDetail, add_files, format_size, get_files, get_status, set_num_threads,
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
fn run_device_login(url: &Url) -> Result<()> {
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

fn try_main() -> Result<()> {
    env_logger::init();

    let cli = Cli::parse();
    if let Some(n) = cli.threads {
        set_num_threads(n);
    }
    let current_dir = std::env::current_dir()?;

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
                    if get_token(&url)?.is_none() {
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
            if cli.json {
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
            let config_path = current_dir.join(CONFIG_FILE_NAME);
            if config_path.exists() {
                bail!("Configuration file already exists");
            }

            // Initializing on the server requires authentication, so run the
            // login flow first if we don't have a token for it yet.
            if get_token(&url)?.is_none() {
                run_device_login(&url)?;
            }
            let mut config = Config::new_server(name, url, None);
            config.set_compression(None);
            if let Some(m) = metadata_folder_name {
                config.set_metadata_folder_name(m);
            }

            if let Backend::Server(s) = config.backend() {
                s.attach()?;
                config.save(current_dir)?;
            }
        }
        Command::Add {
            paths,
            glob,
            message,
            dry_run,
        } => {
            let config =
                Config::find(&current_dir).ok_or_else(|| anyhow!("Not in a DVS repository"))??;
            let dvs_paths = DvsPaths::from_cwd(&config)?;
            let all_paths: Vec<_> = resolve_paths_for_add(paths, glob.as_deref(), &dvs_paths)?
                .into_iter()
                .collect();
            if all_paths.is_empty() {
                return Err(anyhow!("No files to add"));
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
                println!("{}", serde_json::to_string(&results)?);
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
                return Err(anyhow!("Some files failed to add"));
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
            let config =
                Config::find(&current_dir).ok_or_else(|| anyhow!("Not in a DVS repository"))??;
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
                println!("{}", serde_json::to_string(&statuses)?);
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
                return Err(anyhow!("Some files failed to get status"));
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
                let config = Config::find(&current_dir)
                    .ok_or_else(|| anyhow!("Not in a DVS repository"))??;
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
            // Scripts get the same envelope when get cannot start at all.
            let (config, on_conflict, source, dvs_paths, selection) = match setup() {
                Ok(v) => v,
                Err(e) if cli.json => {
                    println!("{}", get_cmd::problem_json(&format!("{e:#}"), dry_run));
                    let code = if e.downcast_ref::<ConfigError>().is_some() {
                        get_cmd::EXIT_USAGE
                    } else {
                        get_cmd::EXIT_FAILED
                    };
                    return Err(Exit(code).into());
                }
                Err(e) => return Err(e),
            };

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
                println!("{}", get_cmd::to_json(&report));
            } else {
                get_cmd::print_human(&report);
            }
            let code = get_cmd::exit_code(&report);
            if code != get_cmd::EXIT_OK {
                return Err(Exit(code).into());
            }
        }
        Command::Info => {
            let config =
                Config::find(&current_dir).ok_or_else(|| anyhow!("Not in a DVS repository"))??;
            let compression = config.compression().to_string();

            match config.backend() {
                Backend::Local(l) => {
                    if cli.json {
                        println!(
                            "{}",
                            json!({
                                "version": dvs::VERSION,
                                "backend": "local",
                                "storage": l.path,
                                "compression": compression,
                            })
                        );
                    } else {
                        println!("dvs {}", dvs::VERSION);
                        println!("backend     local");
                        println!("storage     {}", l.path.display());
                        println!("compression {compression}");
                    }
                }
                Backend::Server(s) => {
                    // Not logged in is an answer, not an error
                    let logged_in = get_token(&s.url)?.is_some();
                    let access = if logged_in { s.access()? } else { None };
                    if cli.json {
                        println!(
                            "{}",
                            json!({
                                "version": dvs::VERSION,
                                "backend": "server",
                                "project": s.name,
                                "server": s.url,
                                "compression": compression,
                                "logged_in": logged_in,
                                "user": access.as_ref().map(|a| &a.user),
                                "access": access.as_ref().map_or("none", |a| a.access.as_str()),
                                "groups": access.as_ref().map_or(&[][..], |a| &a.groups),
                            })
                        );
                    } else {
                        let project = match (&access, logged_in) {
                            (Some(a), _) => format!("{} ({})", s.name, a.access),
                            (None, true) => format!("{} (no access)", s.name),
                            (None, false) => s.name.clone(),
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
            run_device_login(&url)?;
        }
        Command::Login { url: None } => {
            let config =
                Config::find(&current_dir).ok_or_else(|| anyhow!("Not in a DVS repository"))??;

            if let Some(url) = config.server_url() {
                run_device_login(url)?;
            } else {
                bail!("This project is not using a dvs server");
            }
        }
        Command::Logout => {
            let config =
                Config::find(&current_dir).ok_or_else(|| anyhow!("Not in a DVS repository"))??;

            if let Some(url) = config.server_url() {
                let Some(token) = get_token(url)? else {
                    println!("You were not logged in");
                    return Ok(());
                };
                // The local copy goes regardless: the server may be unreachable, and keeping
                // a token the user asked to drop is worse than one revoke call too few.
                if let Err(e) = revoke_token(url, &token) {
                    eprintln!("Warning: could not revoke the token on the server: {e}");
                }
                delete_token(url)?;
                println!("Logged out successfully");
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

fn main() {
    if let Err(e) = try_main() {
        if let Some(Exit(code)) = e.downcast_ref::<Exit>() {
            ::std::process::exit(*code)
        }
        eprintln!("Error: {e}");
        // Configuration problems are usage errors, like clap's: nothing ran.
        let code = if e.downcast_ref::<ConfigError>().is_some() {
            2
        } else {
            1
        };
        ::std::process::exit(code)
    }
}
