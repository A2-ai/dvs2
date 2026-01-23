use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Result, anyhow};
use clap::{Parser, Subcommand};

use dvs::config::{Backend, Config, find_repo_root};
use dvs::file::{FileMetadata, Outcome, get_file, get_status};
use dvs::init::init;

/// Resolve a path to be relative to the repository root.
/// If `canonicalize` is true, the path is canonicalized first (file must exist).
/// Returns (absolute_path, relative_path).
fn resolve_repo_path(
    path: &Path,
    current_dir: &Path,
    repo_root: &Path,
    canonicalize: bool,
) -> Result<(PathBuf, PathBuf)> {
    let absolute_path = if path.is_absolute() {
        path.to_path_buf()
    } else {
        current_dir.join(path)
    };

    let absolute_path = if canonicalize {
        fs::canonicalize(&absolute_path)?
    } else {
        absolute_path
    };

    let relative_path = absolute_path
        .strip_prefix(repo_root)
        .map(|p| p.to_path_buf())
        .map_err(|_| {
            anyhow!(
                "Path {} is not inside repository {}",
                absolute_path.display(),
                repo_root.display()
            )
        })?;

    Ok((absolute_path, relative_path))
}

#[derive(Debug, Subcommand)]
pub enum Command {
    Init {
        path: PathBuf,
        #[clap(long)]
        metadata_folder_name: Option<String>,
    },
    Add {
        path: PathBuf,
        #[clap(long)]
        message: Option<String>,
    },
    Status,
    Get {
        path: PathBuf,
    },
}

#[derive(Parser)]
#[clap(version, author, about, subcommand_negates_reqs = true)]
pub struct Cli {
    #[clap(subcommand)]
    pub command: Command,
}

fn try_main() -> Result<()> {
    env_logger::init();

    let cli = Cli::parse();
    let current_dir = std::env::current_dir()?;
    let canonical_current_dir = fs::canonicalize(&current_dir)?;
    let repo_root =
        find_repo_root(&current_dir).ok_or_else(|| anyhow!("Not in a git repository"))?;
    let canonical_root = fs::canonicalize(&repo_root)?;

    match cli.command {
        Command::Init {
            path,
            metadata_folder_name,
        } => {
            let mut config = Config::new_local(path);
            if let Some(m) = metadata_folder_name {
                config.set_metadata_folder_name(m);
            }
            init(current_dir, config)?;
            println!("DVS Initialized");
        }
        Command::Add { path, message } => {
            let config =
                Config::find(&current_dir).ok_or_else(|| anyhow!("Failed to read config"))??;
            let (absolute_path, relative_path) =
                resolve_repo_path(&path, &current_dir, &canonical_root, true)?;

            match config.backend() {
                Backend::Local(backend) => {
                    let dvs_dir = repo_root.join(config.metadata_folder_name());
                    let metadata = FileMetadata::from_file(&path, message)?;
                    metadata.save_local(&path, &backend.path, &dvs_dir, &relative_path)?;
                    println!("File {} added successfully to DVS", absolute_path.display());
                }
            }
        }
        Command::Status => {
            let config =
                Config::find(&current_dir).ok_or_else(|| anyhow!("Failed to read config"))??;

            match config.backend() {
                Backend::Local(_) => {
                    let dvs_dir = repo_root.join(config.metadata_folder_name());
                    let statuses = get_status(&repo_root, &dvs_dir)?;
                    if statuses.is_empty() {
                        println!("No tracked files");
                    } else {
                        for file_status in statuses {
                            println!("{}: {:?}", file_status.path.display(), file_status.status);
                        }
                    }
                }
            }
        }
        Command::Get { path } => {
            let config =
                Config::find(&current_dir).ok_or_else(|| anyhow!("Failed to read config"))??;
            // Can't canonicalize since file may not exist yet
            let (_, relative_path) =
                resolve_repo_path(&path, &canonical_current_dir, &canonical_root, false)?;

            match config.backend() {
                Backend::Local(backend) => {
                    let dvs_dir = repo_root.join(config.metadata_folder_name());
                    let outcome = get_file(&backend.path, &dvs_dir, &repo_root, &relative_path)?;
                    match outcome {
                        Outcome::Copied => println!("Retrieved {}", relative_path.display()),
                        Outcome::Present => {
                            println!("{} already up to date", relative_path.display())
                        }
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
