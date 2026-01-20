//! DVS Command-Line Interface
//!
//! Provides the `dvs` binary with subcommands for managing data versioning.

mod commands;
mod output;
mod paths;

use fs_err as fs;
use serde::Serialize;
use std::path::PathBuf;
use std::process::ExitCode;

use clap::{CommandFactory, Parser, Subcommand};

use commands::{
    add, config, get, git_status, init, install, log, materialize, merge_repo, pull, push,
    rollback, status, uninstall,
};
use output::Output;

#[derive(Clone, Copy, Debug, Default, clap::ValueEnum)]
pub enum OutputFormat {
    #[default]
    Human,
    Json,
}

/// Output destination for CLI output.
#[derive(Clone, Debug)]
pub enum OutputDest {
    /// Don't redirect output (write to stdout, default)
    Inherit,
    /// Discard output (like /dev/null)
    Null,
    /// Feed output through a pipe before discarding (avoids /dev/null detection)
    Pipe,
    /// Write to a file path
    File(PathBuf),
}

impl Default for OutputDest {
    fn default() -> Self {
        Self::Inherit
    }
}

impl std::str::FromStr for OutputDest {
    type Err = std::convert::Infallible;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Ok(match s {
            "inherit" | "stdout" | "-" => Self::Inherit,
            "null" | "/dev/null" => Self::Null,
            "pipe" => Self::Pipe,
            path => Self::File(PathBuf::from(path)),
        })
    }
}

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

    /// Output destination.
    ///
    /// `inherit`: Don't redirect output (write to stdout, the default).
    /// `null`:    Redirect output to `/dev/null`.
    /// `pipe`:    Feed output through a pipe before discarding (avoids `/dev/null` detection).
    /// `<FILE>`:  Write output to the given file.
    #[arg(
        short,
        long,
        global = true,
        default_value = "inherit",
        value_name = "WHERE"
    )]
    output: OutputDest,

    /// Suppress non-error output
    #[arg(short, long, global = true)]
    quiet: bool,

    #[command(subcommand)]
    command: Command,
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

    /// View or edit DVS configuration
    #[command(subcommand)]
    Config(ConfigCommand),

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

    /// Merge another DVS repository into this one
    #[command(name = "merge-repo")]
    MergeRepo {
        /// Path to source DVS repository
        source: PathBuf,

        /// Place imported files under this subdirectory
        #[arg(long, value_name = "PATH")]
        prefix: Option<PathBuf>,

        /// How to handle path conflicts (abort, skip, overwrite)
        #[arg(long, default_value = "abort", value_name = "MODE")]
        conflict: String,

        /// Verify object hashes during copy
        #[arg(long)]
        verify: bool,

        /// Show what would be merged without making changes
        #[arg(long)]
        dry_run: bool,
    },
}

#[derive(Subcommand)]
pub enum ConfigCommand {
    /// Show all configuration values
    Show,

    /// Get a specific configuration value
    Get {
        /// Configuration key (storage_dir, permissions, group, hash_algo, metadata_format)
        key: String,
    },

    /// Set a configuration value
    Set {
        /// Configuration key
        key: String,

        /// New value
        value: String,
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

/// JSON output for fs pwd command.
#[derive(Serialize)]
struct PwdOutput {
    path: String,
}

/// JSON output for fs ls command.
#[derive(Serialize)]
struct LsOutput {
    path: String,
    entries: Vec<LsEntry>,
}

#[derive(Serialize)]
struct LsEntry {
    name: String,
    #[serde(rename = "type")]
    entry_type: String,
}

/// Build the CLI command for completion generation.
pub fn build_cli() -> clap::Command {
    Cli::command()
}

fn main() -> ExitCode {
    let cli = Cli::parse();

    // Create output handler
    let output = Output::new(cli.format, cli.output, cli.quiet);

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
                    if output.is_json() {
                        output.json(&PwdOutput {
                            path: cwd.display().to_string(),
                        });
                    } else {
                        output.println(&cwd.display().to_string());
                    }
                    Ok(())
                }
                Err(e) => Err(commands::CliError::Io(e)),
            },
            FsCommand::Ls { path } => {
                let target = path.unwrap_or_else(|| PathBuf::from("."));
                match fs::read_dir(&target) {
                    Ok(dir_entries) => {
                        let mut entries = Vec::new();
                        for entry in dir_entries.flatten() {
                            let name = entry.file_name().to_string_lossy().to_string();
                            let is_dir = entry.file_type().map(|t| t.is_dir()).unwrap_or(false);
                            let entry_type = if is_dir { "directory" } else { "file" };

                            entries.push(LsEntry {
                                name: name.clone(),
                                entry_type: entry_type.to_string(),
                            });

                            if !output.is_json() {
                                if is_dir {
                                    output.println(&format!("{}/", name));
                                } else {
                                    output.println(&name);
                                }
                            }
                        }

                        if output.is_json() {
                            output.json(&LsOutput {
                                path: target.display().to_string(),
                                entries,
                            });
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
        Command::Config(cfg_cmd) => {
            let action = match cfg_cmd {
                ConfigCommand::Show => config::ConfigAction::Show,
                ConfigCommand::Get { key } => config::ConfigAction::Get { key },
                ConfigCommand::Set { key, value } => config::ConfigAction::Set { key, value },
            };
            config::run(&output, action)
        }
        Command::MergeRepo {
            source,
            prefix,
            conflict,
            verify,
            dry_run,
        } => {
            let conflict_mode = match dvs_core::ConflictMode::from_str(&conflict) {
                Some(mode) => mode,
                None => {
                    return {
                        output.error(&format!(
                            "Invalid conflict mode: {}. Use 'abort', 'skip', or 'overwrite'",
                            conflict
                        ));
                        ExitCode::FAILURE
                    }
                }
            };
            merge_repo::run(
                &output,
                merge_repo::MergeRepoOptions {
                    source,
                    prefix,
                    conflict_mode,
                    verify,
                    dry_run,
                },
            )
        }
    };

    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            output.error(&e.to_string());
            ExitCode::FAILURE
        }
    }
}
