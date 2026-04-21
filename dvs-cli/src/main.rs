use std::io::IsTerminal;
use std::path::PathBuf;

use anyhow::bail;
use anyhow::{Result, anyhow};
use clap::{Parser, Subcommand};
use indicatif::{MultiProgress, ProgressBar, ProgressStyle};
use serde_json::json;
use tabled::Tabled;

use dvs::config::Config;
use dvs::globbing::{resolve_paths_for_add, resolve_paths_for_get};
use dvs::init::init;
use dvs::paths::DvsPaths;
use dvs::{
    AddDetail, Compression, FileMetadata, FileProgress, GetDetail, Outcome, Status, StatusDetail,
    StatusFilter, add_files, format_size, get_files, get_status, set_num_threads,
};

#[derive(Debug, Subcommand)]
pub enum Command {
    /// Starts a new dvs project.
    /// This will create a `dvs.toml` file in the current folder of where the user is calling the CLI
    /// from.
    Init {
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
    /// Adds the given files to dvs. You can use a glob or paths.
    /// If you pass a directory and a glob, the glob will be ran from that directory.
    /// At least one path or --glob must be provided
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
    Status {
        /// Paths (files or directories) to check status for
        paths: Vec<PathBuf>,
        /// Recursively include files in subdirectories for given directories
        #[clap(long, short)]
        recursive: bool,
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
    /// At least one path or --glob must be provided
    Get {
        #[clap(required_unless_present = "glob")]
        paths: Vec<PathBuf>,
        #[clap(long, short)]
        glob: Option<String>,
        /// Show what would be retrieved without making any actual changes
        #[clap(long)]
        dry_run: bool,
    },
}

#[derive(Parser)]
#[clap(version, author, about, subcommand_negates_reqs = true)]
pub struct Cli {
    /// Output results as JSON
    #[clap(long, global = true)]
    pub json: bool,

    /// Number of threads for parallel operations (0 = auto-detect)
    #[clap(long, global = true)]
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
    add_time: &'a str,
    compression: String,
    message: &'a str,
}

impl<'a> From<&'a FileMetadata> for StatusRowFull<'a> {
    fn from(m: &'a FileMetadata) -> Self {
        Self {
            size: m.size.to_string(),
            hash: m.hashes.blake3.as_str(),
            created_by: m.created_by.as_str(),
            add_time: m.add_time.as_str(),
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

fn try_main() -> Result<()> {
    env_logger::init();

    let cli = Cli::parse();
    if let Some(n) = cli.threads {
        set_num_threads(n);
    }
    let current_dir = std::env::current_dir()?;

    match cli.command {
        Command::Init {
            path: storage_path,
            root_dir,
            metadata_folder_name,
            group,
            no_compression,
        } => {
            let mut config = Config::new_local(&storage_path, group)?;
            if no_compression {
                config.set_compression(Compression::None);
            }
            if let Some(m) = metadata_folder_name {
                config.set_metadata_folder_name(m);
            }
            let root = if let Some(root) = root_dir {
                root
            } else {
                current_dir
            };

            let abs_storage = if storage_path.exists() {
                std::fs::canonicalize(&storage_path)?
            } else if let Some(parent) = storage_path.parent().filter(|p| p.exists()) {
                std::fs::canonicalize(parent)?.join(storage_path.file_name().unwrap())
            } else {
                std::path::absolute(&storage_path)?
            };
            let abs_root = std::fs::canonicalize(&root)?;
            if abs_storage.starts_with(&abs_root) {
                bail!("The given storage path is within the repository.")
            }

            let repo_root = init(&root, config)?;
            if cli.json {
                println!("{}", json!({"status": "initialized"}));
            } else {
                println!("DVS Initialized at {repo_root:?}");
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
                                let stored_info = match stored_size {
                                    Some(ss) => {
                                        format!(" --> saved [{}]", format_size(*ss))
                                    }
                                    None => String::new(),
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
        } => {
            let config =
                Config::find(&current_dir).ok_or_else(|| anyhow!("Not in a DVS repository"))??;
            let dvs_paths = DvsPaths::from_cwd(&config)?;
            let show_all = !current && !absent && !unsynced;

            let filter = if user_paths.is_empty() {
                None
            } else {
                Some(StatusFilter::from_user_paths(
                    user_paths, recursive, &dvs_paths,
                )?)
            };
            let mut statuses = get_status(&dvs_paths, filter.as_ref())?;
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
                    let table = tabled::Table::new(rows).to_string();
                    println!("{table}");
                } else {
                    let rows: Vec<StatusRow> = statuses
                        .iter()
                        .filter_map(|fs| match &fs.detail {
                            StatusDetail::Success { status, metadata } => {
                                let size = match metadata {
                                    Some(m) => m.size.to_string(),
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
                    let table = tabled::Table::new(rows).to_string();
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
            dry_run,
        } => {
            let config =
                Config::find(&current_dir).ok_or_else(|| anyhow!("Not in a DVS repository"))??;
            let dvs_paths = DvsPaths::from_cwd(&config)?;
            let all_paths: Vec<_> = resolve_paths_for_get(paths, glob.as_deref(), &dvs_paths)?
                .into_iter()
                .collect();
            if all_paths.is_empty() {
                return Err(anyhow!("No files to get"));
            }

            let show_progress = !cli.json && !dry_run && std::io::stderr().is_terminal();

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
            let has_errors = results
                .iter()
                .any(|r| matches!(r.detail, GetDetail::Error { .. }));
            if cli.json {
                println!("{}", serde_json::to_string(&results)?);
            } else {
                let mut total_files = 0u64;
                let mut total_bytes = 0u64;
                for result in &results {
                    match &result.detail {
                        GetDetail::Success { outcome, size } => {
                            if *outcome == Outcome::Copied {
                                println!("{} [{}]", result.path.display(), format_size(*size));
                                total_files += 1;
                                total_bytes += size;
                            }
                        }
                        GetDetail::Error { error } => {
                            eprintln!("Error: {} - {}", result.path.display(), error)
                        }
                    }
                }
                if total_files > 0 {
                    println!("Total: {} files, {}", total_files, format_size(total_bytes));
                }
            }
            if has_errors {
                return Err(anyhow!("Some files failed to get"));
            }
        }
    }
    Ok(())
}

fn main() {
    if let Err(e) = try_main() {
        eprintln!("{e:?}");
        ::std::process::exit(1)
    }
}
