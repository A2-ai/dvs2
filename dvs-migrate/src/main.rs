//! `dvs-migrate`: moves a dvs project on local storage onto dvs-server, keeping who added
//! each version and when. Run by an admin with a migrate key from the project's Migration
//! page. It only reads the repo and local storage.

mod client;
mod commands;
mod git;
mod report;
mod storage;
mod walk;

use std::collections::BTreeMap;
use std::io::Write as _;
use std::path::PathBuf;
use std::time::{Instant, SystemTime};

use anyhow::{Context, Result};
use clap::{Parser, Subcommand};
use dvs::ErrorCode;
use dvs::backends::server::coded;
use dvs::operation::{duration_ms, rfc3339_ms};
use serde::Deserialize;
use url::Url;

use crate::client::Client;
use crate::commands::error_json;
use crate::report::{Command, Destination, Report};

#[derive(Parser)]
#[command(name = "dvs-migrate", version, about)]
/// Moves a dvs project's history from local storage onto dvs-server
struct Cli {
    #[command(subcommand)]
    command: Cmd,
}

#[derive(Subcommand)]
enum Cmd {
    /// Walk every commit on every branch and tag and summarize what a run would do; sends nothing
    Plan(Args),
    /// Walk again, upload the content the server lacks and record who added each version
    ///
    /// The server keeps each run's report on the project's Migration page. Run again until
    /// nothing new is sent, then make local storage read-only.
    Run(Args),
}

#[derive(clap::Args)]
struct Args {
    /// The project's git repo: a local clone (its branches and its remotes are walked) or a URL
    repo: String,
    /// dvs-server URL
    #[arg(long, value_name = "URL")]
    server: Option<Url>,
    /// Project name on the server
    #[arg(long, value_name = "NAME")]
    project: Option<String>,
    /// Migrate key from the project's Migration page
    #[arg(long, env = "DVS_MIGRATE_KEY", hide_env_values = true)]
    key: String,
    /// A storage path as written in the project's config, and where it is now (repeatable)
    #[arg(long = "storage-map", value_name = "OLD=NEW", value_parser = parse_mapping)]
    storage_map: Vec<(String, String)>,
    /// TOML file with `server`, `project` and a `[storage_map]` table; flags win over it
    #[arg(long, value_name = "PATH")]
    config: Option<PathBuf>,
    /// Print the full report as JSON instead of the summary
    #[arg(long)]
    json: bool,
}

fn parse_mapping(s: &str) -> Result<(String, String), String> {
    match s.split_once('=') {
        Some((old, new)) if !old.is_empty() && !new.is_empty() => {
            Ok((old.to_string(), new.to_string()))
        }
        _ => Err(format!("expected OLD=NEW, got {s:?}")),
    }
}

#[derive(Deserialize, Default)]
#[serde(deny_unknown_fields)]
struct FileSettings {
    server: Option<Url>,
    project: Option<String>,
    #[serde(default)]
    storage_map: BTreeMap<String, String>,
}

struct Settings {
    client: Client,
    dest: Destination,
    storage_map: BTreeMap<String, String>,
}

fn usage(message: &str) -> anyhow::Error {
    coded(ErrorCode::InvalidArguments, message)
}

fn settings(args: &Args) -> Result<Settings> {
    let file: FileSettings = match &args.config {
        Some(path) => {
            let text = std::fs::read_to_string(path)
                .with_context(|| format!("cannot read {}", path.display()))?;
            toml::from_str(&text).map_err(|e| {
                coded(
                    ErrorCode::InvalidConfiguration,
                    format!("cannot read {}: {e}", path.display()),
                )
            })?
        }
        None => FileSettings::default(),
    };
    let server = args
        .server
        .clone()
        .or(file.server)
        .ok_or_else(|| usage("no server: pass --server or set `server` in --config"))?;
    let project = args
        .project
        .clone()
        .or(file.project)
        .ok_or_else(|| usage("no project: pass --project or set `project` in --config"))?;
    if !args.key.starts_with("dvsm_") {
        return Err(usage(
            "not a migrate key: issue one on the project's Migration page (it starts with dvsm_)",
        ));
    }
    let mut storage_map = file.storage_map;
    storage_map.extend(args.storage_map.iter().cloned());
    Ok(Settings {
        client: Client::new(server.clone(), project.clone(), args.key.clone()),
        dest: Destination { server, project },
        storage_map,
    })
}

fn execute(command: Command, args: &Args, settings: &Settings, report: &mut Report) -> Result<()> {
    let quiet = args.json;
    let mut plan = commands::plan(&args.repo, &settings.storage_map, &settings.client, quiet)?;
    match command {
        Command::Plan => plan.fill(report, None),
        Command::Run => {
            commands::run(&mut plan, &settings.client, report, quiet)?;
            stamp(report);
            settings.client.report(report.for_server())?;
        }
    }
    Ok(())
}

static STARTED: std::sync::OnceLock<(Instant, SystemTime)> = std::sync::OnceLock::new();

fn stamp(report: &mut Report) {
    let (started, started_at) = *STARTED.get_or_init(|| (Instant::now(), SystemTime::now()));
    report.started_at = rfc3339_ms(started_at);
    report.finished_at = rfc3339_ms(SystemTime::now());
    report.duration_ms = duration_ms(started);
}

fn main() {
    STARTED.get_or_init(|| (Instant::now(), SystemTime::now()));
    git::clean_up_on_signal();
    let cli = Cli::parse();
    let (command, args) = match &cli.command {
        Cmd::Plan(args) => (Command::Plan, args),
        Cmd::Run(args) => (Command::Run, args),
    };

    let mut report = Report::new(command);
    let (result, dest) = match settings(args) {
        Ok(s) => (execute(command, args, &s, &mut report), Some(s.dest)),
        Err(e) => (Err(e), None),
    };
    if let Err(error) = &result {
        report.outcome = dvs::RunOutcome::Failed;
        report.error = Some(error_json(error));
    }
    stamp(&mut report);

    let source = git::redact(&args.repo);
    let stdout = &mut std::io::stdout().lock();
    let printed = report
        .print(args.json, &source, dest.as_ref(), stdout)
        .and_then(|()| stdout.flush());
    if let Err(e) = &printed {
        eprintln!("error: {e}");
    }
    for warning in report.warnings() {
        eprintln!("{warning}");
    }
    if command == Command::Run && result.is_ok() {
        let scope = report.scope.as_ref().expect("a run fills the scope");
        if let Some(warning) = commands::storage_writable_warning(scope) {
            eprintln!("{warning}");
        }
    }

    let code = match &result {
        Err(e) if dvs::backend_error_code(e) == Some(ErrorCode::InvalidArguments) => 2,
        Err(_) => 1,
        Ok(()) if printed.is_err() => 1,
        Ok(()) => 0,
    };
    std::process::exit(code);
}
