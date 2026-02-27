use std::path::PathBuf;

use anyhow::{Result, anyhow};
use clap::{Parser, Subcommand};
use serde_json::json;

use dvs::AddDetail;
use dvs::GetDetail;
use dvs::StatusDetail;
use dvs::add_files;
use dvs::config::Config;
use dvs::globbing::{resolve_paths_for_add, resolve_paths_for_get};
use dvs::init::init;
use dvs::paths::DvsPaths;
use dvs::{Compression, Status};
use dvs::{Outcome, get_files, get_status};

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
        /// Unix permissions for storage directory and files (octal, e.g., "770")
        #[clap(long)]
        permissions: Option<String>,
        /// Unix group to set on storage directory and files
        #[clap(long)]
        group: Option<String>,
        /// Disable compression of stored files. Compression defaults to zstd
        #[clap(long)]
        no_compression: bool,
    },
    /// Adds the given files to dvs. You can use a glob or paths.
    /// If you pass a directory and a glob, the glob will be ran from that directory
    Add {
        #[clap(required_unless_present = "glob")]
        paths: Vec<PathBuf>,
        #[clap(long)]
        glob: Option<String>,
        #[clap(long, short)]
        message: Option<String>,
    },
    /// Gets the status of each files in the current repository
    Status {
        #[clap(long)]
        current: bool,
        #[clap(long)]
        absent: bool,
        #[clap(long)]
        unsynced: bool,
    },
    /// Retrieves the given files from dvs storage. You can use a glob or paths.
    /// If you pass a directory and a glob, the glob will be ran from that directory
    Get {
        #[clap(required_unless_present = "glob")]
        paths: Vec<PathBuf>,
        #[clap(long, short)]
        glob: Option<String>,
    },
}

#[derive(Parser)]
#[clap(version, author, about, subcommand_negates_reqs = true)]
pub struct Cli {
    /// Output results as JSON
    #[clap(long, global = true)]
    pub json: bool,

    #[clap(subcommand)]
    pub command: Command,
}

fn try_main() -> Result<()> {
    env_logger::init();

    let cli = Cli::parse();
    let current_dir = std::env::current_dir()?;

    match cli.command {
        Command::Init {
            path,
            root_dir,
            metadata_folder_name,
            permissions,
            group,
            no_compression,
        } => {
            let mut config = Config::new_local(path, permissions, group)?;
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

            let results = add_files(
                all_paths,
                &dvs_paths,
                config.backend(),
                message,
                config.compression(),
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
                        AddDetail::Success { .. } => {
                            println!("Added: {}", result.path.display());
                        }
                    }
                }
            }
            if has_errors {
                return Err(anyhow!("Some files failed to add"));
            }
        }
        Command::Status {
            current,
            absent,
            unsynced,
        } => {
            let config =
                Config::find(&current_dir).ok_or_else(|| anyhow!("Not in a DVS repository"))??;
            let paths = DvsPaths::from_cwd(&config)?;
            let show_all = !current && !absent && !unsynced;

            let mut statuses = get_status(&paths)?;
            if !show_all {
                statuses.retain(|x| match &x.detail {
                    StatusDetail::Success { status } => {
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
                    match &file_status.detail {
                        StatusDetail::Success { status } => {
                            println!("{}: {:?}", file_status.path.display(), status);
                        }
                        StatusDetail::Error { error } => {
                            eprintln!(
                                "Error getting status for {}: {error}",
                                file_status.path.display()
                            );
                        }
                    }
                }
            }
            if has_errors {
                return Err(anyhow!("Some files failed to get status"));
            }
        }
        Command::Get { paths, glob } => {
            let config =
                Config::find(&current_dir).ok_or_else(|| anyhow!("Not in a DVS repository"))??;
            let dvs_paths = DvsPaths::from_cwd(&config)?;
            let all_paths: Vec<_> = resolve_paths_for_get(paths, glob.as_deref(), &dvs_paths)?
                .into_iter()
                .collect();
            if all_paths.is_empty() {
                return Err(anyhow!("No files to get"));
            }

            let results = get_files(all_paths, &dvs_paths, config.backend())?;
            let has_errors = results
                .iter()
                .any(|r| matches!(r.detail, GetDetail::Error { .. }));
            if cli.json {
                println!("{}", serde_json::to_string(&results)?);
            } else {
                for result in &results {
                    match &result.detail {
                        GetDetail::Success { outcome } => match outcome {
                            Outcome::Copied => {
                                println!("Retrieved: {}", result.path.display())
                            }
                            Outcome::Present => {
                                println!("Up to date: {}", result.path.display())
                            }
                        },
                        GetDetail::Error { error } => {
                            eprintln!("Error: {} - {}", result.path.display(), error)
                        }
                    }
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
