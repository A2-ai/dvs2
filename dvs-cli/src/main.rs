//! DVS Command-Line Interface
//!
//! Provides the `dvs` binary with subcommands for managing data versioning.

mod commands;
mod output;
mod paths;

use fs_err as fs;
use std::path::PathBuf;
use std::process::ExitCode;

use clap::{CommandFactory, Parser, Subcommand};

use commands::{
    add, get, git_status, init, install, log, materialize, pull, push, rollback, status, uninstall,
};
use output::Output;

/// DVS - Data Version System
///
/// Version large or sensitive files under Git without tracking them directly.
/// Uses content-addressable storage with blake3 hashing.
#[derive(Parser)]
#[command(name = "dvs")]
#[command(author, version = dvs_core::VERSION_STRING, about, long_about = None)]
#[command(propagate_version = true)]
pub struct Cli {
    /// Change to this directory before running the command
    #[arg(short = 'C', long = "cwd", global = true, value_name = "DIR")]
    cwd: Option<PathBuf>,

    /// Explicit repository root (overrides auto-detection)
    #[arg(long = "repo", global = true, value_name = "DIR")]
    repo: Option<PathBuf>,

    /// Output format
    #[arg(long, global = true, default_value = "human")]
    format: OutputFormat,

    /// Suppress non-error output
    #[arg(short, long, global = true)]
    quiet: bool,

    #[command(subcommand)]
    command: Command,
}

#[derive(Clone, Copy, Debug, Default, clap::ValueEnum)]
pub enum OutputFormat {
    #[default]
    Human,
    Json,
}

#[derive(Subcommand)]
pub enum Command {
    /// Initialize DVS for this repository
    Init {
        /// Path to external storage directory
        storage_dir: PathBuf,

        /// File permissions for stored files (octal, e.g., 664)
        #[arg(long, value_name = "OCTAL")]
        permissions: Option<String>,

        /// Linux group for stored files
        #[arg(long, value_name = "GROUP")]
        group: Option<String>,
    },

    /// Add files to DVS tracking
    Add {
        /// Files or glob patterns to add
        #[arg(required = true)]
        files: Vec<PathBuf>,

        /// Message describing this version
        #[arg(short, long)]
        message: Option<String>,

        /// Metadata file format (json or toml)
        #[arg(long, value_name = "FORMAT")]
        metadata_format: Option<String>,
    },

    /// Retrieve files from DVS storage
    Get {
        /// Files or glob patterns to retrieve
        #[arg(required = true)]
        files: Vec<PathBuf>,
    },

    /// Check status of tracked files
    Status {
        /// Files or glob patterns to check (empty = all tracked files)
        files: Vec<PathBuf>,
    },

    /// Push objects to remote storage
    Push {
        /// Remote URL (overrides manifest base_url)
        #[arg(long, short, value_name = "URL")]
        remote: Option<String>,

        /// Files to push (empty = all objects in manifest)
        files: Vec<PathBuf>,
    },

    /// Pull objects from remote storage
    Pull {
        /// Remote URL (overrides manifest base_url)
        #[arg(long, short, value_name = "URL")]
        remote: Option<String>,

        /// Files to pull (empty = all objects in manifest)
        files: Vec<PathBuf>,
    },

    /// Materialize files from cache to working tree
    Materialize {
        /// Files to materialize (empty = all files in manifest)
        files: Vec<PathBuf>,
    },

    /// View reflog (history of state changes)
    Log {
        /// Maximum number of entries to show
        #[arg(short = 'n', long, value_name = "COUNT")]
        limit: Option<usize>,
    },

    /// Rollback to a previous state
    Rollback {
        /// Target state (state ID or reflog index like @{0})
        target: String,

        /// Force rollback even if working tree is dirty
        #[arg(short, long)]
        force: bool,

        /// Skip materializing data files (only restore metadata)
        #[arg(long)]
        no_materialize: bool,
    },

    /// Filesystem navigation helpers
    #[command(subcommand)]
    Fs(FsCommand),

