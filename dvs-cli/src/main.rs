mod globbing;

use std::path::PathBuf;

use anyhow::{Result, anyhow};
use clap::{Parser, Subcommand};
use serde_json::json;

use crate::globbing::{resolve_paths_for_add, resolve_paths_for_get};
use dvs::Compression;
use dvs::config::Config;
use dvs::file::{Outcome, add_files, get_files, get_status};
use dvs::init::init;
use dvs::paths::DvsPaths;

#[derive(Debug, Subcommand)]
pub enum Command {
    /// Starts a new dvs project.
    /// This will create a `dvs.toml` file in the root folder of where the user is calling the CLI
    /// from. root folder being the place where we find a `.git` folder
    Init {
        /// Where the data will be stored
        path: PathBuf,
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
        #[clap(long)]
        message: Option<String>,
    },
    /// Gets the status of each files in the current repository
    Status,
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
            init(&current_dir, config)?;
            if cli.json {
                println!("{}", json!({"status": "initialized"}));
            } else {
                println!("DVS Initialized");
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
            if cli.json {
                println!("{}", serde_json::to_string(&results)?);
            } else {
                for result in results {
                    println!("Added: {}", result.path.display());
                }
            }
        }
        Command::Status => {
            let config =
                Config::find(&current_dir).ok_or_else(|| anyhow!("Not in a DVS repository"))??;
            let paths = DvsPaths::from_cwd(&config)?;

            let statuses = get_status(&paths)?;
            if cli.json {
                println!("{}", serde_json::to_string(&statuses)?);
            } else if statuses.is_empty() {
                println!("No tracked files");
            } else {
                for file_status in statuses {
                    println!("{}: {:?}", file_status.path.display(), file_status.status);
                }
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
            if cli.json {
                println!("{}", serde_json::to_string(&results)?);
            } else {
                for result in results {
                    match result.outcome {
                        Outcome::Copied => println!("Retrieved: {}", result.path.display()),
                        Outcome::Present => println!("Up to date: {}", result.path.display()),
                    }
                }
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
