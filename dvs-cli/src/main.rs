use std::path::PathBuf;

use anyhow::{Result, anyhow};
use clap::{Parser, Subcommand};
use dvs::audit::parse_audit_log;
use dvs::find_repo_root;
use fs_err as fs;
use serde_json::json;

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
        /// If you want to use a folder name other than `.dvs` for sotring the metadata files
        #[clap(long)]
        metadata_folder_name: Option<String>,
        /// Unix permissions for storage directory and files (octal, e.g., "770")
        #[clap(long)]
        permissions: Option<String>,
        /// Unix group to set on storage directory and files
        #[clap(long)]
        group: Option<String>,
    },
    /// Adds the given files to dvs. You can use a glob or paths
    Add {
        #[clap(required = true)]
        paths: Vec<PathBuf>,
        #[clap(long)]
        message: Option<String>,
    },
    /// Gets the status of each files in the current repository
    Status,
    /// Retrieves the given files from dvs storage. You can use a glob or paths
    Get {
        #[clap(required = true)]
        paths: Vec<PathBuf>,
    },
    Audit,
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
        } => {
            let mut config = Config::new_local(path, permissions, group)?;
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
        Command::Add { paths, message } => {
            let config =
                Config::find(&current_dir).ok_or_else(|| anyhow!("Not in a DVS repository"))??;
            let dvs_paths = DvsPaths::from_cwd(&config)?;

            let results = add_files(paths, &dvs_paths, config.backend(), message)?;
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
        Command::Get { paths } => {
            let config =
                Config::find(&current_dir).ok_or_else(|| anyhow!("Not in a DVS repository"))??;
            let dvs_paths = DvsPaths::from_cwd(&config)?;

            let results = get_files(paths, &dvs_paths, config.backend())?;
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
        Command::Audit => {
            let config =
                Config::find(&current_dir).ok_or_else(|| anyhow!("Not in a DVS repository"))??;
            // FIXME: should the DvsPaths contain the path to the storage directory?
            let _dvs_paths = DvsPaths::from_cwd(&config)?;

            let storage = find_repo_root(current_dir)
                .expect("Not in a DVS repository")
                .join(".storage");
            let audit_path = storage.join("audit.log.jsonl");
            assert!(audit_path.is_file());

            let content = fs::read(&audit_path).unwrap();
            let audit_log_entries = parse_audit_log(&content).unwrap();
            if cli.json {
                println!("{}", serde_json::to_string(&audit_log_entries)?);
            } else if audit_log_entries.is_empty() {
                println!("No audit log available in this DVS repository");
            } else {
                for entry in audit_log_entries {
                    println!("{}", entry)
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