    /// Install git-status-dvs shim and shell completions
    Install {
        /// Directory to install git-status-dvs shim
        #[arg(long, value_name = "DIR")]
        install_dir: Option<PathBuf>,

        /// Only install shell completions (skip shim)
        #[arg(long)]
        completions_only: bool,

        /// Shells to install completions for (bash, zsh, fish, powershell)
        #[arg(long, value_name = "SHELL")]
        shell: Vec<String>,
    },

    /// Uninstall git-status-dvs shim and shell completions
    Uninstall {
        /// Directory where git-status-dvs shim was installed
        #[arg(long, value_name = "DIR")]
        uninstall_dir: Option<PathBuf>,

        /// Only remove shell completions (skip shim)
        #[arg(long)]
        completions_only: bool,

        /// Shells to remove completions for (bash, zsh, fish, powershell)
        #[arg(long, value_name = "SHELL")]
        shell: Vec<String>,
    },

    /// Combined git status and DVS status
    #[command(name = "git-status")]
    GitStatus {
        /// Additional arguments to pass to git status
        #[arg(trailing_var_arg = true)]
        args: Vec<String>,
    },
}

#[derive(Subcommand)]
pub enum FsCommand {
    /// Print current working directory
    Pwd,

    /// List files in directory
    Ls {
        /// Directory to list (default: current directory)
        path: Option<PathBuf>,
    },
}

/// Build the CLI command for completion generation.
pub fn build_cli() -> clap::Command {
    Cli::command()
}

fn main() -> ExitCode {
    let cli = Cli::parse();

    // Create output handler
    let output = Output::new(cli.format, cli.quiet);

    // Change working directory if requested
    if let Some(ref cwd) = cli.cwd {
        if let Err(e) = paths::set_cwd(cwd) {
            output.error(&format!("Failed to change directory: {}", e));
            return ExitCode::FAILURE;
        }
    }

    // Execute the command
    let result: commands::Result<()> = match cli.command {
        Command::Init {
            storage_dir,
            permissions,
            group,
        } => init::run(&output, storage_dir, permissions, group),
        Command::Add {
            files,
            message,
            metadata_format,
        } => add::run(&output, files, message, metadata_format),
        Command::Get { files } => get::run(&output, files),
        Command::Status { files } => status::run(&output, files),
        Command::Push { remote, files } => push::run(&output, remote, files),
        Command::Pull { remote, files } => pull::run(&output, remote, files),
        Command::Materialize { files } => materialize::run(&output, files),
        Command::Log { limit } => log::run(&output, limit),
        Command::Rollback {
            target,
            force,
            no_materialize,
        } => rollback::run(&output, target, force, !no_materialize),
        Command::Fs(fs_cmd) => match fs_cmd {
            FsCommand::Pwd => match std::env::current_dir() {
                Ok(cwd) => {
                    output.println(&cwd.display().to_string());
                    Ok(())
                }
                Err(e) => Err(commands::CliError::Io(e)),
            },
            FsCommand::Ls { path } => {
                let target = path.unwrap_or_else(|| PathBuf::from("."));
                match fs::read_dir(&target) {
                    Ok(entries) => {
                        for entry in entries.flatten() {
                            let name = entry.file_name();
                            let is_dir = entry.file_type().map(|t| t.is_dir()).unwrap_or(false);
                            if is_dir {
                                output.println(&format!("{}/", name.to_string_lossy()));
                            } else {
                                output.println(&name.to_string_lossy());
                            }
                        }
                        Ok(())
                    }
                    Err(e) => Err(commands::CliError::Io(e)),
                }
            }
        },
        Command::Install {
            install_dir,
            completions_only,
            shell,
        } => install::run(&output, install_dir, completions_only, shell),
        Command::Uninstall {
            uninstall_dir,
            completions_only,
            shell,
        } => uninstall::run(&output, uninstall_dir, completions_only, shell),
        Command::GitStatus { args } => git_status::run(&output, args),
    };

    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            output.error(&e.to_string());
            ExitCode::FAILURE
        }
    }
}
